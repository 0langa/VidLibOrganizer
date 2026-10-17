use crate::{ids::*, model::*, platform, *};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    ffi::{OsStr, OsString},
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Default)]
pub struct DirectoryListing {
    /// Exact immediate names returned by enumeration, before any stat calls.
    pub names: Vec<OsString>,
    /// Enumeration can return useful names and still be incomplete.
    pub errors: Vec<String>,
}
/// Read-only OS seam; adapters must not follow links/reparse points for stat.
/// Listing is explicit instead of an opaque iterator so partial enumeration
/// errors and every observed name remain available to coverage consumers.
pub trait InventoryFs {
    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata>;
    fn read_directory(&self, path: &Path) -> io::Result<DirectoryListing> {
        self.open_directory(path)?.read_directory()
    }
    fn open_directory(&self, path: &Path) -> io::Result<Box<dyn InventoryDirectory + '_>> {
        platform::open_directory(path)
    }
    fn directory_is_current(&self, path: &Path, identity: &PhysicalId) -> bool {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return false;
        };
        kind(&metadata) == EntryKind::Directory
            && platform::physical_id(path, &metadata).as_ref() == Some(identity)
    }
    fn physical_id(&self, path: &Path, metadata: &fs::Metadata) -> Option<PhysicalId> {
        platform::physical_id(path, metadata)
    }
}
/// Child operations are relative to this live directory handle, never its
/// display/absolute path. Implementations reject links/reparse points on open.
pub trait InventoryDirectory {
    fn open_child_file(&self, _name: &OsStr) -> io::Result<Box<dyn InventoryFile + '_>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "content reader unavailable",
        ))
    }
    fn observation(&self) -> io::Result<FileObservation>;
    fn read_directory(&self) -> io::Result<DirectoryListing>;
    fn observe_child(&self, name: &OsStr) -> io::Result<FileObservation>;
    fn open_child_directory(&self, name: &OsStr) -> io::Result<Box<dyn InventoryDirectory + '_>>;
}
#[derive(Clone, Debug)]
pub struct FileObservation {
    pub kind: EntryKind,
    pub stamp: FileStamp,
}

pub struct OsInventoryFs;
impl InventoryFs for OsInventoryFs {
    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
        fs::symlink_metadata(path)
    }
}
#[derive(Debug)]
pub enum InventoryError {
    InvalidRequest(String),
}
impl fmt::Display for InventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) => write!(f, "invalid inventory request: {message}"),
        }
    }
}
impl Error for InventoryError {}

pub fn scan(request: &InventoryRequest, control: &JobControl) -> Result<Inventory, InventoryError> {
    scan_with_fs(request, control, &OsInventoryFs)
}

