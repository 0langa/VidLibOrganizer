pub mod coordinator;
pub mod error;
pub mod events;
pub mod journal;
pub mod metadata;
pub mod platform;
pub mod rename;
pub use error::{ErrorCode, OpError, Phase};
pub use platform::{Filesystem, PlatformFs};
pub use rename::{execute, OperationReport};

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, FileTimes, Metadata, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use vidlib_engine::{DuplicateGroup, ScanIssue, ScanMode, ScanResult, VideoFile};

const BUFFER_SIZE: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileAction {
    Move,
    Delete,
}

#[derive(Debug, Default, Serialize)]
pub struct ActionReport {
    /// Original paths whose requested operation finished successfully.
    pub completed: Vec<String>,
    pub issues: Vec<ScanIssue>,
}

/// Validates the complete selection before touching any file. Runtime failures are
/// reported individually; successfully completed actions are not rolled back.
///
/// Metadata and content are rechecked to catch ordinary changes since scanning.
/// This is not a filesystem transaction or protection against a hostile process
/// replacing paths between system calls.
pub fn apply_actions(
    result: &ScanResult,
    paths: &[String],
    action: FileAction,
    destination: Option<&str>,
) -> Result<ActionReport, String> {
    if paths.is_empty() {
        return Err("Select at least one copy.".into());
    }
    let selected: HashSet<&str> = paths.iter().map(String::as_str).collect();
    if selected.len() != paths.len() {
        return Err("The selection contains the same path more than once.".into());
    }

    let mut membership = HashMap::new();
    for (group_index, group) in result.groups.iter().enumerate() {
        for file in &group.files {
            if membership
                .insert(file.path.as_str(), (group_index, file))
                .is_some()
            {
                return Err("The scan contains repeated file paths. Scan again.".into());
            }
        }
    }
    let mut touched = HashSet::new();
    for path in paths {
        let (index, _) = membership
            .get(path.as_str())
            .ok_or_else(|| format!("This file is not in the current scan: {path}"))?;
        touched.insert(*index);
    }

    let destination = match action {
        FileAction::Move => Some(validate_destination(destination)?),
        FileAction::Delete => None,
    };

    // Complete preflight: a bad item late in the request cannot cause an earlier
    // item to be removed. Check unselected copies too, including the keeper.
    let mut keepers = HashMap::new();
    for index in touched {
        let group = &result.groups[index];
        let keeper = group
            .files
            .iter()
            .find(|file| !selected.contains(file.path.as_str()))
            .ok_or_else(|| "Keep at least one original copy in every group.".to_string())?;
        keepers.insert(index, keeper);
        let expected_hash = group_hash(result.mode, group)?;
        let mut canonical_paths = HashSet::new();
        for file in &group.files {
            let _opened = open_validated(file, expected_hash, false)?;
            let canonical = fs::canonicalize(&file.path).map_err(|e| file_error(file, e))?;
            if !canonical_paths.insert(canonical.clone()) {
                return Err(format!(
                    "A group contains the same file through multiple paths: {}",
                    file.path
                ));
            }
            if selected.contains(file.path.as_str())
                && destination
                    .as_deref()
                    .is_some_and(|dir| canonical.parent() == Some(dir))
            {
                return Err(format!(
                    "The destination is already the source folder for {}",
                    file.path
                ));
            }
        }
    }

    let mut report = ActionReport::default();
    for path in paths {
        let (index, file) = membership[path.as_str()];
        let keeper = keepers[&index];
        let expected_hash = group_hash(result.mode, &result.groups[index])?;
        let operation = (|| {
            // On Windows this keeper handle denies writes and deletion until the
            // operation finishes. Other platforms still receive metadata checks.
            let keeper_handle = open_validated(keeper, expected_hash, true)?;
            let mut source = open_validated(file, expected_hash, false)?;
            match action {
                FileAction::Delete => {
                    check_snapshot(
                        keeper,
                        &keeper_handle
                            .metadata()
                            .map_err(|e| file_error(keeper, e))?,
                    )?;
                    check_path(keeper)?;
                    check_path(file)?;
                    fs::remove_file(&file.path).map_err(|e| file_error(file, e))
                }
                FileAction::Move => move_copy(
                    file,
                    &mut source,
                    keeper,
                    &keeper_handle,
                    expected_hash,
                    destination.as_deref().expect("move destination validated"),
                ),
            }
        })();
        match operation {
            Ok(()) => report.completed.push(path.clone()),
            Err(message) => report.issues.push(ScanIssue {
                path: path.clone(),
                message,
            }),
        }
    }
    Ok(report)
}

