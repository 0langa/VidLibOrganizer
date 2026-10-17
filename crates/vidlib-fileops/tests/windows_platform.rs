#![cfg(windows)]
use std::{fs, os::windows::fs::OpenOptionsExt};
use vidlib_fileops::{ErrorCode, Filesystem, PlatformFs};
use vidlib_inventory::{GrantSet, RootBinding, Support};

/// Runtime qualification is pending. This test guards the production gate; the
/// native primitive qualification tests live beside the private Windows adapter.
#[test]
fn unverified_windows_volume_fails_closed_for_file_folder_and_case_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut adapter = PlatformFs::default();
    for directory in [false, true] {
        let source = dir.path().join(if directory { "Folder" } else { "File" });
        let target = dir.path().join(if directory { "folder" } else { "file" });
        if directory {
            fs::create_dir(&source).unwrap();
        } else {
            fs::write(&source, b"source").unwrap();
        }
        let expected = adapter.inspect(&source).unwrap().stamp;
        assert_eq!(
            adapter
                .rename_no_replace(&source, &target, &expected)
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert!(source.exists());
    }
    let scope = GrantSet {
        grants: vec![
            RootBinding::new(dir.path().to_owned())
                .unwrap()
                .source_grant,
        ],
    };
    let snapshot = adapter
        .capture_namespace(&scope, &[dir.path().join("target")])
        .unwrap();
    assert_ne!(
        snapshot.parents[0].capabilities.atomic_rename,
        Support::Supported
    );
    assert_ne!(
        snapshot.parents[0].capabilities.durable_flush,
        Support::Supported
    );
}
#[test]
fn locked_file_never_changes_when_capability_is_unverified() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("locked");
    fs::write(&source, b"safe").unwrap();
    let mut adapter = PlatformFs::default();
    let expected = adapter.inspect(&source).unwrap().stamp;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&source)
        .unwrap();
    assert_eq!(
        adapter
            .rename_no_replace(&source, &dir.path().join("target"), &expected)
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    drop(held);
    assert_eq!(fs::read(source).unwrap(), b"safe");
    assert!(!dir.path().join("target").exists());
}
