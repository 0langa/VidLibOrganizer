mod support;
use std::{collections::HashSet, fs, path::PathBuf};
use support::{fault_scan, FaultFs, Fixture};
use vidlib_inventory::{
    encode_path, scan, CoverageOutcome, EntryKind, InventoryRequest, JobControl, RootBinding,
};

#[test]
fn scan_keeps_unique_empty_and_nonvideo_entries() {
    let fixture = Fixture::new();
    let inventory = scan(&fixture.request(), &JobControl::default()).unwrap();
    assert!(inventory.entries.iter().any(|e| e.display_name == "b.mp4"));
    assert!(inventory
        .entries
        .iter()
        .any(|e| e.display_name == "notes.txt"));
    assert!(inventory
        .entries
        .iter()
        .any(|e| e.display_name == "nested" && e.kind == EntryKind::Directory));
    assert_eq!(inventory.entries.len(), 6);
    assert_eq!(
        inventory
            .entries
            .iter()
            .find(|e| e.display_name == "b.mp4")
            .unwrap()
            .stamp
            .size,
        Some(0)
    );
    assert!(inventory
        .coverage
        .iter()
        .all(|s| s.children_complete && s.subtree_complete));
    assert!(!fixture.root().join(".vidlib-blueprint.json").exists());
}

#[test]
fn overlap_deduplicates_paths_not_hardlinks() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request
        .roots
        .push(RootBinding::new(fixture.root().join("nested")).unwrap());
    request
        .roots
        .push(RootBinding::new(fixture.root().to_path_buf()).unwrap());
    let inventory = scan(&request, &JobControl::default()).unwrap();
    let paths: HashSet<_> = inventory.entries.iter().map(|e| &e.absolute_path).collect();
    assert_eq!(paths.len(), inventory.entries.len());
    let hardlink_entries: Vec<_> = inventory
        .entries
        .iter()
        .filter(|e| e.display_name == "a.mkv" || e.display_name == "alias.mkv")
        .collect();
    assert_eq!(hardlink_entries.len(), 2);
    assert!(hardlink_entries[0].stamp.physical_id.is_some());
    assert_eq!(
        hardlink_entries[0].stamp.physical_id,
        hardlink_entries[1].stamp.physical_id
    );
}

#[test]
fn failed_stat_retains_listed_child() {
    let fixture = Fixture::new();
    let mut adapter = FaultFs::new(JobControl::default());
    adapter.fail_stat = Some(fixture.root().join("notes.txt"));
    let inventory = fault_scan(&fixture, &adapter);
    let failed_scope = inventory
        .coverage
        .iter()
        .find(|s| s.directory == fixture.root())
        .unwrap();
    let failed_name = encode_path(&PathBuf::from("notes.txt"));
    assert!(!failed_scope.subtree_complete);
    assert!(failed_scope.children_complete);
    assert!(failed_scope.listed_names.contains(&failed_name));
    let failed = inventory
        .entries
        .iter()
        .find(|e| e.display_name == "notes.txt")
        .unwrap();
    assert_eq!(failed.kind, EntryKind::Unknown);
    assert!(!failed.issues.is_empty());
}

#[test]
fn cancellation_preserves_coverage() {
    let fixture = Fixture::new();
    let mut adapter = FaultFs::new(JobControl::default());
    adapter.cancel_after_stat = Some(2);
    let inventory = fault_scan(&fixture, &adapter);
    let scope = inventory
        .coverage
        .iter()
        .find(|s| s.directory == fixture.root())
        .unwrap();
    assert_eq!(scope.listed_names.len(), 5);
    assert!(scope.children_complete);
    assert!(!scope.subtree_complete);
    assert_eq!(scope.outcome, CoverageOutcome::Cancelled);
    assert!(inventory.entries.len() < 6);
}

