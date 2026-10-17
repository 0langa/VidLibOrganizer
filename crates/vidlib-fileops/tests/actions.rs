use std::fs::{self, File, FileTimes};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tempfile::TempDir;
use vidlib_engine::{DuplicateGroup, ScanMode, ScanResult, VideoFile};
use vidlib_fileops::{apply_actions, FileAction};

const CONTENT: &[u8] = b"identical video fixture";

struct Fixture {
    dir: TempDir,
    paths: Vec<PathBuf>,
    result: ScanResult,
}

impl Fixture {
    fn new(mode: ScanMode, count: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..count)
            .map(|i| {
                let path = dir.path().join(format!("copy-{i}.mp4"));
                fs::write(&path, CONTENT).unwrap();
                path
            })
            .collect();
        let result = snapshot(mode, &paths);
        Self { dir, paths, result }
    }

    fn selected(&self, indices: &[usize]) -> Vec<String> {
        indices
            .iter()
            .map(|i| path_string(&self.paths[*i]))
            .collect()
    }
}

fn path_string(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

fn snapshot(mode: ScanMode, paths: &[PathBuf]) -> ScanResult {
    let files: Vec<_> = paths
        .iter()
        .map(|path| {
            let metadata = fs::metadata(path).unwrap();
            VideoFile {
                path: path_string(path),
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                size: metadata.len(),
                modified_ms: metadata
                    .modified()
                    .unwrap()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            }
        })
        .collect();
    ScanResult {
        mode,
        groups: vec![DuplicateGroup {
            id: "fixture".into(),
            size: CONTENT.len() as u64,
            hash: (mode == ScanMode::Md5).then(|| format!("{:x}", md5::compute(CONTENT))),
            redundant_bytes: (files.len().saturating_sub(1) * CONTENT.len()) as u64,
            files,
        }],
        issues: vec![],
        files_scanned: paths.len(),
        bytes_scanned: (paths.len() * CONTENT.len()) as u64,
        elapsed_ms: 0,
        cancelled: false,
    }
}

#[test]
fn deleting_a_selected_copy_keeps_the_original() {
    let fixture = Fixture::new(ScanMode::Md5, 2);
    let selected = fixture.selected(&[1]);
    let report = apply_actions(&fixture.result, &selected, FileAction::Delete, None).unwrap();
    assert_eq!(report.completed, selected);
    assert!(report.issues.is_empty());
    assert!(!fixture.paths[1].exists());
    assert_eq!(fs::read(&fixture.paths[0]).unwrap(), CONTENT);
}

#[test]
fn selecting_every_copy_is_rejected_before_any_deletion() {
    let fixture = Fixture::new(ScanMode::Size, 2);
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[0, 1]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.paths.iter().all(|path| path.exists()));
}

#[test]
fn empty_selection_is_rejected() {
    let fixture = Fixture::new(ScanMode::Size, 2);
    assert!(apply_actions(&fixture.result, &[], FileAction::Delete, None).is_err());
}

#[test]
fn duplicate_selection_is_rejected() {
    let fixture = Fixture::new(ScanMode::Size, 3);
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1, 1]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.paths.iter().all(|path| path.exists()));
}

#[test]
fn unknown_path_rejects_the_entire_request() {
    let fixture = Fixture::new(ScanMode::Size, 3);
    let unknown = fixture.dir.path().join("unknown.mp4");
    fs::write(&unknown, CONTENT).unwrap();
    let mut selected = fixture.selected(&[1]);
    selected.push(path_string(&unknown));
    assert!(apply_actions(&fixture.result, &selected, FileAction::Delete, None).is_err());
    assert!(fixture.paths[1].exists());
    assert!(unknown.exists());
}

#[test]
fn stale_selected_file_rejects_every_action() {
    let fixture = Fixture::new(ScanMode::Size, 3);
    fs::write(&fixture.paths[2], b"changed").unwrap();
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1, 2]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.paths.iter().all(|path| path.exists()));
}

#[test]
fn changed_modification_time_rejects_deletion() {
    let fixture = Fixture::new(ScanMode::Size, 2);
    File::options()
        .write(true)
        .open(&fixture.paths[1])
        .unwrap()
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1)))
        .unwrap();
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.paths[1].exists());
}

#[test]
fn missing_kept_copy_prevents_deletion() {
    let fixture = Fixture::new(ScanMode::Md5, 2);
    fs::remove_file(&fixture.paths[0]).unwrap();
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.paths[1].exists());
}

