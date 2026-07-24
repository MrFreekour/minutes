//! Supervised worker boundary for policy-filtered graph projections.
//!
//! SQLite's page/cache/value limits bound important components, but they do
//! not cap all transient allocator use. Product-facing graph reads therefore
//! execute in a short-lived worker with an OS address-space ceiling, a bounded
//! authenticated stream, and one ordered corpus + correction snapshot. macOS
//! uses an XPC peer code-signing requirement before any private frame; other
//! platforms use the bounded child runner. The parent exposes a response only
//! after the journal's post-publication fence succeeds.

use crate::config::Config;
use crate::graph::{
    PolicyGraphSnapshotPayload, PolicyGraphSpeakerCorrection, PolicyGraphStreamSource,
    PolicyProjectionRequest, PolicyProjectionResponse,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[cfg(not(target_os = "macos"))]
const WORKER_MARKER: &str = "MINUTES_POLICY_GRAPH_WORKER_V1";
const WORKER_SCHEMA_VERSION: u32 = 2;
const WORKER_STREAM_MAGIC: &[u8; 8] = b"MGRPHV2\0";
const FRAME_REQUEST: u8 = 1;
const FRAME_VOCABULARY: u8 = 2;
const FRAME_SOURCE: u8 = 3;
const FRAME_END: u8 = 255;
const FRAME_HEADER_BYTES: usize = 12;
const MAX_WORKER_REQUEST_BYTES: usize = 256 * 1024;
const MAX_WORKER_VOCABULARY_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORKER_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORKER_SOURCE_FRAME_BYTES: usize = 32 * 1024 * 1024;
const MAX_WORKER_SOURCE_COUNT: usize = 4_096;
const MAX_WORKER_CORRECTION_COUNT: usize = 10_000;
const MAX_WORKER_CORRECTION_FIELD_BYTES: usize = 64 * 1024;
pub(crate) const MAX_WORKER_INPUT_BYTES: u64 = 112 * 1024 * 1024;
const MAX_WORKER_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;
#[cfg(not(target_os = "macos"))]
const MAX_WORKER_STDERR_BYTES: usize = 64 * 1024;
const WORKER_WALL_CLOCK: Duration = Duration::from_secs(45);
const WORKER_ADDRESS_SPACE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_WORKER_RESULT_ITEMS: usize = 10_000;
const MAX_WORKER_RESPONSE_STRUCTURAL_UNITS: usize = 1_000_000;
const MAX_WORKER_RESPONSE_STRING_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORKER_RESPONSE_SINGLE_STRING_BYTES: usize = 256 * 1024;
const MAX_WORKER_RESPONSE_DEPTH: usize = 64;
#[cfg(not(target_os = "macos"))]
static WORKER_EXECUTABLE: OnceLock<crate::bounded_child::BoundExecutable> = OnceLock::new();
#[cfg(target_os = "macos")]
static WORKER_EXECUTABLE: OnceLock<MacWorkerAuthority> = OnceLock::new();
#[cfg(target_os = "macos")]
const EMBEDDED_GRAPH_WORKER_CDHASH_PREFIX: &[u8] = b"MINUTES_GRAPH_WORKER_CDHASH_V1=";
#[cfg(target_os = "macos")]
#[used]
static EMBEDDED_GRAPH_WORKER_CDHASH: &[u8] =
    b"MINUTES_GRAPH_WORKER_CDHASH_V1=0000000000000000000000000000000000000000";

#[cfg(target_os = "macos")]
#[derive(Clone)]
struct MacWorkerAuthority {
    bundle: PathBuf,
    cdhash: [u8; 20],
    trusted_distribution: bool,
}

#[cfg(target_os = "macos")]
enum MacWorkerTransport {
    AuthenticatedXpc(MacWorkerAuthority),
    /// macOS 11 and source/ad-hoc/default-mode builds retain product parity by
    /// projecting the already policy-filtered normal corpus in process. This
    /// is an explicit compatibility fallback, not an atomic helper claim.
    InProcessFallback,
}

#[cfg(any(test, target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacWorkerTransportChoice {
    InProcessFallback,
    CachedAuthority,
    EmbeddedService,
    MissingService,
}