#[test]
fn partial_and_failed_listings_propagate_incomplete_coverage_to_ancestors() {
    for partial in [false, true] {
        let fixture = Fixture::new();
        let mut adapter = FaultFs::new(JobControl::default());
        if partial {
            adapter.partial_listing = Some(fixture.root().join("nested"));
        } else {
            adapter.fail_listing = Some(fixture.root().join("nested"));
        }
        let inventory = fault_scan(&fixture, &adapter);
        let child = inventory
            .coverage
            .iter()
            .find(|s| s.directory.ends_with("nested"))
            .unwrap();
        assert!(!child.children_complete);
        assert!(!child.subtree_complete);
        assert_eq!(child.outcome, CoverageOutcome::Failed);
        assert!(
            !inventory
                .coverage
                .iter()
                .find(|s| s.directory == fixture.root())
                .unwrap()
                .subtree_complete
        );
        if partial {
            assert!(child
                .listed_names
                .contains(&encode_path(&PathBuf::from("c.mov"))));
        }
    }
}

#[test]
fn explicit_files_are_supported_and_relative_grants_are_rejected() {
    let fixture = Fixture::new();
    assert!(RootBinding::new(PathBuf::from("relative/root")).is_err());
    let request =
        InventoryRequest::new(vec![RootBinding::new(fixture.root().join("a.mkv")).unwrap()]);
    let inventory = scan(&request, &JobControl::default()).unwrap();
    assert_eq!(inventory.entries.len(), 1);
    assert_eq!(
        inventory.entries[0].relative_path,
        encode_path(&PathBuf::from("a.mkv"))
    );
    assert!(scan(&InventoryRequest::new(vec![]), &JobControl::default()).is_err());
}

#[test]
fn protected_marker_and_staging_names_remain_in_listing_but_are_excluded() {
    let fixture = Fixture::new();
    fs::write(fixture.root().join(".vidlib-blueprint.json"), b"{}").unwrap();
    fs::create_dir(fixture.root().join(".vidlib-staging")).unwrap();
    fs::write(fixture.root().join(".vidlib-staging/hidden.mkv"), b"hidden").unwrap();
    let inventory = scan(&fixture.request(), &JobControl::default()).unwrap();
    assert!(!inventory
        .entries
        .iter()
        .any(|e| e.display_name.starts_with(".vidlib-")));
    let scope = inventory
        .coverage
        .iter()
        .find(|s| s.directory == fixture.root())
        .unwrap();
    assert!(scope
        .listed_names
        .contains(&encode_path(&PathBuf::from(".vidlib-blueprint.json"))));
    assert!(scope.subtree_complete);
    assert!(!scope.filter_fingerprint.is_empty());
}

#[cfg(unix)]
#[test]
fn links_are_visible_but_never_followed() {
    let fixture = Fixture::new();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("outside.mkv"), b"outside").unwrap();
    std::os::unix::fs::symlink(external.path(), fixture.root().join("link")).unwrap();
    std::os::unix::fs::symlink("missing-target", fixture.root().join("broken-link")).unwrap();
    let inventory = scan(&fixture.request(), &JobControl::default()).unwrap();
    assert!(inventory
        .entries
        .iter()
        .any(|e| e.display_name == "link" && e.kind == EntryKind::Symlink));
    assert!(inventory
        .entries
        .iter()
        .any(|e| e.display_name == "broken-link" && e.kind == EntryKind::Symlink));
    assert!(!inventory
        .entries
        .iter()
        .any(|e| e.display_name == "outside.mkv"));
    assert!(!inventory
        .coverage
        .iter()
        .any(|s| s.directory.ends_with("link")));
}

#[cfg(unix)]
#[test]
fn non_utf8_names_remain_native_and_carry_transformation_issues() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let fixture = Fixture::new();
    let path = fixture
        .root()
        .join(OsString::from_vec(b"bad-\xff.mkv".to_vec()));
    fs::write(&path, b"native").unwrap();
    let inventory = scan(&fixture.request(), &JobControl::default()).unwrap();
    let entry = inventory
        .entries
        .iter()
        .find(|e| e.absolute_path == path)
        .unwrap();
    assert!(entry
        .issues
        .iter()
        .any(|issue| issue.code == vidlib_inventory::IssueCode::UnsupportedName));
    assert_eq!(
        vidlib_inventory::decode_path(&entry.relative_path).unwrap(),
        path.file_name().unwrap()
    );
    // JSON never serializes the backend absolute PathBuf through a lossy string.
    let json = serde_json::to_value(entry).unwrap();
    assert!(json.get("absolute_path").is_none());
}

