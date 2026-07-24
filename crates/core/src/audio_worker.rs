//! Product boundary for authenticated private-audio helper execution.
//!
//! Release and stable development app bundles use the embedded macOS XPC
//! service. Standalone/source/ad-hoc binaries that cannot prove the service
//! authority fail closed and retain the in-process Whisper fallback.

#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::OnceLock;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
const EMBEDDED_AUDIO_WORKER_CDHASH_PREFIX: &[u8] = b"MINUTES_AUDIO_WORKER_CDHASH_V1=";
#[cfg(target_os = "macos")]
#[used]
static EMBEDDED_AUDIO_WORKER_CDHASH: &[u8] =
    b"MINUTES_AUDIO_WORKER_CDHASH_V1=0000000000000000000000000000000000000000";

#[cfg(target_os = "macos")]
#[derive(Clone)]
struct MacAudioWorkerAuthority {
    bundle: PathBuf,
    cdhash: [u8; 20],
    trusted_distribution: bool,
}

#[cfg(target_os = "macos")]
static AUDIO_WORKER_AUTHORITY: OnceLock<MacAudioWorkerAuthority> = OnceLock::new();

#[cfg(target_os = "macos")]
fn app_bundle_for_current_executable() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let macos = executable.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    (bundle.extension().and_then(|value| value.to_str()) == Some("app"))
        .then(|| bundle.to_path_buf())
}

#[cfg(target_os = "macos")]
fn bind_authority(bundle: PathBuf) -> Result<MacAudioWorkerAuthority, String> {
    let service = bundle
        .join("Contents")
        .join("XPCServices")
        .join("com.useminutes.audio-worker.xpc");
    let executable = service
        .join("Contents")
        .join("MacOS")
        .join("minutes-audio-worker");
    if !executable.is_file() {
        return Err("private audio XPC executable is unavailable".into());
    }
    let manifest = bundle
        .join("Contents")
        .join("Resources")
        .join("minutes-audio-worker.cdhash");
    let encoded = std::fs::read_to_string(&manifest)
        .map_err(|_| "private audio helper integrity manifest is unavailable".to_string())?;
    let encoded = encoded
        .strip_suffix('\n')
        .ok_or_else(|| "private audio helper integrity manifest is malformed".to_string())?;
    let embedded = EMBEDDED_AUDIO_WORKER_CDHASH
        .strip_prefix(EMBEDDED_AUDIO_WORKER_CDHASH_PREFIX)
        .ok_or_else(|| "private audio helper authority marker is malformed".to_string())?;
    let cdhash = verify_worker_cdhash_binding(encoded.as_bytes(), embedded)?;
    Ok(MacAudioWorkerAuthority {
        bundle,
        cdhash,
        trusted_distribution: crate::macos_audio_xpc::current_process_is_trusted_distribution(),
    })
}

#[cfg(target_os = "macos")]
fn authority() -> Result<MacAudioWorkerAuthority, String> {
    if let Some(authority) = AUDIO_WORKER_AUTHORITY.get() {
        return Ok(authority.clone());
    }
    if !crate::macos_audio_xpc::peer_requirement_api_available() {
        return Err("authenticated private-audio XPC is unavailable on this macOS version".into());
    }
    let bundle = app_bundle_for_current_executable()
        .ok_or_else(|| "private audio transport requires an installed Minutes app".to_string())?;
    let authority = bind_authority(bundle)?;
    let _ = AUDIO_WORKER_AUTHORITY.set(authority.clone());
    Ok(authority)
}

/// Whether this process can prove the exact embedded audio service before
/// sending private bytes.
pub fn secure_transport_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        authority().is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
pub(crate) struct PrivateAudioChildSpec {
    pub(crate) executable: PathBuf,
    pub(crate) arguments: Vec<String>,
    pub(crate) attachments: Vec<PathBuf>,
    pub(crate) wall_clock: Duration,
    pub(crate) max_stdout: usize,
    pub(crate) max_stderr: usize,
}