#[cfg(any(test, target_os = "macos"))]
fn choose_macos_worker_transport(
    app_managed: bool,
    trusted_distribution: bool,
    peer_requirement_available: bool,
    cached_authority: bool,
    embedded_service_present: bool,
) -> MacWorkerTransportChoice {
    if !app_managed || !trusted_distribution || !peer_requirement_available {
        MacWorkerTransportChoice::InProcessFallback
    } else if cached_authority {
        MacWorkerTransportChoice::CachedAuthority
    } else if embedded_service_present {
        MacWorkerTransportChoice::EmbeddedService
    } else {
        MacWorkerTransportChoice::MissingService
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkerRequestFrame {
    schema_version: u32,
    request_nonce: [u8; 16],
    request: PolicyProjectionRequest,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkerResponseEnvelope {
    schema_version: u32,
    request_nonce: [u8; 16],
    manifest_sha256: [u8; 32],
    response: PolicyProjectionResponse,
}

struct WorkerInput {
    request: WorkerRequestFrame,
    vocabulary_people: Vec<crate::markdown::EntityRef>,
    sources: Vec<PolicyGraphStreamSource>,
    correction_revision: String,
    manifest_sha256: [u8; 32],
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkerVocabularyFrame {
    correction_revision: String,
    people: Vec<crate::markdown::EntityRef>,
}

struct WorkerStreamReceipt {
    request_nonce: [u8; 16],
    manifest_sha256: Arc<Mutex<Option<[u8; 32]>>>,
}

fn valid_absolute_executable_path(path: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

/// Install a dedicated, already-packaged worker authority before application
/// threads start. On macOS `path` is the separately signed XPC service bundle;
/// on other platforms it remains the bounded helper executable.
pub fn install_policy_projection_worker_executable(path: PathBuf) -> Result<(), String> {
    if !valid_absolute_executable_path(&path) {
        return Err("policy graph worker executable path was invalid".into());
    }
    #[cfg(target_os = "macos")]
    {
        if !crate::macos_graph_xpc::current_process_is_trusted_distribution() {
            return Ok(());
        }
        let authority = bind_macos_worker_authority(path)?;
        WORKER_EXECUTABLE
            .set(authority)
            .map_err(|_| "policy graph worker executable was already installed".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let executable = crate::bounded_child::BoundExecutable::bind(&path).map_err(|_| {
            "policy graph worker executable authority could not be bound".to_string()
        })?;
        WORKER_EXECUTABLE
            .set(executable)
            .map_err(|_| "policy graph worker executable was already installed".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
fn resolve_policy_projection_worker_executable(
) -> Result<crate::bounded_child::BoundExecutable, String> {
    if let Some(executable) = WORKER_EXECUTABLE.get() {
        return executable.try_clone().map_err(|_| {
            "policy graph worker executable authority could not be cloned".to_string()
        });
    }

    let helper_name = format!("minutes-graph-worker{}", std::env::consts::EXE_SUFFIX);
    let helper = std::env::current_exe().ok().and_then(|executable| {
        let parent = executable.parent()?;
        [
            parent.join(&helper_name),
            parent.parent()?.join(&helper_name),
        ]
        .into_iter()
        .find(|candidate| candidate.is_file())
    });

    if let Some(helper) = helper {
        return crate::bounded_child::BoundExecutable::bind(&helper).map_err(|_| {
            "policy graph worker executable authority could not be bound".to_string()
        });
    }
    crate::bounded_child::BoundExecutable::current()
        .map_err(|_| "policy graph worker executable could not be resolved".to_string())
}

#[cfg(target_os = "macos")]
fn resolve_policy_projection_worker_transport() -> Result<MacWorkerTransport, String> {
    let current = std::env::current_exe()
        .map_err(|_| "policy graph executable identity was unavailable".to_string())?;
    let Some(parent) = current.parent() else {
        return Err("policy graph executable package was invalid".into());
    };
    let contents = parent
        .parent()
        .filter(|contents| contents.file_name().is_some_and(|name| name == "Contents"));
    let app_managed = contents.is_some();
    let trusted_distribution = crate::macos_graph_xpc::current_process_is_trusted_distribution();
    let peer_requirement_available = crate::macos_graph_xpc::peer_requirement_api_available();
    let service = contents.map(|contents| {
        contents
            .join("XPCServices")
            .join("com.useminutes.graph-worker.xpc")
    });
    let choice = choose_macos_worker_transport(
        app_managed,
        trusted_distribution,
        peer_requirement_available,
        WORKER_EXECUTABLE.get().is_some(),
        service.as_ref().is_some_and(|service| service.is_dir()),
    );
    match choice {
        MacWorkerTransportChoice::InProcessFallback => Ok(MacWorkerTransport::InProcessFallback),
        MacWorkerTransportChoice::CachedAuthority => Ok(MacWorkerTransport::AuthenticatedXpc(
            WORKER_EXECUTABLE
                .get()
                .expect("transport choice proved a cached authority")
                .clone(),
        )),
        MacWorkerTransportChoice::EmbeddedService => bind_macos_worker_authority(
            service.expect("transport choice proved an embedded service"),
        )
        .map(MacWorkerTransport::AuthenticatedXpc),
        MacWorkerTransportChoice::MissingService => {
            Err("policy graph XPC service is unavailable in this app installation".into())
        }
    }
}

#[cfg(target_os = "macos")]
fn bind_macos_worker_authority(service_bundle: PathBuf) -> Result<MacWorkerAuthority, String> {
    if service_bundle.extension().and_then(|value| value.to_str()) != Some("xpc") {
        return Err("policy graph worker authority was not an XPC service".into());
    }
    let service_contents = service_bundle.join("Contents");
    let executable = service_contents.join("MacOS").join("minutes-graph-worker");
    if !executable.is_file() {
        return Err("policy graph XPC executable was unavailable".into());
    }
    let bundle = service_bundle
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "XPCServices"))
        .and_then(Path::parent)
        .filter(|path| path.file_name().is_some_and(|name| name == "Contents"))
        .and_then(Path::parent)
        .ok_or_else(|| "policy graph XPC service was not inside an application".to_string())?
        .to_path_buf();
    let manifest = bundle
        .join("Contents")
        .join("Resources")
        .join("minutes-graph-worker.cdhash");
    let encoded = std::fs::read_to_string(&manifest)
        .map_err(|_| "policy graph helper integrity manifest was unavailable".to_string())?;
    let encoded = encoded
        .strip_suffix('\n')
        .ok_or_else(|| "policy graph helper integrity manifest was malformed".to_string())?;
    let embedded = EMBEDDED_GRAPH_WORKER_CDHASH
        .strip_prefix(EMBEDDED_GRAPH_WORKER_CDHASH_PREFIX)
        .ok_or_else(|| "policy graph helper executable authority was malformed".to_string())?;
    let embedded_cdhash = verify_macos_graph_worker_cdhash_binding(encoded.as_bytes(), embedded)?;
    Ok(MacWorkerAuthority {
        bundle,
        cdhash: embedded_cdhash,
        trusted_distribution: crate::macos_graph_xpc::current_process_is_trusted_distribution(),
    })
}

#[cfg(target_os = "macos")]
fn verify_macos_graph_worker_cdhash_binding(
    manifest: &[u8],
    embedded: &[u8],
) -> Result<[u8; 20], String> {
    let manifest_cdhash = decode_macos_graph_worker_cdhash(
        manifest,
        "policy graph helper integrity manifest was malformed",
    )?;
    let embedded_cdhash = decode_macos_graph_worker_cdhash(
        embedded,
        "policy graph helper executable authority was unavailable",
    )?;
    if manifest_cdhash != embedded_cdhash {
        Err("policy graph helper authority did not match this application".into())
    } else {
        Ok(embedded_cdhash)
    }
}

#[cfg(target_os = "macos")]
fn decode_macos_graph_worker_cdhash(
    encoded: &[u8],
    malformed_message: &'static str,
) -> Result<[u8; 20], String> {
    if encoded.len() != 40
        || encoded.iter().all(|byte| *byte == b'0')
        || !encoded
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(malformed_message.into());
    }
    let mut cdhash = [0_u8; 20];
    for (index, byte) in cdhash.iter_mut().enumerate() {
        let encoded = std::str::from_utf8(&encoded[index * 2..index * 2 + 2])
            .map_err(|_| malformed_message.to_string())?;
        *byte = u8::from_str_radix(encoded, 16).map_err(|_| malformed_message.to_string())?;
    }
    Ok(cdhash)
}

fn frame_header(tag: u8, payload_len: usize) -> Result<[u8; FRAME_HEADER_BYTES], String> {
    let payload_len = u64::try_from(payload_len)
        .map_err(|_| "policy graph frame length exceeded this platform".to_string())?;
    let mut header = [0_u8; FRAME_HEADER_BYTES];
    header[0] = tag;
    header[4..12].copy_from_slice(&payload_len.to_le_bytes());
    Ok(header)
}

fn encode_frame(tag: u8, payload: Vec<u8>) -> Result<Vec<u8>, String> {
    let header = frame_header(tag, payload.len())?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
    frame.extend_from_slice(&header);
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn checked_u32_bytes(value: &str, label: &str) -> Result<[u8; 4], String> {
    let len = u32::try_from(value.len())
        .map_err(|_| format!("policy graph {label} exceeded its byte budget"))?;
    Ok(len.to_le_bytes())
}

fn encode_source_payload(source: &PolicyGraphStreamSource) -> Result<Vec<u8>, String> {
    let path = source
        .opaque_path
        .to_str()
        .ok_or_else(|| "policy graph source identifier was not UTF-8".to_string())?;
    if path.len() > MAX_WORKER_CORRECTION_FIELD_BYTES
        || source.content.len() > MAX_WORKER_SOURCE_BYTES
        || source.speaker_corrections.len() > MAX_WORKER_CORRECTION_COUNT
    {
        return Err("policy graph source frame exceeded its resource budget".into());
    }
    let mut total = 4usize
        .checked_add(path.len())
        .and_then(|value| value.checked_add(32 + 8 + 4))
        .and_then(|value| value.checked_add(source.content.len()))
        .ok_or_else(|| "policy graph source frame length overflowed".to_string())?;
    for correction in &source.speaker_corrections {
        if correction.speaker_label.len() > MAX_WORKER_CORRECTION_FIELD_BYTES
            || correction.name.len() > MAX_WORKER_CORRECTION_FIELD_BYTES
        {
            return Err("policy graph correction field exceeded its byte budget".into());
        }
        total = total
            .checked_add(8)
            .and_then(|value| value.checked_add(correction.speaker_label.len()))
            .and_then(|value| value.checked_add(correction.name.len()))
            .ok_or_else(|| "policy graph source frame length overflowed".to_string())?;
    }
    if total > MAX_WORKER_SOURCE_FRAME_BYTES {
        return Err("policy graph source frame exceeded its byte budget".into());
    }
    let mut payload = Vec::with_capacity(total);
    payload.extend_from_slice(&checked_u32_bytes(path, "source identifier")?);
    payload.extend_from_slice(path.as_bytes());
    payload.extend_from_slice(&source.content_sha256);
    payload.extend_from_slice(&(source.content.len() as u64).to_le_bytes());
    payload.extend_from_slice(
        &u32::try_from(source.speaker_corrections.len())
            .map_err(|_| "policy graph correction count overflowed".to_string())?
            .to_le_bytes(),
    );
    payload.extend_from_slice(source.content.as_bytes());
    for correction in &source.speaker_corrections {
        payload.extend_from_slice(&checked_u32_bytes(
            &correction.speaker_label,
            "speaker label",
        )?);
        payload.extend_from_slice(correction.speaker_label.as_bytes());
        payload.extend_from_slice(&checked_u32_bytes(&correction.name, "speaker name")?);
        payload.extend_from_slice(correction.name.as_bytes());
    }
    Ok(payload)
}

struct FramedSnapshotReader {
    pending: VecDeque<Vec<u8>>,
    sources: std::vec::IntoIter<PolicyGraphStreamSource>,
    current: std::io::Cursor<Vec<u8>>,
    hasher: Sha256,
    source_count: usize,
    total_bytes: u64,
    ended: bool,
    manifest_sha256: Arc<Mutex<Option<[u8; 32]>>>,
}

impl FramedSnapshotReader {
    fn new(
        request: PolicyProjectionRequest,
        payload: PolicyGraphSnapshotPayload,
    ) -> Result<(Self, WorkerStreamReceipt), String> {
        if payload.sources.len() > MAX_WORKER_SOURCE_COUNT {
            return Err("policy graph source count exceeded its budget".into());
        }
        let mut request_nonce = [0_u8; 16];
        getrandom::fill(&mut request_nonce)
            .map_err(|_| "policy graph request nonce could not be generated".to_string())?;
        let request_payload = serde_json::to_vec(&WorkerRequestFrame {
            schema_version: WORKER_SCHEMA_VERSION,
            request_nonce,
            request,
        })
        .map_err(|_| "policy graph request could not be encoded".to_string())?;
        if request_payload.len() > MAX_WORKER_REQUEST_BYTES {
            return Err("policy graph request exceeded its byte budget".into());
        }
        let vocabulary_payload = serde_json::to_vec(&WorkerVocabularyFrame {
            correction_revision: payload.correction_revision,
            people: payload.vocabulary_people,
        })
        .map_err(|_| "policy graph vocabulary could not be encoded".to_string())?;
        if vocabulary_payload.len() > MAX_WORKER_VOCABULARY_BYTES {
            return Err("policy graph vocabulary exceeded its byte budget".into());
        }
        let manifest_sha256 = Arc::new(Mutex::new(None));
        let receipt = WorkerStreamReceipt {
            request_nonce,
            manifest_sha256: Arc::clone(&manifest_sha256),
        };
        let mut reader = Self {
            pending: VecDeque::from([WORKER_STREAM_MAGIC.to_vec()]),
            sources: payload.sources.into_iter(),
            current: std::io::Cursor::new(Vec::new()),
            hasher: Sha256::new(),
            source_count: 0,
            total_bytes: WORKER_STREAM_MAGIC.len() as u64,
            ended: false,
            manifest_sha256,
        };
        reader.push_hashed_frame(FRAME_REQUEST, request_payload)?;
        reader.push_hashed_frame(FRAME_VOCABULARY, vocabulary_payload)?;
        Ok((reader, receipt))
    }

    fn push_hashed_frame(&mut self, tag: u8, payload: Vec<u8>) -> Result<(), String> {
        let frame = encode_frame(tag, payload)?;
        self.total_bytes = self
            .total_bytes
            .checked_add(frame.len() as u64)
            .ok_or_else(|| "policy graph input byte count overflowed".to_string())?;
        if self.total_bytes > MAX_WORKER_INPUT_BYTES {
            return Err("policy graph input exceeded its byte budget".into());
        }
        self.hasher.update(&frame);
        self.pending.push_back(frame);
        Ok(())
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        if let Some(chunk) = self.pending.pop_front() {
            return Ok(Some(chunk));
        }
        if let Some(source) = self.sources.next() {
            self.source_count = self
                .source_count
                .checked_add(1)
                .ok_or_else(|| "policy graph source count overflowed".to_string())?;
            let payload = encode_source_payload(&source)?;
            let frame = encode_frame(FRAME_SOURCE, payload)?;
            self.total_bytes = self
                .total_bytes
                .checked_add(frame.len() as u64)
                .ok_or_else(|| "policy graph input byte count overflowed".to_string())?;
            if self.total_bytes > MAX_WORKER_INPUT_BYTES {
                return Err("policy graph input exceeded its byte budget".into());
            }
            self.hasher.update(&frame);
            return Ok(Some(frame));
        }
        if self.ended {
            return Ok(None);
        }
        self.ended = true;
        let manifest: [u8; 32] = self.hasher.clone().finalize().into();
        *self
            .manifest_sha256
            .lock()
            .map_err(|_| "policy graph manifest receipt was poisoned".to_string())? =
            Some(manifest);
        let mut payload = Vec::with_capacity(36);
        payload.extend_from_slice(
            &u32::try_from(self.source_count)
                .map_err(|_| "policy graph source count overflowed".to_string())?
                .to_le_bytes(),
        );
        payload.extend_from_slice(&manifest);
        Ok(Some(encode_frame(FRAME_END, payload)?))
    }
}

impl Read for FramedSnapshotReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let read = self.current.read(output)?;
            if read > 0 {
                return Ok(read);
            }
            let Some(chunk) = self.next_chunk().map_err(std::io::Error::other)? else {
                return Ok(0);
            };
            self.current = std::io::Cursor::new(chunk);
        }
    }
}

impl WorkerStreamReceipt {
    fn manifest_sha256(&self) -> Result<[u8; 32], String> {
        self.manifest_sha256
            .lock()
            .map_err(|_| "policy graph manifest receipt was poisoned".to_string())?
            .ok_or_else(|| "policy graph input stream did not complete".to_string())
    }
}

fn read_u32(cursor: &mut std::io::Cursor<&[u8]>, label: &str) -> Result<u32, String> {
    let mut bytes = [0_u8; 4];
    cursor
        .read_exact(&mut bytes)
        .map_err(|_| format!("policy graph {label} was truncated"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(cursor: &mut std::io::Cursor<&[u8]>, label: &str) -> Result<u64, String> {
    let mut bytes = [0_u8; 8];
    cursor
        .read_exact(&mut bytes)
        .map_err(|_| format!("policy graph {label} was truncated"))?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_utf8_field(
    cursor: &mut std::io::Cursor<&[u8]>,
    max_bytes: usize,
    label: &str,
) -> Result<String, String> {
    let length = usize::try_from(read_u32(cursor, label)?)
        .map_err(|_| format!("policy graph {label} length exceeded this platform"))?;
    if length > max_bytes {
        return Err(format!("policy graph {label} exceeded its byte budget"));
    }
    let start = usize::try_from(cursor.position())
        .map_err(|_| format!("policy graph {label} offset exceeded this platform"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| format!("policy graph {label} length overflowed"))?;
    let bytes = cursor
        .get_ref()
        .get(start..end)
        .ok_or_else(|| format!("policy graph {label} was truncated"))?;
    cursor.set_position(end as u64);
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| format!("policy graph {label} was not UTF-8"))
}

fn decode_source_payload(payload: &[u8]) -> Result<PolicyGraphStreamSource, String> {
    if payload.len() > MAX_WORKER_SOURCE_FRAME_BYTES {
        return Err("policy graph source frame exceeded its byte budget".into());
    }
    let mut cursor = std::io::Cursor::new(payload);
    let opaque_path = PathBuf::from(read_utf8_field(
        &mut cursor,
        MAX_WORKER_CORRECTION_FIELD_BYTES,
        "source identifier",
    )?);
    if !opaque_path.is_absolute()
        || !opaque_path.starts_with("/__minutes_graph_source")
        || opaque_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err("policy graph source identifier was invalid".into());
    }
    let mut content_sha256 = [0_u8; 32];
    cursor
        .read_exact(&mut content_sha256)
        .map_err(|_| "policy graph source digest was truncated".to_string())?;
    let content_length = usize::try_from(read_u64(&mut cursor, "source content length")?)
        .map_err(|_| "policy graph source content length exceeded this platform".to_string())?;
    if content_length > MAX_WORKER_SOURCE_BYTES {
        return Err("policy graph source content exceeded its byte budget".into());
    }
    let correction_count = usize::try_from(read_u32(&mut cursor, "source correction count")?)
        .map_err(|_| "policy graph correction count exceeded this platform".to_string())?;
    if correction_count > MAX_WORKER_CORRECTION_COUNT {
        return Err("policy graph correction count exceeded its budget".into());
    }
    let content_start = usize::try_from(cursor.position())
        .map_err(|_| "policy graph source offset exceeded this platform".to_string())?;
    let content_end = content_start
        .checked_add(content_length)
        .ok_or_else(|| "policy graph source content length overflowed".to_string())?;
    let content_bytes = cursor
        .get_ref()
        .get(content_start..content_end)
        .ok_or_else(|| "policy graph source content was truncated".to_string())?;
    let content = std::str::from_utf8(content_bytes)
        .map_err(|_| "policy graph source content was not UTF-8".to_string())?
        .to_owned();
    cursor.set_position(content_end as u64);
    let observed_digest: [u8; 32] = Sha256::digest(content.as_bytes()).into();
    if observed_digest != content_sha256 {
        return Err("policy graph source digest did not match its bytes".into());
    }
    let mut speaker_corrections = Vec::with_capacity(correction_count.min(256));
    for _ in 0..correction_count {
        speaker_corrections.push(PolicyGraphSpeakerCorrection {
            speaker_label: read_utf8_field(
                &mut cursor,
                MAX_WORKER_CORRECTION_FIELD_BYTES,
                "speaker label",
            )?,
            name: read_utf8_field(
                &mut cursor,
                MAX_WORKER_CORRECTION_FIELD_BYTES,
                "speaker name",
            )?,
        });
    }
    if cursor.position() != payload.len() as u64 {
        return Err("policy graph source frame had trailing bytes".into());
    }
    Ok(PolicyGraphStreamSource {
        opaque_path,
        content,
        content_sha256,
        speaker_corrections,
    })
}

fn read_worker_stream(mut input: impl Read) -> Result<WorkerInput, String> {
    let mut magic = [0_u8; 8];
    input
        .read_exact(&mut magic)
        .map_err(|_| "policy graph stream header was truncated".to_string())?;
    if &magic != WORKER_STREAM_MAGIC {
        return Err("policy graph stream header was invalid".into());
    }
    let mut total_bytes = magic.len() as u64;
    let mut hasher = Sha256::new();
    let mut request: Option<WorkerRequestFrame> = None;
    let mut vocabulary: Option<WorkerVocabularyFrame> = None;
    let mut sources = Vec::new();
    let mut seen_source_ids = std::collections::HashSet::new();

    loop {
        let mut header = [0_u8; FRAME_HEADER_BYTES];
        input
            .read_exact(&mut header)
            .map_err(|_| "policy graph frame header was truncated".to_string())?;
        if header[1..4] != [0, 0, 0] {
            return Err("policy graph frame header reserved bytes were nonzero".into());
        }
        let tag = header[0];
        let payload_len = usize::try_from(u64::from_le_bytes(
            header[4..12]
                .try_into()
                .map_err(|_| "policy graph frame length was invalid".to_string())?,
        ))
        .map_err(|_| "policy graph frame length exceeded this platform".to_string())?;
        let tag_limit = match tag {
            FRAME_REQUEST => MAX_WORKER_REQUEST_BYTES,
            FRAME_VOCABULARY => MAX_WORKER_VOCABULARY_BYTES,
            FRAME_SOURCE => MAX_WORKER_SOURCE_FRAME_BYTES,
            FRAME_END => 36,
            _ => return Err("policy graph stream contained an unknown frame".into()),
        };
        if payload_len > tag_limit {
            return Err("policy graph frame exceeded its byte budget".into());
        }
        total_bytes = total_bytes
            .checked_add((FRAME_HEADER_BYTES + payload_len) as u64)
            .ok_or_else(|| "policy graph input byte count overflowed".to_string())?;
        if total_bytes > MAX_WORKER_INPUT_BYTES {
            return Err("policy graph input exceeded its byte budget".into());
        }
        let mut payload = vec![0_u8; payload_len];
        input
            .read_exact(&mut payload)
            .map_err(|_| "policy graph frame payload was truncated".to_string())?;

        match tag {
            FRAME_REQUEST => {
                if request.is_some() || vocabulary.is_some() || !sources.is_empty() {
                    return Err("policy graph request frame was duplicate or out of order".into());
                }
                hasher.update(header);
                hasher.update(&payload);
                let decoded: WorkerRequestFrame = serde_json::from_slice(&payload)
                    .map_err(|_| "policy graph request frame was malformed".to_string())?;
                if decoded.schema_version != WORKER_SCHEMA_VERSION {
                    return Err("policy graph request schema was unsupported".into());
                }
                request = Some(decoded);
            }
            FRAME_VOCABULARY => {
                if request.is_none() || vocabulary.is_some() || !sources.is_empty() {
                    return Err(
                        "policy graph vocabulary frame was duplicate or out of order".into(),
                    );
                }
                hasher.update(header);
                hasher.update(&payload);
                vocabulary = Some(
                    serde_json::from_slice(&payload)
                        .map_err(|_| "policy graph vocabulary frame was malformed".to_string())?,
                );
            }
            FRAME_SOURCE => {
                if request.is_none() || vocabulary.is_none() {
                    return Err("policy graph source frame was out of order".into());
                }
                if sources.len() >= MAX_WORKER_SOURCE_COUNT {
                    return Err("policy graph source count exceeded its budget".into());
                }
                hasher.update(header);
                hasher.update(&payload);
                let source = decode_source_payload(&payload)?;
                if !seen_source_ids.insert(source.opaque_path.clone()) {
                    return Err("policy graph source identifier was duplicated".into());
                }
                sources.push(source);
            }
            FRAME_END => {
                if request.is_none() || vocabulary.is_none() || payload.len() != 36 {
                    return Err("policy graph end frame was out of order".into());
                }
                let expected_count = u32::from_le_bytes(
                    payload[..4]
                        .try_into()
                        .map_err(|_| "policy graph end source count was malformed".to_string())?,
                ) as usize;
                let expected_digest: [u8; 32] = payload[4..]
                    .try_into()
                    .map_err(|_| "policy graph end digest was malformed".to_string())?;
                let observed_digest: [u8; 32] = hasher.finalize().into();
                if expected_count != sources.len() || expected_digest != observed_digest {
                    return Err("policy graph stream manifest did not match".into());
                }
                let mut trailing = [0_u8; 1];
                if input
                    .read(&mut trailing)
                    .map_err(|_| "policy graph trailing-byte check failed".to_string())?
                    != 0
                {
                    return Err("policy graph stream had trailing bytes".into());
                }
                let vocabulary = vocabulary.expect("validated above");
                return Ok(WorkerInput {
                    request: request.expect("validated above"),
                    vocabulary_people: vocabulary.people,
                    sources,
                    correction_revision: vocabulary.correction_revision,
                    manifest_sha256: observed_digest,
                });
            }
            _ => unreachable!("unknown tags rejected above"),
        }
    }
}

#[cfg(windows)]
fn close_inherited_windows_handles_before_authority() -> Result<(), String> {
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetHandleInformation, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Console::GetStdHandle;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    const SYSTEM_EXTENDED_HANDLE_INFORMATION: u32 = 64;
    const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004u32 as i32;
    const MAX_HANDLE_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SystemHandleEntry {
        object: *mut c_void,
        process_id: usize,
        handle_value: usize,
        granted_access: u32,
        creator_backtrace_index: u16,
        object_type_index: u16,
        handle_attributes: u32,
        reserved: u32,
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQuerySystemInformation(
            information_class: u32,
            information: *mut c_void,
            information_bytes: u32,
            returned_bytes: *mut u32,
        ) -> i32;
    }

    let mut capacity = 64 * 1024usize;
    let (storage, returned) = loop {
        if capacity > MAX_HANDLE_SNAPSHOT_BYTES {
            return Err("graph worker inherited-handle inventory exceeded its budget".into());
        }
        let words = capacity.div_ceil(size_of::<usize>());
        let mut storage = vec![0usize; words];
        let mut returned = 0u32;
        let status = unsafe {
            NtQuerySystemInformation(
                SYSTEM_EXTENDED_HANDLE_INFORMATION,
                storage.as_mut_ptr().cast(),
                (words * size_of::<usize>()) as u32,
                &mut returned,
            )
        };
        if status == 0 {
            break (storage, returned as usize);
        }
        if status != STATUS_INFO_LENGTH_MISMATCH {
            return Err("graph worker inherited handles could not be inventoried".into());
        }
        capacity = capacity
            .saturating_mul(2)
            .max(returned as usize)
            .min(MAX_HANDLE_SNAPSHOT_BYTES + 1);
    };

    let header_bytes = size_of::<usize>() * 2;
    if returned < header_bytes {
        return Err("graph worker inherited-handle inventory was malformed".into());
    }
    let base = storage.as_ptr().cast::<u8>();
    let count = unsafe { std::ptr::read_unaligned(base.cast::<usize>()) };
    let entries_bytes = count
        .checked_mul(size_of::<SystemHandleEntry>())
        .and_then(|bytes| bytes.checked_add(header_bytes))
        .ok_or_else(|| "graph worker inherited-handle inventory overflowed".to_string())?;
    if entries_bytes > returned || entries_bytes > storage.len() * size_of::<usize>() {
        return Err("graph worker inherited-handle inventory was truncated".into());
    }

    let preserved = unsafe {
        [
            GetStdHandle(STD_INPUT_HANDLE),
            GetStdHandle(STD_OUTPUT_HANDLE),
            GetStdHandle(STD_ERROR_HANDLE),
        ]
    };
    let process_id = unsafe { GetCurrentProcessId() } as usize;
    for index in 0..count {
        let offset = header_bytes + index * size_of::<SystemHandleEntry>();
        let entry =
            unsafe { std::ptr::read_unaligned(base.add(offset).cast::<SystemHandleEntry>()) };
        if entry.process_id != process_id {
            continue;
        }
        let handle = entry.handle_value as HANDLE;
        if handle.is_null() || handle == INVALID_HANDLE_VALUE || preserved.contains(&handle) {
            continue;
        }
        if unsafe { CloseHandle(handle) } == 0 {
            return Err("graph worker could not retire an inherited handle".into());
        }
    }
    if let Some(value) = std::env::var_os("MINUTES_INTERNAL_TEST_GRAPH_INHERITED_HANDLE") {
        std::env::remove_var("MINUTES_INTERNAL_TEST_GRAPH_INHERITED_HANDLE");
        let value = value
            .to_str()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| "graph worker inherited-handle canary was malformed".to_string())?;
        let mut flags = 0u32;
        if unsafe { GetHandleInformation(value as HANDLE, &mut flags) } != 0 {
            return Err("graph worker retained an ambient inherited handle".into());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn close_inherited_windows_handles_before_authority() -> Result<(), String> {
    Ok(())
}

/// Run the hidden graph worker when the internal marker is present. Both the
/// CLI and desktop executable call this before argument parsing, config loads,
/// migrations, UI setup, or any other side effect.
#[cfg(target_os = "macos")]
pub fn maybe_run_policy_projection_worker() -> Option<i32> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn maybe_run_policy_projection_worker() -> Option<i32> {
    let marker = std::env::var_os(WORKER_MARKER)?;
    std::env::remove_var(WORKER_MARKER);
    if marker != "1" {
        return Some(2);
    }
    Some(run_policy_projection_stream_worker_main())
}

/// Entry point for the bounded stdin/stdout worker on non-macOS platforms.
pub fn run_policy_projection_stream_worker_main() -> i32 {
    let result = run_policy_projection_stream_worker();
    match result {
        Ok(()) => 0,
        Err(error) => {
            // Error text is deliberately bounded and contains no paths or
            // source bytes. The parent maps every nonzero exit to a generic
            // policy-projection failure.
            let message = error.chars().take(512).collect::<String>();
            eprintln!("policy graph worker failed: {message}");
            1
        }
    }
}

/// Entrypoint used only by the embedded macOS XPC service executable.
#[cfg(target_os = "macos")]
pub fn run_policy_projection_xpc_service_main() -> ! {
    crate::macos_graph_xpc::run_service_main()
}

fn run_policy_projection_stream_worker() -> Result<(), String> {
    // Windows std::process inherits ambient inheritable HANDLEs when it
    // redirects stdio. The worker closes every process handle except its three
    // authenticated pipes before it receives any source bytes.
    close_inherited_windows_handles_before_authority()?;
    let stdin = std::io::stdin();
    let input = read_worker_stream(stdin.lock())?;
    let response = policy_projection_response_bytes(input)?;
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    writer
        .write_all(&response)
        .map_err(|_| "graph worker response could not be written".to_string())?;
    writer
        .flush()
        .map_err(|_| "graph worker response could not be flushed".to_string())
}

fn policy_projection_response_bytes(input: WorkerInput) -> Result<Vec<u8>, String> {
    let response = crate::graph::policy_projection_response_from_stream(
        input.sources,
        input.vocabulary_people,
        input.correction_revision,
        &input.request.request,
    )
    .map_err(|error| error.to_string())?;
    let envelope = WorkerResponseEnvelope {
        schema_version: WORKER_SCHEMA_VERSION,
        request_nonce: input.request.request_nonce,
        manifest_sha256: input.manifest_sha256,
        response,
    };
    let response = serde_json::to_vec(&envelope)
        .map_err(|_| "graph worker response could not be serialized".to_string())?;
    if response.len() as u64 > MAX_WORKER_RESPONSE_BYTES {
        return Err("graph worker response exceeded its byte budget".into());
    }
    Ok(response)
}

#[cfg(target_os = "macos")]
pub(crate) fn process_policy_projection_stream_bytes(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.len() as u64 > MAX_WORKER_INPUT_BYTES {
        return Err("policy graph input exceeded its byte budget".into());
    }
    let input = read_worker_stream(std::io::Cursor::new(input))?;
    policy_projection_response_bytes(input)
}

#[cfg(target_os = "macos")]
fn macos_virtual_size() -> Result<u64, String> {
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
        return Err("graph worker could not measure its baseline address space".into());
    }
    Ok(info.virtual_size)
}

#[cfg(target_os = "macos")]
pub(crate) fn prepare_macos_graph_xpc_worker() -> Result<(), String> {
    prepare_macos_graph_xpc_worker_with_wall_clock(WORKER_WALL_CLOCK)
}

#[cfg(target_os = "macos")]
fn prepare_macos_graph_xpc_worker_with_wall_clock(wall_clock: Duration) -> Result<(), String> {
    // The XPC service is deliberately one request per process. Install its
    // immutable process ceilings exactly once before accepting a connection;
    // no later peer can raise the hard limit or extend another request's timer.
    // The XPC runtime owns the service connection descriptors, so unlike the
    // rejected stdin/stdout child design they must not be closed.
    let process_count = libc::rlimit {
        rlim_cur: 1,
        rlim_max: 1,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &process_count) } != 0 {
        return Err("graph worker could not install its no-descendants ceiling".into());
    }

    unsafe extern "C" {
        fn setitimer(
            which: libc::c_int,
            new_value: *const libc::itimerval,
            old_value: *mut libc::itimerval,
        ) -> libc::c_int;
    }
    // Darwin reserves a very large shared-cache virtual range before main()
    // (hundreds of GiB on current macOS), so an absolute 1 GiB RLIMIT_AS is
    // below the process's immutable baseline and the kernel rejects it. Bind
    // the hard limit to the measured post-exec baseline plus exactly the same
    // 1 GiB growth budget used on other platforms.
    let address_space_limit = macos_virtual_size()?
        .checked_add(WORKER_ADDRESS_SPACE_BYTES)
        .ok_or_else(|| "graph worker address-space ceiling overflowed".to_string())?;
    let address_space = libc::rlimit {
        rlim_cur: address_space_limit,
        rlim_max: address_space_limit,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &address_space) } != 0 {
        return Err("graph worker could not install its address-space ceiling".into());
    }
    let timeout_milliseconds = u64::try_from(wall_clock.as_millis())
        .ok()
        .filter(|milliseconds| *milliseconds > 0)
        .ok_or_else(|| "graph worker wall-clock ceiling was invalid".to_string())?;
    let seconds = timeout_milliseconds / 1_000;
    let microseconds = (timeout_milliseconds % 1_000) * 1_000;
    let timer = libc::itimerval {
        it_interval: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        it_value: libc::timeval {
            tv_sec: seconds as libc::time_t,
            tv_usec: microseconds as libc::suseconds_t,
        },
    };
    if unsafe { setitimer(libc::ITIMER_REAL, &timer, std::ptr::null_mut()) } != 0 {
        return Err("graph worker could not install its wall-clock ceiling".into());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn retain_safe_environment(command: &mut crate::bounded_child::BoundedCommand) {
    command.env_clear();
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if let Some(value) = std::env::var_os("MINUTES_INTERNAL_TEST_GRAPH_INHERITED_HANDLE") {
        command.env("MINUTES_INTERNAL_TEST_GRAPH_INHERITED_HANDLE", value);
    }
}

fn validate_worker_response(
    request: &PolicyProjectionRequest,
    response: &PolicyProjectionResponse,
) -> Result<(), String> {
    let valid_stats = |stats: &crate::graph::GraphStats| {
        stats.alias_suggestions.len() <= MAX_WORKER_RESULT_ITEMS
            && stats.alias_clusters.len() <= MAX_WORKER_RESULT_ITEMS
            && stats.alias_clusters.iter().all(|cluster| {
                cluster.members.len() <= MAX_WORKER_RESULT_ITEMS
                    && cluster.slugs.len() == cluster.members.len()
            })
    };
    let valid_people = |people: &[crate::graph::PersonSummary]| {
        people.len() <= MAX_WORKER_RESULT_ITEMS
            && people
                .iter()
                .all(|person| person.top_topics.len() <= MAX_WORKER_RESULT_ITEMS)
    };
    let invalid = || Err("policy graph worker returned an invalid response".to_string());
    match (request, response) {
        (PolicyProjectionRequest::RebuildStats, PolicyProjectionResponse::RebuildStats(stats))
            if valid_stats(stats) => {}
        (
            PolicyProjectionRequest::People {
                limit,
                include_commitments,
                include_stats,
            },
            PolicyProjectionResponse::People(value),
        ) if value.people.len() <= *limit
            && valid_people(&value.people)
            && value.commitments.len() <= *limit
            && (*include_commitments || value.commitments.is_empty())
            && (*include_stats == value.stats.is_some())
            && value.stats.as_ref().is_none_or(valid_stats) => {}
        (
            PolicyProjectionRequest::RelationshipMap { limit },
            PolicyProjectionResponse::RelationshipMap(value),
        ) if value.len() <= *limit && valid_people(value) => {}
        (
            PolicyProjectionRequest::RelationshipContext { limit },
            PolicyProjectionResponse::RelationshipContext(value),
        ) if value.people.len() <= *limit
            && valid_people(&value.people)
            && value.commitments.len() <= *limit => {}
        (
            PolicyProjectionRequest::PersonProfile { .. },
            PolicyProjectionResponse::PersonProfile(value),
        ) if value.recent_meetings.len() <= MAX_WORKER_RESULT_ITEMS
            && value.open_intents.len() <= MAX_WORKER_RESULT_ITEMS
            && value.recent_decisions.len() <= MAX_WORKER_RESULT_ITEMS
            && value.top_topics.len() <= MAX_WORKER_RESULT_ITEMS => {}
        (
            PolicyProjectionRequest::Commitments { limit, .. },
            PolicyProjectionResponse::Commitments(value),
        ) if value.len() <= *limit && value.len() <= MAX_WORKER_RESULT_ITEMS => {}
        (
            PolicyProjectionRequest::LosingTouch { limit },
            PolicyProjectionResponse::LosingTouch(value),
        ) if value.len() <= *limit && value.len() <= MAX_WORKER_RESULT_ITEMS => {}
        (
            PolicyProjectionRequest::ParakeetBoostPhrases { limit },
            PolicyProjectionResponse::ParakeetBoostPhrases(value),
        ) if value.len() <= *limit && value.len() <= MAX_WORKER_RESULT_ITEMS => {}
        _ => return invalid(),
    }
    Ok(())
}

fn preflight_worker_response_allocation(bytes: &[u8]) -> Result<(), String> {
    let mut structural_units = 0usize;
    let mut aggregate_string_bytes = 0usize;
    let mut current_string_bytes = 0usize;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for &byte in bytes {
        if in_string {
            current_string_bytes = current_string_bytes.checked_add(1).ok_or_else(|| {
                "policy graph worker response string budget overflowed".to_string()
            })?;
            aggregate_string_bytes = aggregate_string_bytes.checked_add(1).ok_or_else(|| {
                "policy graph worker response string budget overflowed".to_string()
            })?;
            if current_string_bytes > MAX_WORKER_RESPONSE_SINGLE_STRING_BYTES
                || aggregate_string_bytes > MAX_WORKER_RESPONSE_STRING_BYTES
            {
                return Err("policy graph worker response string budget exceeded".into());
            }
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                current_string_bytes = 0;
                structural_units = structural_units.saturating_add(1);
            }
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                if depth > MAX_WORKER_RESPONSE_DEPTH {
                    return Err("policy graph worker response nesting budget exceeded".into());
                }
                structural_units = structural_units.saturating_add(1);
            }
            b'}' | b']' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "policy graph worker returned malformed JSON".to_string())?;
            }
            b',' => structural_units = structural_units.saturating_add(1),
            _ => {}
        }
        if structural_units > MAX_WORKER_RESPONSE_STRUCTURAL_UNITS {
            return Err("policy graph worker response structure budget exceeded".into());
        }
    }
    if in_string || escaped || depth != 0 {
        return Err("policy graph worker returned malformed JSON".into());
    }
    Ok(())
}

fn validate_worker_envelope_binding(
    receipt: &WorkerStreamReceipt,
    manifest_sha256: [u8; 32],
    envelope: &WorkerResponseEnvelope,
) -> Result<(), String> {
    if envelope.schema_version != WORKER_SCHEMA_VERSION
        || envelope.request_nonce != receipt.request_nonce
        || envelope.manifest_sha256 != manifest_sha256
    {
        Err("policy graph worker response was not bound to its request".into())
    } else {
        Ok(())
    }
}

/// Execute one policy projection in the hard-bounded worker and decode its
/// internal tagged response. No partial stdout is returned on timeout,
/// resource exhaustion, journal invalidation, or unsuccessful exit.
pub fn run_policy_projection_worker(
    config: &Config,
    request: PolicyProjectionRequest,
) -> Result<PolicyProjectionResponse, String> {
    let (mut snapshot, authority) = crate::graph::capture_policy_graph_snapshot(config)
        .map_err(|_| "policy graph inputs could not be captured".to_string())?;
    let opaque_to_live_paths = std::mem::take(&mut snapshot.opaque_to_live_paths);
    let wall_clock = authority.remaining().min(WORKER_WALL_CLOCK);
    if wall_clock.is_zero() {
        return Err("policy graph operation exceeded its wall-clock budget".into());
    }

    #[cfg(target_os = "macos")]
    let mut response = match resolve_policy_projection_worker_transport()? {
        MacWorkerTransport::InProcessFallback => {
            // Compatibility mode is intentionally explicit: source/ad-hoc
            // builds and macOS 11 keep the existing default-user graph
            // experience, but make no claim of atomic helper isolation. The
            // snapshot is already policy-filtered to the normal corpus, and
            // the same ordered post-publication fence below still applies.
            crate::graph::policy_projection_response_from_stream(
                snapshot.sources,
                snapshot.vocabulary_people,
                snapshot.correction_revision,
                &request,
            )
            .map_err(|_| "policy graph projection failed closed".to_string())?
        }
        MacWorkerTransport::AuthenticatedXpc(executable) => {
            let (input, receipt) = FramedSnapshotReader::new(request.clone(), snapshot)?;
            let output = crate::macos_graph_xpc::run(
                &executable.bundle,
                &executable.cdhash,
                executable.trusted_distribution,
                input,
                MAX_WORKER_RESPONSE_BYTES,
                wall_clock,
            )
            .map_err(|_| "policy graph worker could not be supervised".to_string())?;
            let manifest_sha256 = receipt.manifest_sha256()?;
            preflight_worker_response_allocation(&output)?;
            let envelope: WorkerResponseEnvelope = serde_json::from_slice(&output)
                .map_err(|_| "policy graph worker returned an invalid response".to_string())?;
            validate_worker_envelope_binding(&receipt, manifest_sha256, &envelope)?;
            validate_worker_response(&request, &envelope.response)?;
            envelope.response
        }
    };

    #[cfg(not(target_os = "macos"))]
    let mut response = {
        let (input, receipt) = FramedSnapshotReader::new(request.clone(), snapshot)?;
        let executable = resolve_policy_projection_worker_executable()?;
        let mut command = crate::bounded_child::BoundedCommand::from_bound_executable(executable)
            .map_err(|_| {
            "policy graph worker executable authority could not be bound".to_string()
        })?;
        retain_safe_environment(&mut command);
        command
            .env(WORKER_MARKER, "1")
            .address_space_limit(WORKER_ADDRESS_SPACE_BYTES)
            .single_process()
            .close_extra_descriptors();
        let run = crate::bounded_child::run(
            &mut command,
            Some(Box::new(input)),
            crate::bounded_child::StdoutTarget::Capture {
                max_bytes: MAX_WORKER_RESPONSE_BYTES,
            },
            crate::bounded_child::ChildBudget {
                wall_clock,
                stderr_tail: MAX_WORKER_STDERR_BYTES,
            },
        )
        .map_err(|_| "policy graph worker could not be supervised".to_string())?;
        if run.timed_out {
            return Err("policy graph worker exceeded its wall-clock budget".into());
        }
        if !run.output.status.success() {
            return Err("policy graph worker failed closed".into());
        }
        let manifest_sha256 = receipt.manifest_sha256()?;
        preflight_worker_response_allocation(&run.output.stdout)?;
        let envelope: WorkerResponseEnvelope = serde_json::from_slice(&run.output.stdout)
            .map_err(|_| "policy graph worker returned an invalid response".to_string())?;
        validate_worker_envelope_binding(&receipt, manifest_sha256, &envelope)?;
        validate_worker_response(&request, &envelope.response)?;
        envelope.response
    };

    authority
        .finalize()
        .map_err(|_| "policy graph inputs changed before publication".to_string())?;
    crate::graph::rehydrate_policy_projection_paths(&mut response, &opaque_to_live_paths)
        .map_err(|_| "policy graph worker returned an unknown source".to_string())?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_payload(sources: Vec<PolicyGraphStreamSource>) -> PolicyGraphSnapshotPayload {
        PolicyGraphSnapshotPayload {
            sources,
            vocabulary_people: Vec::new(),
            correction_revision: "synthetic-revision-1".into(),
            opaque_to_live_paths: Default::default(),
        }
    }

    fn synthetic_source(path: &str, content: &str) -> PolicyGraphStreamSource {
        PolicyGraphStreamSource {
            opaque_path: PathBuf::from(path),
            content_sha256: Sha256::digest(content.as_bytes()).into(),
            content: content.into(),
            speaker_corrections: Vec::new(),
        }
    }

    fn encoded_stream(sources: Vec<PolicyGraphStreamSource>) -> (Vec<u8>, WorkerStreamReceipt) {
        let (mut stream, receipt) = FramedSnapshotReader::new(
            PolicyProjectionRequest::RelationshipMap { limit: 5 },
            synthetic_payload(sources),
        )
        .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        (bytes, receipt)
    }

    fn frame_offsets(bytes: &[u8]) -> Vec<(usize, u8, usize)> {
        let mut offset = WORKER_STREAM_MAGIC.len();
        let mut frames = Vec::new();
        while offset + FRAME_HEADER_BYTES <= bytes.len() {
            let tag = bytes[offset];
            let payload_len =
                u64::from_le_bytes(bytes[offset + 4..offset + 12].try_into().unwrap()) as usize;
            frames.push((offset, tag, payload_len));
            offset += FRAME_HEADER_BYTES + payload_len;
        }
        frames
    }

    fn person_summary() -> crate::graph::PersonSummary {
        crate::graph::PersonSummary {
            slug: "avery-quinn".into(),
            name: "Avery Quinn".into(),
            meeting_count: 1,
            last_seen: "2026-07-20T12:00:00Z".into(),
            days_since: 1.0,
            open_commitments: 0,
            top_topics: vec!["planning".into()],
            score: 1.0,
            losing_touch: false,
        }
    }

    #[test]
    fn worker_executable_paths_reject_relative_and_parent_traversal() {
        assert!(!valid_absolute_executable_path(Path::new("minutes")));
        assert!(!valid_absolute_executable_path(Path::new(
            "/tmp/../minutes"
        )));
        assert!(valid_absolute_executable_path(Path::new("/tmp/minutes")));
    }

    #[test]
    fn worker_protocol_has_explicit_hard_bounds() {
        const {
            assert!(MAX_WORKER_REQUEST_BYTES <= 256 * 1024);
            assert!(MAX_WORKER_RESPONSE_BYTES <= 32 * 1024 * 1024);
            assert!(WORKER_ADDRESS_SPACE_BYTES <= 1024 * 1024 * 1024);
            assert!(WORKER_WALL_CLOCK.as_secs() <= 45);
        }
    }

    #[test]
    fn macos_transport_selection_preserves_fallback_and_missing_service_truth() {
        assert_eq!(
            choose_macos_worker_transport(true, true, false, true, true),
            MacWorkerTransportChoice::InProcessFallback,
            "a cached authority must not bypass the macOS 11 compatibility fallback"
        );
        assert_eq!(
            choose_macos_worker_transport(true, true, true, true, true),
            MacWorkerTransportChoice::CachedAuthority
        );
        assert_eq!(
            choose_macos_worker_transport(true, true, true, false, false),
            MacWorkerTransportChoice::MissingService,
            "a trusted supported app must fail closed when its service is missing"
        );
        assert_eq!(
            choose_macos_worker_transport(false, true, true, false, false),
            MacWorkerTransportChoice::InProcessFallback,
            "standalone use remains an explicit compatibility fallback"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_worker_kernel_ceiling_refuses_descendant_creation() {
        const CHILD_MARKER: &str = "MINUTES_INTERNAL_TEST_GRAPH_NPROC_CHILD";
        if std::env::var_os(CHILD_MARKER).is_some() {
            std::env::remove_var(CHILD_MARKER);
            super::prepare_macos_graph_xpc_worker()
                .expect("worker containment should install in the isolated subprocess");
            let forked = unsafe { libc::fork() };
            if forked == 0 {
                unsafe { libc::_exit(91) };
            }
            if forked > 0 {
                unsafe {
                    libc::kill(forked, libc::SIGKILL);
                    libc::waitpid(forked, std::ptr::null_mut(), 0);
                }
                std::process::exit(92);
            }
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EAGAIN),
                "the kernel must reject graph-worker descendants"
            );
            std::process::exit(0);
        }

        let output = crate::engine_process::command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "graph_worker::tests::macos_worker_kernel_ceiling_refuses_descendant_creation",
                "--nocapture",
            ])
            .env(CHILD_MARKER, "1")
            .output()
            .expect("isolated worker-containment subprocess should launch");
        assert!(
            output.status.success(),
            "worker descendant ceiling subprocess failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_worker_kernel_wall_clock_retires_a_stalled_service() {
        use std::os::unix::process::ExitStatusExt;

        const TIMER_MARKER: &str = "MINUTES_INTERNAL_TEST_GRAPH_WALL_CLOCK_CHILD";
        if std::env::var_os(TIMER_MARKER).is_some() {
            std::env::remove_var(TIMER_MARKER);
            super::prepare_macos_graph_xpc_worker_with_wall_clock(Duration::from_millis(100))
                .expect("worker wall-clock ceiling should install in the isolated subprocess");
            loop {
                unsafe { libc::pause() };
            }
        }

        let output = crate::engine_process::command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "graph_worker::tests::macos_worker_kernel_wall_clock_retires_a_stalled_service",
                "--nocapture",
            ])
            .env(TIMER_MARKER, "1")
            .output()
            .expect("isolated worker-timer subprocess should launch");
        assert_eq!(
            output.status.signal(),
            Some(libc::SIGALRM),
            "the kernel timer must retire a stalled graph worker"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_helper_manifest_requires_one_exact_lowercase_cdhash() {
        let authority = decode_macos_graph_worker_cdhash(
            b"00112233445566778899aabbccddeeff00112233",
            "malformed",
        )
        .unwrap();
        assert_eq!(
            authority,
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff, 0x00, 0x11, 0x22, 0x33,
            ]
        );

        for malformed in [
            b"0000000000000000000000000000000000000000".as_slice(),
            b"00112233445566778899AABBCCDDEEFF00112233".as_slice(),
            b"00112233445566778899aabbccddeeff0011223344".as_slice(),
            b"00112233445566778899aabbccddeeff0011223g".as_slice(),
            b"00112233445566778899aabbccddeeff001122".as_slice(),
        ] {
            assert!(decode_macos_graph_worker_cdhash(malformed, "malformed").is_err());
        }
        assert!(
            verify_macos_graph_worker_cdhash_binding(
                b"00112233445566778899aabbccddeeff00112233",
                b"10112233445566778899aabbccddeeff00112233",
            )
            .is_err(),
            "a valid signed helper from another package must not replay"
        );
        assert!(verify_macos_graph_worker_cdhash_binding(
            b"00112233445566778899aabbccddeeff00112233",
            b"00112233445566778899aabbccddeeff00112233",
        )
        .is_ok());
    }

    #[test]
    fn framed_snapshot_round_trips_and_receipts_exact_manifest() {
        let content = "# Meeting\n\nSynthetic planning notes.".to_string();
        let source = PolicyGraphStreamSource {
            opaque_path: PathBuf::from("/__minutes_graph_source/test/00000000.md"),
            content_sha256: Sha256::digest(content.as_bytes()).into(),
            content,
            speaker_corrections: vec![PolicyGraphSpeakerCorrection {
                speaker_label: "SPEAKER_00".into(),
                name: "Avery Quinn".into(),
            }],
        };
        let payload = PolicyGraphSnapshotPayload {
            sources: vec![source],
            vocabulary_people: Vec::new(),
            correction_revision: "revision-1".into(),
            opaque_to_live_paths: Default::default(),
        };
        let request = PolicyProjectionRequest::RelationshipMap { limit: 5 };
        let (mut stream, receipt) = FramedSnapshotReader::new(request, payload).unwrap();
        let decoded = read_worker_stream(&mut stream).unwrap();
        assert_eq!(decoded.sources.len(), 1);
        assert_eq!(decoded.correction_revision, "revision-1");
        assert_eq!(decoded.manifest_sha256, receipt.manifest_sha256().unwrap());
    }

    #[test]
    fn framed_stream_rejects_truncation_trailing_bytes_and_digest_tamper() {
        let (mut bytes, _) = encoded_stream(vec![synthetic_source(
            "/__minutes_graph_source/test/00000000.md",
            "Synthetic notes.",
        )]);

        assert!(read_worker_stream(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(read_worker_stream(trailing.as_slice()).is_err());

        let source_tag = frame_offsets(&bytes)
            .into_iter()
            .find_map(|(offset, tag, _)| (tag == FRAME_SOURCE).then_some(offset))
            .unwrap();
        let source_payload = source_tag + FRAME_HEADER_BYTES;
        let path_len = u32::from_le_bytes(
            bytes[source_payload..source_payload + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let digest_offset = source_payload + 4 + path_len;
        bytes[digest_offset] ^= 0xff;
        assert!(read_worker_stream(bytes.as_slice()).is_err());
    }

    #[test]
    fn framed_stream_rejects_magic_reserved_tag_order_and_length_attacks() {
        let (valid, _) = encoded_stream(vec![synthetic_source(
            "/__minutes_graph_source/test/00000000.md",
            "Synthetic notes.",
        )]);

        let mut bad_magic = valid.clone();
        bad_magic[0] ^= 0xff;
        assert!(read_worker_stream(bad_magic.as_slice()).is_err());

        let mut reserved = valid.clone();
        reserved[WORKER_STREAM_MAGIC.len() + 1] = 1;
        assert!(read_worker_stream(reserved.as_slice()).is_err());

        let mut unknown = valid.clone();
        unknown[WORKER_STREAM_MAGIC.len()] = 17;
        assert!(read_worker_stream(unknown.as_slice()).is_err());

        let mut oversized = WORKER_STREAM_MAGIC.to_vec();
        let header = frame_header(FRAME_REQUEST, MAX_WORKER_REQUEST_BYTES + 1).unwrap();
        oversized.extend_from_slice(&header);
        assert!(read_worker_stream(oversized.as_slice()).is_err());

        let frames = frame_offsets(&valid);
        let (request_offset, _, request_len) = frames[0];
        let (vocabulary_offset, _, vocabulary_len) = frames[1];
        let request = &valid[request_offset..request_offset + FRAME_HEADER_BYTES + request_len];
        let vocabulary =
            &valid[vocabulary_offset..vocabulary_offset + FRAME_HEADER_BYTES + vocabulary_len];

        let mut vocabulary_first = WORKER_STREAM_MAGIC.to_vec();
        vocabulary_first.extend_from_slice(vocabulary);
        assert!(read_worker_stream(vocabulary_first.as_slice()).is_err());

        let mut duplicate_request = WORKER_STREAM_MAGIC.to_vec();
        duplicate_request.extend_from_slice(request);
        duplicate_request.extend_from_slice(request);
        assert!(read_worker_stream(duplicate_request.as_slice()).is_err());

        let mut source_before_vocabulary = WORKER_STREAM_MAGIC.to_vec();
        source_before_vocabulary.extend_from_slice(request);
        let source = frames
            .iter()
            .find(|(_, tag, _)| *tag == FRAME_SOURCE)
            .map(|(offset, _, length)| &valid[*offset..*offset + FRAME_HEADER_BYTES + *length])
            .unwrap();
        source_before_vocabulary.extend_from_slice(source);
        assert!(read_worker_stream(source_before_vocabulary.as_slice()).is_err());
    }

    #[test]
    fn framed_stream_rejects_source_namespace_duplicates_and_manifest_replay() {
        let path = "/__minutes_graph_source/test/00000000.md";
        let (duplicate, _) = encoded_stream(vec![
            synthetic_source(path, "First synthetic source."),
            synthetic_source(path, "Second synthetic source."),
        ]);
        assert!(read_worker_stream(duplicate.as_slice()).is_err());

        let (relative, _) =
            encoded_stream(vec![synthetic_source("relative.md", "Synthetic source.")]);
        assert!(read_worker_stream(relative.as_slice()).is_err());

        let (mut count_replay, _) =
            encoded_stream(vec![synthetic_source(path, "Synthetic source.")]);
        let (end_offset, _, _) = frame_offsets(&count_replay)
            .into_iter()
            .find(|(_, tag, _)| *tag == FRAME_END)
            .unwrap();
        count_replay[end_offset + FRAME_HEADER_BYTES] = 2;
        assert!(read_worker_stream(count_replay.as_slice()).is_err());

        let (mut digest_replay, _) =
            encoded_stream(vec![synthetic_source(path, "Synthetic source.")]);
        let (end_offset, _, _) = frame_offsets(&digest_replay)
            .into_iter()
            .find(|(_, tag, _)| *tag == FRAME_END)
            .unwrap();
        digest_replay[end_offset + FRAME_HEADER_BYTES + 4] ^= 0xff;
        assert!(read_worker_stream(digest_replay.as_slice()).is_err());
    }

    #[test]
    fn parent_rejects_stale_wrong_nonce_manifest_and_schema_envelopes() {
        let (_, receipt) = encoded_stream(Vec::new());
        let manifest = receipt.manifest_sha256().unwrap();
        let response = PolicyProjectionResponse::RelationshipMap(Vec::new());
        let mut envelope = WorkerResponseEnvelope {
            schema_version: WORKER_SCHEMA_VERSION,
            request_nonce: receipt.request_nonce,
            manifest_sha256: manifest,
            response,
        };
        assert!(validate_worker_envelope_binding(&receipt, manifest, &envelope).is_ok());

        envelope.schema_version += 1;
        assert!(validate_worker_envelope_binding(&receipt, manifest, &envelope).is_err());
        envelope.schema_version = WORKER_SCHEMA_VERSION;
        envelope.request_nonce[0] ^= 0xff;
        assert!(validate_worker_envelope_binding(&receipt, manifest, &envelope).is_err());
        envelope.request_nonce = receipt.request_nonce;
        envelope.manifest_sha256[0] ^= 0xff;
        assert!(validate_worker_envelope_binding(&receipt, manifest, &envelope).is_err());
    }

    #[test]
    fn parent_rejects_wrong_or_over_cardinality_worker_responses() {
        let request = PolicyProjectionRequest::RelationshipMap { limit: 1 };
        assert!(validate_worker_response(
            &request,
            &PolicyProjectionResponse::Commitments(Vec::new())
        )
        .is_err());
        assert!(validate_worker_response(
            &request,
            &PolicyProjectionResponse::RelationshipMap(vec![person_summary(), person_summary()])
        )
        .is_err());
        assert!(validate_worker_response(
            &request,
            &PolicyProjectionResponse::RelationshipMap(vec![person_summary()])
        )
        .is_ok());
    }

    #[test]
    fn response_preflight_rejects_hostile_cardinality_before_typed_decode() {
        let mut response = br#"{"operation":"parakeet_boost_phrases","value":["#.to_vec();
        for _ in 0..=MAX_WORKER_RESPONSE_STRUCTURAL_UNITS {
            response.extend_from_slice(br#"","#);
        }
        response.extend_from_slice(br#""]}"#);
        assert!(preflight_worker_response_allocation(&response).is_err());
    }
}