#[test]
fn precancelled_scan_records_unvisited_roots_and_progress_is_bounded() {
    let fixture = Fixture::new();
    let (control, receiver) = JobControl::with_progress(1);
    control.cancel();
    let inventory = scan(&fixture.request(), &control).unwrap();
    assert!(inventory.entries.is_empty());
    assert!(!inventory.coverage[0].children_complete);
    assert_eq!(inventory.coverage[0].outcome, CoverageOutcome::Cancelled);
    assert!(receiver.try_iter().count() <= 1);
    assert!(inventory.window.started_ns <= inventory.window.finished_ns);
}

#[test]
fn namespace_model_carries_exact_epochs_and_observed_parent_facts_without_fileops() {
    use vidlib_inventory::{CaseSensitivity, DirectoryFacts, NamespaceSnapshot, RootEvidenceState};
    let fixture = Fixture::new();
    let root = RootBinding::new(fixture.root().to_path_buf()).unwrap();
    assert_eq!(root.evidence.state, RootEvidenceState::Absent);
    let mut namespace = NamespaceSnapshot {
        namespace_epoch: u64::MAX,
        policy_epoch: 9007199254740993,
        ..NamespaceSnapshot::default()
    };
    let parent = DirectoryFacts {
        parent_path: fixture.root().to_path_buf(),
        case_sensitivity: CaseSensitivity::Unknown,
        ..DirectoryFacts::default()
    };
    namespace.parents.push(parent);
    namespace.grants.grants.push(root.source_grant);
    let json = serde_json::to_value(namespace).unwrap();
    assert_eq!(json["namespace_epoch"], u64::MAX.to_string());
    assert_eq!(json["policy_epoch"], "9007199254740993");
    assert!(json["parents"][0].get("parent_path").is_none());
}

#[test]
fn native_modified_timestamp_and_regular_file_size_match_os_observation() {
    let fixture = Fixture::new();
    let inventory = scan(&fixture.request(), &JobControl::default()).unwrap();
    let entry = inventory
        .entries
        .iter()
        .find(|e| e.display_name == "a.mkv")
        .unwrap();
    let metadata = fs::symlink_metadata(&entry.absolute_path).unwrap();
    let expected = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i128;
    assert_eq!(entry.stamp.modified_ns, Some(expected));
    assert_eq!(entry.stamp.size, Some(metadata.len()));
    assert!(inventory
        .entries
        .iter()
        .filter(|e| e.kind != EntryKind::File)
        .all(|e| e.stamp.size.is_none()));
}

#[test]
fn protected_subtrees_are_excluded_when_selected_directly() {
    let fixture = Fixture::new();
    let staging = fixture.root().join(vidlib_inventory::STAGING_NAME_PREFIX);
    fs::create_dir(&staging).unwrap();
    let hidden = staging.join("hidden.mkv");
    fs::write(&hidden, b"hidden").unwrap();
    for selected in [staging, hidden] {
        let request = InventoryRequest::new(vec![RootBinding::new(selected).unwrap()]);
        let inventory = scan(&request, &JobControl::default()).unwrap();
        assert!(inventory.entries.is_empty());
        assert_eq!(inventory.coverage[0].outcome, CoverageOutcome::Excluded);
    }
}

#[cfg(unix)]
#[test]
fn os_directory_listing_refuses_directory_links() {
    use vidlib_inventory::InventoryFs;
    let fixture = Fixture::new();
    let link = fixture.root().join("link");
    std::os::unix::fs::symlink(fixture.root().join("nested"), &link).unwrap();
    assert!(vidlib_inventory::OsInventoryFs
        .read_directory(&link)
        .is_err());
}

#[cfg(windows)]
#[test]
fn os_directory_listing_refuses_directory_reparse_points() {
    use vidlib_inventory::InventoryFs;
    let fixture = Fixture::new();
    let link = fixture.root().join("link");
    // Windows runtime verification needs Developer Mode or symlink privileges.
    std::os::windows::fs::symlink_dir(fixture.root().join("nested"), &link).unwrap();
    assert!(vidlib_inventory::OsInventoryFs
        .read_directory(&link)
        .is_err());
}
