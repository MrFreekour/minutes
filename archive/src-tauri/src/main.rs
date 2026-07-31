#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use minutes_archive_convert::{
    run_worker_process as run_convert_worker, BoundedConverter,
    WORKER_MARKER as CONVERT_WORKER_MARKER,
};
use minutes_archive_core::retrieval::{LegalSearchResponse, VaultId};
use minutes_archive_core::vault::{
    build_authorized_document_vault, AuthorizedDocumentVault, DocumentVaultBuildReport,
    DocumentVaultLimits,
};
use minutes_archive_core::{
    approve_roots, scan_approved_roots, validate_approved_roots, ApprovedRoot, CensusLimits,
    CensusReport, CensusStatus,
};
use minutes_archive_semantic::{
    run_worker_process as run_semantic_worker, BoundedSemanticEngine,
    WORKER_MARKER as SEMANTIC_WORKER_MARKER,
};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::State;
use tauri_plugin_dialog::DialogExt;

#[derive(Debug)]
struct ApprovedLocation {
    id: u64,
    root: ApprovedRoot,
}

#[derive(Debug, Default)]
struct ScanControl {
    running: bool,
    cancelled: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Default)]
struct SessionState {
    locations: Vec<ApprovedLocation>,
    last_report: Option<CensusReport>,
    text_vault: Option<AuthorizedDocumentVault>,
    scan: ScanControl,
}

#[derive(Debug)]
struct ArchiveState {
    session: Mutex<SessionState>,
    next_location_id: AtomicU64,
}

