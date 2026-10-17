//! Read-only video discovery and duplicate matching.
//!
//! Size matches are candidates, not proof of identical contents. MD5 is used for
//! accidental duplicate detection, not authentication. Redundant bytes are a
//! logical estimate: hard links and filesystem compression can change disk savings.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "m4v", "webm", "mpg", "mpeg", "mpe", "m2v", "m2ts", "mts",
    "ts", "vob", "ogv", "flv", "f4v", "3gp", "3g2", "divx", "asf", "rm", "rmvb",
];

const HASH_BUFFER_SIZE: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    Size,
    Md5,
}

#[derive(Clone, Debug, Serialize)]
pub struct VideoFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DuplicateGroup {
    pub id: String,
    pub size: u64,
    pub hash: Option<String>,
    pub files: Vec<VideoFile>,
    pub redundant_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanIssue {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ScanProgress {
    pub phase: String,
    pub files_found: usize,
    pub bytes_found: u64,
    pub files_hashed: usize,
    pub files_to_hash: usize,
    pub bytes_hashed: u64,
    pub bytes_to_hash: u64,
    pub current_path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanResult {
    pub mode: ScanMode,
    pub groups: Vec<DuplicateGroup>,
    pub issues: Vec<ScanIssue>,
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub elapsed_ms: u64,
    pub cancelled: bool,
}

struct Candidate {
    file: VideoFile,
    path: PathBuf,
    modified: SystemTime,
}

pub fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            VIDEO_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

/// Enumerate each selected root once, then optionally hash matching-size files.
///
/// Callbacks run synchronously on the caller's thread, including after each hash
/// chunk. UI adapters should throttle delivery (preserving phase transitions).
/// Run this function on a worker thread; cancellation is checked between reads.
pub fn scan(
    roots: Vec<String>,
    mode: ScanMode,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(ScanProgress),
) -> Result<ScanResult, String> {
    if roots.is_empty() {
        return Err("Choose at least one library folder to scan.".into());
    }
    let started = Instant::now();
    let mut result = ScanResult {
        mode,
        groups: Vec::new(),
        issues: Vec::new(),
        files_scanned: 0,
        bytes_scanned: 0,
        elapsed_ms: 0,
        cancelled: false,
    };
    let mut progress = ScanProgress {
        phase: "discovering".into(),
        ..ScanProgress::default()
    };
    on_progress(progress.clone());
    let roots = prepare_roots(roots, &cancel, &mut result.issues);
    let mut by_size: BTreeMap<u64, Vec<Candidate>> = BTreeMap::new();
    let mut visited = HashSet::new();

    'roots: for root in roots {
        let mut walker = WalkDir::new(&root).follow_links(false).into_iter();
        loop {
            if cancel.load(Ordering::Relaxed) {
                break 'roots;
            }
            let Some(entry) = walker.next() else {
                break;
            };
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    add_issue(
                        &mut result.issues,
                        error.path().unwrap_or(&root),
                        error.to_string(),
                    );
                    continue;
                }
            };
            let path = entry.path();
            progress.current_path = path.to_string_lossy().into_owned();
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    add_issue(&mut result.issues, path, error.to_string());
                    on_progress(progress.clone());
                    continue;
                }
            };
            if is_link(&metadata) {
                if entry.file_type().is_dir() {
                    walker.skip_current_dir();
                }
                on_progress(progress.clone());
                continue;
            }
            if metadata.is_file() && metadata.len() > 0 && is_video_file(path) {
                match make_candidate(path, &metadata) {
                    Ok(candidate) if visited.insert(candidate.path.clone()) => {
                        progress.files_found += 1;
                        progress.bytes_found =
                            progress.bytes_found.saturating_add(candidate.file.size);
                        by_size
                            .entry(candidate.file.size)
                            .or_default()
                            .push(candidate);
                    }
                    Ok(_) => {}
                    Err(error) => add_issue(&mut result.issues, path, error.to_string()),
                }
            }
            on_progress(progress.clone());
        }
    }
    result.files_scanned = progress.files_found;
    result.bytes_scanned = progress.bytes_found;
    let candidates: Vec<_> = by_size
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .collect();
    match mode {
        ScanMode::Size => {
            result.groups = candidates
                .into_iter()
                .map(|(size, files)| {
                    make_group(
                        size,
                        None,
                        files.into_iter().map(|candidate| candidate.file).collect(),
                    )
                })
                .collect();
        }
        ScanMode::Md5 if !cancel.load(Ordering::Relaxed) => {
            progress.phase = "hashing".into();
            progress.current_path.clear();
            progress.files_to_hash = candidates.iter().map(|(_, files)| files.len()).sum();
            progress.bytes_to_hash = candidates.iter().fold(0_u64, |total, (size, files)| {
                total.saturating_add(size.saturating_mul(files.len() as u64))
            });
            on_progress(progress.clone());
            let mut buffer = vec![0; HASH_BUFFER_SIZE];
            let mut matches: BTreeMap<(u64, String), Vec<VideoFile>> = BTreeMap::new();
            'hashing: for (_, files) in candidates {
                for candidate in files {
                    if cancel.load(Ordering::Relaxed) {
                        break 'hashing;
                    }
                    progress.current_path = candidate.file.path.clone();
                    on_progress(progress.clone());
                    match hash_candidate(
                        &candidate,
                        &cancel,
                        &mut buffer,
                        &mut progress,
                        &mut on_progress,
                    ) {
                        Ok(Some(hash)) => {
                            progress.files_hashed += 1;
                            matches
                                .entry((candidate.file.size, hash))
                                .or_default()
                                .push(candidate.file);
                            on_progress(progress.clone());
                        }
                        Ok(None) => break 'hashing,
                        Err(error) => {
                            add_issue(&mut result.issues, &candidate.path, error.to_string())
                        }
                    }
                }
            }
            result.groups = matches
                .into_iter()
                .filter(|(_, files)| files.len() > 1)
                .map(|((size, hash), files)| make_group(size, Some(hash), files))
                .collect();
        }
        ScanMode::Md5 => {}
    }
    result.groups.sort_by(|left, right| {
        right
            .redundant_bytes
            .cmp(&left.redundant_bytes)
            .then_with(|| left.id.cmp(&right.id))
    });
    result.cancelled = cancel.load(Ordering::Relaxed);
    result.elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    progress.phase = if result.cancelled {
        "cancelled"
    } else {
        "complete"
    }
    .into();
    progress.current_path.clear();
    on_progress(progress);
    Ok(result)
}

