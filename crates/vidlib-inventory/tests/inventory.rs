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

#[test]
fn overlapping_bindings_validate_evidence_before_deduplicating_traversal() {
    use vidlib_inventory::{IssueCode, PathPlatform, PhysicalId, RootEvidenceState};
    let fixture = Fixture::new();
    let mut nested = RootBinding::new(fixture.root().join("nested")).unwrap();
    nested.evidence.expected_identity = Some(PhysicalId {
        platform: PathPlatform::UnixBytes,
        volume_identity: vec![0],
        file_identity: vec![0],
    });
    nested.evidence.state = RootEvidenceState::Matches;
    let nested_id = nested.id;
    let mut request = fixture.request();
    request.roots.push(nested);
    let inventory = scan(&request, &JobControl::default()).unwrap();
    let binding = inventory
        .roots
        .iter()
        .find(|root| root.id == nested_id)
        .unwrap();
    assert_eq!(binding.evidence.state, RootEvidenceState::Mismatch);
    assert!(binding.evidence.observed_identity.is_some());
    assert!(inventory
        .issues
        .iter()
        .any(|issue| issue.code == IssueCode::EvidenceMismatch));
    assert!(inventory
        .coverage
        .iter()
        .any(|scope| scope.root_id == nested_id && scope.outcome == CoverageOutcome::Failed));
    assert_eq!(inventory.entries.len(), 6);
}

#[test]
fn overlapping_bindings_retain_their_own_observation_and_coverage_association() {
    let fixture = Fixture::new();
    let nested = RootBinding::new(fixture.root().join("nested")).unwrap();
    let nested_id = nested.id;
    let mut request = fixture.request();
    request.roots.push(nested);
    let inventory = scan(&request, &JobControl::default()).unwrap();
    let binding = inventory
        .roots
        .iter()
        .find(|root| root.id == nested_id)
        .unwrap();
    assert!(binding.evidence.observed_identity.is_some());
    let scope = inventory
        .coverage
        .iter()
        .find(|scope| scope.root_id == nested_id)
        .unwrap();
    assert!(scope.children_complete && scope.subtree_complete);
    assert_eq!(scope.relative_directory, None);
    assert_eq!(inventory.entries.len(), 6);
}

#[test]
fn reserved_ascii_case_variants_are_conservatively_excluded_for_unknown_filesystems() {
    let fixture = Fixture::new();
    let marker = fixture.root().join(".VIDLIB-BLUEPRINT.JSON");
    let staging = fixture.root().join(".VIDLIB-STAGING-ATTEMPT");
    fs::write(&marker, b"{}").unwrap();
    fs::create_dir(&staging).unwrap();
    let hidden = staging.join("hidden.mkv");
    fs::write(&hidden, b"hidden").unwrap();
    let inventory = scan(&fixture.request(), &JobControl::default()).unwrap();
    assert!(!inventory
        .entries
        .iter()
        .any(|entry| entry.absolute_path == marker
            || entry.absolute_path == staging
            || entry.absolute_path == hidden));
    for selected in [marker, staging, hidden] {
        let request = InventoryRequest::new(vec![RootBinding::new(selected).unwrap()]);
        let inventory = scan(&request, &JobControl::default()).unwrap();
        assert!(inventory.entries.is_empty());
        assert_eq!(inventory.coverage[0].outcome, CoverageOutcome::Excluded);
    }
}

#[test]
fn protected_roots_conservatively_match_case_aliases_at_component_boundaries() {
    let fixture = Fixture::new();
    let protected = fixture.root().join("protected");
    fs::create_dir(&protected).unwrap();
    fs::write(protected.join("hidden.mkv"), b"hidden").unwrap();
    let sibling = fixture.root().join("protected-other");
    fs::create_dir(&sibling).unwrap();
    fs::write(sibling.join("visible.mkv"), b"visible").unwrap();
    let mut request = fixture.request();
    request
        .policy
        .protected_roots
        .push(fixture.root().join("PrOtEcTeD"));
    let inventory = scan(&request, &JobControl::default()).unwrap();
    assert!(!inventory
        .entries
        .iter()
        .any(|entry| entry.absolute_path.starts_with(&protected)));
    assert!(inventory
        .entries
        .iter()
        .any(|entry| entry.display_name == "visible.mkv"));
    request.roots = vec![RootBinding::new(protected.join("hidden.mkv")).unwrap()];
    let direct = scan(&request, &JobControl::default()).unwrap();
    assert!(direct.entries.is_empty());
    assert_eq!(direct.coverage[0].outcome, CoverageOutcome::Excluded);
}

#[cfg(windows)]
#[test]
fn windows_protected_roots_match_native_unicode_case_aliases() {
    let fixture = Fixture::new();
    let protected = fixture.root().join("Média");
    fs::create_dir(&protected).unwrap();
    fs::write(protected.join("hidden.mkv"), b"hidden").unwrap();
    let mut request = fixture.request();
    request
        .policy
        .protected_roots
        .push(fixture.root().join("MÉDIA"));
    let inventory = scan(&request, &JobControl::default()).unwrap();
    assert!(!inventory
        .entries
        .iter()
        .any(|entry| entry.absolute_path.starts_with(&protected)));
}

