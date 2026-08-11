#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use minutes_archive_convert::{
    run_worker_process as run_convert_worker, BoundedConverter,
    WORKER_MARKER as CONVERT_WORKER_MARKER,
};
use minutes_archive_core::retrieval::{LegalSearchResponse, VaultId};
use minutes_archive_core::vault::BuildProgress;
use minutes_archive_core::vault::{
    build_authorized_document_vault, AuthorizedDocumentVault, DocumentVaultBuildReport,
    DocumentVaultLimits, ExcludedFolder,
};
use minutes_archive_core::{
    authorize_roots, portable_identity_for, reduce_approved_roots, relative_path_within,
    scan_approved_roots, validate_approved_roots, ApprovedRoot, CensusLimits, CensusReport,
    CensusStatus, FileIdentity,
};
use minutes_archive_ocr::{BoundedTranscriber, WORKER_MARKER as OCR_WORKER_MARKER};
use minutes_archive_semantic::{
    run_worker_process as run_semantic_worker, BoundedSemanticEngine,
    WORKER_MARKER as SEMANTIC_WORKER_MARKER,
};
use minutes_archive_worker_control::LiveWorkerProcesses;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::UpdaterExt;

const NATIVE_LIFECYCLE_SELFTEST_MARKER: &str = "--archive-native-lifecycle-selftest";
/// Proves the SIGNED application can actually run its own workers.
///
/// Everything else was verified on a build that could not do this. A
/// Developer ID signature with the hardened runtime is bound to its bundle, so
/// when the workers were copied to a temp directory and run from there, the
/// copy failed validation and the kernel killed it -- every notarized build
/// was unable to index a single document. Signature, staple, Gatekeeper and
/// launch all passed, the window opened, and the first click on "Build
/// document pilot" failed. The gap was that local testing used an
/// ad-hoc-signed app, whose copy runs fine, and CI exercised the unsigned
/// build. This mode closes it: run it against the notarized artifact.
const SIGNED_WORKER_SELFTEST_MARKER: &str = "--archive-signed-worker-selftest";

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
    /// Folders inside approved locations that the build must not enter.
    ///
    /// Chosen through the same native panel as the locations themselves, so a
    /// folder path still never crosses into the webview in either direction --
    /// the interface asks for the panel and is told a count.
    exclusions: Vec<SessionExclusion>,
    last_report: Option<CensusReport>,
    text_vault: Option<AuthorizedDocumentVault>,
    scan: ScanControl,
}

/// The single network exception, and the thing that keeps it single.
///
/// Minutes Archive is network-denied everywhere it can be: both worker
/// processes run under a seatbelt profile carrying `(deny network*)`, and the
/// parent holds no entitlement it could use to reach anywhere. The updater
/// makes the parent the one exception, because the attorney this is built for
/// cannot be asked to notice a release and download the application again.
///
/// An exception kept by convention becomes a habit. So the window is closed by
/// the session's own state rather than by remembering not to call: the instant
/// anything of the operator's archive is in this process -- an approved folder,
/// a census, an index, a scan in flight -- every network operation is refused
/// for the rest of the session, and the refusal is what the interface shows.
#[derive(Debug, Default)]
struct NetworkWindow {
    /// Closed the moment this session takes on anything of the archive, and
    /// never reopened.
    ///
    /// Deliberately here rather than in `SessionState`: `purge_session`
    /// replaces that whole value with a default, and a latch a reset can clear
    /// is not a latch. Removing an approved location does not un-see it either
    /// -- approve, remove, then check would otherwise reopen the window.
    archive_seen: AtomicBool,
    /// Spent by the one permitted check.
    ///
    /// One-shot rather than rate-limited. A repeated check is a beacon whether
    /// or not it carries anything.
    check_spent: AtomicBool,
    /// Spent by the one permitted download, which happens only if the operator
    /// asks for it.
    download_spent: AtomicBool,
    /// True while either bounded network operation is alive.
    ///
    /// The webview disables folder controls during those operations, but the
    /// Rust boundary must also arbitrate a compromised or racing caller. A
    /// folder-panel command that arrives while this is true closes the network
    /// window permanently but does not open the panel; the operator can choose
    /// the folder after the request finishes, and no archive state can overlap
    /// the request.
    operation_in_flight: AtomicBool,
}

/// The one update this session may install, held between the check and consent.
///
/// Held rather than re-checked. Consent must not cost a second request to the
/// same file, and re-checking at install time would let the answer change
/// between what the operator was shown and what gets installed.
struct OfferedUpdate(tauri_plugin_updater::Update);

impl std::fmt::Debug for OfferedUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The version only. The rest of `Update` carries a download URL and a
        // signature that have no business in a debug line.
        formatter
            .debug_struct("OfferedUpdate")
            .field("version", &self.0.version)
            .finish()
    }
}

/// What the launch check found, and the update it may install.
#[derive(Debug, Default)]
struct UpdateSlot {
    report: UpdateReport,
    offered: Option<OfferedUpdate>,
}

/// What the interface is told about the update check.
///
/// Every state is visible: there is no branch here that checks quietly, and no
/// branch that installs without the operator asking. The only two values that
/// ever come from the network are `offered` and the presence of an update at
/// all -- both are semver strings the plugin has already parsed. Release notes
/// are deliberately not carried: they are remote-controlled prose, and an
/// attorney reading them inside a tool that promises to show only their own
/// documents is a worse trade than not showing them.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum UpdateReport {
    /// No check has run in this session yet.
    #[default]
    NotChecked,
    Checking,
    /// The check ran and this build is the current one.
    Current {
        installed: String,
    },
    /// The check ran and a newer signed build exists. Nothing is downloaded
    /// until the operator asks.
    Available {
        installed: String,
        offered: String,
    },
    /// The window was closed before the check could run. Not an error: this is
    /// the boundary doing its job, and it says so in those words.
    Refused {
        reason: String,
    },
    /// The check ran and could not complete. The application is unaffected.
    Unavailable {
        reason: String,
    },
    Installing,
    /// A signed build was verified and written into place.
    Installed {
        offered: String,
    },
    /// Verification or replacement failed. The current app remains intact and
    /// the notarized DMG is the explicit recovery path.
    InstallFailed {
        offered: String,
    },
}

/// Shown when the session has already seen the operator's archive.
const WINDOW_CLOSED_REFUSAL: &str =
    "Minutes Archive has already opened this session's archive, so it will not \
     use the network again until you quit and reopen it.";

/// Shown when one of the two bounded update steps is attempted twice.
const WINDOW_SPENT_REFUSAL: &str =
    "Minutes Archive allows each update step once per session, and this step \
     already ran.";

/// Shown when a folder-panel command races the bounded update operation.
const NETWORK_BUSY_REFUSAL: &str =
    "Wait for the update check or installation to finish, then choose folders.";

/// Shown when the request could not be completed, for any reason at all.
///
/// One sentence for offline, endpoint down, DNS failure, malformed JSON, and a
/// signature that does not verify. Distinguishing them would tell the operator
/// nothing they can act on, and the raw error text is the one place a URL or a
/// local path could leak into the interface.
const UPDATE_UNAVAILABLE: &str =
    "Minutes Archive could not complete the update check. No archive-derived \
     data, query string, or request body was sent, and nothing changed.";

#[derive(Debug)]
struct ArchiveState {
    session: Mutex<SessionState>,
    next_location_id: AtomicU64,
    /// Every converter, recogniser, and semantic worker alive in this process.
    /// Purge kills their process groups before the desktop process exits.
    live_workers: LiveWorkerProcesses,
    /// The single network exception. See `NetworkWindow`.
    network: NetworkWindow,
    /// What the launch check found, and the update it may install.
    update: Mutex<UpdateSlot>,
    /// Snapshot directories of workers that are currently alive.
    ///
    /// During a vault build the converter and engine live inside a blocking
    /// task, not in `session`, so the close handler owns nothing to drop and
    /// `exit(0)` leaves both 40 MB snapshots behind. Registering the paths at
    /// creation lets the purge reclaim them whichever way the app exits.
    live_snapshots: Arc<Mutex<Vec<std::path::PathBuf>>>,
    /// Live counts for the build in flight, polled by the interface.
    ///
    /// A build over tens of thousands of documents with no visible progress is
    /// indistinguishable from a hung one. Counts only -- no filename, no path,
    /// nothing derived from a document -- so this cannot become a channel for
    /// anything but the two numbers.
    build_progress: Mutex<Arc<BuildProgress>>,
}

