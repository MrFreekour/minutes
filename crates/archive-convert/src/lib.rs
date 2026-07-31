//! Bounded byte-to-text conversion for untrusted legal documents.
//!
//! Parsing functions are public for deterministic fixture tests. Production
//! callers must use the worker entry point so PDF and ZIP/XML parsing never
//! occurs in the Tauri process.

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;
use zip::ZipArchive;

pub const WORKER_MARKER: &str = "--minutes-archive-convert-worker-v1";
pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_BLOCKS: usize = 10_000;
pub const MAX_DOCX_ENTRIES: usize = 2_000;
pub const MAX_DOCX_XML_BYTES: usize = 24 * 1024 * 1024;
const WORKER_CPU_SECONDS: u64 = 15;
const WORKER_MEMORY_GROWTH_BYTES: u64 = 1024 * 1024 * 1024;
const WORKER_DEADLINE: Duration = Duration::from_secs(20);
const MAX_WORKER_STDERR_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Pdf,
    Docx,
}

impl SourceFormat {
    pub fn parse(value: &str) -> Result<Self, ConversionError> {
        match value {
            "pdf" => Ok(Self::Pdf),
            "docx" => Ok(Self::Docx),
            _ => Err(ConversionError::UnsupportedFormat),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorFlow {
    HardBoundary,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertedBlock {
    pub source_anchor: String,
    pub text: String,
    pub flow: AnchorFlow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertedDocument {
    pub format: SourceFormat,
    pub blocks: Vec<ConvertedBlock>,
    pub warnings: Vec<String>,
}

impl ConvertedDocument {
    pub fn validate(&self) -> Result<(), ConversionError> {
        if self.blocks.len() > MAX_BLOCKS {
            return Err(ConversionError::OutputBudgetExceeded);
        }
        let mut output_bytes = 0usize;
        for block in &self.blocks {
            if block.source_anchor.is_empty()
                || block.source_anchor.len() > 128
                || block
                    .source_anchor
                    .bytes()
                    .any(|byte| byte.is_ascii_control())
                || block.text.contains('\0')
            {
                return Err(ConversionError::MalformedOutput);
            }
            output_bytes = output_bytes
                .checked_add(block.text.len())
                .ok_or(ConversionError::OutputBudgetExceeded)?;
            if output_bytes > MAX_OUTPUT_BYTES {
                return Err(ConversionError::OutputBudgetExceeded);
            }
        }
        if self.warnings.len() > 32
            || self
                .warnings
                .iter()
                .any(|warning| warning.len() > 256 || warning.chars().any(char::is_control))
        {
            return Err(ConversionError::MalformedOutput);
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConversionError {
    #[error("the source format is not supported")]
    UnsupportedFormat,
    #[error("the source is empty or exceeds the input budget")]
    InputBudgetExceeded,
    #[error("the source could not be converted")]
    MalformedSource,
    #[error("the converted document exceeded its output budget")]
    OutputBudgetExceeded,
    #[error("the converter emitted malformed output")]
    MalformedOutput,
    #[error("the conversion worker could not install its security boundary")]
    SecurityBoundaryUnavailable,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorkerError {
    #[error("the conversion worker executable is unavailable or mutable")]
    ExecutableUnavailable,
    #[error("the conversion worker security self-test failed")]
    SecuritySelfTestFailed,
    #[error("the conversion worker exceeded its deadline or output budget")]
    WorkerBudgetExceeded,
    #[error("the conversion worker stopped without a valid result")]
    WorkerFailed,
    #[error("the source was refused by the bounded converter")]
    SourceRefused,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkerResponse {
    document: Option<ConvertedDocument>,
    error: Option<String>,
}

pub struct BoundedConverter {
    _snapshot_directory: tempfile::TempDir,
    executable_path: PathBuf,
    executable: fs::File,
    executable_bytes: u64,
    executable_digest: [u8; 32],
}

impl std::fmt::Debug for BoundedConverter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BoundedConverter([private immutable worker snapshot])")
    }
}

impl BoundedConverter {
    /// Path of the private worker snapshot directory.
    ///
    /// Exposed so the app can reclaim it explicitly. `exit(0)` does not
    /// unwind, and during a vault build this object lives inside a blocking
    /// task rather than in shared session state, so nothing the close handler
    /// can reach owns it and no destructor will run.
    pub fn snapshot_directory(&self) -> &Path {
        self._snapshot_directory.path()
    }

    pub fn bind(worker_executable: &Path) -> Result<Self, WorkerError> {
        let canonical =
            fs::canonicalize(worker_executable).map_err(|_| WorkerError::ExecutableUnavailable)?;
        let lexical =
            fs::symlink_metadata(&canonical).map_err(|_| WorkerError::ExecutableUnavailable)?;
        if lexical.file_type().is_symlink() || !lexical.is_file() {
            return Err(WorkerError::ExecutableUnavailable);
        }
        let mut source_options = fs::OpenOptions::new();
        source_options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            source_options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut source = source_options
            .open(&canonical)
            .map_err(|_| WorkerError::ExecutableUnavailable)?;
        let source_metadata = source
            .metadata()
            .map_err(|_| WorkerError::ExecutableUnavailable)?;
        if !source_metadata.is_file() {
            return Err(WorkerError::ExecutableUnavailable);
        }

        let snapshot_directory = tempfile::Builder::new()
            .prefix("minutes-archive-worker-")
            .tempdir()
            .map_err(|_| WorkerError::ExecutableUnavailable)?;
        #[cfg(unix)]
        fs::set_permissions(
            snapshot_directory.path(),
            <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .map_err(|_| WorkerError::ExecutableUnavailable)?;
        let executable_path = snapshot_directory.path().join("worker");
        let mut snapshot_options = fs::OpenOptions::new();
        snapshot_options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            snapshot_options.mode(0o500);
        }
        let mut snapshot = snapshot_options
            .open(&executable_path)
            .map_err(|_| WorkerError::ExecutableUnavailable)?;
        std::io::copy(&mut source, &mut snapshot)
            .map_err(|_| WorkerError::ExecutableUnavailable)?;
        snapshot
            .sync_all()
            .map_err(|_| WorkerError::ExecutableUnavailable)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            snapshot
                .set_permissions(fs::Permissions::from_mode(0o500))
                .map_err(|_| WorkerError::ExecutableUnavailable)?;
        }
        drop(snapshot);
        let executable =
            fs::File::open(&executable_path).map_err(|_| WorkerError::ExecutableUnavailable)?;
        let (executable_bytes, executable_digest) =
            digest_file(&executable).map_err(|_| WorkerError::ExecutableUnavailable)?;
        if executable_bytes != source_metadata.len() {
            return Err(WorkerError::ExecutableUnavailable);
        }
        let converter = Self {
            _snapshot_directory: snapshot_directory,
            executable_path,
            executable,
            executable_bytes,
            executable_digest,
        };
        converter.verify_sandbox()?;
        Ok(converter)
    }

    pub fn convert(
        &self,
        format: SourceFormat,
        source: &[u8],
    ) -> Result<ConvertedDocument, WorkerError> {
        if source.is_empty() || source.len() > MAX_SOURCE_BYTES {
            return Err(WorkerError::SourceRefused);
        }
        self.verify_executable()?;
        let mut input = Vec::with_capacity(8 + source.len());
        input.extend_from_slice(&(source.len() as u64).to_le_bytes());
        input.extend_from_slice(source);
        let output = self.launch(format.as_str(), input)?;
        if !output.success {
            return Err(WorkerError::SourceRefused);
        }
        let response: WorkerResponse =
            serde_json::from_slice(&output.stdout).map_err(|_| WorkerError::WorkerFailed)?;
        let document = response.document.ok_or(WorkerError::SourceRefused)?;
        if response.error.is_some() || document.format != format {
            return Err(WorkerError::WorkerFailed);
        }
        document.validate().map_err(|_| WorkerError::WorkerFailed)?;
        Ok(document)
    }

    fn verify_sandbox(&self) -> Result<(), WorkerError> {
        self.verify_executable()?;
        let output = self.launch("sandbox-self-test", Vec::new())?;
        if output.success {
            Ok(())
        } else {
            Err(WorkerError::SecuritySelfTestFailed)
        }
    }

    fn verify_executable(&self) -> Result<(), WorkerError> {
        let metadata = fs::symlink_metadata(&self.executable_path)
            .map_err(|_| WorkerError::ExecutableUnavailable)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(WorkerError::ExecutableUnavailable);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o222 != 0 {
                return Err(WorkerError::ExecutableUnavailable);
            }
        }
        let (bytes, digest) =
            digest_file(&self.executable).map_err(|_| WorkerError::ExecutableUnavailable)?;
        if bytes != self.executable_bytes || digest != self.executable_digest {
            return Err(WorkerError::ExecutableUnavailable);
        }
        Ok(())
    }

    fn launch(&self, operation: &str, input: Vec<u8>) -> Result<WorkerOutput, WorkerError> {
        let mut command = Command::new(&self.executable_path);
        command
            .arg(WORKER_MARKER)
            .arg(operation)
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                command.pre_exec(|| {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let mut child = command.spawn().map_err(|_| WorkerError::WorkerFailed)?;
        let mut stdin = child.stdin.take().ok_or(WorkerError::WorkerFailed)?;
        let stdout = child.stdout.take().ok_or(WorkerError::WorkerFailed)?;
        let stderr = child.stderr.take().ok_or(WorkerError::WorkerFailed)?;
        let input_writer = thread::spawn(move || {
            let result = stdin.write_all(&input).and_then(|_| stdin.flush());
            drop(stdin);
            result
        });
        let stdout_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take((MAX_OUTPUT_BYTES as u64).saturating_add(1))
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let stderr_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr
                .take((MAX_WORKER_STDERR_BYTES as u64).saturating_add(1))
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });

        let deadline = Instant::now() + WORKER_DEADLINE;
        let exit_status = loop {
            match child.try_wait().map_err(|_| WorkerError::WorkerFailed)? {
                Some(exit_status) => break exit_status,
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                None => {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(WorkerError::WorkerBudgetExceeded);
                }
            }
        };
        input_writer
            .join()
            .map_err(|_| WorkerError::WorkerFailed)?
            .map_err(|_| WorkerError::WorkerFailed)?;
        let stdout = stdout_reader
            .join()
            .map_err(|_| WorkerError::WorkerFailed)?
            .map_err(|_| WorkerError::WorkerFailed)?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| WorkerError::WorkerFailed)?
            .map_err(|_| WorkerError::WorkerFailed)?;
        if stdout.len() > MAX_OUTPUT_BYTES || stderr.len() > MAX_WORKER_STDERR_BYTES {
            return Err(WorkerError::WorkerBudgetExceeded);
        }
        Ok(WorkerOutput {
            success: exit_status.success(),
            stdout,
        })
    }
}

#[derive(Debug)]
struct WorkerOutput {
    success: bool,
    stdout: Vec<u8>,
}

fn digest_file(file: &fs::File) -> Result<(u64, [u8; 32]), std::io::Error> {
    use std::io::{Seek, SeekFrom};

    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let bytes = std::io::copy(&mut file, &mut hasher)?;
    Ok((bytes, hasher.finalize().into()))
}

pub fn convert_bytes(
    format: SourceFormat,
    bytes: &[u8],
) -> Result<ConvertedDocument, ConversionError> {
    if bytes.is_empty() || bytes.len() > MAX_SOURCE_BYTES {
        return Err(ConversionError::InputBudgetExceeded);
    }
    let document = match format {
        SourceFormat::Pdf => convert_pdf(bytes)?,
        SourceFormat::Docx => convert_docx(bytes)?,
    };
    document.validate()?;
    Ok(document)
}

fn convert_pdf(bytes: &[u8]) -> Result<ConvertedDocument, ConversionError> {
    let pages = pdf_extract::extract_text_from_mem_by_pages(bytes)
        .map_err(|_| ConversionError::MalformedSource)?;
    if pages.len() > MAX_BLOCKS {
        return Err(ConversionError::OutputBudgetExceeded);
    }
    let mut blocks = Vec::new();
    let mut output_bytes = 0usize;
    for (index, page) in pages.into_iter().enumerate() {
        let text = normalize_extracted_text(&page);
        if text.is_empty() {
            continue;
        }
        output_bytes = output_bytes
            .checked_add(text.len())
            .ok_or(ConversionError::OutputBudgetExceeded)?;
        if output_bytes > MAX_OUTPUT_BYTES {
            return Err(ConversionError::OutputBudgetExceeded);
        }
        blocks.push(ConvertedBlock {
            source_anchor: format!("page:{:04}", index + 1),
            text,
            flow: AnchorFlow::HardBoundary,
        });
    }
    let warnings = if blocks.is_empty() {
        vec!["ocr_required_or_no_extractable_text".to_string()]
    } else {
        Vec::new()
    };
    Ok(ConvertedDocument {
        format: SourceFormat::Pdf,
        blocks,
        warnings,
    })
}

fn convert_docx(bytes: &[u8]) -> Result<ConvertedDocument, ConversionError> {
    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor).map_err(|_| ConversionError::MalformedSource)?;
    if archive.len() > MAX_DOCX_ENTRIES {
        return Err(ConversionError::InputBudgetExceeded);
    }
    if archive.decompressed_size().is_some_and(|size| {
        size > MAX_OUTPUT_BYTES as u128 || size > MAX_DOCX_XML_BYTES as u128 * 4
    }) {
        return Err(ConversionError::OutputBudgetExceeded);
    }
    if archive
        .has_overlapping_files()
        .map_err(|_| ConversionError::MalformedSource)?
    {
        return Err(ConversionError::MalformedSource);
    }
    let document_xml = archive
        .by_name("word/document.xml")
        .map_err(|_| ConversionError::MalformedSource)?;
    if document_xml.size() > MAX_DOCX_XML_BYTES as u64 {
        return Err(ConversionError::OutputBudgetExceeded);
    }
    let mut xml = Vec::new();
    document_xml
        .take((MAX_DOCX_XML_BYTES as u64).saturating_add(1))
        .read_to_end(&mut xml)
        .map_err(|_| ConversionError::MalformedSource)?;
    if xml.len() > MAX_DOCX_XML_BYTES {
        return Err(ConversionError::OutputBudgetExceeded);
    }
    docx_paragraphs(&xml)
}

fn docx_paragraphs(xml: &[u8]) -> Result<ConvertedDocument, ConversionError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut paragraphs = Vec::new();
    let mut paragraph = String::new();
    let mut paragraph_ordinal: usize = 0;
    let mut in_text = false;
    let mut output_bytes = 0usize;

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => {
                let name = event.name();
                if local_name(name.as_ref()) == b"t" {
                    in_text = true;
                }
            }
            Ok(Event::Empty(event)) => match local_name(event.name().as_ref()) {
                b"tab" => paragraph.push('\t'),
                b"br" | b"cr" => paragraph.push('\n'),
                // `<w:p/>` is a self-closing empty paragraph and arrives as
                // Empty rather than Start/End. Word emits these constantly as
                // spacers, and each one still occupies a paragraph position
                // in the document a reader is asked to navigate to.
                b"p" => paragraph_ordinal += 1,
                _ => {}
            },
            Ok(Event::Text(event)) if in_text => {
                let decoded = event
                    .decode()
                    .map_err(|_| ConversionError::MalformedSource)?;
                paragraph.push_str(&decoded);
            }
            Ok(Event::GeneralRef(reference)) if in_text => {
                if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|_| ConversionError::MalformedSource)?
                {
                    paragraph.push(character);
                } else {
                    let name = reference
                        .decode()
                        .map_err(|_| ConversionError::MalformedSource)?;
                    let value = quick_xml::escape::resolve_xml_entity(&name)
                        .ok_or(ConversionError::MalformedSource)?;
                    paragraph.push_str(value);
                }
            }
            Ok(Event::End(event)) => match local_name(event.name().as_ref()) {
                b"t" => in_text = false,
                b"p" => {
                    // Count every <w:p> element, including empty spacers and
                    // paragraphs inside tables. The anchor previously used
                    // the number of paragraphs emitted so far, so any dropped
                    // empty paragraph shifted it: "paragraph:000003" did not
                    // locate the third paragraph in Word, and the drift grew
                    // monotonically through the document. A lawyer asked to
                    // verify a quote at that anchor lands somewhere else.
                    paragraph_ordinal += 1;
                    let text = normalize_extracted_text(&paragraph);
                    paragraph.clear();
                    if !text.is_empty() {
                        output_bytes = output_bytes
                            .checked_add(text.len())
                            .ok_or(ConversionError::OutputBudgetExceeded)?;
                        if output_bytes > MAX_OUTPUT_BYTES || paragraphs.len() >= MAX_BLOCKS {
                            return Err(ConversionError::OutputBudgetExceeded);
                        }
                        paragraphs.push(ConvertedBlock {
                            source_anchor: format!("paragraph:{paragraph_ordinal:06}"),
                            text,
                            flow: AnchorFlow::Continue,
                        });
                    }
                }
                _ => {}
            },
            Ok(Event::DocType(_)) => return Err(ConversionError::MalformedSource),
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(ConversionError::MalformedSource),
        }
        buffer.clear();
    }
    Ok(ConvertedDocument {
        format: SourceFormat::Docx,
        blocks: paragraphs,
        warnings: Vec::new(),
    })
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn normalize_extracted_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

pub fn run_worker_process(format: &str) -> i32 {
    if install_worker_security_boundary().is_err() {
        return 70;
    }
    if format == "sandbox-self-test" {
        return sandbox_self_test();
    }
    let format = match SourceFormat::parse(format) {
        Ok(format) => format,
        Err(_) => return 64,
    };
    let response = std::panic::catch_unwind(|| {
        let mut stdin = std::io::stdin().lock();
        let bytes = read_worker_input(&mut stdin)?;
        convert_bytes(format, &bytes)
    });
    let response = match response {
        Ok(Ok(document)) => WorkerResponse {
            document: Some(document),
            error: None,
        },
        Ok(Err(error)) => WorkerResponse {
            document: None,
            error: Some(error.to_string()),
        },
        Err(_) => WorkerResponse {
            document: None,
            error: Some("the source could not be converted".to_string()),
        },
    };
    let output = match serde_json::to_vec(&response) {
        Ok(output) if output.len() <= MAX_OUTPUT_BYTES => output,
        _ => return 74,
    };
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(&output).is_err() || stdout.flush().is_err() {
        return 74;
    }
    if response.document.is_some() {
        0
    } else {
        65
    }
}

fn sandbox_self_test() -> i32 {
    let network_denied = std::net::TcpListener::bind("127.0.0.1:0").is_err();
    let filesystem_denied = std::fs::read("/etc/passwd").is_err();
    if network_denied && filesystem_denied {
        0
    } else {
        71
    }
}

fn read_worker_input(reader: &mut impl Read) -> Result<Vec<u8>, ConversionError> {
    let mut length_bytes = [0u8; 8];
    reader
        .read_exact(&mut length_bytes)
        .map_err(|_| ConversionError::MalformedSource)?;
    let length = usize::try_from(u64::from_le_bytes(length_bytes))
        .map_err(|_| ConversionError::InputBudgetExceeded)?;
    if length == 0 || length > MAX_SOURCE_BYTES {
        return Err(ConversionError::InputBudgetExceeded);
    }
    let mut bytes = vec![0u8; length];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| ConversionError::MalformedSource)?;
    let mut trailing = [0u8; 1];
    if reader
        .read(&mut trailing)
        .map_err(|_| ConversionError::MalformedSource)?
        != 0
    {
        return Err(ConversionError::MalformedSource);
    }
    Ok(bytes)
}