fn prepare_roots(
    roots: Vec<String>,
    cancel: &AtomicBool,
    issues: &mut Vec<ScanIssue>,
) -> Vec<PathBuf> {
    let mut canonical = Vec::new();
    for root in roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let path = Path::new(&root);
        let resolved = (|| -> io::Result<PathBuf> {
            let metadata = fs::symlink_metadata(path)?;
            if is_link(&metadata) {
                return Err(io::Error::other(
                    "Symbolic links and junctions are not scanned. Choose the original folder.",
                ));
            }
            if !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "This path is not a folder.",
                ));
            }
            fs::canonicalize(path)
        })();
        match resolved {
            Ok(path) => canonical.push(path),
            Err(error) => add_issue(issues, path, error.to_string()),
        }
    }
    canonical.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    let mut unique: Vec<PathBuf> = Vec::new();
    for root in canonical {
        if !unique.iter().any(|parent| root.starts_with(parent)) {
            unique.push(root);
        }
    }
    unique
}

fn make_candidate(path: &Path, metadata: &Metadata) -> io::Result<Candidate> {
    let path = fs::canonicalize(path)?;
    let modified = metadata.modified()?;
    Ok(Candidate {
        file: VideoFile {
            path: path.to_string_lossy().into_owned(),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            size: metadata.len(),
            modified_ms: modified
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64,
        },
        path,
        modified,
    })
}

fn is_link(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn unchanged(candidate: &Candidate, metadata: &Metadata) -> bool {
    metadata.is_file()
        && !is_link(metadata)
        && metadata.len() == candidate.file.size
        && metadata.modified().ok() == Some(candidate.modified)
}

fn changed_error() -> io::Error {
    io::Error::other("File changed during the scan; excluded from MD5 results. Scan again when it is no longer being modified.")
}

fn hash_candidate(
    candidate: &Candidate,
    cancel: &AtomicBool,
    buffer: &mut [u8],
    progress: &mut ScanProgress,
    on_progress: &mut impl FnMut(ScanProgress),
) -> io::Result<Option<String>> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    if !unchanged(candidate, &fs::symlink_metadata(&candidate.path)?) {
        return Err(changed_error());
    }
    let mut file = File::open(&candidate.path)?;
    if !unchanged(candidate, &file.metadata()?) {
        return Err(changed_error());
    }
    let mut context = md5::Context::new();
    let mut remaining = candidate.file.size;
    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let chunk_size = remaining.min(buffer.len() as u64) as usize;
        let count = match file.read(&mut buffer[..chunk_size]) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            other => other?,
        };
        if count == 0 {
            return Err(changed_error());
        }
        context.consume(&buffer[..count]);
        remaining -= count as u64;
        progress.bytes_hashed = progress.bytes_hashed.saturating_add(count as u64);
        on_progress(progress.clone());
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    if !unchanged(candidate, &file.metadata()?)
        || !unchanged(candidate, &fs::symlink_metadata(&candidate.path)?)
    {
        return Err(changed_error());
    }
    Ok(Some(format!("{:x}", context.finalize())))
}

fn make_group(size: u64, hash: Option<String>, mut files: Vec<VideoFile>) -> DuplicateGroup {
    files.sort_by(|left, right| left.path.cmp(&right.path));
    DuplicateGroup {
        id: match &hash {
            Some(hash) => format!("md5-{size}-{hash}"),
            None => format!("size-{size}"),
        },
        size,
        hash,
        redundant_bytes: size.saturating_mul(files.len().saturating_sub(1) as u64),
        files,
    }
}

fn add_issue(issues: &mut Vec<ScanIssue>, path: &Path, message: String) {
    issues.push(ScanIssue {
        path: path.to_string_lossy().into_owned(),
        message,
    });
}
