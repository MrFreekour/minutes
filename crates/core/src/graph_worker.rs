//! Supervised process boundary for policy-filtered graph projections.
//!
//! SQLite's page/cache/value limits bound important components, but they do
//! not cap all transient allocator use. Product-facing graph reads therefore
//! execute in a short-lived child with an OS address-space ceiling, a complete
//! process-tree deadline, bounded stdin/stdout/stderr, and one ordered corpus +
//! correction snapshot. The parent exposes captured stdout only after the
//! child has completed the journal's post-publication fence successfully.

use crate::config::Config;
use crate::graph::{PolicyProjectionRequest, PolicyProjectionResponse};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const WORKER_MARKER: &str = "MINUTES_POLICY_GRAPH_WORKER_V1";
const WORKER_SCHEMA_VERSION: u32 = 1;
const MAX_WORKER_REQUEST_BYTES: u64 = 256 * 1024;
const MAX_WORKER_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_WORKER_STDERR_BYTES: usize = 64 * 1024;
const WORKER_WALL_CLOCK: Duration = Duration::from_secs(45);
const WORKER_ADDRESS_SPACE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_WORKER_RESULT_ITEMS: usize = 10_000;
const MAX_WORKER_RESPONSE_STRUCTURAL_UNITS: usize = 1_000_000;
const MAX_WORKER_RESPONSE_STRING_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORKER_RESPONSE_SINGLE_STRING_BYTES: usize = 256 * 1024;
const MAX_WORKER_RESPONSE_DEPTH: usize = 64;
static WORKER_EXECUTABLE: OnceLock<crate::bounded_child::BoundExecutable> = OnceLock::new();