fn group_hash(mode: ScanMode, group: &DuplicateGroup) -> Result<Option<&str>, String> {
    match mode {
        ScanMode::Size => Ok(None),
        ScanMode::Md5 => group
            .hash
            .as_deref()
            .filter(|hash| hash.len() == 32 && hash.bytes().all(|c| c.is_ascii_hexdigit()))
            .map(Some)
            .ok_or_else(|| "An MD5 group has no valid content hash. Scan again.".into()),
    }
}

fn file_error(file: &VideoFile, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", file.path)
}

fn is_link(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn reject_link_components(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("Expected an absolute path: {}", path.display()));
    }
    for component in path.ancestors() {
        let metadata =
            fs::symlink_metadata(component).map_err(|e| format!("{}: {e}", component.display()))?;
        if is_link(&metadata) {
            return Err(format!(
                "Links and reparse points cannot be changed: {}",
                component.display()
            ));
        }
    }
    Ok(())
}

fn check_snapshot(file: &VideoFile, metadata: &Metadata) -> Result<(), String> {
    if !metadata.is_file() || is_link(metadata) {
        return Err(file_error(
            file,
            "The path is no longer a regular file. Scan again.",
        ));
    }
    let modified_ms = metadata
        .modified()
        .map_err(|e| file_error(file, e))?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    if metadata.len() != file.size || modified_ms != file.modified_ms {
        return Err(file_error(
            file,
            "The file changed since the scan. Scan again.",
        ));
    }
    Ok(())
}

fn check_path(file: &VideoFile) -> Result<(), String> {
    reject_link_components(Path::new(&file.path))?;
    check_snapshot(
        file,
        &fs::symlink_metadata(&file.path).map_err(|e| file_error(file, e))?,
    )
}

fn open_validated(file: &VideoFile, hash: Option<&str>, keeper: bool) -> Result<File, String> {
    check_path(file)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        const FILE_SHARE_DELETE: u32 = 4;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.share_mode(FILE_SHARE_READ | if keeper { 0 } else { FILE_SHARE_DELETE });
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(not(windows))]
    let _ = keeper;
    let mut opened = options.open(&file.path).map_err(|e| file_error(file, e))?;
    check_snapshot(file, &opened.metadata().map_err(|e| file_error(file, e))?)?;
    if let Some(expected) = hash {
        let actual = hash_reader(&mut opened).map_err(|e| file_error(file, e))?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(file_error(
                file,
                "The content changed since the MD5 scan. Scan again.",
            ));
        }
        opened
            .seek(SeekFrom::Start(0))
            .map_err(|e| file_error(file, e))?;
    }
    check_snapshot(file, &opened.metadata().map_err(|e| file_error(file, e))?)?;
    check_path(file)?;
    Ok(opened)
}

fn hash_reader(reader: &mut impl Read) -> std::io::Result<String> {
    let mut context = md5::Context::new();
    let mut buffer = vec![0; BUFFER_SIZE];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        context.consume(&buffer[..read]);
    }
    Ok(format!("{:x}", context.finalize()))
}

fn validate_destination(destination: Option<&str>) -> Result<PathBuf, String> {
    let destination = Path::new(
        destination
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Choose an existing destination folder.".to_string())?,
    );
    reject_link_components(destination)?;
    if !fs::symlink_metadata(destination)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("The destination must be an existing folder.".into());
    }
    fs::canonicalize(destination).map_err(|e| format!("Cannot open destination folder: {e}"))
}

fn create_destination(directory: &Path, source: &Path) -> Result<(PathBuf, File), String> {
    reject_link_components(directory)?;
    let name = source
        .file_name()
        .ok_or_else(|| "The source has no file name.".to_string())?;
    for suffix in 0..10_000 {
        let name = if suffix == 0 {
            name.to_os_string()
        } else {
            let mut name = source
                .file_stem()
                .unwrap_or(source.as_os_str())
                .to_os_string();
            name.push(format!(" ({suffix})"));
            if let Some(extension) = source.extension() {
                name.push(".");
                name.push(extension);
            }
            name
        };
        let path = directory.join(name);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Cannot create {}: {error}", path.display())),
        }
    }
    Err("Too many conflicting names in the destination folder.".into())
}

