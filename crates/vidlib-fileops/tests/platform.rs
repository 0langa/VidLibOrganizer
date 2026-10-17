#![cfg(target_os = "linux")]
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use vidlib_fileops::{
    platform::{Durability, MetadataUpdate},
    ErrorCode, Filesystem, PlatformFs,
};
use vidlib_inventory::{
    CaseSensitivity, FileStamp, GrantSet, InventoryFs, OsInventoryFs, RootBinding, Support,
};

// Independent inventory observations let mutation tests fail at the operation,
// rather than depending on the adapter's own inspection implementation.
fn stamp(path: &Path) -> FileStamp {
    let parent = OsInventoryFs
        .open_directory(path.parent().unwrap())
        .unwrap();
    parent
        .observe_child(path.file_name().unwrap())
        .unwrap()
        .stamp
}
fn grants(path: &Path) -> GrantSet {
    GrantSet {
        grants: vec![RootBinding::new(path.to_owned()).unwrap().source_grant],
    }
}

#[test]
fn existing_target_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    fs::write(&source, b"source").unwrap();
    fs::write(&target, b"existing").unwrap();
    let err = PlatformFs::default()
        .rename_no_replace(&source, &target, &stamp(&source))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::DestinationExists);
    assert_eq!(fs::read(target).unwrap(), b"existing");
    assert_eq!(fs::read(source).unwrap(), b"source");
}
#[test]
fn target_created_after_inspection_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    fs::write(&source, b"source").unwrap();
    let mut adapter = PlatformFs::default();
    let namespace = adapter
        .capture_namespace(&grants(dir.path()), std::slice::from_ref(&target))
        .unwrap();
    assert!(!namespace.parents[0]
        .entries
        .iter()
        .any(|e| e.absolute_path == target));
    fs::write(&target, b"late arrival").unwrap();
    let err = adapter
        .rename_no_replace(&source, &target, &stamp(&source))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::DestinationExists);
    assert_eq!(fs::read(target).unwrap(), b"late arrival");
    assert_eq!(fs::read(source).unwrap(), b"source");
}
#[test]
fn directory_case_policy_is_per_parent() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("a");
    let second = dir.path().join("b");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    fs::write(first.join("A"), b"one").unwrap();
    fs::write(second.join("a"), b"two").unwrap();
    let adapter = PlatformFs {
        namespace_epoch: 42,
        policy_epoch: 7,
        ..Default::default()
    };
    let snapshot = adapter
        .capture_namespace(
            &grants(dir.path()),
            &[first.join("new"), second.join("new")],
        )
        .unwrap();
    assert_eq!(snapshot.parents.len(), 2);
    for (facts, expected) in snapshot.parents.iter().zip([&first, &second]) {
        assert_eq!(&facts.parent_path, expected);
        assert_eq!(facts.stamp.physical_id, stamp(expected).physical_id);
        assert_eq!(facts.case_sensitivity, CaseSensitivity::Sensitive);
        assert_eq!(facts.entries.len(), 1);
        assert!(facts.children_complete);
        assert_eq!(facts.source_grants.len(), 1);
        assert_eq!(facts.max_component_units, Some(255));
    }
    assert_eq!((snapshot.namespace_epoch, snapshot.policy_epoch), (42, 7));
    // Different actual filesystem: procfs has no verified mutation/name policy.
    let mut scope = grants(dir.path());
    scope.grants.extend(grants(Path::new("/proc")).grants);
    let snapshot = adapter
        .capture_namespace(&scope, &[first.join("new"), "/proc/new".into()])
        .unwrap();
    assert_ne!(
        snapshot.parents[0].case_sensitivity,
        snapshot.parents[1].case_sensitivity
    );
    assert_ne!(
        snapshot.parents[1].capabilities.atomic_rename,
        Support::Supported
    );
}
#[test]
fn reparse_ancestor_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("source"), b"source").unwrap();
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let err = PlatformFs::default()
        .rename_no_replace(
            &alias.join("source"),
            &dir.path().join("target"),
            &stamp(&real.join("source")),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ReparsePoint);
    assert_eq!(fs::read(real.join("source")).unwrap(), b"source");
    assert!(!dir.path().join("target").exists());
}
#[test]
fn rename_files_directories_and_case_only_preserves_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut adapter = PlatformFs::default();
    for directory in [false, true] {
        let source = dir.path().join(if directory { "Folder" } else { "File" });
        let target = dir.path().join(if directory { "folder" } else { "file" });
        if directory {
            fs::create_dir(&source).unwrap();
            fs::write(source.join("child"), b"child").unwrap();
        } else {
            fs::write(&source, b"source").unwrap();
        }
        let before = stamp(&source);
        let outcome = adapter
            .rename_no_replace(&source, &target, &before)
            .unwrap();
        assert_eq!(outcome.observation.stamp.physical_id, before.physical_id);
        assert_eq!(outcome.durability, Durability::Synced);
        assert!(!source.exists());
        assert_eq!(
            fs::read(if directory {
                target.join("child")
            } else {
                target
            })
            .unwrap(),
            if directory {
                b"child".as_slice()
            } else {
                b"source".as_slice()
            }
        );
    }
}
#[test]
fn stale_source_or_missing_identity_cannot_mutate() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    fs::write(&source, b"first").unwrap();
    let before = stamp(&source);
    fs::write(&source, b"changed content").unwrap();
    let mut adapter = PlatformFs::default();
    assert_eq!(
        adapter
            .rename_no_replace(&source, &target, &before)
            .unwrap_err()
            .code,
        ErrorCode::StaleState
    );
    assert_eq!(
        adapter
            .rename_no_replace(&source, &target, &FileStamp::default())
            .unwrap_err()
            .code,
        ErrorCode::StaleState
    );
    assert_eq!(fs::read(&source).unwrap(), b"changed content");
    assert!(!target.exists());
}
#[test]
fn protected_names_and_outside_grants_fail_before_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fs::write(&source, b"safe").unwrap();
    let mut adapter = PlatformFs::default();
    for name in [".VIDLIB-BLUEPRINT.JSON", ".VIDLIB-STAGING-reserved"] {
        assert_eq!(
            adapter
                .rename_no_replace(&source, &dir.path().join(name), &stamp(&source))
                .unwrap_err()
                .code,
            ErrorCode::ProtectedEntry
        );
    }
    assert_eq!(
        adapter
            .capture_namespace(&GrantSet::default(), std::slice::from_ref(&source))
            .unwrap_err()
            .code,
        ErrorCode::OutsideGrant
    );
    assert!(source.exists());
}
#[test]
fn namespace_keeps_native_names_and_protected_entries() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let raw = std::ffi::OsString::from_vec(vec![0xff]);
    fs::write(dir.path().join(raw), b"raw").unwrap();
    fs::write(dir.path().join(".VIDLIB-BLUEPRINT.JSON"), b"marker").unwrap();
    let snapshot = PlatformFs::default()
        .capture_namespace(&grants(dir.path()), &[dir.path().join("target")])
        .unwrap();
    let entries = &snapshot.parents[0].entries;
    assert!(entries
        .iter()
        .any(|e| e.name.bytes == [0xff] && !e.protected));
    assert!(entries.iter().any(|e| e.protected));
}
#[test]
fn create_new_staging_publish_and_owned_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let mut adapter = PlatformFs::default();
    let stage = dir.path().join(".vidlib-staging-unique");
    let mut owned = adapter.create_staging(&stage).unwrap();
    owned.file.write_all(b"verified copy").unwrap();
    owned.file.sync_all().unwrap();
    assert_eq!(
        adapter.create_staging(&stage).unwrap_err().code,
        ErrorCode::DestinationExists
    );
    assert_eq!(fs::read(&stage).unwrap(), b"verified copy");
    // Publishing is the same no-replace primitive, authorized by exact owned identity.
    assert_eq!(
        adapter.refresh_staging(&mut owned).unwrap(),
        Durability::Synced
    );
    let published = dir.path().join("published");
    adapter
        .publish_staging(&owned.ownership, &published)
        .unwrap();
    assert_eq!(fs::read(published).unwrap(), b"verified copy");
    assert!(!stage.exists());
    let folder = adapter
        .create_directory(&dir.path().join("owned-folder"))
        .unwrap();
    assert_eq!(
        adapter.create_directory(&folder.path).unwrap_err().code,
        ErrorCode::DestinationExists
    );
    adapter.remove_owned(&folder).unwrap();
    assert!(!folder.path.exists());
    let owned = adapter.create_staging(&stage).unwrap();
    adapter.remove_owned(&owned.ownership).unwrap();
    assert!(!stage.exists());
}
#[test]
fn cleanup_does_not_remove_replaced_or_nonempty_owned_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut adapter = PlatformFs::default();
    let stage = dir.path().join(".vidlib-staging-owned");
    let owned = adapter.create_staging(&stage).unwrap();
    fs::rename(&stage, dir.path().join("moved-original")).unwrap();
    fs::write(&stage, b"replacement").unwrap();
    assert_eq!(
        adapter.remove_owned(&owned.ownership).unwrap_err().code,
        ErrorCode::StaleState
    );
    assert_eq!(fs::read(&stage).unwrap(), b"replacement");
    let folder = adapter
        .create_directory(&dir.path().join("folder"))
        .unwrap();
    fs::write(folder.path.join("child"), b"safe").unwrap();
    assert!(adapter.remove_owned(&folder).is_err());
    assert_eq!(fs::read(folder.path.join("child")).unwrap(), b"safe");
}
#[test]
fn verified_read_and_metadata_setters_use_fresh_handles() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fs::write(&source, b"contents").unwrap();
    let mut adapter = PlatformFs::default();
    let before = stamp(&source);
    let mut bytes = vec![];
    adapter
        .open_verified(&source, &before)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"contents");
    let update = MetadataUpdate {
        modified_ns: Some(1_700_000_000_123_456_789),
        accessed_ns: Some(1_600_000_000_987_654_321),
        readonly: Some(true),
        ..Default::default()
    };
    let changed = adapter.set_metadata(&source, &before, &update).unwrap();
    assert_eq!(changed.observation.stamp.modified_ns, update.modified_ns);
    assert_eq!(changed.observation.stamp.accessed_ns, update.accessed_ns);
    assert!(fs::metadata(&source).unwrap().permissions().readonly());
    assert_eq!(changed.durability, Durability::Synced);
    assert_eq!(
        adapter.verify(&source, &before).unwrap_err().code,
        ErrorCode::StaleState
    );
    assert_eq!(
        adapter
            .set_metadata(
                &source,
                &changed.observation.stamp,
                &MetadataUpdate {
                    created_ns: Some(0),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
}
#[test]
fn staging_cannot_bypass_an_explicitly_protected_root() {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join(".vidlib-staging-protected");
    let mut adapter = PlatformFs::default();
    adapter.policy.protected_roots.push(staging.clone());
    assert_eq!(
        adapter.create_staging(&staging).unwrap_err().code,
        ErrorCode::ProtectedEntry
    );
    assert!(!staging.exists());
}
#[test]
fn directories_and_symlink_destinations_are_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("source-child"), b"safe").unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("target-child"), b"safe too").unwrap();
    let mut adapter = PlatformFs::default();
    assert_eq!(
        adapter
            .rename_no_replace(&source, &target, &stamp(&source))
            .unwrap_err()
            .code,
        ErrorCode::DestinationExists
    );
    assert_eq!(fs::read(target.join("target-child")).unwrap(), b"safe too");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(
        adapter
            .rename_no_replace(&source, &link, &stamp(&source))
            .unwrap_err()
            .code,
        ErrorCode::DestinationExists
    );
    assert!(link.is_symlink());
    assert_eq!(fs::read(source.join("source-child")).unwrap(), b"safe");
}
#[test]
fn unknown_filesystem_is_read_only_to_mutation_seam() {
    let mut adapter = PlatformFs::default();
    assert_eq!(
        adapter
            .create_directory(Path::new("/proc/vidlib-task02-unsupported"))
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
}
#[test]
fn tmpfs_native_capability_fixture() {
    let dir = tempfile::tempdir_in("/dev/shm").unwrap();
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    fs::write(&source, b"source").unwrap();
    fs::write(&target, b"existing").unwrap();
    let mut adapter = PlatformFs::default();
    assert_eq!(
        adapter
            .rename_no_replace(&source, &target, &stamp(&source))
            .unwrap_err()
            .code,
        ErrorCode::DestinationExists
    );
    assert_eq!(fs::read(&target).unwrap(), b"existing");
    fs::remove_file(&target).unwrap();
    let outcome = adapter
        .rename_no_replace(&source, &target, &stamp(&source))
        .unwrap();
    assert_eq!(outcome.durability, Durability::Synced);
    assert_eq!(fs::read(target).unwrap(), b"source");
}
#[test]
fn planner_and_adapter_share_native_reserved_name_and_length_policy() {
    use vidlib_inventory::{
        DirectoryFacts, NamePolicy, NameViolation, NativePathRecord, PathPlatform,
    };
    let facts = DirectoryFacts {
        name_policy: NamePolicy::Windows,
        max_component_units: Some(255),
        trailing_dots_spaces_allowed: Some(false),
        reserved_names: vec!["CON".into(), "COM1".into(), "COM¹".into()],
        ..Default::default()
    };
    let native = |name: &str| NativePathRecord {
        platform: PathPlatform::WindowsUtf16Le,
        bytes: name.encode_utf16().flat_map(u16::to_le_bytes).collect(),
    };
    for name in ["CON", "con.mp4", "COM1.ext", "COM¹.mp4"] {
        assert_eq!(
            facts.validate_name(&native(name)),
            Err(NameViolation::ReservedName)
        );
    }
    for name in ["x:", "x/y", "x\\y", "a\0b", "a\u{1}b", "a*b"] {
        assert!(facts.validate_name(&native(name)).is_err());
    }
    assert_eq!(
        facts.validate_name(&native("trailing.")),
        Err(NameViolation::TrailingDotOrSpace)
    );
    assert_eq!(
        facts.validate_name(&native(&"a".repeat(256))),
        Err(NameViolation::TooLong)
    );
    assert_eq!(facts.validate_name(&native("convention.mp4")), Ok(()));
    let raw = NativePathRecord {
        platform: PathPlatform::WindowsUtf16Le,
        bytes: vec![0, 0xd8],
    };
    assert_eq!(facts.validate_name(&raw), Ok(())); // unpaired surrogate is preserved
    assert_eq!(
        DirectoryFacts::default().validate_name(&native("safe")),
        Err(NameViolation::UnknownPolicy)
    );
}
#[test]
fn every_candidate_is_grant_checked_before_parent_deduplication() {
    let dir = tempfile::tempdir().unwrap();
    let selected = dir.path().join("selected");
    let sibling = dir.path().join("sibling");
    fs::write(&selected, b"selected").unwrap();
    let adapter = PlatformFs::default();
    assert_eq!(
        adapter
            .capture_namespace(&grants(&selected), &[selected.clone(), sibling.clone()])
            .unwrap_err()
            .code,
        ErrorCode::OutsideGrant
    );
    fs::write(&sibling, b"sibling").unwrap();
    let mut scope = grants(&selected);
    scope.grants.extend(grants(&sibling).grants);
    let snapshot = adapter
        .capture_namespace(&scope, &[selected, sibling])
        .unwrap();
    assert_eq!(snapshot.parents.len(), 1);
    assert_eq!(snapshot.parents[0].source_grants.len(), 2);
}

#[test]
fn staging_refresh_rejects_a_path_replaced_while_content_handle_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let stage = dir.path().join(".vidlib-staging-refresh");
    let mut adapter = PlatformFs::default();
    let mut owned = adapter.create_staging(&stage).unwrap();
    owned.file.write_all(b"ours").unwrap();
    fs::rename(&stage, dir.path().join("original")).unwrap();
    fs::write(&stage, b"foreign").unwrap();
    assert_eq!(
        adapter.refresh_staging(&mut owned).unwrap_err().code,
        ErrorCode::StaleState
    );
    assert_eq!(fs::read(stage).unwrap(), b"foreign");
}

