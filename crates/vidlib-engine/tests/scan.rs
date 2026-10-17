use std::fs;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tempfile::tempdir;
use vidlib_engine::{scan, ScanMode, ScanResult};

fn run(root: &Path, mode: ScanMode) -> ScanResult {
    scan(
        vec![root.to_string_lossy().into_owned()],
        mode,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap()
}

#[test]
fn fast_scan_groups_exact_sizes_recursively_without_content_comparison() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("first.MP4"), b"abc").unwrap();
    fs::write(root.path().join("nested/second.mkv"), b"xyz").unwrap();
    fs::write(root.path().join("unique.mov"), b"unique").unwrap();
    fs::write(root.path().join("notes.txt"), b"abc").unwrap();
    fs::write(root.path().join("empty.mp4"), b"").unwrap();
    let result = run(root.path(), ScanMode::Size);
    assert_eq!(result.files_scanned, 3);
    assert_eq!(result.bytes_scanned, 12);
    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].files.len(), 2);
    assert_eq!(result.groups[0].size, 3);
    assert_eq!(result.groups[0].redundant_bytes, 3);
    assert_eq!(result.groups[0].hash, None);
    assert!(result.issues.is_empty());
}

#[test]
fn md5_separates_same_size_collisions_and_hashes_complete_candidates_only() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("a.mp4"), b"abc").unwrap();
    fs::write(root.path().join("b.mp4"), b"abc").unwrap();
    fs::write(root.path().join("collision.mp4"), b"xyz").unwrap();
    fs::write(root.path().join("unique.mp4"), b"a unique length").unwrap();
    let mut updates = Vec::new();
    let result = scan(
        vec![root.path().to_string_lossy().into_owned()],
        ScanMode::Md5,
        Arc::new(AtomicBool::new(false)),
        |update| updates.push(update),
    )
    .unwrap();
    assert_eq!(result.groups.len(), 1);
    assert_eq!(
        result.groups[0].hash.as_deref(),
        Some("900150983cd24fb0d6963f7d28e17f72")
    );
    assert_eq!(result.groups[0].files.len(), 2);
    let last = updates.last().unwrap();
    assert_eq!(last.phase, "complete");
    assert_eq!(last.files_to_hash, 3);
    assert_eq!(last.files_hashed, 3);
    assert_eq!(last.bytes_to_hash, 9);
    assert_eq!(last.bytes_hashed, 9);
}

#[test]
fn overlapping_roots_and_repeated_paths_are_counted_once() {
    let root = tempdir().unwrap();
    let nested = root.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(root.path().join("a.mp4"), b"same").unwrap();
    fs::write(nested.join("b.mp4"), b"same").unwrap();
    let result = scan(
        vec![
            nested.to_string_lossy().into_owned(),
            root.path().to_string_lossy().into_owned(),
            root.path().join("nested/..").to_string_lossy().into_owned(),
        ],
        ScanMode::Size,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.files_scanned, 2);
    assert_eq!(result.groups[0].files.len(), 2);
}

#[test]
fn invalid_roots_produce_issues_without_discarding_valid_roots() {
    let root = tempdir().unwrap();
    let file = root.path().join("a.mp4");
    fs::write(&file, b"abc").unwrap();
    let result = scan(
        vec![
            root.path().join("missing").to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
            root.path().to_string_lossy().into_owned(),
        ],
        ScanMode::Size,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.files_scanned, 1);
    assert_eq!(result.issues.len(), 2);
}

#[test]
fn no_roots_is_a_validation_error() {
    assert!(scan(
        vec![],
        ScanMode::Size,
        Arc::new(AtomicBool::new(false)),
        |_| {}
    )
    .is_err());
}

#[test]
fn cancellation_before_scan_preserves_cancelled_state() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("a.mp4"), b"abc").unwrap();
    let mut last = None;
    let result = scan(
        vec![root.path().to_string_lossy().into_owned()],
        ScanMode::Size,
        Arc::new(AtomicBool::new(true)),
        |progress| last = Some(progress),
    )
    .unwrap();
    assert!(result.cancelled);
    assert_eq!(result.files_scanned, 0);
    assert_eq!(last.unwrap().phase, "cancelled");
}

#[test]
fn cancellation_mid_hash_stops_before_finishing_current_file() {
    let root = tempdir().unwrap();
    let contents = vec![42; 4 * 1024 * 1024];
    fs::write(root.path().join("a.mp4"), &contents).unwrap();
    fs::write(root.path().join("b.mp4"), &contents).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut last = None;
    let result = scan(
        vec![root.path().to_string_lossy().into_owned()],
        ScanMode::Md5,
        cancel.clone(),
        |progress| {
            if progress.bytes_hashed > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
            last = Some(progress);
        },
    )
    .unwrap();
    assert!(result.cancelled);
    assert!(result.groups.is_empty());
    let last = last.unwrap();
    assert!(last.bytes_hashed > 0 && last.bytes_hashed < 4 * 1024 * 1024);
    assert_eq!(last.files_hashed, 0);
    assert_eq!(last.phase, "cancelled");
}