impl Default for ArchiveState {
    fn default() -> Self {
        Self {
            session: Mutex::new(SessionState::default()),
            next_location_id: AtomicU64::new(1),
            live_workers: LiveWorkerProcesses::default(),
            network: NetworkWindow::default(),
            update: Mutex::new(UpdateSlot::default()),
            live_snapshots: Arc::new(Mutex::new(Vec::new())),
            build_progress: Mutex::new(Arc::new(BuildProgress::default())),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocationSummary {
    id: u64,
    label: String,
    /// What this location contributed to the last census, or `None` before
    /// one has run.
    ///
    /// The rows are deliberately indistinguishable by name, which leaves an
    /// owner with several matters approved unable to tell them apart at a
    /// glance. Counts separate them without naming anything: "4,102 items"
    /// beside "12 items" is the difference between a matter archive and a
    /// folder of exhibits, and neither number says whose.
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    regular_file_bytes: Option<u64>,
}

/// The result of a folder-picker round.
///
/// `folded` counts the chosen folders that some approved location already
/// covers. They are not failures and not losses -- every document beneath them
/// is still indexed through the containing location -- but the owner picked
/// them deliberately and is owed an account of where they went.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocationChoice {
    locations: Vec<LocationSummary>,
    folded: usize,
    /// Skipped folders cancelled because the owner explicitly chose them.
    ///
    /// Choosing a folder in the picker is the owner's latest word on it, and it
    /// beats an older skip. The count is reported because the skip was also
    /// deliberate once, and its silent disappearance would be the same lie in
    /// the other direction.
    unskipped: usize,
    /// Skipped folders forgotten because the location holding them was folded.
    ///
    /// Silently dropping these reads *more* than the owner asked for, not less,
    /// so nothing is lost from the index -- but the folder they pointed at was
    /// excluded on purpose. On the archive this pilot is for that is 2,873
    /// screenshots and roughly seventeen minutes of text recognition arriving
    /// unannounced, in an index they believed excluded them.
    forgotten_skips: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapState {
    /// Which build this is, for a support conversation.
    ///
    /// The version alone does not identify a build: two candidates carried the
    /// same version and one of them could not index a single document. The
    /// short digest of the running executable is what the signed provenance
    /// record already names a candidate by, so it is the thing to ask for when
    /// someone reports a problem -- and it is computed from the file on disk
    /// rather than compiled in, so it cannot claim to be a build it is not.
    build_identity: String,
    locations: Vec<LocationSummary>,
    scan_running: bool,
    report: Option<CensusReport>,
    text_vault_report: Option<DocumentVaultBuildReport>,
    /// Carried here so a reloaded webview shows the result of the check that
    /// already ran instead of asking for another one. The window would refuse
    /// the second request anyway; this is what stops the interface from having
    /// to ask to find that out.
    update: UpdateReport,
}

/// Version plus a short digest of the executable that is actually running.
///
/// Reads nothing but its own binary and reaches no network. Update traffic is
/// handled separately by the one-shot, launch-only window below; identifying
/// the build itself never depends on that window or on a remote service.
fn build_identity() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let digest = std::env::current_exe()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .map(|bytes| {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(&bytes);
            digest
                .iter()
                .take(6)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .unwrap_or_else(|| "unidentified".to_string());
    format!("v{version} · build {digest}")
}

fn lock_error() -> String {
    "Minutes Archive could not access its private session state.".to_string()
}

fn safe_census_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn location_summaries(locations: &[ApprovedLocation]) -> Vec<LocationSummary> {
    location_summaries_with(locations, None)
}

/// Location rows, with the last census's per-location totals when there is a
/// census to draw them from.
///
/// Totals are positional against the approved roots, exactly as the census
/// produced them. A report from a different set of locations -- one taken
/// before a folder was added or removed -- would misattribute every row, so a
/// length mismatch drops the numbers rather than showing wrong ones. A row
/// without a count is a smaller failure than a row with someone else's.
fn location_summaries_with(
    locations: &[ApprovedLocation],
    report: Option<&CensusReport>,
) -> Vec<LocationSummary> {
    let totals = report
        .map(|report| report.per_location.as_slice())
        .filter(|totals| totals.len() == locations.len());
    locations
        .iter()
        .enumerate()
        .map(|(index, location)| {
            let totals = totals.and_then(|totals| totals.get(index));
            LocationSummary {
                id: location.id,
                label: format!("Approved location {}", index + 1),
                artifacts: totals.map(|totals| totals.artifacts),
                regular_file_bytes: totals.map(|totals| totals.regular_file_bytes),
            }
        })
        .collect()
}

fn ensure_scan_idle(session: &SessionState) -> Result<(), String> {
    if session.scan.running {
        return Err("Wait for the current census to finish or cancel it first.".to_string());
    }
    Ok(())
}

/// True once this session holds anything that came from the operator's archive.
///
/// Reads the live session rather than a flag someone has to remember to set, so
/// a field added to `SessionState` later -- another cache, another report --
/// closes the window even if whoever adds it never thinks about the updater.
/// The latch in `NetworkWindow` covers the other direction: state that was
/// here and has since been cleared.
fn session_has_seen_archive(session: &SessionState) -> bool {
    !session.locations.is_empty()
        || session.last_report.is_some()
        || session.text_vault.is_some()
        || session.scan.running
}

/// Closes the network window for the rest of the session.
fn close_network_window(state: &ArchiveState) {
    state.network.archive_seen.store(true, Ordering::SeqCst);
}

/// Starts an archive interaction only when no updater request is alive.
///
/// Setting `archive_seen` first makes the race one-sided: either this call wins
/// and the network claim refuses, or the network claim is already alive and
/// this call refuses before opening the native panel. They can never both
/// proceed.
fn begin_archive_interaction(state: &ArchiveState) -> Result<(), String> {
    close_network_window(state);
    if state.network.operation_in_flight.load(Ordering::SeqCst) {
        return Err(NETWORK_BUSY_REFUSAL.to_string());
    }
    Ok(())
}

/// Releases the exclusive updater-operation claim on every return path.
struct NetworkOperationClaim<'a> {
    in_flight: &'a AtomicBool,
}

impl Drop for NetworkOperationClaim<'_> {
    fn drop(&mut self) {
        self.in_flight.store(false, Ordering::SeqCst);
    }
}

/// Refuses unless this is still a launch-time, archive-free session.
///
/// Every network operation in the application goes through here, and there is
/// no second path. `spend` is the one-shot latch for the operation being
/// claimed, so an interface that is compromised -- or merely looping on a bug
/// -- cannot turn the launch check or consented download into a poll.
fn claim_network_window<'a>(
    state: &'a ArchiveState,
    spend: &AtomicBool,
) -> Result<NetworkOperationClaim<'a>, String> {
    if state.network.archive_seen.load(Ordering::SeqCst) {
        return Err(WINDOW_CLOSED_REFUSAL.to_string());
    }
    state
        .network
        .operation_in_flight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| NETWORK_BUSY_REFUSAL.to_string())?;
    let claim = NetworkOperationClaim {
        in_flight: &state.network.operation_in_flight,
    };

    // `archive_seen` may have changed between the optimistic read above and
    // the exclusive claim. A folder command that raced us sets it before it
    // checks this claim, so this second read decides which side proceeds.
    if state.network.archive_seen.load(Ordering::SeqCst) {
        return Err(WINDOW_CLOSED_REFUSAL.to_string());
    }
    {
        let session = state.session.lock().map_err(|_| lock_error())?;
        if session_has_seen_archive(&session) {
            // Observing it is enough to close it permanently. Whatever put the
            // archive into this session, the window does not reopen when it
            // goes away again.
            close_network_window(state);
            return Err(WINDOW_CLOSED_REFUSAL.to_string());
        }
    }
    spend
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| WINDOW_SPENT_REFUSAL.to_string())?;
    // One last read closes the interleaving where an archive interaction set
    // its latch while the live session was being inspected. If it set the
    // latch after this read, it necessarily observes `operation_in_flight` and
    // refuses before opening a panel.
    if state.network.archive_seen.load(Ordering::SeqCst) {
        return Err(WINDOW_CLOSED_REFUSAL.to_string());
    }
    Ok(claim)
}

/// Records what the operator should see, and returns it for the same reason.
///
/// Leaves any held offer alone. Reporting a refusal is not a reason to discard
/// an offer the operator was already shown: the gate decides whether it can be
/// installed, and it decides that at install time.
fn publish_update_report(state: &ArchiveState, report: UpdateReport) -> UpdateReport {
    if let Ok(mut slot) = state.update.lock() {
        slot.report = report.clone();
    }
    report
}

/// Holds the offer the operator may consent to, alongside the report naming it.
fn publish_update_offer(
    state: &ArchiveState,
    report: UpdateReport,
    offered: OfferedUpdate,
) -> UpdateReport {
    if let Ok(mut slot) = state.update.lock() {
        slot.report = report.clone();
        slot.offered = Some(offered);
    }
    report
}

/// What the launch check found, for an interface that has just loaded.
#[tauri::command]
fn archive_update_report(state: State<'_, ArchiveState>) -> UpdateReport {
    state
        .update
        .lock()
        .map(|slot| slot.report.clone())
        .unwrap_or_default()
}

/// The one automatic network request Minutes Archive is allowed to make.
///
/// A plain GET of a static JSON file. Nothing is appended to it: the configured
/// endpoint carries none of the `{{current_version}}`, `{{target}}` or
/// `{{arch}}` placeholders the plugin would otherwise substitute, `clear_headers`
/// removes anything a caller might have attached, and no identifier, count, or
/// value derived from a document exists in this process yet to send. The
/// request is made before the operator has approved a folder, so there is
/// nothing about their archive to leak even in principle.
///
/// Never returns `Err`. A refusal, an unreachable endpoint, a malformed file
/// and an offline Mac are all ordinary reports the interface renders, because
/// the application must work exactly as it does today when the check fails.
#[tauri::command]
async fn check_for_archive_update(
    app: tauri::AppHandle,
    state: State<'_, ArchiveState>,
) -> Result<UpdateReport, String> {
    let _network_claim = match claim_network_window(&state, &state.network.check_spent) {
        Ok(claim) => claim,
        Err(reason) => {
            return Ok(publish_update_report(
                &state,
                UpdateReport::Refused { reason },
            ));
        }
    };

    let installed = app.package_info().version.to_string();
    publish_update_report(&state, UpdateReport::Checking);

    // Keep launch usable when the release host is slow or unreachable. The
    // folder controls stay disabled while this request is alive so the app
    // cannot begin holding archive state while its automatic network request is
    // still in flight.
    let outcome = match app
        .updater_builder()
        .clear_headers()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(updater) => updater.check().await,
        Err(error) => Err(error),
    };

    Ok(match outcome {
        Ok(Some(mut update)) => {
            // The manifest check is deliberately short; the consented app
            // download is larger and gets its own bounded request lifetime.
            update.timeout = Some(std::time::Duration::from_secs(10 * 60));
            let offered = update.version.clone();
            publish_update_offer(
                &state,
                UpdateReport::Available { installed, offered },
                OfferedUpdate(update),
            )
        }
        Ok(None) => publish_update_report(&state, UpdateReport::Current { installed }),
        Err(_) => publish_update_report(
            &state,
            UpdateReport::Unavailable {
                reason: UPDATE_UNAVAILABLE.to_string(),
            },
        ),
    })
}

/// Downloads, verifies, and atomically swaps in the update the operator asked for.
///
/// `Update::download` verifies the minisign signature against the public key in
/// `tauri.conf.json` -- the same key the main Minutes application signs with --
/// before returning bytes. The pinned plugin's macOS installer is deliberately
/// not used: it moves the current app into a temporary backup and has failure
/// paths that can delete that backup. The replacement below is first extracted
/// and code-signature checked in a sibling directory, then exchanged with the
/// running bundle in one filesystem operation. A failed exchange changes
/// neither path.
#[tauri::command]
async fn install_archive_update(state: State<'_, ArchiveState>) -> Result<UpdateReport, String> {
    // The same window, for the same reasons. Consent does not reopen it: if the
    // operator approved a folder while the offer was on screen, the answer is
    // no, and quitting and reopening is the way to take it.
    let _network_claim = match claim_network_window(&state, &state.network.download_spent) {
        Ok(claim) => claim,
        Err(reason) => {
            return Ok(publish_update_report(
                &state,
                UpdateReport::Refused { reason },
            ));
        }
    };

    let offered = state
        .update
        .lock()
        .map_err(|_| lock_error())?
        .offered
        .take();
    let Some(offered) = offered else {
        return Ok(publish_update_report(
            &state,
            UpdateReport::Refused {
                reason: "No update was offered in this session.".to_string(),
            },
        ));
    };

    let version = offered.0.version.clone();
    publish_update_report(&state, UpdateReport::Installing);
    let downloaded = offered.0.download(|_, _| {}, || {}).await;
    Ok(
        match downloaded.and_then(|bytes| {
            install_verified_archive(&bytes).map_err(tauri_plugin_updater::Error::Io)
        }) {
            Ok(()) => publish_update_report(&state, UpdateReport::Installed { offered: version }),
            // Includes a signature or local code-signature failure. The atomic
            // exchange has not happened, so the current application is unchanged.
            Err(_) => {
                publish_update_report(&state, UpdateReport::InstallFailed { offered: version })
            }
        },
    )
}

#[cfg(target_os = "macos")]
fn running_app_bundle_path() -> std::io::Result<PathBuf> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let macos = executable
        .parent()
        .ok_or_else(|| std::io::Error::other("running executable has no parent"))?;
    let contents = macos
        .parent()
        .ok_or_else(|| std::io::Error::other("running executable is not in an app bundle"))?;
    let app = contents
        .parent()
        .ok_or_else(|| std::io::Error::other("running executable is not in an app bundle"))?;
    if macos.file_name().and_then(|name| name.to_str()) != Some("MacOS")
        || contents.file_name().and_then(|name| name.to_str()) != Some("Contents")
        || app.extension().and_then(|extension| extension.to_str()) != Some("app")
    {
        return Err(std::io::Error::other(
            "running executable is not in a macOS app bundle",
        ));
    }
    Ok(app.to_path_buf())
}

#[cfg(target_os = "macos")]
fn extract_update_archive(bytes: &[u8], destination: &Path) -> std::io::Result<PathBuf> {
    use flate2::read::GzDecoder;
    use std::ffi::OsStr;
    use std::io::Cursor;

    let mut archive = tar::Archive::new(GzDecoder::new(Cursor::new(bytes)));
    let expected_root = OsStr::new("Minutes Archive.app");
    let mut saw_payload = false;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut components = path.components();
        match components.next() {
            Some(std::path::Component::Normal(component)) if component == expected_root => {}
            _ => {
                return Err(std::io::Error::other(
                    "update archive has an unexpected top-level path",
                ));
            }
        }
        if components.any(|component| !matches!(component, std::path::Component::Normal(_))) {
            return Err(std::io::Error::other(
                "update archive contains an unsafe path",
            ));
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(std::io::Error::other(
                "update archive contains a link or special file",
            ));
        }
        if !entry.unpack_in(destination)? {
            return Err(std::io::Error::other(
                "update archive tried to escape its staging directory",
            ));
        }
        saw_payload = true;
    }
    let staged_app = destination.join(expected_root);
    if !saw_payload || !staged_app.is_dir() {
        return Err(std::io::Error::other(
            "update archive does not contain Minutes Archive.app",
        ));
    }
    Ok(staged_app)
}