#[test]
fn changed_kept_copy_hash_prevents_deletion_even_with_restored_timestamp() {
    let fixture = Fixture::new(ScanMode::Md5, 2);
    let original_time = fs::metadata(&fixture.paths[0]).unwrap().modified().unwrap();
    fs::write(&fixture.paths[0], vec![b'!'; CONTENT.len()]).unwrap();
    File::options()
        .write(true)
        .open(&fixture.paths[0])
        .unwrap()
        .set_times(FileTimes::new().set_modified(original_time))
        .unwrap();
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.paths[1].exists());
}

#[test]
fn missing_md5_hash_rejects_actions() {
    let mut fixture = Fixture::new(ScanMode::Md5, 2);
    fixture.result.groups[0].hash = None;
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.paths[1].exists());
}

#[test]
fn size_candidates_can_be_deleted_without_content_equality() {
    let mut fixture = Fixture::new(ScanMode::Size, 2);
    fs::write(&fixture.paths[1], vec![b'!'; CONTENT.len()]).unwrap();
    fixture.result = snapshot(ScanMode::Size, &fixture.paths);
    let report = apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Delete,
        None,
    )
    .unwrap();
    assert_eq!(report.completed.len(), 1);
    assert_eq!(fs::read(&fixture.paths[0]).unwrap(), CONTENT);
    assert!(!fixture.paths[1].exists());
}

#[test]
fn move_preserves_content_and_kept_original() {
    let fixture = Fixture::new(ScanMode::Md5, 2);
    let destination = fixture.dir.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let selected = fixture.selected(&[1]);
    let report = apply_actions(
        &fixture.result,
        &selected,
        FileAction::Move,
        Some(destination.to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(report.completed, selected);
    assert!(report.issues.is_empty());
    assert!(!fixture.paths[1].exists());
    assert_eq!(fs::read(destination.join("copy-1.mp4")).unwrap(), CONTENT);
    assert_eq!(fs::read(&fixture.paths[0]).unwrap(), CONTENT);
}

#[test]
fn move_preserves_original_modification_timestamp() {
    let mut fixture = Fixture::new(ScanMode::Md5, 2);
    let old_date = UNIX_EPOCH + std::time::Duration::from_millis(1_600_000_000_456);
    File::options()
        .write(true)
        .open(&fixture.paths[1])
        .unwrap()
        .set_times(FileTimes::new().set_modified(old_date))
        .unwrap();
    let recorded_date = fs::metadata(&fixture.paths[1]).unwrap().modified().unwrap();
    fixture.result = snapshot(ScanMode::Md5, &fixture.paths);
    let destination = fixture.dir.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let report = apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Move,
        Some(destination.to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(report.completed.len(), 1);
    assert_eq!(
        fs::metadata(destination.join("copy-1.mp4"))
            .unwrap()
            .modified()
            .unwrap(),
        recorded_date
    );
}

#[test]
fn move_collision_never_overwrites_existing_file() {
    let fixture = Fixture::new(ScanMode::Md5, 2);
    let destination = fixture.dir.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let occupied = destination.join("copy-1.mp4");
    fs::write(&occupied, b"unrelated existing file").unwrap();
    let report = apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Move,
        Some(destination.to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(report.completed.len(), 1);
    assert_eq!(fs::read(occupied).unwrap(), b"unrelated existing file");
    let moved: Vec<_> = fs::read_dir(destination)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap() != "copy-1.mp4")
        .collect();
    assert_eq!(moved.len(), 1);
    assert_eq!(fs::read(&moved[0]).unwrap(), CONTENT);
    assert!(!fixture.paths[1].exists());
}

#[test]
fn selected_files_with_same_name_both_move_without_overwrite() {
    let mut fixture = Fixture::new(ScanMode::Md5, 3);
    for i in 1..3 {
        let directory = fixture.dir.path().join(format!("source-{i}"));
        fs::create_dir(&directory).unwrap();
        let renamed = directory.join("same.mp4");
        fs::rename(&fixture.paths[i], &renamed).unwrap();
        fixture.paths[i] = renamed;
    }
    fixture.result = snapshot(ScanMode::Md5, &fixture.paths);
    let destination = fixture.dir.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let report = apply_actions(
        &fixture.result,
        &fixture.selected(&[1, 2]),
        FileAction::Move,
        Some(destination.to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(report.completed.len(), 2);
    let moved: Vec<_> = fs::read_dir(destination)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(moved.len(), 2);
    for path in moved {
        assert_eq!(fs::read(path).unwrap(), CONTENT);
    }
    assert!(fixture.paths[0].exists());
}

#[test]
fn invalid_move_destination_prevents_changes() {
    let fixture = Fixture::new(ScanMode::Size, 2);
    let missing = fixture.dir.path().join("does-not-exist");
    for destination in [
        None,
        Some(missing.to_str().unwrap()),
        Some(fixture.paths[0].to_str().unwrap()),
    ] {
        assert!(apply_actions(
            &fixture.result,
            &fixture.selected(&[1]),
            FileAction::Move,
            destination
        )
        .is_err());
        assert!(fixture.paths[1].exists());
    }
}

#[test]
fn move_to_source_directory_is_rejected() {
    let fixture = Fixture::new(ScanMode::Size, 2);
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Move,
        Some(fixture.dir.path().to_str().unwrap())
    )
    .is_err());
    assert!(fixture.paths.iter().all(|p| p.exists()));
}

#[test]
fn stale_group_late_in_request_prevents_changes_to_an_earlier_group() {
    let mut fixture = Fixture::new(ScanMode::Size, 4);
    let mut second = fixture.result.groups[0].clone();
    second.id = "second".into();
    second.files = second.files.split_off(2);
    fixture.result.groups[0].files.truncate(2);
    fixture.result.groups.push(second);
    fs::remove_file(&fixture.paths[2]).unwrap();
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1, 3]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.paths[1].exists());
    assert!(fixture.paths[3].exists());
}

#[test]
fn changed_selected_content_rejects_md5_actions_with_restored_timestamp() {
    let fixture = Fixture::new(ScanMode::Md5, 2);
    let original_time = fs::metadata(&fixture.paths[1]).unwrap().modified().unwrap();
    fs::write(&fixture.paths[1], vec![b'?'; CONTENT.len()]).unwrap();
    File::options()
        .write(true)
        .open(&fixture.paths[1])
        .unwrap()
        .set_times(FileTimes::new().set_modified(original_time))
        .unwrap();
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.paths.iter().all(|p| p.exists()));
}

