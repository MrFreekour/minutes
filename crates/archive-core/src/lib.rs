//! Security-bounded primitives for the separate Archive desktop application.
//!
//! The first shipped primitive is a metadata-only census. It deliberately has
//! no API for opening regular files, hashing document bytes, or serializing
//! approved filesystem paths.

use cap_std::ambient_authority;
use cap_std::fs::{Dir, Metadata as CapMetadata};
use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

pub mod retrieval;

pub const CENSUS_SCHEMA: &str = "minutes.archive-census.v1";
pub const MAX_APPROVED_ROOTS: usize = 32;

const PACKAGE_EXTENSIONS: &[(&str, &str)] = &[
    (".pages", "apple_document"),
    (".key", "presentation"),
    (".numbers", "spreadsheet"),
    (".rtfd", "word_processing"),
];

const COMPOUND_EXTENSIONS: &[&str] = &[".tar.gz", ".tar.bz2", ".tar.xz"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CensusLimits {
    pub max_artifacts: u64,
    pub max_directories: u64,
    pub max_depth: u32,
}

impl Default for CensusLimits {
    fn default() -> Self {
        Self {
            max_artifacts: 2_000_000,
            max_directories: 500_000,
            max_depth: 256,
        }
    }
}

impl CensusLimits {
    fn validate(self) -> Result<Self, CensusError> {
        if self.max_artifacts == 0 || self.max_directories == 0 || self.max_depth == 0 {
            return Err(CensusError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CensusError {
    #[error("no archive locations were approved")]
    NoRoots,
    #[error("too many archive locations were approved")]
    TooManyRoots,
    #[error("archive census limits must be greater than zero")]
    InvalidLimits,
    #[error("approved location {location} is unavailable")]
    RootUnavailable { location: usize },
    #[error("approved location {location} must be a directory")]
    RootNotDirectory { location: usize },
    #[error("approved location {location} is a symbolic link or reparse point")]
    RootIsLink { location: usize },
    #[error("approved location {location} is too broad")]
    RootTooBroad { location: usize },
    #[error("the approved locations contain a duplicate")]
    DuplicateRoot,
    #[error("the approved locations overlap")]
    OverlappingRoots,
    #[error("an approved location changed while authority was established")]
    RootChanged,
}

/// One native folder-picker authorization bound to its canonical directory
/// identity at approval time.
///
/// The path and identity remain private, the type is not serializable, and its
/// debug representation is redacted. Native callers retain this object and
/// pass it back to `scan_approved_roots`; a pathname alone is not authority.
#[derive(Clone)]
pub struct ApprovedRoot {
    canonical_path: PathBuf,
    identity: FileIdentity,
}

impl std::fmt::Debug for ApprovedRoot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ApprovedRoot([redacted])")
    }
}

/// Establishes native directory capabilities from explicit folder-picker
/// results.
pub fn approve_roots(roots: &[PathBuf]) -> Result<Vec<ApprovedRoot>, CensusError> {
    if roots.is_empty() {
        return Err(CensusError::NoRoots);
    }
    if roots.len() > MAX_APPROVED_ROOTS {
        return Err(CensusError::TooManyRoots);
    }

    let canonical_home = dirs::home_dir().and_then(|path| fs::canonicalize(path).ok());
    let mut approved = Vec::with_capacity(roots.len());

    for (index, root) in roots.iter().enumerate() {
        let location = index + 1;
        let lexical_metadata =
            fs::symlink_metadata(root).map_err(|_| CensusError::RootUnavailable { location })?;
        if metadata_is_link_or_reparse(&lexical_metadata) {
            return Err(CensusError::RootIsLink { location });
        }
        if !lexical_metadata.is_dir() {
            return Err(CensusError::RootNotDirectory { location });
        }

        let resolved =
            fs::canonicalize(root).map_err(|_| CensusError::RootUnavailable { location })?;
        let metadata =
            fs::metadata(&resolved).map_err(|_| CensusError::RootUnavailable { location })?;
        if !metadata.is_dir() {
            return Err(CensusError::RootNotDirectory { location });
        }
        if resolved.parent().is_none()
            || canonical_home
                .as_ref()
                .is_some_and(|home| home == &resolved)
        {
            return Err(CensusError::RootTooBroad { location });
        }

        approved.push(ApprovedRoot {
            identity: std_metadata_identity(&metadata, &resolved),
            canonical_path: resolved,
        });
    }

    validate_approved_roots(&approved)?;
    Ok(approved)
}

/// Checks set-level duplicate and overlap invariants without re-authorizing
/// stored roots by pathname.
pub fn validate_approved_roots(roots: &[ApprovedRoot]) -> Result<(), CensusError> {
    if roots.is_empty() {
        return Err(CensusError::NoRoots);
    }
    if roots.len() > MAX_APPROVED_ROOTS {
        return Err(CensusError::TooManyRoots);
    }
    let mut identities = HashSet::with_capacity(roots.len());
    for root in roots {
        if !identities.insert(root.identity) {
            return Err(CensusError::DuplicateRoot);
        }
    }
    for left in 0..roots.len() {
        for right in (left + 1)..roots.len() {
            if roots[left]
                .canonical_path
                .starts_with(&roots[right].canonical_path)
                || roots[right]
                    .canonical_path
                    .starts_with(&roots[left].canonical_path)
            {
                return Err(CensusError::OverlappingRoots);
            }
        }
    }
    Ok(())
}

/// Compatibility helper for trusted native diagnostics.
///
/// New long-lived application code should retain `ApprovedRoot` values from
/// `approve_roots` instead of retaining these paths.
pub fn validate_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>, CensusError> {
    Ok(approve_roots(roots)?
        .into_iter()
        .map(|root| root.canonical_path)
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FileIdentity {
    first: u64,
    second: u64,
}

#[cfg(not(unix))]
fn fallback_path_identity(path: &Path) -> FileIdentity {
    use std::hash::{Hash, Hasher};
    let mut first = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut first);
    let first = first.finish();
    let mut second = std::collections::hash_map::DefaultHasher::new();
    first.hash(&mut second);
    FileIdentity {
        first,
        second: second.finish(),
    }
}

#[cfg(unix)]
fn std_metadata_identity(metadata: &fs::Metadata, _path: &Path) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        first: metadata.dev(),
        second: metadata.ino(),
    }
}

#[cfg(windows)]
fn std_metadata_identity(metadata: &fs::Metadata, path: &Path) -> FileIdentity {
    use std::os::windows::fs::MetadataExt;
    match (metadata.volume_serial_number(), metadata.file_index()) {
        (Some(first), Some(second)) => FileIdentity {
            first: u64::from(first),
            second,
        },
        _ => fallback_path_identity(path),
    }
}

#[cfg(not(any(unix, windows)))]
fn std_metadata_identity(_metadata: &fs::Metadata, path: &Path) -> FileIdentity {
    fallback_path_identity(path)
}

#[cfg(unix)]
fn cap_metadata_identity(metadata: &CapMetadata) -> FileIdentity {
    use cap_std::fs::MetadataExt;
    FileIdentity {
        first: metadata.dev(),
        second: metadata.ino(),
    }
}

#[cfg(windows)]
fn cap_metadata_identity(metadata: &CapMetadata) -> Option<FileIdentity> {
    use cap_std::fs::MetadataExt;
    match (metadata.volume_serial_number(), metadata.file_index()) {
        (Some(first), Some(second)) => Some(FileIdentity {
            first: u64::from(first),
            second,
        }),
        _ => None,
    }
}

#[cfg(not(any(unix, windows)))]
fn cap_metadata_identity(_metadata: &CapMetadata) -> Option<FileIdentity> {
    None
}

#[cfg(unix)]
fn cap_identity_matches(metadata: &CapMetadata, expected: FileIdentity) -> bool {
    cap_metadata_identity(metadata) == expected
}

#[cfg(not(unix))]
fn cap_identity_matches(metadata: &CapMetadata, expected: FileIdentity) -> bool {
    cap_metadata_identity(metadata).is_none_or(|identity| identity == expected)
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn cap_metadata_is_link_or_reparse(metadata: &CapMetadata) -> bool {
    use cap_std::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn cap_metadata_is_link_or_reparse(metadata: &CapMetadata) -> bool {
    metadata.file_type().is_symlink()
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CensusStatus {
    Complete,
    Partial,
    Bounded,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CensusPrivacy {
    pub document_content_read: bool,
    pub filenames_emitted: bool,
    pub paths_emitted: bool,
    pub symlinks_followed: bool,
    pub hashes_computed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CensusSummary {
    pub approved_locations: u64,
    pub artifacts: u64,
    pub regular_files: u64,
    pub packages: u64,
    pub regular_file_bytes: u64,
    pub directories_scanned: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FormatSummary {
    pub extension: String,
    pub category: String,
    pub files: u64,
    pub packages: u64,
    pub regular_file_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CategorySummary {
    pub category: String,
    pub artifacts: u64,
    pub regular_file_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct CensusSignals {
    pub symlinks_skipped: u64,
    pub hidden_artifacts: u64,
    pub icloud_placeholders: u64,
    pub zero_byte_files: u64,
    pub permission_mode_unreadable: u64,
    pub special_files_skipped: u64,
    pub metadata_errors: u64,
    pub directory_errors: u64,
    pub max_depth: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CensusReport {
    pub schema: &'static str,
    pub status: CensusStatus,
    pub privacy: CensusPrivacy,
    pub summary: CensusSummary,
    pub formats: Vec<FormatSummary>,
    pub categories: Vec<CategorySummary>,
    pub age_buckets: BTreeMap<String, u64>,
    pub size_buckets: BTreeMap<String, u64>,
    pub signals: CensusSignals,
}

#[derive(Debug, Clone, Default)]
struct FormatAggregate {
    files: u64,
    packages: u64,
    bytes: u64,
}

#[derive(Debug)]
struct CensusAccumulator {
    status: CensusStatus,
    approved_locations: u64,
    regular_files: u64,
    packages: u64,
    regular_file_bytes: u64,
    directories_scanned: u64,
    formats: BTreeMap<String, FormatAggregate>,
    category_counts: BTreeMap<String, u64>,
    category_bytes: BTreeMap<String, u64>,
    age_buckets: BTreeMap<String, u64>,
    size_buckets: BTreeMap<String, u64>,
    signals: CensusSignals,
}

impl CensusAccumulator {
    fn new(approved_locations: usize) -> Self {
        Self {
            status: CensusStatus::Complete,
            approved_locations: approved_locations as u64,
            regular_files: 0,
            packages: 0,
            regular_file_bytes: 0,
            directories_scanned: 0,
            formats: BTreeMap::new(),
            category_counts: BTreeMap::new(),
            category_bytes: BTreeMap::new(),
            age_buckets: BTreeMap::new(),
            size_buckets: BTreeMap::new(),
            signals: CensusSignals::default(),
        }
    }

    fn artifacts(&self) -> u64 {
        self.regular_files + self.packages
    }

    fn mark_partial(&mut self) {
        if self.status == CensusStatus::Complete {
            self.status = CensusStatus::Partial;
        }
    }

    fn record_file(&mut self, name: &OsStr, metadata: &CapMetadata) {
        let extension = extension_for_name(name);
        let category = category_for_extension(&extension).to_string();
        let size = metadata.len();

        self.regular_files += 1;
        self.regular_file_bytes = self.regular_file_bytes.saturating_add(size);
        let format = self.formats.entry(extension.clone()).or_default();
        format.files += 1;
        format.bytes = format.bytes.saturating_add(size);
        *self.category_counts.entry(category.clone()).or_default() += 1;
        let category_bytes = self.category_bytes.entry(category).or_default();
        *category_bytes = category_bytes.saturating_add(size);
        *self
            .size_buckets
            .entry(size_bucket(size).to_string())
            .or_default() += 1;
        *self
            .age_buckets
            .entry(age_bucket(metadata).to_string())
            .or_default() += 1;

        if is_hidden(name) {
            self.signals.hidden_artifacts += 1;
        }
        if extension == ".icloud" {
            self.signals.icloud_placeholders += 1;
        }
        if size == 0 {
            self.signals.zero_byte_files += 1;
        }
        if permission_mode_unreadable(metadata) {
            self.signals.permission_mode_unreadable += 1;
        }
    }

    fn record_package(&mut self, name: &OsStr, extension: &str, category: &str) {
        self.packages += 1;
        self.formats
            .entry(extension.to_string())
            .or_default()
            .packages += 1;
        *self
            .category_counts
            .entry(category.to_string())
            .or_default() += 1;
        if is_hidden(name) {
            self.signals.hidden_artifacts += 1;
        }
    }

    fn finish(self) -> CensusReport {
        let formats = self
            .formats
            .into_iter()
            .map(|(extension, aggregate)| FormatSummary {
                category: category_for_extension(&extension).to_string(),
                extension,
                files: aggregate.files,
                packages: aggregate.packages,
                regular_file_bytes: aggregate.bytes,
            })
            .collect();
        let categories = self
            .category_counts
            .into_iter()
            .map(|(category, artifacts)| CategorySummary {
                regular_file_bytes: self
                    .category_bytes
                    .get(&category)
                    .copied()
                    .unwrap_or_default(),
                category,
                artifacts,
            })
            .collect();
        CensusReport {
            schema: CENSUS_SCHEMA,
            status: self.status,
            privacy: CensusPrivacy {
                document_content_read: false,
                filenames_emitted: false,
                paths_emitted: false,
                symlinks_followed: false,
                hashes_computed: false,
            },
            summary: CensusSummary {
                approved_locations: self.approved_locations,
                artifacts: self.regular_files + self.packages,
                regular_files: self.regular_files,
                packages: self.packages,
                regular_file_bytes: self.regular_file_bytes,
                directories_scanned: self.directories_scanned,
            },
            formats,
            categories,
            age_buckets: self.age_buckets,
            size_buckets: self.size_buckets,
            signals: self.signals,
        }
    }
}

#[derive(Debug)]
struct PendingDirectory {
    directory: Dir,
    depth: u32,
}

/// Scans approved roots without opening a regular file.
///
/// Directory traversal is rooted in `cap_std::fs::Dir` handles. Stable
/// symlinks and reparse points are counted and skipped. A child directory is
/// accepted only when the identity observed before opening it matches the
/// opened handle.
pub fn scan_roots(
    roots: &[PathBuf],
    limits: CensusLimits,
    cancelled: &AtomicBool,
) -> Result<CensusReport, CensusError> {
    let approved = approve_roots(roots)?;
    scan_approved_roots(&approved, limits, cancelled)
}

/// Scans only directory identities retained from explicit approval.
pub fn scan_approved_roots(
    roots: &[ApprovedRoot],
    limits: CensusLimits,
    cancelled: &AtomicBool,
) -> Result<CensusReport, CensusError> {
    let limits = limits.validate()?;
    validate_approved_roots(roots)?;
    let mut stack = Vec::with_capacity(roots.len());

    for root in roots {
        let lexical_metadata =
            fs::symlink_metadata(&root.canonical_path).map_err(|_| CensusError::RootChanged)?;
        if metadata_is_link_or_reparse(&lexical_metadata) || !lexical_metadata.is_dir() {
            return Err(CensusError::RootChanged);
        }
        let current_path =
            fs::canonicalize(&root.canonical_path).map_err(|_| CensusError::RootChanged)?;
        if current_path != root.canonical_path {
            return Err(CensusError::RootChanged);
        }
        let ambient_metadata =
            fs::metadata(&root.canonical_path).map_err(|_| CensusError::RootChanged)?;
        if std_metadata_identity(&ambient_metadata, &root.canonical_path) != root.identity {
            return Err(CensusError::RootChanged);
        }
        let directory = Dir::open_ambient_dir(&root.canonical_path, ambient_authority())
            .map_err(|_| CensusError::RootChanged)?;
        let opened_metadata = directory
            .dir_metadata()
            .map_err(|_| CensusError::RootChanged)?;
        if !cap_identity_matches(&opened_metadata, root.identity) {
            return Err(CensusError::RootChanged);
        }
        stack.push(PendingDirectory {
            directory,
            depth: 0,
        });
    }

    let mut census = CensusAccumulator::new(roots.len());

    while let Some(current) = stack.pop() {
        if cancelled.load(Ordering::Acquire) {
            census.status = CensusStatus::Cancelled;
            return Ok(census.finish());
        }
        if census.directories_scanned >= limits.max_directories {
            census.status = CensusStatus::Bounded;
            return Ok(census.finish());
        }
        census.signals.max_depth = census.signals.max_depth.max(current.depth);
        let entries = match current.directory.entries() {
            Ok(entries) => entries,
            Err(_) => {
                census.signals.directory_errors += 1;
                census.mark_partial();
                continue;
            }
        };
        census.directories_scanned += 1;

        for entry_result in entries {
            if cancelled.load(Ordering::Acquire) {
                census.status = CensusStatus::Cancelled;
                return Ok(census.finish());
            }
            if census.artifacts() >= limits.max_artifacts {
                census.status = CensusStatus::Bounded;
                return Ok(census.finish());
            }

            let entry = match entry_result {
                Ok(entry) => entry,
                Err(_) => {
                    census.signals.directory_errors += 1;
                    census.mark_partial();
                    continue;
                }
            };
            let name = entry.file_name();
            let metadata = match current.directory.symlink_metadata(&name) {
                Ok(metadata) => metadata,
                Err(_) => {
                    census.signals.metadata_errors += 1;
                    census.mark_partial();
                    continue;
                }
            };

            if cap_metadata_is_link_or_reparse(&metadata) {
                census.signals.symlinks_skipped += 1;
                continue;
            }

            if metadata.is_dir() {
                let extension = extension_for_name(&name);
                if let Some(category) = package_category(&extension) {
                    census.record_package(&name, &extension, category);
                    continue;
                }
                if current.depth >= limits.max_depth {
                    census.status = CensusStatus::Bounded;
                    return Ok(census.finish());
                }

                let expected = cap_metadata_identity_portable(&metadata);
                let child = match entry.open_dir() {
                    Ok(child) => child,
                    Err(_) => {
                        census.signals.directory_errors += 1;
                        census.mark_partial();
                        continue;
                    }
                };
                let opened_metadata = match child.dir_metadata() {
                    Ok(metadata) => metadata,
                    Err(_) => {
                        census.signals.metadata_errors += 1;
                        census.mark_partial();
                        continue;
                    }
                };
                if cap_metadata_is_link_or_reparse(&opened_metadata)
                    || expected
                        .zip(cap_metadata_identity_portable(&opened_metadata))
                        .is_some_and(|(before, after)| before != after)
                {
                    census.signals.symlinks_skipped += 1;
                    census.mark_partial();
                    continue;
                }
                stack.push(PendingDirectory {
                    directory: child,
                    depth: current.depth + 1,
                });
            } else if metadata.is_file() {
                census.record_file(&name, &metadata);
            } else {
                census.signals.special_files_skipped += 1;
            }
        }
    }

    Ok(census.finish())
}

#[cfg(unix)]
fn cap_metadata_identity_portable(metadata: &CapMetadata) -> Option<FileIdentity> {
    Some(cap_metadata_identity(metadata))
}

#[cfg(not(unix))]
fn cap_metadata_identity_portable(metadata: &CapMetadata) -> Option<FileIdentity> {
    cap_metadata_identity(metadata)
}

fn is_hidden(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

fn extension_for_name(name: &OsStr) -> String {
    let Some(name) = name.to_str() else {
        return "<other>".to_string();
    };
    let lowered = name.to_lowercase();
    for extension in COMPOUND_EXTENSIONS {
        if lowered.ends_with(extension) {
            return (*extension).to_string();
        }
    }
    let Some(extension) = Path::new(&lowered).extension().and_then(OsStr::to_str) else {
        return "<none>".to_string();
    };
    if extension.is_empty()
        || extension.len() > 12
        || !extension.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && matches!(byte, b'+' | b'_' | b'-'))
        })
    {
        return "<other>".to_string();
    }
    format!(".{extension}")
}

fn package_category(extension: &str) -> Option<&'static str> {
    PACKAGE_EXTENSIONS
        .iter()
        .find_map(|(candidate, category)| (*candidate == extension).then_some(*category))
}

fn category_for_extension(extension: &str) -> &'static str {
    match extension {
        ".pdf" => "pdf",
        ".doc" | ".docm" | ".docx" | ".dot" | ".dotm" | ".dotx" | ".odt" | ".rtf" | ".rtfd"
        | ".wpd" | ".wps" => "word_processing",
        ".eml" | ".mbox" | ".msg" | ".olm" | ".ost" | ".pst" => "email",
        ".htm" | ".html" | ".md" | ".text" | ".txt" | ".xml" => "plain_text",
        ".bmp" | ".gif" | ".heic" | ".jpeg" | ".jpg" | ".png" | ".tif" | ".tiff" => "image_or_scan",
        ".csv" | ".numbers" | ".ods" | ".xls" | ".xlsb" | ".xlsm" | ".xlsx" => "spreadsheet",
        ".key" | ".odp" | ".ppt" | ".pptm" | ".pptx" => "presentation",
        ".7z" | ".bz2" | ".gz" | ".rar" | ".tar" | ".tar.bz2" | ".tar.gz" | ".tar.xz" | ".xz"
        | ".zip" => "archive",
        ".accdb" | ".db" | ".mdb" | ".sqlite" | ".sqlite3" => "database",
        ".pages" => "apple_document",
        ".icloud" => "icloud_placeholder",
        _ => "other",
    }
}

fn size_bucket(size: u64) -> &'static str {
    if size == 0 {
        "zero"
    } else if size < 100 * 1024 {
        "under_100_kib"
    } else if size < 1024 * 1024 {
        "100_kib_to_1_mib"
    } else if size < 10 * 1024 * 1024 {
        "1_mib_to_10_mib"
    } else if size < 100 * 1024 * 1024 {
        "10_mib_to_100_mib"
    } else {
        "100_mib_or_more"
    }
}

fn age_bucket(metadata: &CapMetadata) -> &'static str {
    let Ok(modified) = metadata.modified() else {
        return "unknown";
    };
    let year = DateTime::<Utc>::from(modified.into_std()).year();
    match year {
        ..=1999 => "through_1999",
        2000..=2009 => "2000_2009",
        2010..=2019 => "2010_2019",
        2020..=2024 => "2020_2024",
        _ => "2025_or_later",
    }
}

#[cfg(unix)]
fn permission_mode_unreadable(metadata: &CapMetadata) -> bool {
    use cap_std::fs::PermissionsExt;
    metadata.permissions().mode() & 0o444 == 0
}

#[cfg(not(unix))]
fn permission_mode_unreadable(_metadata: &CapMetadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    fn synthetic_root(temp: &TempDir, name: &str) -> PathBuf {
        let root = temp.path().join(name);
        fs::create_dir(&root).expect("create synthetic root");
        root
    }

    fn scan(root: &Path) -> CensusReport {
        scan_roots(
            &[root.to_path_buf()],
            CensusLimits::default(),
            &AtomicBool::new(false),
        )
        .expect("synthetic census")
    }

    #[test]
    fn census_reports_aggregates_without_names_paths_or_content() {
        let temp = TempDir::new().expect("temp");
        let root = synthetic_root(&temp, "synthetic-archive");
        let secret_filename = "Privileged Client Alpha Settlement.pdf";
        let secret_content = "SYNTHETIC_SECRET_CANARY";
        fs::write(root.join(secret_filename), secret_content).expect("pdf");
        fs::write(root.join("legacy.WPD"), b"wpd").expect("wpd");
        fs::write(root.join("letter.DOCX"), b"docx").expect("docx");
        fs::write(root.join(".hidden-client.rtf"), b"").expect("hidden");
        fs::write(root.join("README"), b"x").expect("no extension");
        fs::write(root.join("placeholder.pdf.icloud"), b"").expect("placeholder");

        let package = root.join("Synthetic Matter.pages");
        fs::create_dir(&package).expect("package");
        fs::write(package.join("index.xml"), b"PACKAGE_SECRET_CANARY").expect("package file");

        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join(secret_filename), root.join("alias-to-secret"))
            .expect("symlink");

        let report = scan(&root);
        let serialized = serde_json::to_string(&report).expect("serialize");

        assert_eq!(report.schema, CENSUS_SCHEMA);
        assert_eq!(report.status, CensusStatus::Complete);
        assert_eq!(report.summary.regular_files, 6);
        assert_eq!(report.summary.packages, 1);
        assert_eq!(report.signals.icloud_placeholders, 1);
        assert_eq!(report.signals.hidden_artifacts, 1);
        #[cfg(unix)]
        assert_eq!(report.signals.symlinks_skipped, 1);
        assert!(!serialized.contains("index.xml"));
        assert!(!serialized.contains(secret_filename));
        assert!(!serialized.contains(secret_content));
        assert!(!serialized.contains("PACKAGE_SECRET_CANARY"));
        assert!(!serialized.contains(&temp.path().to_string_lossy().to_string()));

        let formats = report
            .formats
            .iter()
            .map(|item| (item.extension.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(formats[".pdf"].files, 1);
        assert_eq!(formats[".wpd"].files, 1);
        assert_eq!(formats[".docx"].files, 1);
        assert_eq!(formats[".pages"].packages, 1);
        assert!(!formats.contains_key(".xml"));
    }

    #[test]
    fn census_never_needs_regular_file_read_permission() {
        let temp = TempDir::new().expect("temp");
        let root = synthetic_root(&temp, "metadata-only");
        let file = root.join("unreadable.pdf");
        fs::write(&file, b"must not be opened").expect("file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).expect("make unreadable");
        }

        let report = scan(&root);
        assert_eq!(report.summary.regular_files, 1);
        assert!(!report.privacy.document_content_read);
        #[cfg(unix)]
        assert_eq!(report.signals.permission_mode_unreadable, 1);
    }

    #[test]
    fn package_contents_are_not_traversed() {
        let temp = TempDir::new().expect("temp");
        let root = synthetic_root(&temp, "packages");
        let package = root.join("Matter.pages");
        fs::create_dir(&package).expect("package");
        fs::write(package.join("inside.pdf"), b"private").expect("inside");

        let report = scan(&root);
        assert_eq!(report.summary.packages, 1);
        assert_eq!(report.summary.regular_files, 0);
        assert_eq!(report.summary.directories_scanned, 1);
    }

    #[test]
    fn multiple_non_overlapping_roots_form_one_census() {
        let temp = TempDir::new().expect("temp");
        let first = synthetic_root(&temp, "first");
        let second = synthetic_root(&temp, "second");
        fs::write(first.join("one.pdf"), b"one").expect("first file");
        fs::write(second.join("two.docx"), b"two").expect("second file");

        let report = scan_roots(
            &[first, second],
            CensusLimits::default(),
            &AtomicBool::new(false),
        )
        .expect("multi-root census");
        assert_eq!(report.summary.approved_locations, 2);
        assert_eq!(report.summary.regular_files, 2);
    }

    #[test]
    fn overlapping_and_duplicate_roots_are_refused() {
        let temp = TempDir::new().expect("temp");
        let root = synthetic_root(&temp, "root");
        let child = root.join("child");
        fs::create_dir(&child).expect("child");

        assert_eq!(
            validate_roots(&[root.clone(), root.clone()]),
            Err(CensusError::DuplicateRoot)
        );
        assert_eq!(
            validate_roots(&[root, child]),
            Err(CensusError::OverlappingRoots)
        );
    }

    #[test]
    fn approved_root_replacement_is_not_reauthorized_by_path() {
        let temp = TempDir::new().expect("temp");
        let root = synthetic_root(&temp, "approved-root");
        fs::write(root.join("original.pdf"), b"original").expect("original");
        let approved = approve_roots(std::slice::from_ref(&root)).expect("approval");

        let moved = temp.path().join("moved-original");
        fs::rename(&root, &moved).expect("move original");
        fs::create_dir(&root).expect("replacement root");
        fs::write(root.join("replacement.pdf"), b"replacement").expect("replacement");

        assert_eq!(
            scan_approved_roots(&approved, CensusLimits::default(), &AtomicBool::new(false)),
            Err(CensusError::RootChanged)
        );
    }

    #[test]
    fn home_and_filesystem_roots_are_refused() {
        let home = dirs::home_dir().expect("home");
        assert!(matches!(
            validate_roots(&[home]),
            Err(CensusError::RootTooBroad { .. })
        ));

        let filesystem_root = PathBuf::from(std::path::MAIN_SEPARATOR.to_string());
        assert!(matches!(
            validate_roots(&[filesystem_root]),
            Err(CensusError::RootTooBroad { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn root_symlinks_are_refused_and_nested_symlinks_are_skipped() {
        let temp = TempDir::new().expect("temp");
        let root = synthetic_root(&temp, "root");
        let target = synthetic_root(&temp, "target");
        fs::write(target.join("outside.pdf"), b"outside").expect("outside");
        std::os::unix::fs::symlink(&target, root.join("nested")).expect("nested link");
        let root_link = temp.path().join("root-link");
        std::os::unix::fs::symlink(&root, &root_link).expect("root link");

        assert!(matches!(
            validate_roots(&[root_link]),
            Err(CensusError::RootIsLink { .. })
        ));
        let report = scan(&root);
        assert_eq!(report.summary.regular_files, 0);
        assert_eq!(report.signals.symlinks_skipped, 1);
    }

    #[test]
    fn census_honors_artifact_bounds_and_cancellation() {
        let temp = TempDir::new().expect("temp");
        let root = synthetic_root(&temp, "bounded");
        for index in 0..4 {
            fs::write(root.join(format!("synthetic-{index}.txt")), b"synthetic").expect("file");
        }
        let bounded = scan_roots(
            std::slice::from_ref(&root),
            CensusLimits {
                max_artifacts: 2,
                ..CensusLimits::default()
            },
            &AtomicBool::new(false),
        )
        .expect("bounded");
        assert_eq!(bounded.status, CensusStatus::Bounded);
        assert_eq!(bounded.summary.artifacts, 2);

        let cancelled = AtomicBool::new(true);
        let cancelled_report =
            scan_roots(&[root], CensusLimits::default(), &cancelled).expect("cancelled");
        assert_eq!(cancelled_report.status, CensusStatus::Cancelled);
        assert_eq!(cancelled_report.summary.artifacts, 0);
    }

    #[test]
    fn unusual_extensions_are_collapsed() {
        assert_eq!(extension_for_name(OsStr::new("document")), "<none>");
        assert_eq!(
            extension_for_name(OsStr::new("document.client-secret-extension")),
            "<other>"
        );
        assert_eq!(extension_for_name(OsStr::new("ARCHIVE.TAR.GZ")), ".tar.gz");
    }
}