impl Default for ArchiveState {
    fn default() -> Self {
        Self {
            session: Mutex::new(SessionState::default()),
            next_location_id: AtomicU64::new(1),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocationSummary {
    id: u64,
    label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapState {
    locations: Vec<LocationSummary>,
    scan_running: bool,
    report: Option<CensusReport>,
    text_vault_report: Option<DocumentVaultBuildReport>,
}

fn lock_error() -> String {
    "Minutes Archive could not access its private session state.".to_string()
}

fn safe_census_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn location_summaries(locations: &[ApprovedLocation]) -> Vec<LocationSummary> {
    locations
        .iter()
        .enumerate()
        .map(|(index, location)| LocationSummary {
            id: location.id,
            label: format!("Approved location {}", index + 1),
        })
        .collect()
}

fn ensure_scan_idle(session: &SessionState) -> Result<(), String> {
    if session.scan.running {
        return Err("Wait for the current census to finish or cancel it first.".to_string());
    }
    Ok(())
}

#[tauri::command]
fn archive_bootstrap(state: State<'_, ArchiveState>) -> Result<BootstrapState, String> {
    let session = state.session.lock().map_err(|_| lock_error())?;
    Ok(BootstrapState {
        locations: location_summaries(&session.locations),
        scan_running: session.scan.running,
        report: session.last_report.clone(),
        text_vault_report: session
            .text_vault
            .as_ref()
            .map(|vault| vault.build_report().clone()),
    })
}

#[tauri::command]
async fn choose_archive_locations(
    app: tauri::AppHandle,
    state: State<'_, ArchiveState>,
) -> Result<Vec<LocationSummary>, String> {
    {
        let session = state.session.lock().map_err(|_| lock_error())?;
        ensure_scan_idle(&session)?;
    }
    let selected = app
        .dialog()
        .file()
        .set_title("Choose archive locations")
        .blocking_pick_folders();
    let Some(selected) = selected else {
        let session = state.session.lock().map_err(|_| lock_error())?;
        return Ok(location_summaries(&session.locations));
    };

    let selected = selected
        .into_iter()
        .map(|path| {
            path.into_path()
                .map_err(|_| "The selected location is not a local folder.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let new_roots = approve_roots(&selected).map_err(safe_census_error)?;

    let mut session = state.session.lock().map_err(|_| lock_error())?;
    ensure_scan_idle(&session)?;
    let mut combined = session
        .locations
        .iter()
        .map(|location| location.root.clone())
        .collect::<Vec<_>>();
    combined.extend(new_roots.iter().cloned());
    validate_approved_roots(&combined).map_err(safe_census_error)?;

    for root in new_roots {
        session.locations.push(ApprovedLocation {
            id: state.next_location_id.fetch_add(1, Ordering::Relaxed),
            root,
        });
    }
    session.last_report = None;
    session.text_vault = None;
    Ok(location_summaries(&session.locations))
}

#[tauri::command]
fn remove_archive_location(
    location_id: u64,
    state: State<'_, ArchiveState>,
) -> Result<Vec<LocationSummary>, String> {
    let mut session = state.session.lock().map_err(|_| lock_error())?;
    ensure_scan_idle(&session)?;
    let before = session.locations.len();
    session
        .locations
        .retain(|location| location.id != location_id);
    if session.locations.len() == before {
        return Err("That approved location is no longer available.".to_string());
    }
    session.last_report = None;
    session.text_vault = None;
    Ok(location_summaries(&session.locations))
}

#[tauri::command]
async fn run_archive_census(state: State<'_, ArchiveState>) -> Result<CensusReport, String> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let roots = {
        let mut session = state.session.lock().map_err(|_| lock_error())?;
        if session.locations.is_empty() {
            return Err("Choose at least one archive location first.".to_string());
        }
        if session.scan.running {
            return Err("A census is already running.".to_string());
        }
        session.scan.running = true;
        session.scan.cancelled = Some(Arc::clone(&cancelled));
        session.last_report = None;
        session.text_vault = None;
        session
            .locations
            .iter()
            .map(|location| location.root.clone())
            .collect::<Vec<_>>()
    };

    let scan_result = tauri::async_runtime::spawn_blocking(move || {
        scan_approved_roots(&roots, CensusLimits::default(), &cancelled)
    })
    .await
    .map_err(|_| "The private census worker stopped unexpectedly.".to_string());

    {
        let mut session = state.session.lock().map_err(|_| lock_error())?;
        session.scan.running = false;
        session.scan.cancelled = None;
    }

    let report = scan_result?.map_err(safe_census_error)?;
    if report.status != CensusStatus::Cancelled {
        state.session.lock().map_err(|_| lock_error())?.last_report = Some(report.clone());
    }
    Ok(report)
}

#[tauri::command]
fn cancel_archive_census(state: State<'_, ArchiveState>) -> Result<bool, String> {
    let session = state.session.lock().map_err(|_| lock_error())?;
    let Some(cancelled) = &session.scan.cancelled else {
        return Ok(false);
    };
    cancelled.store(true, Ordering::Release);
    Ok(true)
}

fn refuse_link_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("The report destination cannot be a symbolic link.".to_string())
        }
        Ok(metadata) if !metadata.is_file() => {
            Err("The report destination must be a regular file.".to_string())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("The report destination is unavailable.".to_string()),
    }
}

fn write_private_report(path: &Path, report: &CensusReport) -> Result<(), String> {
    refuse_link_target(path)?;
    let json = serde_json::to_vec_pretty(report)
        .map_err(|_| "Minutes Archive could not prepare the aggregate report.".to_string())?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Minutes Archive could not save the aggregate report.".to_string())?;
    file.write_all(&json)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|_| "Minutes Archive could not finish saving the aggregate report.".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| {
                "Minutes Archive could not protect the aggregate report permissions.".to_string()
            })?;
    }
    Ok(())
}

#[tauri::command]
async fn export_archive_census(
    app: tauri::AppHandle,
    state: State<'_, ArchiveState>,
) -> Result<bool, String> {
    let report = {
        let session = state.session.lock().map_err(|_| lock_error())?;
        ensure_scan_idle(&session)?;
        session
            .last_report
            .clone()
            .ok_or_else(|| "Run a complete census before exporting a report.".to_string())?
    };
    let selected = app
        .dialog()
        .file()
        .set_title("Save aggregate archive census")
        .set_file_name("archive-census.json")
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(false);
    };
    let path = selected
        .into_path()
        .map_err(|_| "The report destination is not a local file.".to_string())?;
    write_private_report(&path, &report)?;
    Ok(true)
}

#[tauri::command]
async fn build_archive_text_vault(
    state: State<'_, ArchiveState>,
) -> Result<DocumentVaultBuildReport, String> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let roots = {
        let mut session = state.session.lock().map_err(|_| lock_error())?;
        if session.locations.is_empty() {
            return Err("Choose at least one archive location first.".to_string());
        }
        if session.last_report.is_none() {
            return Err(
                "Run and review the metadata-only census before opening any documents.".to_string(),
            );
        }
        if session.scan.running {
            return Err("Another private archive operation is already running.".to_string());
        }
        session.scan.running = true;
        session.scan.cancelled = Some(Arc::clone(&cancelled));
        session.text_vault = None;
        session
            .locations
            .iter()
            .map(|location| location.root.clone())
            .collect::<Vec<_>>()
    };