#[test]
fn move_preserves_content_across_many_streaming_chunks() {
    let mut fixture = Fixture::new(ScanMode::Size, 2);
    let content: Vec<u8> = (0..1_048_593).map(|n| (n % 251) as u8).collect();
    for path in &fixture.paths {
        fs::write(path, &content).unwrap();
    }
    fixture.result = snapshot(ScanMode::Size, &fixture.paths);
    let destination = fixture.dir.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let report = apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Move,
        Some(destination.to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(report.completed.len(), 1);
    assert_eq!(fs::read(destination.join("copy-1.mp4")).unwrap(), content);
    assert!(!fixture.paths[1].exists());
}

#[cfg(windows)]
#[test]
fn locked_source_failure_keeps_verified_destination_and_reports_its_path() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new(ScanMode::Md5, 2);
    let destination = fixture.dir.path().join("destination");
    fs::create_dir(&destination).unwrap();
    // This real reader allows reads but denies final deletion on Windows.
    let lock = File::options()
        .read(true)
        .share_mode(1)
        .open(&fixture.paths[1])
        .unwrap();
    let result = apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Move,
        Some(destination.to_str().unwrap()),
    );
    drop(lock);
    let report = result.unwrap();
    assert!(report.completed.is_empty());
    assert_eq!(report.issues.len(), 1);
    assert!(report.issues[0].message.contains("copy-1.mp4"));
    assert!(report.issues[0].message.contains("verified copy"));
    assert_eq!(fs::read(&fixture.paths[1]).unwrap(), CONTENT);
    assert_eq!(fs::read(destination.join("copy-1.mp4")).unwrap(), CONTENT);
}

#[cfg(windows)]
#[test]
fn directory_junction_cannot_redirect_source_actions() {
    use std::os::windows::process::CommandExt;
    let mut fixture = Fixture::new(ScanMode::Size, 2);
    let aliases = tempfile::tempdir().unwrap();
    let junction = aliases.path().join("junction");
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(fixture.dir.path())
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "junction fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fixture.paths[1] = junction.join("copy-1.mp4");
    fixture.result = snapshot(ScanMode::Size, &fixture.paths);
    let result = apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Delete,
        None,
    );
    fs::remove_dir(&junction).unwrap();
    assert!(result.is_err());
    assert!(fixture.dir.path().join("copy-1.mp4").exists());
}

#[cfg(unix)]
#[test]
fn symlink_source_is_rejected_without_removing_its_target() {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::new(ScanMode::Size, 2);
    let link = fixture.dir.path().join("link.mp4");
    symlink(&fixture.paths[1], &link).unwrap();
    fixture.paths[1] = link;
    fixture.result = snapshot(ScanMode::Size, &fixture.paths);
    assert!(apply_actions(
        &fixture.result,
        &fixture.selected(&[1]),
        FileAction::Delete,
        None
    )
    .is_err());
    assert!(fixture.dir.path().join("copy-1.mp4").exists());
}