#[test]
fn files_changed_during_hashing_are_excluded_and_reported() {
    let root = tempdir().unwrap();
    let contents = vec![42; 3 * 1024 * 1024];
    fs::write(root.path().join("a.mp4"), &contents).unwrap();
    fs::write(root.path().join("b.mp4"), &contents).unwrap();
    let mut changed = false;
    let result = scan(
        vec![root.path().to_string_lossy().into_owned()],
        ScanMode::Md5,
        Arc::new(AtomicBool::new(false)),
        |progress| {
            if progress.bytes_hashed > 0 && !changed {
                fs::OpenOptions::new()
                    .write(true)
                    .open(&progress.current_path)
                    .unwrap()
                    .set_len(7)
                    .unwrap();
                changed = true;
            }
        },
    )
    .unwrap();
    assert!(changed);
    assert!(result.groups.is_empty());
    assert_eq!(result.issues.len(), 1);
    assert!(result.issues[0].message.to_lowercase().contains("changed"));
}

#[test]
fn complete_hash_detects_a_difference_beyond_the_first_buffer() {
    let root = tempdir().unwrap();
    let contents = vec![42; 2 * 1024 * 1024 + 1];
    fs::write(root.path().join("a.mp4"), &contents).unwrap();
    fs::write(root.path().join("b.mp4"), &contents).unwrap();
    let mut different = contents;
    *different.last_mut().unwrap() = 43;
    fs::write(root.path().join("c.mp4"), different).unwrap();
    let result = run(root.path(), ScanMode::Md5);
    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].files.len(), 2);
    assert!(result.groups[0]
        .files
        .iter()
        .all(|file| file.name != "c.mp4"));
}

#[test]
fn modification_time_change_without_length_change_invalidates_hash() {
    let root = tempdir().unwrap();
    let contents = vec![42; 2 * 1024 * 1024];
    fs::write(root.path().join("a.mp4"), &contents).unwrap();
    fs::write(root.path().join("b.mp4"), &contents).unwrap();
    let mut changed = false;
    let result = scan(
        vec![root.path().to_string_lossy().into_owned()],
        ScanMode::Md5,
        Arc::new(AtomicBool::new(false)),
        |progress| {
            if progress.bytes_hashed > 0 && !changed {
                let file = fs::File::options()
                    .write(true)
                    .open(&progress.current_path)
                    .unwrap();
                let modified = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
                file.set_times(fs::FileTimes::new().set_modified(modified))
                    .unwrap();
                changed = true;
            }
        },
    )
    .unwrap();
    assert!(changed);
    assert!(result.groups.is_empty());
    assert_eq!(result.issues.len(), 1);
    assert!(result.issues[0].message.to_lowercase().contains("changed"));
}

#[test]
fn missing_candidate_reports_issue_and_keeps_other_duplicate_results() {
    let root = tempdir().unwrap();
    for name in ["a.mp4", "b.mp4", "c.mp4"] {
        fs::write(root.path().join(name), b"abc").unwrap();
    }
    let mut removed = false;
    let result = scan(
        vec![root.path().to_string_lossy().into_owned()],
        ScanMode::Md5,
        Arc::new(AtomicBool::new(false)),
        |progress| {
            if progress.phase == "hashing" && !removed {
                fs::remove_file(root.path().join("c.mp4")).unwrap();
                removed = true;
            }
        },
    )
    .unwrap();
    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].files.len(), 2);
    assert_eq!(result.issues.len(), 1);
    assert!(result.issues[0].path.ends_with("c.mp4"));
}

#[test]
fn empty_library_completes_without_issues_or_hash_work() {
    let root = tempdir().unwrap();
    let mut last = None;
    let result = scan(
        vec![root.path().to_string_lossy().into_owned()],
        ScanMode::Md5,
        Arc::new(AtomicBool::new(false)),
        |progress| last = Some(progress),
    )
    .unwrap();
    assert_eq!(result.files_scanned, 0);
    assert!(result.groups.is_empty());
    assert!(result.issues.is_empty());
    assert!(!result.cancelled);
    let last = last.unwrap();
    assert_eq!(last.files_to_hash, 0);
    assert_eq!(last.bytes_to_hash, 0);
    assert_eq!(last.phase, "complete");
}

#[cfg(windows)]
fn directory_link(link: &Path, target: &Path) {
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction -Path $env:VIDLIB_TEST_LINK -Target $env:VIDLIB_TEST_TARGET | Out-Null"])
        .env("VIDLIB_TEST_LINK", link)
        .env("VIDLIB_TEST_TARGET", target)
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn directory_link(link: &Path, target: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(any(windows, unix))]
#[test]
fn directory_links_are_not_followed_or_counted_as_extra_videos() {
    let root = tempdir().unwrap();
    let external = tempdir().unwrap();
    fs::write(root.path().join("a.mp4"), b"abc").unwrap();
    fs::write(external.path().join("b.mp4"), b"abc").unwrap();
    directory_link(&root.path().join("external-link"), external.path());
    directory_link(&root.path().join("loop-link"), root.path());
    let result = run(root.path(), ScanMode::Size);
    assert_eq!(result.files_scanned, 1);
    assert!(result.groups.is_empty());
}

#[cfg(any(windows, unix))]
#[test]
fn selected_directory_link_is_reported_instead_of_followed() {
    let root = tempdir().unwrap();
    let external = tempdir().unwrap();
    fs::write(external.path().join("a.mp4"), b"abc").unwrap();
    let link = root.path().join("library-link");
    directory_link(&link, external.path());
    let result = run(&link, ScanMode::Size);
    assert_eq!(result.files_scanned, 0);
    assert_eq!(result.issues.len(), 1);
}