pub fn scan_with_fs(
    request: &InventoryRequest,
    control: &JobControl,
    fs: &dyn InventoryFs,
) -> Result<Inventory, InventoryError> {
    if request.roots.is_empty() {
        return Err(InventoryError::InvalidRequest(
            "at least one native source is required".into(),
        ));
    }
    for root in &request.roots {
        if !root.path.is_absolute()
            || root
                .path
                .components()
                .any(|c| matches!(c, Component::ParentDir))
            || root.path != root.source_grant.path
        {
            return Err(InventoryError::InvalidRequest(
                "root must match its absolute backend source grant".into(),
            ));
        }
    }
    let started_ns = platform::now_ns();
    let mut scanner = Scanner {
        fs,
        control,
        policy: &request.policy,
        fingerprint: fingerprint(&request.policy),
        entries: Vec::new(),
        coverage: Vec::new(),
        issues: Vec::new(),
        seen_entries: HashSet::new(),
        seen_directories: HashMap::new(),
    };
    control.report_progress(Progress {
        phase: ProgressPhase::Scanning,
        done: 0,
        total: None,
        current_entry: None,
    });
    // Shallow roots first makes ownership stable under overlapping input order.
    let mut roots = request.roots.clone();
    roots.sort_by(|left, right| {
        left.path
            .components()
            .count()
            .cmp(&right.path.components().count())
            .then(left.path.cmp(&right.path))
    });
    for root in &mut roots {
        // A binding's previous result is not an observation from this scan.
        root.evidence.observed_identity = None;
        root.evidence.state = if root.evidence.expected_identity.is_some() {
            RootEvidenceState::Unavailable
        } else {
            RootEvidenceState::Absent
        };
        if scanner.protected(&root.path) {
            scanner.empty_scope(root, &root.path, CoverageOutcome::Excluded, None);
            continue;
        }
        if control.is_cancelled() {
            scanner.empty_scope(
                root,
                &root.path,
                CoverageOutcome::Cancelled,
                Some(issue(
                    IssueCode::Cancelled,
                    None,
                    "cancelled before source observation",
                )),
            );
            continue;
        }
        match fs.symlink_metadata(&root.path) {
            Ok(metadata) => {
                let identity = fs.physical_id(&root.path, &metadata);
                root.evidence.observed_identity = identity.clone();
                if let Some(expected) = &root.evidence.expected_identity {
                    root.evidence.state = match &identity {
                        Some(observed) if expected == observed => RootEvidenceState::Matches,
                        Some(_) => RootEvidenceState::Mismatch,
                        None => RootEvidenceState::Unavailable,
                    };
                }
                if identity.is_some() {
                    root.capabilities.physical_identity = Support::Supported;
                }
                if root.evidence.state == RootEvidenceState::Mismatch {
                    let mismatch = issue(
                        IssueCode::EvidenceMismatch,
                        None,
                        "source identity differs from binding evidence",
                    );
                    scanner.empty_scope(root, &root.path, CoverageOutcome::Failed, Some(mismatch));
                    continue;
                }
                if let Some(index) = scanner.seen_directories.get(&root.path) {
                    let mut coverage = scanner.coverage[*index].clone();
                    coverage.root_id = root.id;
                    coverage.relative_directory = None;
                    scanner.coverage.push(coverage);
                    continue;
                }
                if scanner.seen_entries.contains(&root.path) {
                    continue;
                }
                if kind(&metadata) == EntryKind::Directory {
                    let expected = FileObservation {
                        kind: EntryKind::Directory,
                        stamp: platform::stamp(&metadata, identity),
                    };
                    if root.source_grant.source == GrantSource::NativeSelectedEntry {
                        if let Some(base) = root.path.parent() {
                            scanner.entry(root, base, &root.path, Ok(expected.clone()));
                        }
                    }
                    scanner.directory(
                        root,
                        &root.path,
                        &root.path,
                        fs.open_directory(&root.path),
                        &expected,
                        &[],
                    );
                } else {
                    let base = root.path.parent().unwrap_or(&root.path);
                    let observation = FileObservation {
                        kind: kind(&metadata),
                        stamp: platform::stamp(&metadata, identity),
                    };
                    scanner.entry(root, base, &root.path, Ok(observation));
                }
            }
            Err(error) => {
                let failed = issue(IssueCode::StatFailed, None, error.to_string());
                scanner.empty_scope(root, &root.path, CoverageOutcome::Failed, Some(failed));
            }
        }
    }
    control.report_progress(Progress {
        phase: if control.is_cancelled() {
            ProgressPhase::Cancelled
        } else {
            ProgressPhase::Complete
        },
        done: scanner.entries.len() as u64,
        total: None,
        current_entry: None,
    });
    Ok(Inventory {
        id: InventoryId::new(),
        generation: request.generation,
        roots,
        entries: scanner.entries,
        coverage: scanner.coverage,
        issues: scanner.issues,
        window: ObservationWindow {
            started_ns,
            finished_ns: platform::now_ns(),
        },
    })
}

