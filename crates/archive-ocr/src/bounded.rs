//! Spawning the recogniser as its own process, one page at a time.
//!
//! Mirrors `BoundedConverter`, including the part that was learned the hard
//! way: the worker executes IN PLACE from the application bundle and is never
//! copied. A Developer ID signature with the hardened runtime is bound to its
//! bundle, so a copy fails validation and the kernel kills it, which made every
//! notarized build unable to run its workers at all. Identity is pinned with an
//! open descriptor instead, and checked before each launch.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::{OcrError, RecognizedPage, MAX_IMAGE_BYTES, MAX_TEXT_BYTES};

/// A dense page genuinely takes seconds; beyond this something is wrong.
const PAGE_DEADLINE: Duration = Duration::from_secs(90);

pub const WORKER_MARKER: &str = "--minutes-archive-ocr-worker-v1";

/// Device and inode of the pinned worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> FileIdentity {
    FileIdentity {
        device: 0,
        inode: 0,
    }
}

pub struct BoundedTranscriber {
    executable_path: PathBuf,
    executable: fs::File,
    executable_identity: FileIdentity,
}

impl std::fmt::Debug for BoundedTranscriber {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BoundedTranscriber([pinned worker executable])")
    }
}

impl BoundedTranscriber {
    pub fn bind(worker_executable: &Path) -> Result<Self, OcrError> {
        let canonical =
            fs::canonicalize(worker_executable).map_err(|_| OcrError::RecognizerUnavailable)?;
        let lexical =
            fs::symlink_metadata(&canonical).map_err(|_| OcrError::RecognizerUnavailable)?;
        if lexical.file_type().is_symlink() || !lexical.is_file() {
            return Err(OcrError::RecognizerUnavailable);
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let executable = options
            .open(&canonical)
            .map_err(|_| OcrError::RecognizerUnavailable)?;
        let metadata = executable
            .metadata()
            .map_err(|_| OcrError::RecognizerUnavailable)?;
        if !metadata.is_file() {
            return Err(OcrError::RecognizerUnavailable);
        }
        let transcriber = Self {
            executable_identity: file_identity(&metadata),
            executable_path: canonical,
            executable,
        };
        transcriber.verify_sandbox()?;
        Ok(transcriber)
    }

    fn verify_sandbox(&self) -> Result<(), OcrError> {
        self.verify_executable()?;
        let output = self.launch("sandbox-self-test", &[])?;
        if output.success {
            Ok(())
        } else {
            Err(OcrError::SecurityBoundaryUnavailable)
        }
    }

    /// The path must still lead to the inode this object pinned.
    fn verify_executable(&self) -> Result<(), OcrError> {
        let metadata = fs::symlink_metadata(&self.executable_path)
            .map_err(|_| OcrError::RecognizerUnavailable)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(OcrError::RecognizerUnavailable);
        }
        if file_identity(&metadata) != self.executable_identity {
            return Err(OcrError::RecognizerUnavailable);
        }
        let pinned = self
            .executable
            .metadata()
            .map_err(|_| OcrError::RecognizerUnavailable)?;
        if file_identity(&pinned) != self.executable_identity {
            return Err(OcrError::RecognizerUnavailable);
        }
        Ok(())
    }

    /// Read one page.
    pub fn transcribe(&self, image: &[u8]) -> Result<RecognizedPage, OcrError> {
        if image.is_empty() || image.len() > MAX_IMAGE_BYTES {
            return Err(OcrError::ImageRefused);
        }
        self.verify_executable()?;
        let output = self.launch("recognize", image)?;
        if !output.success {
            return Err(OcrError::MalformedImage);
        }
        serde_json::from_slice(&output.stdout).map_err(|_| OcrError::MalformedImage)
    }

    fn launch(&self, operation: &str, input: &[u8]) -> Result<WorkerOutput, OcrError> {
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
                    // Its own process group, so a page that will not finish can
                    // be killed without reaching anything else.
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let mut child = command
            .spawn()
            .map_err(|_| OcrError::RecognizerUnavailable)?;
        let mut stdin = child.stdin.take().ok_or(OcrError::RecognizerUnavailable)?;
        let stdout = child.stdout.take().ok_or(OcrError::RecognizerUnavailable)?;
        let image = input.to_vec();
        let writer = thread::spawn(move || {
            let result = stdin.write_all(&image).and_then(|_| stdin.flush());
            drop(stdin);
            result
        });
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take((MAX_TEXT_BYTES as u64).saturating_add(4096))
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });

        let deadline = Instant::now() + PAGE_DEADLINE;
        let status = loop {
            match child.try_wait().map_err(|_| OcrError::MalformedImage)? {
                Some(status) => break status,
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                None => {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(OcrError::OutputBudgetExceeded);
                }
            }
        };
        let _ = writer.join();
        let stdout = reader
            .join()
            .map_err(|_| OcrError::MalformedImage)?
            .map_err(|_| OcrError::MalformedImage)?;
        if stdout.len() > MAX_TEXT_BYTES + 4096 {
            return Err(OcrError::OutputBudgetExceeded);
        }
        Ok(WorkerOutput {
            success: status.success(),
            stdout,
        })
    }
}

struct WorkerOutput {
    success: bool,
    stdout: Vec<u8>,
}