#[test]
fn internal_owned_staging_keeps_ordinary_namespace_protection() {
    let dir = tempfile::tempdir().unwrap();
    let stage = dir.path().join(".vidlib-staging-owned-contract");
    let mut adapter = PlatformFs::default();
    assert!(adapter.policy.excludes(&stage));
    let staging = adapter
        .create_staging(&stage)
        .expect("owned create-new staging remains supported");
    let ordinary = dir.path().join("ordinary");
    assert_eq!(
        adapter
            .rename_no_replace(&stage, &ordinary, &staging.ownership.stamp)
            .unwrap_err()
            .code,
        ErrorCode::ProtectedEntry
    );
    adapter.remove_owned(&staging.ownership).unwrap();
    assert!(!stage.exists());
}
#[test]
fn owned_staging_cannot_borrow_cycle_or_explicit_policy_authority() {
    let dir = tempfile::tempdir().unwrap();
    for mode in 0..5 {
        let mut adapter = PlatformFs::default();
        let stage = dir.path().join(if mode == 0 {
            ".vidlib-staging-rename-unowned-0"
        } else {
            ".vidlib-staging-explicit"
        });
        match mode {
            1 => adapter.policy.marker_name = ".vidlib-staging-explicit".into(),
            2 => adapter.policy.guarded_roots.push(stage.clone()),
            3 => adapter.policy.protected_roots.push(stage.clone()),
            4 => adapter
                .policy
                .staging_prefixes
                .push(".vidlib-staging-explicit".into()),
            _ => {}
        }
        assert_eq!(
            adapter.create_staging(&stage).unwrap_err().code,
            ErrorCode::ProtectedEntry
        );
        // Even a caller-supplied ownership record cannot turn an unowned cycle
        // reservation into copy publication or cleanup authority.
        fs::write(&stage, b"unowned").unwrap();
        let owned = vidlib_fileops::platform::OwnedEntry {
            path: stage.clone(),
            stamp: stamp(&stage),
        };
        assert_eq!(
            adapter
                .publish_staging(&owned, &dir.path().join("published"))
                .unwrap_err()
                .code,
            ErrorCode::ProtectedEntry
        );
        assert_eq!(
            adapter.remove_owned(&owned).unwrap_err().code,
            ErrorCode::ProtectedEntry
        );
        assert_eq!(fs::read(&stage).unwrap(), b"unowned");
        fs::remove_file(stage).unwrap();
    }
}