    let worker_executable = std::env::current_exe()
        .map_err(|_| "Minutes Archive could not bind its document converter.".to_string())?;
    let build_result = tauri::async_runtime::spawn_blocking(move || {
        let vault_id = VaultId::parse("local-private-vault")
            .map_err(|_| "Minutes Archive could not establish the private vault.".to_string())?;
        let converter =
            BoundedConverter::bind(&worker_executable).map_err(|error| error.to_string())?;
        let semantic_engine =
            BoundedSemanticEngine::bind(&worker_executable).map_err(|error| error.to_string())?;
        build_authorized_document_vault(
            vault_id,
            &roots,
            DocumentVaultLimits::default(),
            &cancelled,
            &converter,
            semantic_engine,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "The private document-index worker stopped unexpectedly.".to_string());

    {
        let mut session = state.session.lock().map_err(|_| lock_error())?;
        session.scan.running = false;
        session.scan.cancelled = None;
    }

    let vault = build_result??;
    let report = vault.build_report().clone();
    state.session.lock().map_err(|_| lock_error())?.text_vault = Some(vault);
    Ok(report)
}

#[tauri::command]
fn search_archive_text_vault(
    query: String,
    state: State<'_, ArchiveState>,
) -> Result<LegalSearchResponse, String> {
    let session = state.session.lock().map_err(|_| lock_error())?;
    ensure_scan_idle(&session)?;
    let vault = session.text_vault.as_ref().ok_or_else(|| {
        "Build the private text index before searching. No partial index was retained.".to_string()
    })?;
    vault
        .interpret_and_search(query)
        .map_err(|error| error.to_string())
}

fn main() {
    let mut arguments = std::env::args();
    let _program = arguments.next();
    let marker = arguments.next();
    if matches!(
        marker.as_deref(),
        Some(CONVERT_WORKER_MARKER | SEMANTIC_WORKER_MARKER)
    ) {
        let operation = arguments.next().unwrap_or_default();
        if arguments.next().is_some() {
            std::process::exit(64);
        }
        let status = match marker.as_deref() {
            Some(CONVERT_WORKER_MARKER) => run_convert_worker(&operation),
            Some(SEMANTIC_WORKER_MARKER) => run_semantic_worker(&operation),
            _ => unreachable!("worker marker was already validated"),
        };
        std::process::exit(status);
    }

    tauri::Builder::default()
        .manage(ArchiveState::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            archive_bootstrap,
            choose_archive_locations,
            remove_archive_location,
            run_archive_census,
            cancel_archive_census,
            export_archive_census,
            build_archive_text_vault,
            search_archive_text_vault,
        ])
        .run(tauri::generate_context!())
        .expect("Minutes Archive failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    fn synthetic_report(temp: &TempDir) -> CensusReport {
        let root = temp.path().join("approved");
        fs::create_dir(&root).expect("approved root");
        fs::write(
            root.join("Privileged Client Name.pdf"),
            b"SYNTHETIC_CONTENT_CANARY",
        )
        .expect("synthetic document");
        minutes_archive_core::scan_roots(&[root], CensusLimits::default(), &AtomicBool::new(false))
            .expect("synthetic report")
    }

    #[test]
    fn location_summaries_never_expose_paths() {
        let temp = TempDir::new().expect("temp");
        let client_alpha = temp.path().join("client-alpha");
        let client_beta = temp.path().join("client-beta");
        fs::create_dir(&client_alpha).expect("alpha");
        fs::create_dir(&client_beta).expect("beta");
        let mut roots =
            approve_roots(&[client_alpha, client_beta]).expect("approve synthetic roots");
        let locations = vec![
            ApprovedLocation {
                id: 7,
                root: roots.remove(0),
            },
            ApprovedLocation {
                id: 8,
                root: roots.remove(0),
            },
        ];
        let serialized =
            serde_json::to_string(&location_summaries(&locations)).expect("serialize summaries");
        assert_eq!(
            serialized,
            r#"[{"id":7,"label":"Approved location 1"},{"id":8,"label":"Approved location 2"}]"#
        );
        assert!(!serialized.contains("client-alpha"));
        assert!(!serialized.contains("client-beta"));
    }

    #[test]
    fn exported_report_is_private_and_contains_no_source_canaries() {
        let temp = TempDir::new().expect("temp");
        let report = synthetic_report(&temp);
        let output = temp.path().join("archive-census.json");
        write_private_report(&output, &report).expect("export");
        let exported = fs::read_to_string(&output).expect("read aggregate export");
        assert!(!exported.contains("Privileged Client Name"));
        assert!(!exported.contains("SYNTHETIC_CONTENT_CANARY"));
        assert!(!exported.contains(&temp.path().to_string_lossy().to_string()));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&output)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn exported_report_refuses_symbolic_link_destination() {
        let temp = TempDir::new().expect("temp");
        let report = synthetic_report(&temp);
        let real_output = temp.path().join("real.json");
        fs::write(&real_output, b"do not overwrite").expect("real output");
        let link_output = temp.path().join("linked.json");
        std::os::unix::fs::symlink(&real_output, &link_output).expect("link");

        assert_eq!(
            write_private_report(&link_output, &report),
            Err("The report destination cannot be a symbolic link.".to_string())
        );
        assert_eq!(
            fs::read(&real_output).expect("preserved output"),
            b"do not overwrite"
        );
    }
}