struct BoundedVecWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedVecWriter {
    fn new(limit: u64) -> Result<Self, String> {
        let limit = usize::try_from(limit)
            .map_err(|_| "graph worker byte budget exceeded this platform".to_string())?;
        Ok(Self {
            bytes: Vec::new(),
            limit,
        })
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedVecWriter {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        let next =
            self.bytes.len().checked_add(input.len()).ok_or_else(|| {
                std::io::Error::other("graph worker request byte budget overflowed")
            })?;
        if next > self.limit {
            return Err(std::io::Error::other(
                "graph worker request exceeded its byte budget",
            ));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkerEnvelope {
    schema_version: u32,
    output_dir: PathBuf,
    correction_state_dir: PathBuf,
    request: PolicyProjectionRequest,
}

fn valid_absolute_directory_path(path: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

/// Install a dedicated, already-packaged worker executable before application
/// threads start. The macOS desktop uses its bundled `minutes` CLI sidecar so
/// the address-space limit applies to a small non-GUI process rather than the
/// Tauri/WebKit host and its large shared-framework mappings.
pub fn install_policy_projection_worker_executable(path: PathBuf) -> Result<(), String> {
    if !valid_absolute_directory_path(&path) {
        return Err("policy graph worker executable path was invalid".into());
    }
    let executable = crate::bounded_child::BoundExecutable::bind(&path)
        .map_err(|_| "policy graph worker executable authority could not be bound".to_string())?;
    WORKER_EXECUTABLE
        .set(executable)
        .map_err(|_| "policy graph worker executable was already installed".to_string())
}

fn read_worker_request() -> Result<WorkerEnvelope, String> {
    let mut input = std::io::stdin().take(MAX_WORKER_REQUEST_BYTES + 1);
    let mut bytes = Vec::new();
    input
        .read_to_end(&mut bytes)
        .map_err(|_| "graph worker request could not be read".to_string())?;
    if bytes.len() as u64 > MAX_WORKER_REQUEST_BYTES {
        return Err("graph worker request exceeded its byte budget".into());
    }
    let envelope: WorkerEnvelope = serde_json::from_slice(&bytes)
        .map_err(|_| "graph worker request was malformed".to_string())?;
    if envelope.schema_version != WORKER_SCHEMA_VERSION
        || !valid_absolute_directory_path(&envelope.output_dir)
        || !valid_absolute_directory_path(&envelope.correction_state_dir)
    {
        return Err("graph worker request failed structural validation".into());
    }
    Ok(envelope)
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
pub fn maybe_run_policy_projection_worker() -> Option<i32> {
    let marker = std::env::var_os(WORKER_MARKER)?;
    std::env::remove_var(WORKER_MARKER);
    if marker != "1" {
        return Some(2);
    }
    let result = (|| -> Result<(), String> {
        // Windows std::process inherits ambient inheritable HANDLEs when it
        // redirects stdio. This immutable worker closes every process handle
        // except its three authenticated pipes before it reads corpus paths or
        // receives any other authority.
        close_inherited_windows_handles_before_authority()?;
        let envelope = read_worker_request()?;
        // Resolve every correction reader to the exact state namespace that
        // the parent serialized; do not rediscover HOME inside the child.
        std::env::set_var("MINUTES_HOME", &envelope.correction_state_dir);
        let config = Config {
            output_dir: envelope.output_dir,
            ..Config::default()
        };
        let stdout = std::io::stdout();
        let mut writer = stdout.lock();
        crate::graph::write_policy_projection_response(&config, &envelope.request, &mut writer)
            .map_err(|error| error.to_string())?;
        writer
            .flush()
            .map_err(|_| "graph worker response could not be flushed".to_string())?;
        Ok(())
    })();
    match result {
        Ok(()) => Some(0),
        Err(error) => {
            // Error text is deliberately bounded and contains no paths or
            // source bytes. The parent maps every nonzero exit to a generic
            // policy-projection failure.
            let message = error.chars().take(512).collect::<String>();
            eprintln!("policy graph worker failed: {message}");
            Some(1)
        }
    }
}

fn retain_safe_environment(command: &mut crate::bounded_child::BoundedCommand) {
    command.env_clear();
    for name in ["HOME", "TMPDIR", "TMP", "TEMP", "SystemRoot", "WINDIR"] {
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

/// Execute one policy projection in the hard-bounded worker and decode its
/// internal tagged response. No partial stdout is returned on timeout,
/// resource exhaustion, journal invalidation, or unsuccessful exit.
pub fn run_policy_projection_worker(
    config: &Config,
    request: PolicyProjectionRequest,
) -> Result<PolicyProjectionResponse, String> {
    let output_dir = config
        .output_dir
        .canonicalize()
        .map_err(|_| "policy graph corpus directory could not be verified".to_string())?;
    let correction_state_dir = crate::overlays::correction_state_dir();
    if !valid_absolute_directory_path(&correction_state_dir) {
        return Err("policy graph correction namespace could not be verified".into());
    }
    let envelope = WorkerEnvelope {
        schema_version: WORKER_SCHEMA_VERSION,
        output_dir,
        correction_state_dir: correction_state_dir.clone(),
        request,
    };
    let mut input = BoundedVecWriter::new(MAX_WORKER_REQUEST_BYTES)?;
    serde_json::to_writer(&mut input, &envelope)
        .map_err(|_| "policy graph worker request could not be encoded".to_string())?;
    let input = input.finish();
    let executable = match WORKER_EXECUTABLE.get() {
        Some(executable) => executable.try_clone().map_err(|_| {
            "policy graph worker executable authority could not be cloned".to_string()
        })?,
        None => crate::bounded_child::BoundExecutable::current()
            .map_err(|_| "policy graph worker executable could not be resolved".to_string())?,
    };
    let mut command = crate::bounded_child::BoundedCommand::from_bound_executable(executable)
        .map_err(|_| "policy graph worker executable authority could not be bound".to_string())?;
    retain_safe_environment(&mut command);
    command
        .env(WORKER_MARKER, "1")
        .env("MINUTES_HOME", correction_state_dir)
        .address_space_limit(WORKER_ADDRESS_SPACE_BYTES)
        .single_process()
        .close_extra_descriptors();
    let run = crate::bounded_child::run(
        &mut command,
        Some(Box::new(std::io::Cursor::new(input))),
        crate::bounded_child::StdoutTarget::Capture {
            max_bytes: MAX_WORKER_RESPONSE_BYTES,
        },
        crate::bounded_child::ChildBudget {
            wall_clock: WORKER_WALL_CLOCK,
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
    preflight_worker_response_allocation(&run.output.stdout)?;
    let response = serde_json::from_slice(&run.output.stdout)
        .map_err(|_| "policy graph worker returned an invalid response".to_string())?;
    validate_worker_response(&envelope.request, &response)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn worker_paths_reject_relative_and_parent_traversal() {
        assert!(!valid_absolute_directory_path(Path::new("meetings")));
        assert!(!valid_absolute_directory_path(Path::new("/tmp/../private")));
        assert!(valid_absolute_directory_path(Path::new("/tmp/private")));
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
    fn bounded_request_writer_rejects_before_retaining_overflow_bytes() {
        let mut writer = BoundedVecWriter::new(8).unwrap();
        assert_eq!(writer.write(b"12345678").unwrap(), 8);
        assert!(writer.write(b"9").is_err());
        assert_eq!(writer.finish(), b"12345678");
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