#[test]
fn replacing_a_listed_directory_with_an_external_link_never_observes_external_children() {
    let fixture = Fixture::new();
    let nested = fixture.root().join("nested");
    fs::create_dir(nested.join("sub")).unwrap();
    fs::write(nested.join("sub/inside.mkv"), b"inside").unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::create_dir(external.path().join("sub")).unwrap();
    fs::write(external.path().join("sub/outside.mkv"), b"outside").unwrap();
    fs::write(external.path().join("c.mov"), b"outside-c").unwrap();
    let mut adapter = FaultFs::new(JobControl::default());
    adapter.swap_after_listing = Some((
        nested.clone(),
        fixture.root().join("moved-nested"),
        external.path().to_path_buf(),
    ));
    let inventory = fault_scan(&fixture, &adapter);
    assert!(!inventory
        .entries
        .iter()
        .any(|entry| entry.display_name == "outside.mkv"));
    let scope = inventory
        .coverage
        .iter()
        .find(|scope| scope.directory == nested)
        .unwrap();
    assert!(!scope.subtree_complete);
    assert_eq!(scope.outcome, CoverageOutcome::Failed);
    assert!(scope
        .listed_names
        .contains(&encode_path(&PathBuf::from("sub"))));
    assert!(
        !inventory
            .coverage
            .iter()
            .find(|scope| scope.directory == fixture.root())
            .unwrap()
            .subtree_complete
    );
}

#[test]
fn failed_binding_observation_drops_previously_matched_evidence() {
    use vidlib_inventory::{PathPlatform, PhysicalId, RootEvidenceState};
    let fixture = Fixture::new();
    let mut missing = RootBinding::new(fixture.root().join("missing")).unwrap();
    let old = PhysicalId {
        platform: PathPlatform::UnixBytes,
        volume_identity: vec![1],
        file_identity: vec![2],
    };
    missing.evidence.expected_identity = Some(old.clone());
    missing.evidence.observed_identity = Some(old);
    missing.evidence.state = RootEvidenceState::Matches;
    let missing_id = missing.id;
    let mut request = fixture.request();
    request.roots.push(missing);
    let inventory = scan(&request, &JobControl::default()).unwrap();
    let binding = inventory
        .roots
        .iter()
        .find(|root| root.id == missing_id)
        .unwrap();
    assert_eq!(binding.evidence.state, RootEvidenceState::Unavailable);
    assert!(binding.evidence.observed_identity.is_none());
}

#[test]
fn unavailable_directory_identity_preserves_safe_entries_with_incomplete_coverage() {
    use vidlib_inventory::{InventoryFs, RootEvidenceState};
    let fixture = Fixture::new();
    for (omit_root_identity, omit_handle_identity) in [(true, false), (false, true), (true, true)] {
        let mut adapter = FaultFs::new(JobControl::default());
        adapter.omit_root_identity = omit_root_identity;
        adapter.omit_handle_identity = omit_handle_identity;
        let mut request = fixture.request();
        let metadata = fs::symlink_metadata(fixture.root()).unwrap();
        request.roots[0].evidence.expected_identity =
            vidlib_inventory::OsInventoryFs.physical_id(fixture.root(), &metadata);
        let inventory =
            vidlib_inventory::scan_with_fs(&request, &adapter.control, &adapter).unwrap();
        assert_eq!(
            inventory.entries.len(),
            6,
            "root missing={omit_root_identity}, handle missing={omit_handle_identity}"
        );
        assert!(inventory
            .entries
            .iter()
            .any(|entry| entry.display_name == "c.mov"));
        let scope = inventory
            .coverage
            .iter()
            .find(|scope| scope.directory == fixture.root())
            .unwrap();
        assert_eq!(scope.listed_names.len(), 5);
        assert!(scope.children_complete);
        assert!(!scope.subtree_complete);
        assert_eq!(scope.outcome, CoverageOutcome::Failed);
        assert!(scope
            .issues
            .iter()
            .any(|issue| issue.code == vidlib_inventory::IssueCode::IdentityUnavailable));
        if omit_root_identity {
            assert_eq!(
                inventory.roots[0].evidence.state,
                RootEvidenceState::Unavailable
            );
        }
    }
}

#[test]
fn mismatched_identity_or_invalid_directory_handle_still_rejects_traversal() {
    let fixture = Fixture::new();
    for invalid_kind in [false, true] {
        let mut adapter = FaultFs::new(JobControl::default());
        adapter.wrong_handle_identity = !invalid_kind;
        adapter.invalid_directory_kind = invalid_kind;
        let inventory = fault_scan(&fixture, &adapter);
        assert!(inventory.entries.is_empty());
        assert!(inventory.coverage[0].listed_names.is_empty());
        assert!(!inventory.coverage[0].subtree_complete);
        assert_eq!(inventory.coverage[0].outcome, CoverageOutcome::Failed);
    }
}