struct Scanner<'a> {
    fs: &'a dyn InventoryFs,
    control: &'a JobControl,
    policy: &'a ProtectedPolicy,
    fingerprint: String,
    entries: Vec<EntrySnapshot>,
    coverage: Vec<ScopeCoverage>,
    issues: Vec<InventoryIssue>,
    seen_entries: HashSet<PathBuf>,
    seen_directories: HashMap<PathBuf, usize>,
}
impl Scanner<'_> {
    fn protected(&self, path: &Path) -> bool {
        self.policy.excludes(path)
    }
    fn empty_scope(
        &mut self,
        root: &RootBinding,
        directory: &Path,
        outcome: CoverageOutcome,
        error: Option<InventoryIssue>,
    ) -> usize {
        let now = platform::now_ns();
        let index = self.coverage.len();
        let issues: Vec<_> = error.into_iter().collect();
        self.issues.extend(issues.clone());
        self.coverage.push(ScopeCoverage {
            root_id: root.id,
            directory: directory.to_path_buf(),
            relative_directory: relative(directory, &root.path),
            listed_names: Vec::new(),
            children_complete: false,
            subtree_complete: outcome == CoverageOutcome::Excluded,
            outcome,
            filter_fingerprint: self.fingerprint.clone(),
            window: ObservationWindow {
                started_ns: now,
                finished_ns: now,
            },
            issues,
        });
        index
    }
    fn directory(
        &mut self,
        root: &RootBinding,
        base: &Path,
        directory: &Path,
        opened: io::Result<Box<dyn InventoryDirectory + '_>>,
        expected: &FileObservation,
        ancestors: &[(PathBuf, Option<PhysicalId>)],
    ) -> bool {
        if let Some(index) = self.seen_directories.get(directory) {
            return self.coverage[*index].subtree_complete;
        }
        let index = self.empty_scope(root, directory, CoverageOutcome::Failed, None);
        self.seen_directories.insert(directory.to_path_buf(), index);
        if self.control.is_cancelled() {
            self.finish_scope(
                index,
                false,
                CoverageOutcome::Cancelled,
                vec![issue(
                    IssueCode::Cancelled,
                    relative(directory, base),
                    "directory observation cancelled",
                )],
            );
            return false;
        }
        let handle = match opened {
            Ok(handle) => handle,
            Err(error) => {
                self.finish_scope(
                    index,
                    false,
                    CoverageOutcome::Failed,
                    vec![issue(
                        IssueCode::ListingFailed,
                        relative(directory, base),
                        error.to_string(),
                    )],
                );
                return false;
            }
        };
        let observed = match handle.observation() {
            Ok(observed) if observed.kind == EntryKind::Directory => observed,
            _ => {
                self.finish_scope(
                    index,
                    false,
                    CoverageOutcome::Failed,
                    vec![issue(
                        IssueCode::DirectoryChanged,
                        relative(directory, base),
                        "invalid or non-directory retained handle",
                    )],
                );
                return false;
            }
        };
        let identity = match (&expected.stamp.physical_id, observed.stamp.physical_id) {
            (Some(expected), Some(observed)) if expected != &observed => {
                self.finish_scope(
                    index,
                    false,
                    CoverageOutcome::Failed,
                    vec![issue(
                        IssueCode::DirectoryChanged,
                        relative(directory, base),
                        "directory identity changed between stat and open",
                    )],
                );
                return false;
            }
            (Some(_), Some(observed)) => Some(observed),
            // A valid handle still anchors child operations. Missing optional
            // identity prevents complete coverage, not safe read-only observations.
            _ => None,
        };
        let mut anchors = ancestors.to_vec();
        anchors.push((directory.to_path_buf(), identity));
        if !self.anchors_current(&anchors) {
            self.finish_scope(
                index,
                false,
                CoverageOutcome::Failed,
                vec![issue(
                    IssueCode::DirectoryChanged,
                    relative(directory, base),
                    "observed directory or ancestor no longer matches the live namespace",
                )],
            );
            return false;
        }
        let mut listing = match handle.read_directory() {
            Ok(listing) => listing,
            Err(error) => {
                self.finish_scope(
                    index,
                    false,
                    CoverageOutcome::Failed,
                    vec![issue(
                        IssueCode::ListingFailed,
                        relative(directory, base),
                        error.to_string(),
                    )],
                );
                return false;
            }
        };
        listing.names.sort();
        listing.names.dedup();
        self.coverage[index].listed_names = listing
            .names
            .iter()
            .map(|name| encode_path(Path::new(name)))
            .collect();
        self.coverage[index].children_complete = listing.errors.is_empty();
        let identity_available = anchors.iter().all(|(_, identity)| identity.is_some());
        let mut complete = listing.errors.is_empty() && identity_available;
        let mut issues: Vec<_> = listing
            .errors
            .into_iter()
            .map(|error| issue(IssueCode::ListingFailed, relative(directory, base), error))
            .collect();
        if !identity_available {
            issues.push(issue(IssueCode::IdentityUnavailable, relative(directory, base),
                "optional directory or ancestor identity unavailable; safe handle observations retained with incomplete coverage"));
        }
        for name in listing.names {
            if self.control.is_cancelled() {
                complete = false;
                break;
            }
            if !self.anchors_current(&anchors) {
                complete = false;
                break;
            }
            let name_path = Path::new(&name);
            if name_path.components().count() != 1
                || !matches!(name_path.components().next(), Some(Component::Normal(_)))
            {
                complete = false;
                self.coverage[index].children_complete = false;
                issues.push(issue(
                    IssueCode::ListingFailed,
                    relative(directory, base),
                    "enumeration returned a non-child name",
                ));
                continue;
            }
            let path = directory.join(&name);
            if self.protected(&path) {
                continue;
            }
            let observation = handle.observe_child(&name);
            let child_directory = observation
                .as_ref()
                .ok()
                .filter(|observed| observed.kind == EntryKind::Directory)
                .cloned();
            if !self.entry(root, base, &path, observation) {
                complete = false;
            }
            if let Some(expected) = child_directory {
                if !self.directory(
                    root,
                    base,
                    &path,
                    handle.open_child_directory(&name),
                    &expected,
                    &anchors,
                ) {
                    complete = false;
                }
            }
        }
        if !self.anchors_current(&anchors) {
            complete = false;
            issues.push(issue(
                IssueCode::DirectoryChanged,
                relative(directory, base),
                "directory or ancestor changed during observation",
            ));
        }
        let outcome = if self.control.is_cancelled() {
            complete = false;
            issues.push(issue(
                IssueCode::Cancelled,
                relative(directory, base),
                "directory subtree observation cancelled",
            ));
            CoverageOutcome::Cancelled
        } else if complete {
            CoverageOutcome::Complete
        } else {
            CoverageOutcome::Failed
        };
        self.finish_scope(index, complete, outcome, issues);
        complete
    }
    fn anchors_current(&self, anchors: &[(PathBuf, Option<PhysicalId>)]) -> bool {
        anchors.iter().all(|(path, identity)| {
            identity
                .as_ref()
                .is_none_or(|identity| self.fs.directory_is_current(path, identity))
        })
    }
    fn finish_scope(
        &mut self,
        index: usize,
        complete: bool,
        outcome: CoverageOutcome,
        issues: Vec<InventoryIssue>,
    ) {
        self.issues.extend(issues.clone());
        let scope = &mut self.coverage[index];
        scope.subtree_complete = complete;
        scope.outcome = outcome;
        scope.issues.extend(issues);
        scope.window.finished_ns = platform::now_ns();
    }
    fn entry(
        &mut self,
        root: &RootBinding,
        base: &Path,
        path: &Path,
        metadata: io::Result<FileObservation>,
    ) -> bool {
        if !self.seen_entries.insert(path.to_path_buf()) {
            return metadata.is_ok();
        }
        let Some(relative_path) = relative(path, base) else {
            self.issues.push(issue(
                IssueCode::InvalidSource,
                None,
                "entry is not a validated root-relative path",
            ));
            return false;
        };
        let mut issues = Vec::new();
        let complete = metadata.is_ok();
        let (kind, stamp) = match metadata {
            Ok(observed) => (observed.kind, observed.stamp),
            Err(error) => {
                issues.push(issue(
                    IssueCode::StatFailed,
                    Some(relative_path.clone()),
                    error.to_string(),
                ));
                (EntryKind::Unknown, FileStamp::default())
            }
        };
        if path.file_name().is_some_and(|name| name.to_str().is_none()) {
            issues.push(issue(
                IssueCode::UnsupportedName,
                Some(relative_path.clone()),
                "name cannot be used in Unicode transformations without loss",
            ));
        }
        self.issues.extend(issues.clone());
        let id = EntryId::new();
        self.entries.push(EntrySnapshot {
            id,
            root_id: root.id,
            relative_path,
            absolute_path: path.to_path_buf(),
            display_name: path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            display_path: path.to_string_lossy().into_owned(),
            kind,
            stamp,
            observed_ns: platform::now_ns(),
            issues,
        });
        self.control.report_progress(Progress {
            phase: ProgressPhase::Scanning,
            done: self.entries.len() as u64,
            total: None,
            current_entry: Some(id),
        });
        complete
    }
}
fn relative(path: &Path, base: &Path) -> Option<NativePathRecord> {
    let record = encode_path(path.strip_prefix(base).ok()?);
    record.validate_relative().ok()?;
    Some(record)
}
fn issue(
    code: IssueCode,
    relative_path: Option<NativePathRecord>,
    message: impl Into<String>,
) -> InventoryIssue {
    InventoryIssue {
        code,
        relative_path,
        message: message.into(),
    }
}
pub(crate) fn kind(metadata: &fs::Metadata) -> EntryKind {
    if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else if platform::attributes(metadata).is_some_and(|attributes| attributes & 0x400 != 0) {
        EntryKind::ReparsePoint
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}
fn fingerprint(policy: &ProtectedPolicy) -> String {
    // A versioned, length-delimited exact policy encoding avoids hash collisions.
    let mut result = format!(
        "inventory-filter-v2|marker:{}:{}",
        policy.marker_name.len(),
        policy.marker_name
    );
    let mut prefixes = policy.staging_prefixes.clone();
    prefixes.sort();
    for prefix in prefixes {
        result.push_str(&format!("|prefix:{}:{prefix}", prefix.len()));
    }
    let mut paths: Vec<_> = policy
        .protected_roots
        .iter()
        .map(|path| encode_path(path).bytes)
        .collect();
    paths.sort();
    for bytes in paths {
        result.push_str(&format!("|root:{}:", bytes.len()));
        for byte in bytes {
            result.push_str(&format!("{byte:02x}"));
        }
    }
    result
}

pub(crate) fn validate_child_name(name: &OsStr) -> io::Result<()> {
    let path = Path::new(name);
    if path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || encode_path(path).validate_relative().is_err()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a single native child name",
        ));
    }
    Ok(())
}