#[cfg(target_os = "macos")]
fn reject_links_in_tree(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "staged update contains a symbolic link",
        ));
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            reject_links_in_tree(&entry?.path())?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn verify_staged_archive_app(app: &Path) -> std::io::Result<()> {
    // Match Apple's designated requirement for a Developer ID Application
    // signed by the Minutes team. `codesign --verify` alone proves only that a
    // signature is internally consistent; this external requirement also
    // proves that the chain anchors at Apple and carries the Developer ID
    // Application certificate extensions.
    const DEVELOPER_ID_REQUIREMENT: &str = r#"=identifier "com.useminutes.archive" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = "63TMLKT8HN""#;

    reject_links_in_tree(app)?;
    let executable = app.join("Contents/MacOS/minutes-archive-app");
    if !executable.is_file() {
        return Err(std::io::Error::other(
            "staged update has no Archive executable",
        ));
    }

    let verification = std::process::Command::new("/usr/bin/codesign")
        .args([
            "--verify",
            "--deep",
            "--strict",
            "--verbose=4",
            "--test-requirement",
            DEVELOPER_ID_REQUIREMENT,
        ])
        .arg(app)
        .output()?;
    if !verification.status.success() {
        return Err(std::io::Error::other(
            "staged update failed its Developer ID signature check",
        ));
    }
    let identity = std::process::Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=4"])
        .arg(app)
        .output()?;
    if !identity.status.success() {
        return Err(std::io::Error::other(
            "staged update identity could not be read",
        ));
    }
    let details = String::from_utf8_lossy(&identity.stderr);
    if !details
        .lines()
        .any(|line| line == "TeamIdentifier=63TMLKT8HN")
        || !details
            .lines()
            .any(|line| line == "Identifier=com.useminutes.archive")
    {
        return Err(std::io::Error::other(
            "staged update is not the signed Minutes Archive application",
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn atomic_swap_paths(first: &Path, second: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let first = CString::new(first.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("application path contains a null byte"))?;
    let second = CString::new(second.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("staging path contains a null byte"))?;
    // Both paths are siblings on the same volume. RENAME_SWAP is one atomic
    // filesystem transaction: success leaves the new app at the original path
    // and the old app in staging; failure changes neither path.
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            first.as_ptr(),
            libc::AT_FDCWD,
            second.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn install_verified_archive(bytes: &[u8]) -> std::io::Result<()> {
    let current_app = running_app_bundle_path()?;
    let parent = current_app
        .parent()
        .ok_or_else(|| std::io::Error::other("application bundle has no parent"))?;
    let staging = tempfile::Builder::new()
        .prefix(".minutes-archive-update-")
        .tempdir_in(parent)?;
    let staged_app = extract_update_archive(bytes, staging.path())?;
    verify_staged_archive_app(&staged_app)?;
    atomic_swap_paths(&current_app, &staged_app)
}

#[cfg(not(target_os = "macos"))]
fn install_verified_archive(_bytes: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "Minutes Archive updates are supported only on macOS",
    ))
}

/// Counts for the build in flight.
///
/// Polled rather than pushed: two integers on a timer needs no event channel,
/// and a channel that exists only to carry progress is a channel that could
/// later carry something else. Nothing derived from a document crosses here.
#[derive(serde::Serialize)]
struct UiBuildProgress {
    examined: u64,
    indexed: u64,
}

/// Show a document in Finder, named by opaque id.
///
/// The interface has never received a path and still does not: it sends back
/// the id it was given on the card, and the path is resolved here, used to ask
/// Finder to select the file, and dropped. Nothing about the location crosses
/// into the webview, so the property the census screen states -- "the interface
/// receives opaque location numbers, not folder paths" -- is unchanged.
#[tauri::command]
fn reveal_archive_document(
    document_id: String,
    state: tauri::State<'_, ArchiveState>,
) -> Result<(), String> {
    let document_id = minutes_archive_core::retrieval::DocumentId::parse(document_id)
        .map_err(|_| "That document could not be identified.".to_string())?;
    let session = state
        .session
        .lock()
        .map_err(|_| "Minutes Archive could not read its session.".to_string())?;
    let vault = session
        .text_vault
        .as_ref()
        .ok_or_else(|| "There is no open index.".to_string())?;
    // Refuses if the file moved or its bytes no longer match the revision the
    // quotation was checked against.
    let path = vault
        .source_path_for_reveal(&document_id)
        .ok_or_else(|| {
            "That document changed since it was searched, so Minutes Archive will not show it. Run the search again first."
                .to_string()
        })?;
    std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(&path)
        .status()
        .map_err(|_| "Finder could not be asked to show the document.".to_string())?;
    Ok(())
}

/// Show an approved location in Finder, named by opaque id.
///
/// "Approved location 1" cannot be told from "Approved location 2", and an
/// owner with three matters approved has no way to confirm he approved the
/// folders he meant to. The obvious fix -- put the folder's name on the row --
/// is the wrong one: in a practice the folder name is the sensitive part
/// ("Smith v. Acme -- privileged" says more than any path does), and sending
/// it would turn a checkable rule, no filesystem-derived string crosses into
/// the webview, into a judgement call every later feature has to make again.
///
/// So the answer is the one the evidence cards already use. The webview sends
/// back the id it was given; the path is resolved here, handed to Finder, and
/// dropped. The owner sees the real folder in its real place, which answers
/// the question better than a label could, and nothing crosses.
#[tauri::command]
fn reveal_archive_location(
    location_id: u64,
    state: tauri::State<'_, ArchiveState>,
) -> Result<(), String> {
    let session = state.session.lock().map_err(|_| lock_error())?;
    let location = session
        .locations
        .iter()
        .find(|location| location.id == location_id)
        .ok_or_else(|| "That location is no longer approved.".to_string())?;
    let path = location.root.canonical_path();
    // The same care the document reveal takes: a location that was replaced
    // by a symlink, or is no longer a directory, is refused rather than
    // followed. Approval was granted to a folder, not to whatever now sits
    // at its path.
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| "That location is no longer readable.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("That location is no longer the folder that was approved.".to_string());
    }
    std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .status()
        .map_err(|_| "Finder could not be asked to show the location.".to_string())?;
    Ok(())
}

#[tauri::command]
fn archive_index_progress(state: tauri::State<'_, ArchiveState>) -> UiBuildProgress {
    let progress = state
        .build_progress
        .lock()
        .map(|slot| Arc::clone(&slot))
        .unwrap_or_default();
    UiBuildProgress {
        examined: progress.examined(),
        indexed: progress.indexed(),
    }
}

#[tauri::command]
fn archive_bootstrap(state: State<'_, ArchiveState>) -> Result<BootstrapState, String> {
    let session = state.session.lock().map_err(|_| lock_error())?;
    Ok(BootstrapState {
        build_identity: build_identity(),
        locations: location_summaries_with(&session.locations, session.last_report.as_ref()),
        scan_running: session.scan.running,
        report: session.last_report.clone(),
        text_vault_report: session
            .text_vault
            .as_ref()
            .map(|vault| vault.build_report().clone()),
        update: state
            .update
            .lock()
            .map(|slot| slot.report.clone())
            .unwrap_or_default(),
    })
}

#[tauri::command]
async fn choose_archive_locations(
    app: tauri::AppHandle,
    state: State<'_, ArchiveState>,
) -> Result<LocationChoice, String> {
    {
        let session = state.session.lock().map_err(|_| lock_error())?;
        ensure_scan_idle(&session)?;
    }
    // Launch is over the moment the operator reaches for a folder, so the
    // network window closes here rather than after a folder is actually
    // approved. Cancelling the panel does not reopen it: the intent to point
    // this application at an archive is the event, not the outcome.
    begin_archive_interaction(&state)?;
    let selected = app
        .dialog()
        .file()
        .set_title("Choose archive locations")
        .blocking_pick_folders();
    // AppKit records the chosen directory the moment the panel closes, so
    // erase it here rather than only at exit -- a crash between the two would
    // otherwise leave the path on disk.
    native_panel_state::forget();
    let Some(selected) = selected else {
        let session = state.session.lock().map_err(|_| lock_error())?;
        return Ok(LocationChoice {
            folded: 0,
            forgotten_skips: 0,
            unskipped: 0,
            // Nothing was approved or removed, so the last census still
            // describes exactly these rows and its counts still belong on them.
            locations: location_summaries_with(&session.locations, session.last_report.as_ref()),
        });
    };

    let selected = selected
        .into_iter()
        .map(|path| {
            path.into_path()
                .map_err(|_| "The selected location is not a local folder.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let new_roots = authorize_roots(&selected).map_err(safe_census_error)?;

    let mut session = state.session.lock().map_err(|_| lock_error())?;
    ensure_scan_idle(&session)?;

    // A folder chosen twice, or chosen inside one that is already approved, is
    // folded into the location that covers it. Refusing the batch instead
    // discarded every other folder the owner had just picked and left them
    // with an empty list and the word "overlap".
    //
    // Existing locations come first so that re-choosing an approved folder
    // keeps the location already on screen, id and all, rather than replacing
    // it with an identical one.
    let existing = session.locations.len();
    let mut combined = session
        .locations
        .iter()
        .map(|location| location.root.clone())
        .collect::<Vec<_>>();
    combined.extend(new_roots.iter().cloned());
    let kept = reduce_approved_roots(&combined);
    let folded = combined.len().saturating_sub(kept.len());

    let surviving = kept
        .iter()
        .map(|&index| combined[index].clone())
        .collect::<Vec<_>>();
    validate_approved_roots(&surviving).map_err(safe_census_error)?;

    let mut locations = Vec::with_capacity(kept.len());
    for index in kept {
        match session.locations.get(index) {
            Some(location) if index < existing => locations.push(ApprovedLocation {
                id: location.id,
                root: location.root.clone(),
            }),
            _ => locations.push(ApprovedLocation {
                id: state.next_location_id.fetch_add(1, Ordering::Relaxed),
                root: combined[index].clone(),
            }),
        }
    }
    session.locations = locations;
    // Choosing a folder is the owner's latest word on it, and it must beat an
    // older "skip". Without this, approving a previously skipped folder folds
    // it into the location that covers it, the interface says "covered by that
    // one" -- and the exclusion goes on suppressing it, so the owner is told
    // the folder is in while the build deliberately never reads it. An
    // exclusion equal to the chosen folder, or an ancestor of it, is the one
    // doing the suppressing and is dropped; an exclusion strictly inside it
    // does not contradict the choice and stays.
    let session = &mut *session;
    let unskipped = cancel_contradicted_skips(
        &session.locations,
        &mut session.exclusions,
        &combined[existing..],
    );
    // A location folded into one that covers it takes its skipped folders with
    // it. They were named relative to a root that is no longer in the list, and
    // silently reattaching them to the containing root would skip folders the
    // owner never pointed at.
    let surviving_ids = session
        .locations
        .iter()
        .map(|location| location.id)
        .collect::<Vec<_>>();
    let skips_before = session.exclusions.len();
    session
        .exclusions
        .retain(|exclusion| surviving_ids.contains(&exclusion.location_id));
    let forgotten_skips = skips_before.saturating_sub(session.exclusions.len());
    session.last_report = None;
    session.text_vault = None;
    Ok(LocationChoice {
        folded,
        forgotten_skips,
        unskipped,
        locations: location_summaries(&session.locations),
    })
}

/// A skipped folder, held against the location's stable id rather than its
/// position in the list.
///
/// `ExcludedFolder` names its root by index, which is correct for one build and
/// wrong to store: removing a location, or folding one into a folder that
/// covers it, renumbers everything after it, and an exclusion would quietly
/// start skipping a folder in some other location. The index is derived at
/// build time from the list as it stands then, and an exclusion whose location
/// is gone simply does not appear.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionExclusion {
    location_id: u64,
    relative_path: std::path::PathBuf,
    identity: FileIdentity,
}

/// Cancels every skip the owner's newest folder choices contradict, and says
/// how many.
///
/// Choosing a folder in the picker is the owner's latest word on it, and it
/// must beat an older "skip". Without this, approving a previously skipped
/// folder folds it into the location that covers it, the interface says
/// "covered by that one" -- and the exclusion goes on suppressing it, so the
/// owner is told the folder is in while the build deliberately never reads it.
///
/// An exclusion equal to the chosen folder, or an ancestor of it, is the one
/// doing the suppressing and is dropped. An exclusion strictly *inside* the
/// chosen folder does not contradict the choice and stays. A chosen folder
/// that became its own location resolves to itself with an empty relative
/// path, which no stored exclusion can equal or be an ancestor of, so nothing
/// is dropped for it.
fn cancel_contradicted_skips(
    locations: &[ApprovedLocation],
    exclusions: &mut Vec<SessionExclusion>,
    chosen: &[ApprovedRoot],
) -> usize {
    let mut unskipped = 0usize;
    for root in chosen {
        let Some((keeper_id, relative)) = locations.iter().find_map(|location| {
            relative_path_within(&location.root, root).map(|relative| (location.id, relative))
        }) else {
            continue;
        };
        let before = exclusions.len();
        exclusions.retain(|exclusion| {
            exclusion.location_id != keeper_id
                || exclusion.relative_path.as_os_str().is_empty()
                || !(relative == exclusion.relative_path
                    || relative.starts_with(&exclusion.relative_path))
        });
        unskipped += before - exclusions.len();
    }
    unskipped
}

/// Resolves stored exclusions against the locations as they stand right now.
fn exclusions_for_build(session: &SessionState) -> Vec<ExcludedFolder> {
    session
        .exclusions
        .iter()
        .filter_map(|exclusion| {
            let root_index = session
                .locations
                .iter()
                .position(|location| location.id == exclusion.location_id)?;
            Some(ExcludedFolder {
                root_index,
                relative_path: exclusion.relative_path.clone(),
                identity: exclusion.identity,
            })
        })
        .collect()
}

/// What a round of the "skip folders" panel did.
///
/// Counts only. `outside` is the number of chosen folders that are not inside
/// any approved location -- they are not silently dropped, because an operator
/// who believes a folder is being skipped and is wrong ends up with an index
/// they cannot account for.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExclusionChoice {
    skipped: usize,
    outside: usize,
    refused_whole_location: usize,
    /// Every folder currently being skipped, counted here rather than tallied
    /// by the interface. A running total kept on the other side of the boundary
    /// drifts the moment a location is removed, and a confidently wrong count
    /// of what is being skipped is worse than none.
    total: usize,
}

/// Choose folders inside approved locations that the build must not read.
///
/// A thirty-year archive is not uniformly relevant: the folder that prompted
/// this held 2,873 screenshots and screen recordings, which cost OCR time and
/// index nothing an attorney would search for. The alternative was to approve
/// the parent whole or not at all.
#[tauri::command]
async fn choose_archive_exclusions(
    app: tauri::AppHandle,
    state: State<'_, ArchiveState>,
) -> Result<ExclusionChoice, String> {
    {
        let session = state.session.lock().map_err(|_| lock_error())?;
        ensure_scan_idle(&session)?;
        if session.locations.is_empty() {
            return Err("Approve at least one location before choosing what to skip.".to_string());
        }
    }
    let selected = app
        .dialog()
        .file()
        .set_title("Choose folders to skip")
        .blocking_pick_folders();
    native_panel_state::forget();
    let Some(selected) = selected else {
        let session = state.session.lock().map_err(|_| lock_error())?;
        return Ok(ExclusionChoice {
            skipped: 0,
            outside: 0,
            refused_whole_location: 0,
            total: session.exclusions.len(),
        });
    };

    let mut session = state.session.lock().map_err(|_| lock_error())?;
    ensure_scan_idle(&session)?;
    let mut choice = ExclusionChoice {
        skipped: 0,
        outside: 0,
        refused_whole_location: 0,
        total: 0,
    };

    for path in selected {
        let Ok(path) = path.into_path() else {
            choice.outside += 1;
            continue;
        };
        let Ok(canonical) = fs::canonicalize(&path) else {
            choice.outside += 1;
            continue;
        };
        let Some((location_id, relative)) = session.locations.iter().find_map(|location| {
            canonical
                .strip_prefix(location.root.canonical_path())
                .ok()
                .map(|relative| (location.id, relative.to_path_buf()))
        }) else {
            choice.outside += 1;
            continue;
        };
        // An empty relative path is the approved location itself. Excluding it
        // would silently index nothing from that location while it still sat
        // in the list looking approved; removing the location says the same
        // thing honestly.
        if relative.as_os_str().is_empty() {
            choice.refused_whole_location += 1;
            continue;
        }
        // One inspection both verifies that the selected path is still a
        // directory and captures the identity the exclusion will bind. A
        // second lookup here let a rename/replacement land between those two
        // facts and transferred the skip to the replacement.
        let Some(identity) = fs::symlink_metadata(&canonical)
            .ok()
            .filter(|metadata| metadata.is_dir())
            .and_then(|metadata| portable_identity_for(&metadata))
        else {
            choice.outside += 1;
            continue;
        };
        let exclusion = SessionExclusion {
            location_id,
            relative_path: relative,
            identity,
        };
        if !session.exclusions.contains(&exclusion) {
            session.exclusions.push(exclusion);
        }
        choice.skipped += 1;
    }

    if choice.skipped > 0 {
        session.last_report = None;
        session.text_vault = None;
    }
    choice.total = session.exclusions.len();
    Ok(choice)
}

/// Forget every skipped folder, so the next build reads the locations whole.
#[tauri::command]
fn clear_archive_exclusions(state: State<'_, ArchiveState>) -> Result<usize, String> {
    let mut session = state.session.lock().map_err(|_| lock_error())?;
    ensure_scan_idle(&session)?;
    let cleared = session.exclusions.len();
    session.exclusions.clear();
    if cleared > 0 {
        session.last_report = None;
        session.text_vault = None;
    }
    Ok(cleared)
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
    // Skipped folders belong to the location that contained them. Leaving them
    // behind means a later location could inherit them by id reuse, and it
    // makes the count of what is being skipped a lie.
    session
        .exclusions
        .retain(|exclusion| exclusion.location_id != location_id);
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

/// Whether an existing destination is an alias for an inode with other names.
fn is_multiply_linked(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.nlink() > 1
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn refuse_link_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("The report destination cannot be a symbolic link.".to_string())
        }
        Ok(metadata) if !metadata.is_file() => {
            Err("The report destination must be a regular file.".to_string())
        }
        // A hard link IS a regular file, so both checks above pass it, and
        // `O_NOFOLLOW` refuses symlinks only. Truncating it destroys whatever
        // inode the name is an alias for -- so choosing a report name that
        // happens to be linked to a client document replaces that document
        // with census JSON. Same link class as the ingestion boundary, on the
        // write side, and destructive rather than disclosive.
        Ok(metadata) if is_multiply_linked(&metadata) => Err(
            "The report destination is a hard link to another file. Choose a different name."
                .to_string(),
        ),
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
    // Same mechanism as the open panel: the save panel records its directory
    // in the app's own preference domain.
    native_panel_state::forget();
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
    let (roots, exclusions) = {
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
        // Resolved here, against the location list as it stands, so a stored
        // exclusion can never point at a location that has since moved.
        let exclusions = exclusions_for_build(&session);
        let roots = session
            .locations
            .iter()
            .map(|location| location.root.clone())
            .collect::<Vec<_>>();
        (roots, exclusions)
    };

    let worker_executable = std::env::current_exe()
        .map_err(|_| "Minutes Archive could not bind its document converter.".to_string())?;
    let worker_process_control = state.live_workers.control(Arc::clone(&cancelled));
    // Kept live: `purge_session` drains it, and a worker that needs scratch
    // space again should register here. Nothing populates it today.
    let _snapshot_registry = Arc::clone(&state.live_snapshots);
    // Reset before the build, so a second build does not continue the first
    // one's numbers.
    let progress = Arc::new(BuildProgress::default());
    if let Ok(mut slot) = state.build_progress.lock() {
        *slot = Arc::clone(&progress);
    }
    let build_result = tauri::async_runtime::spawn_blocking(move || {
        let vault_id = VaultId::parse("local-private-vault")
            .map_err(|_| "Minutes Archive could not establish the private vault.".to_string())?;
        let converter = BoundedConverter::bind_with_process_control(
            &worker_executable,
            worker_process_control.clone(),
        )
        .map_err(|error| error.to_string())?;
        // Binding the on-device model must not be able to deny the operator an
        // index. Exact evidence is the product; semantic suggestions are an
        // optional aid the interface already labels review-not-verified. A Mac
        // without Apple's linguistic asset previously got NO search at all.
        let semantic_engine = BoundedSemanticEngine::bind_with_process_control(
            &worker_executable,
            worker_process_control.clone(),
        )
        .ok();
        // Same reasoning for the recogniser: a Mac where Vision cannot start
        // must still index everything that is not a scan. Scans then stay
        // counted as needing OCR, which is what they were before this existed.
        let transcriber = BoundedTranscriber::bind_with_process_control(
            &worker_executable,
            worker_process_control,
        )
        .ok();
        // Neither worker copies itself any more -- both execute in place from
        // the bundle -- so there is no snapshot directory left to reclaim. The
        // registry stays because it is what `purge_session` drains, and a
        // future worker that does need scratch space should register it here.
        build_authorized_document_vault(
            vault_id,
            &roots,
            DocumentVaultLimits {
                excluded_paths: exclusions,
                ..DocumentVaultLimits::default()
            },
            &cancelled,
            &converter,
            transcriber.as_ref(),
            Some(progress.as_ref()),
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

/// What the interface is allowed to see of a search result.
///
/// The retrieval types carry more than the interface renders: a SHA-256 of
/// every matched document's full bytes, its length, the vault and document
/// ids, lexical rank, matched concepts, and semantic similarity. An
/// independent reviewer found all of it crossing the IPC boundary and none of
/// it rendered -- a content hash of every privileged match sitting in the
/// WebView's JS heap for no purpose, which is a confirmation-of-possession
/// oracle against a known corpus. With `script-src 'self'` and no remote
/// content the exploit path is narrow, but the field is gratuitous, and the
/// app's whole claim is that the interface receives the minimum it needs.
///
/// Projecting here rather than trimming the retrieval type keeps the evidence
/// record complete where it is used for verification, and keeps the boundary
/// honest by construction: a field added to `EvidenceCard` later does not
/// silently reach the webview.
#[derive(serde::Serialize)]
struct UiEvidenceCard {
    /// The opaque id, so a card can ask for itself to be shown in Finder.
    ///
    /// An id, never a path: it means nothing outside this session and resolves
    /// only against sources the vault already holds. The interface still
    /// receives no filename and no location.
    document_id: String,
    document_title: String,
    provision_heading: Option<String>,
    source_anchor: String,
    exact_excerpt: String,
    sentence_count: u32,
    source_converter: String,
    why_matched: String,
    index_fresh: bool,
}

impl From<&minutes_archive_core::retrieval::EvidenceCard> for UiEvidenceCard {
    fn from(card: &minutes_archive_core::retrieval::EvidenceCard) -> Self {
        Self {
            document_id: card.document_id.as_str().to_string(),
            document_title: card.document_title.clone(),
            provision_heading: card.provision_heading.clone(),
            source_anchor: card.source_anchor.clone(),
            exact_excerpt: card.exact_excerpt.clone(),
            sentence_count: card.sentence_count,
            source_converter: card.source_converter.clone(),
            why_matched: card.why_matched.clone(),
            index_fresh: card.index_fresh,
        }
    }
}

#[derive(serde::Serialize)]
struct UiSemanticCard {
    document_id: String,
    document_title: String,
    provision_heading: Option<String>,
    source_anchor: String,
    exact_excerpt: String,
    sentence_count: u32,
    source_converter: String,
    why_suggested: String,
    index_fresh: bool,
}

/// A passage read out of an image, kept apart from every card that quotes a
/// source. The field names differ from `UiEvidenceCard` on purpose: the
/// interface cannot render one as the other by reaching for `exact_excerpt`.
#[derive(serde::Serialize)]
struct UiTranscribedCard {
    document_id: String,
    document_title: String,
    page_anchor: String,
    transcribed_text: String,
    lowest_line_confidence: f32,
    transcriber: String,
    why_transcribed: String,
    index_fresh: bool,
}

impl From<&minutes_archive_core::retrieval::TranscribedCard> for UiTranscribedCard {
    fn from(card: &minutes_archive_core::retrieval::TranscribedCard) -> Self {
        Self {
            document_id: card.document_id.as_str().to_string(),
            document_title: card.document_title.clone(),
            page_anchor: card.page_anchor.clone(),
            transcribed_text: card.transcribed_text.clone(),
            lowest_line_confidence: card.lowest_line_confidence,
            transcriber: card.transcriber.clone(),
            why_transcribed: card.why_transcribed.clone(),
            index_fresh: card.index_fresh,
        }
    }
}

#[derive(serde::Serialize)]
struct UiDocumentCard {
    document_title: String,
    matched_concepts: Vec<minutes_archive_core::retrieval::LegalConcept>,
    criterion_evidence: Vec<UiEvidenceCard>,
    criterion_evidence_truncated: bool,
    why_matched: String,
    index_fresh: bool,
}

#[derive(serde::Serialize)]
struct UiSearchResponse {
    query: minutes_archive_core::retrieval::LegalQuery,
    evidence: Vec<UiEvidenceCard>,
    documents: Vec<UiDocumentCard>,
    semantic_suggestions: Vec<UiSemanticCard>,
    transcriptions: Vec<UiTranscribedCard>,
    lexical_candidates_considered: usize,
    semantic_candidates_considered: usize,
    semantic_query_applied: bool,
    stale_evidence_withdrawn: u64,
    inferred_boundary_evidence_withdrawn: u64,
}

impl From<LegalSearchResponse> for UiSearchResponse {
    fn from(response: LegalSearchResponse) -> Self {
        Self {
            query: response.query,
            evidence: response.evidence.iter().map(UiEvidenceCard::from).collect(),
            documents: response
                .documents
                .iter()
                .map(|document| UiDocumentCard {
                    document_title: document.document_title.clone(),
                    matched_concepts: document.matched_concepts.clone(),
                    criterion_evidence: document
                        .criterion_evidence
                        .iter()
                        .map(UiEvidenceCard::from)
                        .collect(),
                    criterion_evidence_truncated: document.criterion_evidence_truncated,
                    why_matched: document.why_matched.clone(),
                    index_fresh: document.index_fresh,
                })
                .collect(),
            semantic_suggestions: response
                .semantic_suggestions
                .iter()
                .map(|card| UiSemanticCard {
                    document_id: card.document_id.as_str().to_string(),
                    document_title: card.document_title.clone(),
                    provision_heading: card.provision_heading.clone(),
                    source_anchor: card.source_anchor.clone(),
                    exact_excerpt: card.exact_excerpt.clone(),
                    sentence_count: card.sentence_count,
                    source_converter: card.source_converter.clone(),
                    why_suggested: card.why_suggested.clone(),
                    index_fresh: card.index_fresh,
                })
                .collect(),
            transcriptions: response
                .transcriptions
                .iter()
                .map(UiTranscribedCard::from)
                .collect(),
            lexical_candidates_considered: response.lexical_candidates_considered,
            semantic_candidates_considered: response.semantic_candidates_considered,
            semantic_query_applied: response.semantic_query_applied,
            stale_evidence_withdrawn: response.stale_evidence_withdrawn,
            inferred_boundary_evidence_withdrawn: response.inferred_boundary_evidence_withdrawn,
        }
    }
}

#[tauri::command]
fn search_archive_text_vault(
    query: String,
    state: State<'_, ArchiveState>,
) -> Result<UiSearchResponse, String> {
    let session = state.session.lock().map_err(|_| lock_error())?;
    ensure_scan_idle(&session)?;
    let vault = session.text_vault.as_ref().ok_or_else(|| {
        "Build the private text index before searching. No partial index was retained.".to_string()
    })?;
    vault
        .interpret_and_search(query)
        .map(UiSearchResponse::from)
        .map_err(|error| error.to_string())
}

/// Bind the converter to this executable and convert one synthetic document.
///
/// Deliberately end to end through the real `BoundedConverter`: the failure
/// this exists to catch was in binding, not in parsing, and it only appears
/// when the running executable carries a bundle-bound signature.
/// The recognizer worker, reached through the same binary as the others.
///
/// Its absence is what made `BoundedTranscriber::bind` fail: the marker fell
/// through to the GUI branch, so binding launched a second copy of the
/// application instead of a worker, the self-test never passed, and every scan
/// was silently skipped as an unsupported format. Nothing reported it, because
/// the engine is optional by design and `.ok()` swallowed the failure.
fn run_ocr_worker(operation: &str) -> i32 {
    if minutes_archive_ocr::install_worker_security_boundary().is_err() {
        return 70;
    }
    if operation == "sandbox-self-test" {
        return minutes_archive_ocr::sandbox_self_test();
    }
    if operation != "recognize" {
        return 64;
    }
    use std::io::{Read, Write};
    // Bounded on the way in, mirroring the standalone worker: this embedded
    // copy is the one `BoundedTranscriber::bind(current_exe)` actually runs
    // in the app, so an unbounded read here would leave production trusting
    // the parent's size check while only the test binary held on its own.
    let mut image = Vec::new();
    if std::io::stdin()
        .lock()
        .take(minutes_archive_ocr::MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut image)
        .is_err()
    {
        return 65;
    }
    let outcome = std::panic::catch_unwind(|| minutes_archive_ocr::recognize_page(&image));
    let Ok(Ok(page)) = outcome else {
        return 66;
    };
    let Ok(encoded) = serde_json::to_vec(&page) else {
        return 67;
    };
    if std::io::stdout().lock().write_all(&encoded).is_err() {
        return 68;
    }
    0
}

fn run_signed_worker_selftest() -> i32 {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("signed-worker-selftest: current_exe failed: {error}");
            return 70;
        }
    };
    let converter = match minutes_archive_convert::BoundedConverter::bind(&executable) {
        Ok(converter) => converter,
        Err(error) => {
            eprintln!("signed-worker-selftest: converter bind failed: {error}");
            return 71;
        }
    };
    // A minimal synthetic document, inline so the check needs no fixture on
    // the runner and never touches a real file.
    const SOURCE: &[u8] =
        b"7. CONFIDENTIALITY\nConfidential Information includes affiliate data.\n";
    match converter.convert(minutes_archive_convert::SourceFormat::Docx, SOURCE) {
        Ok(_) => {}
        Err(minutes_archive_convert::WorkerError::SourceRefused) => {
            // Expected: those bytes are not a real DOCX container. What
            // matters is that the worker ran and answered, which it cannot do
            // if the signature check killed it.
        }
        Err(error) => {
            eprintln!("signed-worker-selftest: worker did not run: {error}");
            return 72;
        }
    }
    // The recognizer is bound against THIS binary, not the standalone worker.
    // Exercising the standalone one is what let a missing marker ship: that
    // executable of course understood its own marker, while the application it
    // actually runs inside did not, and binding it launched a second copy of
    // the app instead of a worker.
    if let Err(error) = minutes_archive_ocr::BoundedTranscriber::bind(&executable) {
        eprintln!("signed-worker-selftest: recognizer bind failed: {error}");
        return 73;
    }
    match minutes_archive_semantic::BoundedSemanticEngine::bind(&executable) {
        Ok(_) => println!("signed_worker_selftest=passed converter=bound ocr=bound semantic=bound"),
        // Absent on a runner without Apple's linguistic asset, which is not a
        // signing failure and must not fail the check.
        Err(error) => println!(
            "signed_worker_selftest=passed converter=bound ocr=bound semantic=unavailable ({error})"
        ),
    }
    0
}

fn main() {
    // One descriptor per indexed document, held for the session so the
    // live-source fence can re-read through it. macOS gives a GUI application a
    // soft limit of 256, which stopped a build at 237 of 16,621 and reported
    // the rest as unreadable. The implementation lives in archive-core so a
    // harness can lower the ceiling and prove this still completes; nothing in
    // `main` is reachable from a test.
    minutes_archive_core::vault::raise_open_file_ceiling();
    let mut arguments = std::env::args();
    let _program = arguments.next();
    let marker = arguments.next();
    if matches!(
        marker.as_deref(),
        Some(CONVERT_WORKER_MARKER | SEMANTIC_WORKER_MARKER | OCR_WORKER_MARKER)
    ) {
        let operation = arguments.next().unwrap_or_default();
        if arguments.next().is_some() {
            std::process::exit(64);
        }
        let status = match marker.as_deref() {
            Some(CONVERT_WORKER_MARKER) => run_convert_worker(&operation),
            Some(SEMANTIC_WORKER_MARKER) => run_semantic_worker(&operation),
            Some(OCR_WORKER_MARKER) => run_ocr_worker(&operation),
            _ => unreachable!("worker marker was already validated"),
        };
        std::process::exit(status);
    }
    if marker.as_deref() == Some(SIGNED_WORKER_SELFTEST_MARKER) {
        if arguments.next().is_some() {
            std::process::exit(64);
        }
        std::process::exit(run_signed_worker_selftest());
    }
    // An unrecognised flag must never reach the GUI branch.
    //
    // This is how a missing worker marker became a second copy of the
    // application on the owner's screen: `BoundedTranscriber::bind` launched
    // this binary with the OCR marker, nothing matched, and execution fell
    // through to `tauri::Builder`. The bind then failed, every scan was skipped
    // as unsupported, and the visible symptom was a window nobody asked for
    // stealing focus mid-build. Refusing anything flag-shaped that is not
    // understood turns that whole class of mistake into an immediate exit.
    if marker
        .as_deref()
        .is_some_and(|value| value.starts_with("--") && value != NATIVE_LIFECYCLE_SELFTEST_MARKER)
    {
        eprintln!("Minutes Archive: unrecognised option");
        std::process::exit(64);
    }
    let native_lifecycle_selftest = marker.as_deref() == Some(NATIVE_LIFECYCLE_SELFTEST_MARKER);
    if native_lifecycle_selftest && arguments.next().is_some() {
        std::process::exit(64);
    }

    tauri::Builder::default()
        .manage(ArchiveState::default())
        .plugin(tauri_plugin_dialog::init())
        // Registered for the Rust side only. `archive-main` deliberately does
        // not carry `updater:default`, so the plugin's own commands are not
        // reachable from the webview: the only way to the network is through
        // the two gated commands below, and the capability file stays the
        // literal statement the security packet cites -- no updater capability
        // is exposed to the interface.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            // Erase any panel state a PREVIOUS run left behind, before this
            // one can show a window. The erasure after each panel and at
            // graceful exit cannot run under SIGKILL or Force Quit, so a
            // forced termination can leave NSOSPLastRootDirectory on disk. It
            // cannot be prevented -- AppKit writes it, and no hook runs after
            // a kill -- but it can be bounded: with this, residue survives
            // only until the app is next opened, rather than indefinitely.
            native_panel_state::forget();
            if native_lifecycle_selftest {
                let window = app.get_webview_window("main").ok_or_else(|| {
                    std::io::Error::other("Archive native lifecycle self-test found no main window")
                })?;
                if !window.is_visible()? {
                    return Err(std::io::Error::other(
                        "Archive native lifecycle self-test found a hidden main window",
                    )
                    .into());
                }
                println!("archive_native_window=visible");
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    println!("archive_native_close=requested");
                    if let Err(error) = window.close() {
                        eprintln!("archive_native_close_error={error}");
                    }
                });
            }
            Ok(())
        })
        .on_window_event(move |window, event| {
            if window.label() == "main"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                // Archive has no tray mode. Exiting with the only window prevents
                // an invisible process from retaining privileged source text,
                // FTS rows, or semantic vectors after the user closes the app.
                if native_lifecycle_selftest {
                    println!("archive_native_close_event=received");
                }
                // Release the session explicitly first. `exit(0)` terminates
                // the process without unwinding, so no destructor ever runs:
                // the worker snapshot directories are owned by `TempDir`
                // fields whose cleanup is `Drop`, and they were surviving the
                // process as two 40 MB copies of the executable in $TMPDIR.
                // Any future zeroization written as a destructor would have
                // been skipped the same way. Dropping the session here runs
                // that cleanup while the process is still alive.
                let app_handle = window.app_handle().clone();
                purge_session(&app_handle);
                app_handle.exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            archive_bootstrap,
            choose_archive_locations,
            choose_archive_exclusions,
            clear_archive_exclusions,
            remove_archive_location,
            run_archive_census,
            cancel_archive_census,
            export_archive_census,
            build_archive_text_vault,
            archive_index_progress,
            reveal_archive_document,
            reveal_archive_location,
            search_archive_text_vault,
            archive_update_report,
            check_for_archive_update,
            install_archive_update,
        ])
        .build(tauri::generate_context!())
        .expect("Minutes Archive failed to start")
        .run(|app_handle, event| {
            // The window-close handler alone is not enough. Cmd-Q maps to
            // `[NSApp terminate:]`, which never calls `windowShouldClose:`,
            // so `CloseRequested` never fires -- and Cmd-Q is how most Mac
            // users quit an app. `Exit` covers that path, the Quit menu item,
            // and any other route out of the run loop.
            if matches!(event, tauri::RunEvent::Exit) {
                purge_session(app_handle);
            }
        });
}

/// Erases the location record AppKit keeps after a native panel is used.
///
/// `NSOpenPanel` writes the last directory into the app's own preference
/// domain as an `NSOSPLastRootDirectory` bookmark. An independent reviewer
/// decoded that blob and recovered the full path of the approved archive --
/// volume name, volume UUID, and every directory component -- and it survived
/// application exit. `~/Library/Preferences` carries no TCC protection, so a
/// post-install script, a sync agent, a backup, or a forensic image reads the
/// exact on-disk location of a client archive with no prompt. Folder names in
/// legal practice are client names.
///
/// The app tells the operator on screen that it receives "opaque location
/// numbers, not folder paths", and it exits on window close so that nothing
/// privileged outlives the session. This closes the one artifact that did.
///
/// No application code writes these keys; AppKit does, so they are removed
/// after the panel closes and again when the session is purged.
#[cfg(target_os = "macos")]
mod native_panel_state {
    use std::ffi::c_void;

    type CFStringRef = *const c_void;
    type CFPropertyListRef = *const c_void;

    extern "C" {
        static kCFPreferencesCurrentApplication: CFStringRef;
        fn CFStringCreateWithBytes(
            allocator: *const c_void,
            bytes: *const u8,
            num_bytes: isize,
            encoding: u32,
            is_external_representation: u8,
        ) -> CFStringRef;
        fn CFPreferencesSetAppValue(
            key: CFStringRef,
            value: CFPropertyListRef,
            application_id: CFStringRef,
        );
        fn CFPreferencesAppSynchronize(application_id: CFStringRef) -> u8;
        fn CFRelease(cf: *const c_void);
    }

    const UTF8: u32 = 0x0800_0100;

    /// Every key AppKit is known to write from an open or save panel. Removing
    /// a key the domain does not have is a no-op, so listing the save-panel
    /// and recent-places keys costs nothing and covers the export path too.
    const PANEL_KEYS: &[&str] = &[
        "NSOSPLastRootDirectory",
        "NSNavLastRootDirectory",
        "NSNavLastCurrentDirectory",
        "NSNavRecentPlaces",
        "NSNavPanelExpandedSizeForOpenMode",
        "NSNavPanelExpandedSizeForSaveMode",
        "NSWindow Frame GoToSheet",
        "NSWindow Frame NSNavPanelAutosaveName",
    ];

    pub fn forget() {
        // SAFETY: every pointer is either a CFString this function created and
        // releases, or the framework-owned current-application constant. A
        // null value is CFPreferences' documented "remove this key".
        unsafe {
            for key in PANEL_KEYS {
                let cf_key = CFStringCreateWithBytes(
                    std::ptr::null(),
                    key.as_ptr(),
                    key.len() as isize,
                    UTF8,
                    0,
                );
                if cf_key.is_null() {
                    continue;
                }
                CFPreferencesSetAppValue(
                    cf_key,
                    std::ptr::null(),
                    kCFPreferencesCurrentApplication,
                );
                CFRelease(cf_key);
            }
            CFPreferencesAppSynchronize(kCFPreferencesCurrentApplication);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native_panel_state {
    pub fn forget() {}
}

/// Releases everything the session owns while the process is still alive.
///
/// `exit(0)` terminates without unwinding, so no destructor runs at exit: the
/// worker snapshot directories are owned by `TempDir` fields whose cleanup is
/// `Drop`, and anything written as a destructor later would be skipped the
/// same way.
fn purge_session(app_handle: &tauri::AppHandle) {
    let Some(state) = app_handle.try_state::<ArchiveState>() else {
        return;
    };
    purge_archive_state(&state);
}

fn purge_archive_state(state: &ArchiveState) {
    // Recover from poisoning rather than skipping the purge. A panic anywhere
    // under this lock would otherwise leave the session permanently
    // un-purgeable, and a poisoned app is precisely the app a user closes.
    let mut session = state
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(cancelled) = session.scan.cancelled.take() {
        cancelled.store(true, Ordering::Release);
    }
    *session = SessionState::default();
    drop(session);

    // Child processes survive their parent on POSIX. Kill every registered
    // process group while the desktop process is still alive; each launcher
    // then reaps and deregisters its child as cancellation unwinds the build.
    state.live_workers.terminate_all();

    // The offer holds a download URL and a signature. Neither is privileged,
    // but nothing here should outlive the session it was fetched in, and the
    // window is closed by then in any case.
    let mut update = state
        .update
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    update.offered = None;
    drop(update);

    let mut snapshots = state
        .live_snapshots
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for snapshot in snapshots.drain(..) {
        let _ = fs::remove_dir_all(&snapshot);
    }
    drop(snapshots);

    native_panel_state::forget();
}

#[cfg(test)]
mod tests {
    use minutes_archive_core::approve_roots;

    #[test]
    #[cfg(unix)]
    fn purge_terminates_a_registered_worker_process() {
        use minutes_archive_worker_control::RegisteredChild;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let state = ArchiveState::default();
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut session = state.session.lock().expect("session");
            session.scan.running = true;
            session.scan.cancelled = Some(Arc::clone(&cancelled));
        }
        let control = state.live_workers.control(Arc::clone(&cancelled));
        let mut command = Command::new("/bin/sleep");
        command
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut worker =
            RegisteredChild::spawn(&mut command, Some(&control)).expect("spawn real worker");
        assert_eq!(state.live_workers.live_count(), 1);

        purge_archive_state(&state);
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            match worker.try_wait().expect("check worker") {
                Some(status) => break status,
                None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                None => panic!("the worker survived purge"),
            }
        };
        assert!(!status.success(), "the purge let the worker exit normally");
        assert!(cancelled.load(Ordering::Acquire));
        assert_eq!(
            state.live_workers.live_count(),
            0,
            "the reaped worker remained registered"
        );

        let mut late_command = Command::new("/bin/sleep");
        late_command.arg("60");
        let late_spawn = RegisteredChild::spawn(&mut late_command, Some(&control));
        assert_eq!(
            late_spawn
                .expect_err("purge allowed a late worker to start")
                .kind(),
            std::io::ErrorKind::Interrupted
        );
    }

    /// Exporting the census must never overwrite a hard-linked document.
    ///
    /// An independent reviewer found that `refuse_link_target` rejects
    /// symlinks but accepts any regular file, and `O_NOFOLLOW` does not refuse
    /// a hard link either -- so `truncate(true)` destroys whatever inode the
    /// destination name is an alias for. Same link class as the ingestion
    /// escape, but on the write side, and destructive rather than disclosive:
    /// a client document is replaced by census JSON.
    #[test]
    #[cfg(unix)]
    fn exporting_over_a_hard_link_never_destroys_the_linked_document() {
        let temp = tempfile::tempdir().expect("temp");
        let precious = temp.path().join("client-matter.txt");
        let original = b"PRIVILEGED CLIENT DOCUMENT";
        std::fs::write(&precious, original).expect("write precious");

        // The operator picks a report name that is an alias for that document.
        let destination = temp.path().join("archive-census.json");
        std::fs::hard_link(&precious, &destination).expect("hard link");

        let refusal = refuse_link_target(&destination);
        assert!(
            refusal.is_err(),
            "a multiply linked destination was accepted for truncation"
        );
        assert_eq!(
            std::fs::read(&precious).expect("read back"),
            original,
            "the linked client document was modified"
        );

        // An ordinary new destination is still allowed.
        assert!(refuse_link_target(&temp.path().join("fresh-report.json")).is_ok());
    }

    /// The interface must not receive a content hash of every match.
    ///
    /// An independent reviewer found `source_revision.sha256` and `byte_len`
    /// crossing IPC unrendered. Serializing the projection is the only way to
    /// prove the boundary: asserting on struct fields would pass even if the
    /// command went back to returning the retrieval type.
    #[test]
    fn the_ui_projection_carries_no_hash_identifier_or_rank() {
        use minutes_archive_core::retrieval::{
            interpret_legal_query, normalize_text_document, CurrentRevisionSet, DocumentId,
            LegalIndex, VaultId,
        };

        let vault = VaultId::parse("dto-probe").expect("vault");
        let document = normalize_text_document(
            DocumentId::parse("probe-doc").expect("id"),
            "Probe Document",
            b"7. CONFIDENTIALITY\nRecipient shall protect Confidential Information and its affiliates.",
        )
        .expect("normalize");
        let mut index = LegalIndex::new(vault.clone()).expect("index");
        index.replace_document(&document).expect("replace");
        let revisions = CurrentRevisionSet::from_documents([&document]);
        let query = interpret_legal_query("Find confidentiality provisions covering affiliates.")
            .expect("query");
        let response = index.search(&vault, query, &revisions).expect("search");
        assert!(
            !response.evidence.is_empty(),
            "fixture returned no evidence"
        );

        let full = serde_json::to_string(&response).expect("serialize full");
        let projected =
            serde_json::to_string(&UiSearchResponse::from(response)).expect("serialize projection");

        // The full record really does carry these; the projection must not.
        //
        // `document_id` was on this list and has been deliberately removed from
        // it, so that a card can ask for its own source to be shown in Finder.
        // What that reveals is nothing: the id is a synthetic counter,
        // `document-{n:016x}`, generated during the build and meaningless
        // outside the session. It is not derived from the filename, the path or
        // the contents, and `valid_opaque_id` restricts it to lowercase
        // alphanumerics and hyphens, so a path could not be smuggled through it
        // even by accident. The path itself is resolved behind the boundary and
        // never returned. The assertion below pins that.
        for field in ["sha256", "byte_len", "vault_id", "lexical_rank"] {
            assert!(
                full.contains(field),
                "fixture no longer exercises {field}; this test would pass vacuously"
            );
            assert!(
                !projected.contains(field),
                "{field} still reaches the interface"
            );
        }
        // ...while what the interface renders survives.
        assert!(projected.contains("exact_excerpt"));
        assert!(projected.contains("why_matched"));
        assert!(projected.contains("source_anchor"));

        // The id crosses, and it is opaque. Nothing about where the document
        // lives goes with it.
        assert!(
            projected.contains("document_id"),
            "cards cannot ask for their source without an id"
        );
        assert!(
            !projected.contains("probe-doc.txt") && !projected.contains('/'),
            "the projection carried a filename or a path: {projected}"
        );
    }

    #[test]
    fn document_card_truncation_and_reason_survive_the_ui_projection() {
        use minutes_archive_core::retrieval::{
            normalize_text_document, CurrentRevisionSet, DocumentId, LegalConcept, LegalIndex,
            LegalQuery, MatchScope, VaultId,
        };

        let mut clauses = (0..64)
            .map(|ordinal| {
                format!(
                    "{}. CONFIDENTIALITY\nEach party shall protect Confidential Information.",
                    ordinal + 1
                )
            })
            .collect::<Vec<_>>();
        clauses.push(
            "65. ASSIGNMENT\nNeither party may assign this Agreement without consent.".to_string(),
        );
        let document = normalize_text_document(
            DocumentId::parse("dto-sixty-four-plus-one").expect("id"),
            "Sixty Four Plus One",
            clauses.join("\n\n").as_bytes(),
        )
        .expect("normalize");
        let vault = VaultId::parse("dto-document-card").expect("vault");
        let mut index = LegalIndex::new(vault.clone()).expect("index");
        index.replace_document(&document).expect("replace");
        let revisions = CurrentRevisionSet::from_documents([&document]);
        let query = LegalQuery {
            raw: "Find documents containing confidentiality and assignment.".to_string(),
            scope: MatchScope::AnywhereInDocument,
            required_concepts: vec![LegalConcept::Confidentiality, LegalConcept::Assignment],
            excluded_concepts: Vec::new(),
            exact_phrase: None,
            max_sentences: None,
            limit: 20,
        };
        let response = index.search(&vault, query, &revisions).expect("search");
        let projected = serde_json::to_value(UiSearchResponse::from(response)).expect("serialize");
        let card = &projected["documents"][0];

        assert_eq!(card["criterion_evidence_truncated"], true);
        assert_eq!(
            card["why_matched"],
            "Matched confidentiality, assignment across 64 provisions in this document."
        );
        assert_eq!(
            card["matched_concepts"],
            serde_json::json!(["confidentiality", "assignment"])
        );
        let assignment_is_shown = card["criterion_evidence"]
            .as_array()
            .is_some_and(|evidence| {
                evidence.iter().any(|passage| {
                    passage["exact_excerpt"]
                        .as_str()
                        .is_some_and(|excerpt| excerpt.contains("assign"))
                })
            });
        assert!(
            assignment_is_shown,
            "concept-preserving displacement did not put the late assignment evidence on the card"
        );
    }

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

    fn directory_identity(path: &std::path::Path) -> FileIdentity {
        portable_identity_for(&fs::metadata(path).expect("directory metadata"))
            .expect("portable directory identity")
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

    /// Per-location counts distinguish the rows and never reach the export.
    ///
    /// The rows are deliberately unnameable, which left an owner unable to
    /// tell one approved matter from another. Counts fix that without naming
    /// anything -- but the exported report has been through a privacy review
    /// and carries a versioned schema, so the numbers stop at the interface.
    #[test]
    fn per_location_counts_reach_the_interface_and_not_the_export() {
        let temp = TempDir::new().expect("temp");
        let big = temp.path().join("matter-archive");
        let small = temp.path().join("exhibits");
        fs::create_dir(&big).expect("big");
        fs::create_dir(&small).expect("small");
        for index in 0..7u32 {
            fs::write(
                big.join(format!("agreement-{index:02}.txt")),
                "x".repeat(100),
            )
            .expect("write");
        }
        fs::write(small.join("exhibit-a.txt"), "y".repeat(10)).expect("write");

        let approved = approve_roots(&[big, small]).expect("approve");
        let report =
            scan_approved_roots(&approved, CensusLimits::default(), &AtomicBool::new(false))
                .expect("census");

        // Positional against the approved roots, and each artifact counted
        // once under the location it was actually found in.
        assert_eq!(report.per_location.len(), 2);
        assert_eq!(report.per_location[0].artifacts, 7);
        assert_eq!(report.per_location[0].regular_file_bytes, 700);
        assert_eq!(report.per_location[1].artifacts, 1);
        assert_eq!(report.per_location[1].regular_file_bytes, 10);
        // The per-location totals must reconcile with the aggregate, or one
        // of the two numbers on screen is lying about the same archive.
        let summed: u64 = report.per_location.iter().map(|t| t.artifacts).sum();
        assert_eq!(summed, report.summary.artifacts);

        // The interface receives them on the rows it could not otherwise tell
        // apart.
        let mut roots = approved.into_iter();
        let locations = [
            ApprovedLocation {
                id: 1,
                root: roots.next().expect("first"),
            },
            ApprovedLocation {
                id: 2,
                root: roots.next().expect("second"),
            },
        ];
        let serialized = serde_json::to_string(&location_summaries_with(&locations, Some(&report)))
            .expect("serialize");
        assert!(serialized.contains(r#""artifacts":7"#), "{serialized}");
        assert!(serialized.contains(r#""artifacts":1"#), "{serialized}");
        // The interface reads these by name, and the struct renames to
        // camelCase -- a mismatch here showed every location as "0 B" while
        // the item count was right, which is the kind of half-correct row
        // that reads as a real number.
        assert!(
            serialized.contains(r#""regularFileBytes":700"#),
            "{serialized}"
        );
        assert!(!serialized.contains("matter-archive"), "{serialized}");
        assert!(!serialized.contains("exhibits"), "{serialized}");

        // The export does not, and stays exactly the reviewed shape.
        let exported = serde_json::to_string(&report).expect("export");
        assert!(
            !exported.contains("per_location"),
            "per-location totals must not enter the reviewed export shape: {exported}"
        );

        // A report taken against a different set of locations misattributes
        // every row, so its numbers are dropped rather than shown wrong.
        let one_location = [ApprovedLocation {
            id: 1,
            root: approve_roots(std::slice::from_ref(&temp.path().join("matter-archive")))
                .ok()
                .and_then(|mut roots| roots.pop())
                .expect("re-approve"),
        }];
        let mismatched =
            serde_json::to_string(&location_summaries_with(&one_location, Some(&report)))
                .expect("serialize");
        assert!(
            !mismatched.contains("artifacts"),
            "a report from a different location set must not label these rows: {mismatched}"
        );
    }

    /// Revealing a location resolves its path here and never hands one out.
    ///
    /// The reveal exists because the labels are deliberately
    /// indistinguishable, so it must not become the hole the labels avoid:
    /// the command takes the same opaque id the interface was given, and the
    /// only thing it returns is success or a sentence.
    #[test]
    fn revealing_a_location_takes_an_opaque_id_and_returns_no_path() {
        let temp = TempDir::new().expect("temp");
        let client_alpha = temp.path().join("client-alpha");
        fs::create_dir(&client_alpha).expect("alpha");
        let mut roots = approve_roots(std::slice::from_ref(&client_alpha)).expect("approve");
        let locations = [ApprovedLocation {
            id: 7,
            root: roots.remove(0),
        }];

        // The lookup the command performs, on the id the interface holds.
        let found = locations
            .iter()
            .find(|location| location.id == 7)
            .expect("the approved id must resolve");
        assert_eq!(
            found.root.canonical_path(),
            client_alpha.canonicalize().expect("canonical")
        );

        // An id the session does not know resolves to nothing, so a stale or
        // invented id cannot reach the filesystem at all.
        assert!(locations.iter().all(|location| location.id != 99));

        // A location replaced by a symlink is refused rather than followed:
        // approval was granted to a folder, not to whatever now sits there.
        let elsewhere = temp.path().join("elsewhere");
        fs::create_dir(&elsewhere).expect("elsewhere");
        let swapped = temp.path().join("swapped");
        std::os::unix::fs::symlink(&elsewhere, &swapped).expect("symlink");
        let metadata = fs::symlink_metadata(&swapped).expect("metadata");
        assert!(
            metadata.file_type().is_symlink(),
            "the guard's condition must be the one that fires here"
        );
    }

    /// Choosing a folder must cancel the skip that was suppressing it.
    ///
    /// The contradiction this pins: approve a location, skip a folder inside
    /// it, then choose that folder in the picker because you changed your
    /// mind. The fold reports it as "covered" by the containing location --
    /// and without cancellation the exclusion keeps suppressing it, so the
    /// owner is told the folder is in while the build never reads it. The
    /// index the owner believes in and the index that exists diverge, which is
    /// the one failure this application must never have.
    #[test]
    fn choosing_a_skipped_folder_cancels_the_skip_and_nothing_else() {
        let temp = TempDir::new().expect("temp");
        let parent = temp.path().join("life");
        let attachments = parent.join("attachments");
        let deeper = attachments.join("movs");
        let other = parent.join("matters");
        fs::create_dir_all(&deeper).expect("folders");
        fs::create_dir_all(&other).expect("other");

        let mut roots = approve_roots(std::slice::from_ref(&parent)).expect("approve parent");
        let locations = vec![ApprovedLocation {
            id: 7,
            root: roots.remove(0),
        }];
        let mut exclusions = vec![
            SessionExclusion {
                location_id: 7,
                relative_path: std::path::PathBuf::from("attachments"),
                identity: directory_identity(&attachments),
            },
            // Inside the chosen folder: does not contradict the choice.
            SessionExclusion {
                location_id: 7,
                relative_path: std::path::PathBuf::from("attachments/movs"),
                identity: directory_identity(&deeper),
            },
            // Unrelated: must survive untouched.
            SessionExclusion {
                location_id: 7,
                relative_path: std::path::PathBuf::from("matters"),
                identity: directory_identity(&other),
            },
        ];

        // The owner picks the skipped folder itself.
        let chosen =
            approve_roots(std::slice::from_ref(&attachments)).expect("approve attachments");
        let unskipped = cancel_contradicted_skips(&locations, &mut exclusions, &chosen);

        assert_eq!(unskipped, 1, "exactly the suppressing skip is cancelled");
        assert!(
            !exclusions
                .iter()
                .any(|exclusion| exclusion.relative_path.as_os_str() == "attachments"),
            "the skip that suppressed the chosen folder survived"
        );
        assert!(
            exclusions
                .iter()
                .any(|exclusion| exclusion.relative_path.as_os_str() == "attachments/movs"),
            "a skip inside the chosen folder was wrongly cancelled"
        );
        assert!(
            exclusions
                .iter()
                .any(|exclusion| exclusion.relative_path.as_os_str() == "matters"),
            "an unrelated skip was wrongly cancelled"
        );

        // Choosing deeper than the skip: the ancestor skip is the suppressor
        // and must be cancelled too.
        let mut exclusions = vec![SessionExclusion {
            location_id: 7,
            relative_path: std::path::PathBuf::from("attachments"),
            identity: directory_identity(&attachments),
        }];
        let chosen = approve_roots(&[deeper]).expect("approve deeper");
        assert_eq!(
            cancel_contradicted_skips(&locations, &mut exclusions, &chosen),
            1,
            "an ancestor skip suppresses the chosen folder and must be cancelled"
        );
    }

    /// A skipped folder must follow its location, not its position.
    ///
    /// Exclusions are stored against the location's stable id and resolved to a
    /// root index only at build time. Storing the index instead would mean that
    /// removing the first location silently moved every exclusion onto whatever
    /// location took its place -- folders the owner never pointed at would stop
    /// being read, with only a count to show for it, which is the one failure
    /// this build cannot have.
    #[test]
    fn a_skipped_folder_follows_its_location_when_the_list_changes() {
        let temp = TempDir::new().expect("temp");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).expect("first");
        fs::create_dir(&second).expect("second");
        fs::create_dir(second.join("attachments")).expect("attachments");
        let mut roots = approve_roots(&[first, second.clone()]).expect("approve synthetic roots");
        let mut session = SessionState {
            locations: vec![
                ApprovedLocation {
                    id: 7,
                    root: roots.remove(0),
                },
                ApprovedLocation {
                    id: 8,
                    root: roots.remove(0),
                },
            ],
            exclusions: vec![SessionExclusion {
                location_id: 8,
                relative_path: std::path::PathBuf::from("attachments"),
                identity: directory_identity(&second.join("attachments")),
            }],
            ..SessionState::default()
        };

        let resolved = exclusions_for_build(&session);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].root_index, 1, "second location is at index 1");

        // Drop the first location. The exclusion still belongs to location 8,
        // which has moved to index 0.
        session.locations.retain(|location| location.id != 7);
        let resolved = exclusions_for_build(&session);
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].root_index, 0,
            "the exclusion followed its location instead of staying at index 1"
        );

        // Drop the location it belongs to. The exclusion resolves to nothing
        // rather than attaching itself to a location that never had it.
        session.locations.clear();
        assert!(exclusions_for_build(&session).is_empty());
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

    /// A pristine session is the only one that may reach the network.
    ///
    /// Stated first so the refusal tests below cannot pass vacuously: if the
    /// window never opened at all they would still be green, and the feature
    /// would be broken rather than safe.
    #[test]
    fn a_launch_session_that_has_seen_nothing_may_check_once() {
        let state = ArchiveState::default();
        let claim = claim_network_window(&state, &state.network.check_spent)
            .expect("a pristine session may claim its launch check");
        drop(claim);
        // ...and exactly once. The claim on screen and in Peter's disclosure is
        // "one request", so a second call is refused even though nothing about
        // the session has changed.
        assert_eq!(
            claim_network_window(&state, &state.network.check_spent).err(),
            Some(WINDOW_SPENT_REFUSAL.to_string())
        );
    }

    /// An approved location ends the network window for the whole session.
    #[test]
    fn an_update_check_is_refused_once_a_location_is_approved() {
        let temp = TempDir::new().expect("temp");
        let client = temp.path().join("client-matter");
        fs::create_dir(&client).expect("client folder");
        let mut roots = approve_roots(&[client]).expect("approve");

        let state = ArchiveState::default();
        // Only the approved location closes the window: no census has run and
        // no index exists, so this test pins that one condition alone.
        state
            .session
            .lock()
            .expect("session")
            .locations
            .push(ApprovedLocation {
                id: 1,
                root: roots.remove(0),
            });

        assert_eq!(
            claim_network_window(&state, &state.network.check_spent).err(),
            Some(WINDOW_CLOSED_REFUSAL.to_string()),
            "an update check was permitted after a folder was approved"
        );
        assert!(
            !state.network.check_spent.load(Ordering::Acquire),
            "a refused check still consumed the session's one request"
        );
        // The download path is the same gate, so consenting to an install the
        // operator was offered before approving a folder is refused too.
        assert_eq!(
            claim_network_window(&state, &state.network.download_spent).err(),
            Some(WINDOW_CLOSED_REFUSAL.to_string()),
            "an update download was permitted after a folder was approved"
        );
    }

    /// An existing index ends the network window for the whole session.
    ///
    /// Built through the real vault builder rather than by setting a flag: the
    /// invariant is about the process holding the operator's document text, and
    /// a test that stubs the vault would keep passing if the field it guards
    /// were renamed or replaced.
    #[test]
    fn an_update_check_is_refused_once_an_index_exists() {
        use minutes_archive_core::vault::build_authorized_text_vault;

        let temp = TempDir::new().expect("temp");
        let client = temp.path().join("client-matter");
        fs::create_dir(&client).expect("client folder");
        fs::write(
            client.join("agreement.txt"),
            b"7. CONFIDENTIALITY\nRecipient shall protect Confidential Information.\n",
        )
        .expect("synthetic document");
        let roots = approve_roots(&[client]).expect("approve");
        let vault = build_authorized_text_vault(
            VaultId::parse("gate-probe").expect("vault"),
            &roots,
            DocumentVaultLimits::default(),
            &AtomicBool::new(false),
        )
        .expect("index");
        assert_eq!(
            vault.build_report().indexed_documents,
            1,
            "the fixture built an empty index; this test would prove nothing"
        );

        let state = ArchiveState::default();
        // No approved locations recorded and no census report: the index alone
        // is what must close the window.
        state.session.lock().expect("session").text_vault = Some(vault);

        assert_eq!(
            claim_network_window(&state, &state.network.check_spent).err(),
            Some(WINDOW_CLOSED_REFUSAL.to_string()),
            "an update check was permitted while an index of the operator's documents was open"
        );
    }

    /// Removing what closed the window does not reopen it.
    ///
    /// The live-state read alone would let approve, remove, then check through,
    /// because after the removal the session looks untouched again. It is not:
    /// the operator has pointed this application at an archive, and the process
    /// has held its roots.
    #[test]
    fn removing_an_approved_location_does_not_reopen_the_window() {
        let temp = TempDir::new().expect("temp");
        let client = temp.path().join("client-matter");
        fs::create_dir(&client).expect("client folder");
        let mut roots = approve_roots(&[client]).expect("approve");

        let state = ArchiveState::default();
        {
            let mut session = state.session.lock().expect("session");
            close_network_window(&state);
            session.locations.push(ApprovedLocation {
                id: 1,
                root: roots.remove(0),
            });
            assert_eq!(session.locations.len(), 1);
            // What `remove_archive_location` does to the session.
            session.locations.clear();
            session.last_report = None;
            session.text_vault = None;
            assert!(
                !session_has_seen_archive(&session),
                "the session no longer looks touched, which is what makes the latch necessary"
            );
        }

        assert_eq!(
            claim_network_window(&state, &state.network.check_spent).err(),
            Some(WINDOW_CLOSED_REFUSAL.to_string()),
            "removing the approved location reopened the network window"
        );
    }

    /// A running census closes the window even before it has produced a report.
    #[test]
    fn an_update_check_is_refused_while_a_census_is_running() {
        let state = ArchiveState::default();
        state.session.lock().expect("session").scan.running = true;
        assert_eq!(
            claim_network_window(&state, &state.network.check_spent).err(),
            Some(WINDOW_CLOSED_REFUSAL.to_string())
        );
    }

    /// A direct IPC race cannot overlap the updater with a native folder panel.
    ///
    /// UI buttons are disabled while the request is alive, but that is a
    /// convenience, not the boundary. This exercises the Rust arbitration
    /// that a compromised webview cannot bypass.
    #[test]
    fn an_archive_interaction_cannot_overlap_a_network_operation() {
        let state = ArchiveState::default();
        let claim = claim_network_window(&state, &state.network.check_spent)
            .expect("the launch check should own the operation slot");

        assert_eq!(
            begin_archive_interaction(&state),
            Err(NETWORK_BUSY_REFUSAL.to_string()),
            "a folder panel could open while update traffic was alive"
        );
        assert!(
            state.network.archive_seen.load(Ordering::SeqCst),
            "the racing folder request did not close the network window"
        );

        drop(claim);
        assert_eq!(
            claim_network_window(&state, &state.network.download_spent).err(),
            Some(WINDOW_CLOSED_REFUSAL.to_string()),
            "the completed update operation reopened the raced window"
        );
    }

    #[test]
    fn a_network_operation_cannot_start_after_archive_interaction_begins() {
        let state = ArchiveState::default();
        begin_archive_interaction(&state).expect("an idle launch may open its folder panel");
        assert_eq!(
            claim_network_window(&state, &state.network.check_spent).err(),
            Some(WINDOW_CLOSED_REFUSAL.to_string())
        );
        assert!(
            !state.network.check_spent.load(Ordering::Acquire),
            "a refused race consumed the one launch check"
        );
    }

    #[cfg(target_os = "macos")]
    fn synthetic_update_archive(include_link: bool) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Cursor;

        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for path in [
            "Minutes Archive.app/",
            "Minutes Archive.app/Contents/",
            "Minutes Archive.app/Contents/MacOS/",
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Directory);
            header.set_mode(0o755);
            header.set_size(0);
            header.set_cksum();
            archive
                .append_data(&mut header, path, std::io::empty())
                .expect("append directory");
        }

        let payload = b"synthetic signed executable";
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o755);
        header.set_size(payload.len() as u64);
        header.set_cksum();
        archive
            .append_data(
                &mut header,
                "Minutes Archive.app/Contents/MacOS/minutes-archive-app",
                Cursor::new(payload),
            )
            .expect("append executable");

        if include_link {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_mode(0o777);
            header.set_size(0);
            header.set_cksum();
            archive
                .append_link(
                    &mut header,
                    "Minutes Archive.app/Contents/escape",
                    "/tmp/outside",
                )
                .expect("append link");
        }

        archive
            .into_inner()
            .expect("finish tar")
            .finish()
            .expect("finish gzip")
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn updater_extraction_accepts_only_one_link_free_archive_bundle() {
        let destination = TempDir::new().expect("destination");
        let app = extract_update_archive(&synthetic_update_archive(false), destination.path())
            .expect("extract safe update");
        assert_eq!(
            fs::read(app.join("Contents/MacOS/minutes-archive-app")).expect("read payload"),
            b"synthetic signed executable"
        );

        let refused = TempDir::new().expect("refused destination");
        assert!(
            extract_update_archive(&synthetic_update_archive(true), refused.path()).is_err(),
            "a linked updater archive crossed the staging boundary"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn updater_replacement_is_one_atomic_exchange() {
        let temp = TempDir::new().expect("temp");
        let current = temp.path().join("Minutes Archive.app");
        let staged = temp.path().join("staged.app");
        fs::create_dir(&current).expect("current app");
        fs::create_dir(&staged).expect("staged app");
        fs::write(current.join("identity"), b"current").expect("current marker");
        fs::write(staged.join("identity"), b"staged").expect("staged marker");

        atomic_swap_paths(&current, &staged).expect("atomic app exchange");
        assert_eq!(
            fs::read(current.join("identity")).expect("new app"),
            b"staged"
        );
        assert_eq!(
            fs::read(staged.join("identity")).expect("old app"),
            b"current"
        );
    }

    /// A refusal never reaches the operator as a raw error string.
    ///
    /// `UpdateReport` is the only thing that crosses IPC for the updater, and
    /// the endpoint URL, the download URL and the signature must stay behind
    /// the boundary the same way document paths do.
    #[test]
    fn the_update_report_carries_no_endpoint_or_signature() {
        let report = UpdateReport::Available {
            installed: "0.1.0".to_string(),
            offered: "0.2.0".to_string(),
        };
        let serialized = serde_json::to_string(&report).expect("serialize report");
        assert_eq!(
            serialized,
            r#"{"state":"available","installed":"0.1.0","offered":"0.2.0"}"#
        );
        for forbidden in ["http", "github", "signature", "/"] {
            assert!(
                !serialized.contains(forbidden),
                "{forbidden} reached the interface"
            );
        }

        let refused = serde_json::to_string(&UpdateReport::Refused {
            reason: WINDOW_CLOSED_REFUSAL.to_string(),
        })
        .expect("serialize refusal");
        assert!(refused.contains("quit and reopen"));
        assert!(!refused.contains("http"));

        let failed = serde_json::to_string(&UpdateReport::InstallFailed {
            offered: "0.2.0".to_string(),
        })
        .expect("serialize failed install");
        assert_eq!(failed, r#"{"state":"installFailed","offered":"0.2.0"}"#);
        assert!(!failed.contains("http"));
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