fn move_copy(
    file: &VideoFile,
    source: &mut File,
    keeper: &VideoFile,
    keeper_handle: &File,
    expected_hash: Option<&str>,
    destination: &Path,
) -> Result<(), String> {
    let (target_path, mut target) = create_destination(destination, Path::new(&file.path))?;
    let copied = (|| {
        let mut digest = md5::Context::new();
        let mut buffer = vec![0; BUFFER_SIZE];
        let mut bytes = 0u64;
        loop {
            let count = source.read(&mut buffer).map_err(|e| file_error(file, e))?;
            if count == 0 {
                break;
            }
            target
                .write_all(&buffer[..count])
                .map_err(|e| format!("Writing destination: {e}"))?;
            digest.consume(&buffer[..count]);
            bytes = bytes
                .checked_add(count as u64)
                .ok_or("File size overflow")?;
        }
        if bytes != file.size {
            return Err("The source length changed during the move.".into());
        }
        target
            .sync_all()
            .map_err(|e| format!("Syncing destination: {e}"))?;
        target.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let source_hash = format!("{:x}", digest.finalize());
        let target_hash =
            hash_reader(&mut target).map_err(|e| format!("Verifying destination: {e}"))?;
        if source_hash != target_hash
            || expected_hash.is_some_and(|expected| !source_hash.eq_ignore_ascii_case(expected))
            || target.metadata().map_err(|e| e.to_string())?.len() != bytes
        {
            return Err("The destination content did not verify; the source was kept.".into());
        }
        let source_metadata = source.metadata().map_err(|e| file_error(file, e))?;
        check_snapshot(file, &source_metadata)?;
        check_path(file)?;
        check_snapshot(
            keeper,
            &keeper_handle
                .metadata()
                .map_err(|e| file_error(keeper, e))?,
        )?;
        check_path(keeper)?;
        let modified = source_metadata
            .modified()
            .map_err(|e| file_error(file, e))?;
        target
            .set_times(FileTimes::new().set_modified(modified))
            .map_err(|e| format!("Preserving the original video date: {e}"))?;
        target
            .sync_all()
            .map_err(|e| format!("Syncing destination metadata: {e}"))?;
        Ok(())
    })();
    drop(target);
    if let Err(error) = copied {
        return match fs::remove_file(&target_path) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!(
                "{error} Partial destination remains at {}: {cleanup}",
                target_path.display()
            )),
        };
    }
    // Preserve the verified destination if removal fails. Deleting it here would
    // discard the user's only completed copy after a later source failure.
    fs::remove_file(&file.path).map_err(|e| {
        format!(
            "The verified copy is at {}, but the source could not be removed: {e}",
            target_path.display(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_source_after_validation_cleans_partial_destination() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.mp4");
        let keeper_path = dir.path().join("keeper.mp4");
        let destination = dir.path().join("destination");
        fs::create_dir(&destination).unwrap();
        for path in [&source_path, &keeper_path] {
            fs::write(path, b"original video bytes").unwrap();
        }
        let snapshot = |path: &Path| {
            let metadata = fs::metadata(path).unwrap();
            VideoFile {
                path: path.to_str().unwrap().into(),
                name: path.file_name().unwrap().to_str().unwrap().into(),
                size: metadata.len(),
                modified_ms: metadata
                    .modified()
                    .unwrap()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            }
        };
        let file = snapshot(&source_path);
        let keeper = snapshot(&keeper_path);
        // Reproduce a file truncated after the preflight snapshot without relying
        // on timing a competing writer against a fast local copy.
        fs::write(&source_path, b"short").unwrap();
        let mut source = File::open(&source_path).unwrap();
        let keeper_handle = File::open(&keeper_path).unwrap();
        let result = move_copy(
            &file,
            &mut source,
            &keeper,
            &keeper_handle,
            None,
            &destination,
        );
        assert!(result.is_err());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        assert_eq!(fs::read(&source_path).unwrap(), b"short");
        assert_eq!(fs::read(&keeper_path).unwrap(), b"original video bytes");
    }
}

pub mod recovery;

pub mod undo;

pub mod copy;
