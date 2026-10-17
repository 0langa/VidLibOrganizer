use crate::{ids::*, model::*, platform, *};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    ffi::OsString,
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
    fn read_directory(&self, path: &Path) -> io::Result<DirectoryListing>;
    fn physical_id(&self, path: &Path, metadata: &fs::Metadata) -> Option<PhysicalId> {
        platform::physical_id(path, metadata)
    }
}
pub struct OsInventoryFs;
impl InventoryFs for OsInventoryFs {
    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
        fs::symlink_metadata(path)
    }
    fn read_directory(&self, path: &Path) -> io::Result<DirectoryListing> {
        platform::read_directory(path)
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
        if scanner.seen_directories.contains_key(&root.path)
            || scanner.seen_entries.contains(&root.path)
        {
            continue;
        }
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
                if kind(&metadata) == EntryKind::Directory {
                    scanner.directory(root, &root.path, &root.path);
                } else {
                    let base = root.path.parent().unwrap_or(&root.path);
                    scanner.entry(root, base, &root.path, Ok(metadata));
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
        self.seen_directories.insert(directory.to_path_buf(), index);
        index
    }
    fn directory(&mut self, root: &RootBinding, base: &Path, directory: &Path) -> bool {
        if let Some(index) = self.seen_directories.get(directory) {
            return self.coverage[*index].subtree_complete;
        }
        let index = self.empty_scope(root, directory, CoverageOutcome::Failed, None);
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
        let mut listing = match self.fs.read_directory(directory) {
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
        let mut complete = listing.errors.is_empty();
        let mut issues: Vec<_> = listing
            .errors
            .into_iter()
            .map(|error| issue(IssueCode::ListingFailed, relative(directory, base), error))
            .collect();
        for name in listing.names {
            if self.control.is_cancelled() {
                complete = false;
                break;
            }
            // The adapter seam also has to obey the immediate-child contract.
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
            let path = directory.join(name);
            if self.protected(&path) {
                continue;
            }
            let metadata = self.fs.symlink_metadata(&path);
            let is_directory = metadata
                .as_ref()
                .is_ok_and(|m| kind(m) == EntryKind::Directory);
            if !self.entry(root, base, &path, metadata) {
                complete = false;
            }
            if is_directory && !self.directory(root, base, &path) {
                complete = false;
            }
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
        metadata: io::Result<fs::Metadata>,
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
            Ok(metadata) => {
                let kind = kind(&metadata);
                (
                    kind,
                    platform::stamp(&metadata, self.fs.physical_id(path, &metadata)),
                )
            }
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
fn kind(metadata: &fs::Metadata) -> EntryKind {
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
        "inventory-filter-v1|marker:{}:{}",
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