fn install_worker_security_boundary() -> Result<(), ConversionError> {
    install_resource_limits()?;
    install_platform_sandbox()
}

#[cfg(unix)]
fn install_resource_limits() -> Result<(), ConversionError> {
    let cpu = libc::rlimit {
        rlim_cur: WORKER_CPU_SECONDS,
        rlim_max: WORKER_CPU_SECONDS,
    };
    let file_size = libc::rlimit {
        rlim_cur: MAX_OUTPUT_BYTES as u64,
        rlim_max: MAX_OUTPUT_BYTES as u64,
    };
    let open_files = libc::rlimit {
        rlim_cur: 16,
        rlim_max: 16,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_CPU, &cpu) } != 0
        || unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &file_size) } != 0
        || unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &open_files) } != 0
    {
        return Err(ConversionError::SecurityBoundaryUnavailable);
    }
    install_address_space_limit()
}

#[cfg(not(unix))]
fn install_resource_limits() -> Result<(), ConversionError> {
    Err(ConversionError::SecurityBoundaryUnavailable)
}

#[cfg(target_os = "macos")]
fn install_address_space_limit() -> Result<(), ConversionError> {
    use mach2::kern_return::KERN_SUCCESS;
    use mach2::task::task_info;
    use mach2::task_info::{
        task_basic_info_64, task_info_t, TASK_BASIC_INFO_64, TASK_BASIC_INFO_64_COUNT,
    };
    use mach2::traps::mach_task_self;

    let mut info = task_basic_info_64::default();
    let mut count = TASK_BASIC_INFO_64_COUNT;
    let status = unsafe {
        task_info(
            mach_task_self(),
            TASK_BASIC_INFO_64,
            (&mut info as *mut task_basic_info_64).cast::<libc::c_int>() as task_info_t,
            &mut count,
        )
    };
    if status != KERN_SUCCESS || count != TASK_BASIC_INFO_64_COUNT {
        return Err(ConversionError::SecurityBoundaryUnavailable);
    }
    let limit = info
        .virtual_size
        .checked_add(WORKER_MEMORY_GROWTH_BYTES)
        .ok_or(ConversionError::SecurityBoundaryUnavailable)?;
    let address_space = libc::rlimit {
        rlim_cur: limit,
        rlim_max: limit,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &address_space) } != 0 {
        return Err(ConversionError::SecurityBoundaryUnavailable);
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn install_address_space_limit() -> Result<(), ConversionError> {
    let address_space = libc::rlimit {
        rlim_cur: 2 * 1024 * 1024 * 1024,
        rlim_max: 2 * 1024 * 1024 * 1024,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &address_space) } != 0 {
        return Err(ConversionError::SecurityBoundaryUnavailable);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_platform_sandbox() -> Result<(), ConversionError> {
    use std::ffi::{c_char, c_int, CStr};
    use std::ptr;

    #[link(name = "System")]
    unsafe extern "C" {
        fn sandbox_init(
            profile: *const c_char,
            flags: u64,
            error_buffer: *mut *mut c_char,
        ) -> c_int;
        fn sandbox_free_error(error_buffer: *mut c_char);
    }

    const PROFILE: &CStr = c"(version 1)
(deny default)
(allow process-info*)
(allow sysctl-read)
(allow file-read-data (subpath \"/dev/fd\"))
(allow file-write-data (subpath \"/dev/fd\"))
";
    let mut error_buffer = ptr::null_mut();
    let status = unsafe { sandbox_init(PROFILE.as_ptr(), 0, &mut error_buffer) };
    if !error_buffer.is_null() {
        unsafe { sandbox_free_error(error_buffer) };
    }
    if status != 0 {
        return Err(ConversionError::SecurityBoundaryUnavailable);
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn install_platform_sandbox() -> Result<(), ConversionError> {
    Err(ConversionError::SecurityBoundaryUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn synthetic_docx(document_xml: &str) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut cursor);
            writer
                .start_file(
                    "word/document.xml",
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .expect("document entry");
            writer.write_all(document_xml.as_bytes()).expect("xml");
            writer.finish().expect("zip");
        }
        cursor.seek(SeekFrom::Start(0)).expect("rewind");
        cursor.into_inner()
    }

    fn synthetic_pdf() -> Vec<u8> {
        let stream = b"BT /F1 12 Tf 72 720 Td (7. CONFIDENTIALITY) Tj 0 -20 Td (Confidential Information includes affiliate data.) Tj ET";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes(),
                stream.to_vec(),
                b"\nendstream".to_vec(),
            ]
            .concat(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn docx_conversion_preserves_paragraph_anchors_and_text() {
        let bytes = synthetic_docx(
            r#"<w:document xmlns:w="urn:test"><w:body>
            <w:p><w:r><w:t>7. CONFIDENTIALITY</w:t></w:r></w:p>
            <w:p><w:r><w:t>Confidential Information &amp; affiliate data.</w:t></w:r></w:p>
            </w:body></w:document>"#,
        );
        let document = convert_bytes(SourceFormat::Docx, &bytes).expect("convert");
        assert_eq!(document.blocks.len(), 2);
        assert_eq!(document.blocks[0].source_anchor, "paragraph:000001");
        assert_eq!(
            document.blocks[1].text,
            "Confidential Information & affiliate data."
        );
    }

    #[test]
    fn docx_paragraph_anchors_survive_empty_spacers_and_table_cells() {
        // Word documents routinely carry empty spacer paragraphs and
        // paragraphs inside tables. Anchoring on the count of paragraphs
        // *emitted* meant every skipped empty paragraph shifted the anchor,
        // so a lawyer told "paragraph 3" and asked to verify the quote in
        // Word landed somewhere else, with the drift growing through the
        // document.
        let bytes = synthetic_docx(
            r#"<w:document xmlns:w="urn:test"><w:body>
            <w:p><w:r><w:t>Recitals paragraph one.</w:t></w:r></w:p>
            <w:p/>
            <w:p><w:r><w:t>   </w:t></w:r></w:p>
            <w:p/>
            <w:p><w:r><w:t>Seller shall indemnify and hold harmless the Buyer.</w:t></w:r></w:p>
            </w:body></w:document>"#,
        );
        let document = convert_bytes(SourceFormat::Docx, &bytes).expect("convert");
        assert_eq!(document.blocks.len(), 2);
        assert_eq!(document.blocks[0].source_anchor, "paragraph:000001");
        // Fifth <w:p> in the file, not the second one emitted.
        assert_eq!(
            document.blocks[1].source_anchor, "paragraph:000005",
            "anchor must name the paragraph's position in the document, got {}",
            document.blocks[1].source_anchor
        );
        assert_eq!(
            document.blocks[1].text,
            "Seller shall indemnify and hold harmless the Buyer."
        );
    }

    #[test]
    fn docx_doctype_and_input_budgets_fail_closed() {
        let malicious = synthetic_docx(
            r#"<!DOCTYPE x [<!ENTITY e SYSTEM "file:///etc/passwd">]>
            <w:document xmlns:w="urn:test"><w:p><w:r><w:t>&e;</w:t></w:r></w:p></w:document>"#,
        );
        assert_eq!(
            convert_bytes(SourceFormat::Docx, &malicious),
            Err(ConversionError::MalformedSource)
        );
        assert_eq!(
            convert_bytes(SourceFormat::Docx, &[]),
            Err(ConversionError::InputBudgetExceeded)
        );
    }

    #[test]
    fn pdf_conversion_preserves_page_anchors() {
        let document = convert_bytes(SourceFormat::Pdf, &synthetic_pdf()).expect("convert");
        assert_eq!(document.blocks.len(), 1);
        assert_eq!(document.blocks[0].source_anchor, "page:0001");
        assert!(document.blocks[0].text.contains("CONFIDENTIALITY"));
        assert!(document.blocks[0].text.contains("affiliate data"));
    }

    #[test]
    fn converted_output_validation_rejects_control_anchors() {
        let document = ConvertedDocument {
            format: SourceFormat::Pdf,
            blocks: vec![ConvertedBlock {
                source_anchor: "page:\n1".to_string(),
                text: "Evidence".to_string(),
                flow: AnchorFlow::HardBoundary,
            }],
            warnings: Vec::new(),
        };
        assert_eq!(document.validate(), Err(ConversionError::MalformedOutput));
    }
}