#[cfg(target_os = "macos")]
pub(crate) fn run_private_audio_child(
    spec: PrivateAudioChildSpec,
    mut audio: impl std::io::Read + std::io::Seek,
) -> Result<crate::macos_audio_xpc::AudioChildOutput, String> {
    let authority = authority()?;
    let executable_cdhash = crate::macos_audio_xpc::static_code_cdhash(&spec.executable)?;
    let audio_len = crate::macos_audio_child::private_reader_len(&mut audio)
        .map_err(|_| "private audio input length could not be established".to_string())?;
    audio
        .rewind()
        .map_err(|_| "private audio input could not be rewound".to_string())?;
    let attachments = spec
        .attachments
        .iter()
        .map(|path| open_exact_attachment(path))
        .collect::<Result<Vec<_>, _>>()?;
    let wall_clock_ms = u64::try_from(spec.wall_clock.as_millis())
        .map_err(|_| "private audio child wall clock exceeds this platform".to_string())?;
    let request = crate::macos_audio_xpc::AudioChildRequest {
        metadata: crate::macos_audio_xpc::AudioChildMetadata {
            schema_version: 1,
            executable: spec.executable,
            executable_cdhash,
            arguments: spec.arguments,
            environment: child_environment(),
            audio_len,
            attachment_count: attachments.len(),
            max_stdout: spec.max_stdout,
            max_stderr: spec.max_stderr,
            wall_clock_ms,
        },
        attachments,
    };
    crate::macos_audio_xpc::run(
        &authority.bundle,
        &authority.cdhash,
        authority.trusted_distribution,
        request,
        audio,
    )
}

#[cfg(target_os = "macos")]
fn open_exact_attachment(path: &Path) -> Result<std::fs::File, String> {
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "private audio attachment path contains a NUL byte".to_string())?;
    let descriptor = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if descriptor < 0 {
        return Err("private audio attachment could not be opened exactly".into());
    }
    let file = unsafe { std::fs::File::from_raw_fd(descriptor) };
    let metadata = file
        .metadata()
        .map_err(|_| "private audio attachment metadata is unavailable".to_string())?;
    if !metadata.is_file() {
        return Err("private audio attachment is not a regular file".into());
    }
    Ok(file)
}

#[cfg(target_os = "macos")]
fn child_environment() -> Vec<(String, String)> {
    ["HOME", "TMPDIR", "PATH", "LANG", "LC_ALL", "LC_CTYPE"]
        .into_iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_string(), value))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn verify_worker_cdhash_binding(manifest: &[u8], embedded: &[u8]) -> Result<[u8; 20], String> {
    let manifest = decode_worker_cdhash(
        manifest,
        "private audio helper integrity manifest is malformed",
    )?;
    let embedded = decode_worker_cdhash(
        embedded,
        "private audio helper executable authority is unavailable",
    )?;
    if manifest == embedded {
        Ok(manifest)
    } else {
        Err("private audio helper authority does not match this application".into())
    }
}

#[cfg(target_os = "macos")]
fn decode_worker_cdhash(
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
        let pair = std::str::from_utf8(&encoded[index * 2..index * 2 + 2])
            .map_err(|_| malformed_message.to_string())?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| malformed_message.to_string())?;
    }
    Ok(cdhash)
}

#[cfg(target_os = "macos")]
pub fn run_xpc_service_main() -> ! {
    crate::macos_audio_xpc::run_service_main()
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn worker_cdhash_binding_is_exact_lowercase_and_nonzero() {
        let encoded = b"1234567890abcdef1234567890abcdef12345678";
        assert_eq!(
            verify_worker_cdhash_binding(encoded, encoded)
                .unwrap()
                .len(),
            20
        );
        assert!(
            verify_worker_cdhash_binding(encoded, b"1234567890abcdef1234567890abcdef12345679")
                .is_err()
        );
        assert!(decode_worker_cdhash(b"0000000000000000000000000000000000000000", "bad").is_err());
        assert!(decode_worker_cdhash(b"1234567890ABCDEF1234567890ABCDEF12345678", "bad").is_err());
    }
}
