//! Read-only domain fixtures: no filesystem creation or display-path authority.
use std::path::{Path, PathBuf};
use vidlib_inventory::{ids::*, *};
use vidlib_rename::*;

fn path(value: &str) -> PathBuf {
    if cfg!(windows) {
        Path::new("C:\\media").join(value)
    } else {
        Path::new("/media").join(value)
    }
}
fn fixture(names: &[(&str, EntryKind)]) -> (Inventory, EvaluatedBatch, NamespaceSnapshot, PlanRef) {
    let root = RootBinding::new(path("")).unwrap();
    let entries: Vec<_> = names
        .iter()
        .map(|(name, kind)| EntrySnapshot {
            id: EntryId::new(),
            root_id: root.id,
            relative_path: encode_path(Path::new(name)),
            absolute_path: path(name),
            display_name: name.to_string(),
            display_path: path(name).display().to_string(),
            kind: *kind,
            stamp: FileStamp {
                physical_id: Some(PhysicalId {
                    platform: encode_path(Path::new(name)).platform,
                    volume_identity: vec![1],
                    file_identity: name.as_bytes().to_vec(),
                }),
                size: Some(10),
                ..Default::default()
            },
            observed_ns: 0,
            issues: vec![],
        })
        .collect();
    let coverage = entries
        .iter()
        .filter(|e| e.kind == EntryKind::Directory)
        .map(|e| ScopeCoverage {
            root_id: root.id,
            directory: e.absolute_path.clone(),
            relative_directory: Some(e.relative_path.clone()),
            listed_names: vec![],
            children_complete: true,
            subtree_complete: true,
            outcome: CoverageOutcome::Complete,
            filter_fingerprint: "complete".into(),
            window: Default::default(),
            issues: vec![],
        })
        .collect();
    let inventory = Inventory {
        id: InventoryId::new(),
        generation: 1,
        roots: vec![root],
        entries,
        coverage,
        window: Default::default(),
        issues: vec![],
    };
    let selection: Vec<_> = inventory.entries.iter().map(|e| e.id).collect();
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: selection.clone(),
        selection_filter: None,
        numbering_order: selection.clone(),
        numbering_scope: Default::default(),
        rules: vec![],
        companions: CompanionPolicy::None,
        destination: Destination::SameDirectory,
        collision_policy: CollisionPolicy::Block,
        metadata_changes: vec![],
    };
    let frozen = FrozenInputs {
        inventory_id: inventory.id,
        generation: 1,
        now_ns: 123,
        timezone: FrozenTimezone::Utc,
        primary_order: selection,
        numbering_scope: Default::default(),
        values: vec![],
    };
    let batch = evaluate(&inventory, &recipe, &frozen).unwrap();
    let mut namespace = NamespaceSnapshot {
        grants: batch.grants.clone(),
        ..Default::default()
    };
    for entry in &inventory.entries {
        let parent = entry.absolute_path.parent().unwrap();
        if !namespace.parents.iter().any(|f| f.parent_path == parent) {
            namespace.parents.push(DirectoryFacts {
                stamp: FileStamp {
                    physical_id: Some(PhysicalId {
                        platform: encode_path(parent).platform,
                        volume_identity: vec![1],
                        file_identity: vec![0],
                    }),
                    ..Default::default()
                },
                parent_path: parent.to_owned(),
                source_grants: vec![inventory.roots[0].source_grant.id],
                case_sensitivity: CaseSensitivity::Sensitive,
                normalization: NameNormalization::Preserved,
                max_component_units: Some(255),
                name_policy: if cfg!(windows) {
                    NamePolicy::Windows
                } else {
                    NamePolicy::Posix
                },
                trailing_dots_spaces_allowed: Some(!cfg!(windows)),
                capabilities: FilesystemCapabilities {
                    read_only: Some(false),
                    physical_identity: Support::Supported,
                    atomic_rename: Support::Supported,
                    ..Default::default()
                },
                children_complete: true,
                ..Default::default()
            });
        }
        namespace
            .parents
            .iter_mut()
            .find(|f| f.parent_path == parent)
            .unwrap()
            .entries
            .push(NamespaceEntry {
                name: encode_path(Path::new(entry.absolute_path.file_name().unwrap())),
                absolute_path: entry.absolute_path.clone(),
                kind: entry.kind,
                stamp: entry.stamp.clone(),
                protected: false,
            });
    }
    let version = PlanRef {
        session_id: SessionId::new(),
        plan_id: PlanId::new(),
        revision: 1,
    };
    (inventory, batch, namespace, version)
}
fn target(batch: &mut EvaluatedBatch, index: usize, name: &str) {
    batch.rows[index].target_path = path(name);
    batch.rows[index].status = RowStatus::Changed;
}
fn steps(plan: &RenamePlan) -> Vec<&ExecutionStep> {
    plan.execution
        .groups
        .iter()
        .flat_map(|g| &g.steps)
        .collect()
}
// Independent interpreter catches overwrite, stale descendant locations and missing steps.
fn simulate(inventory: &Inventory, plan: &RenamePlan) -> Vec<(EntryId, PathBuf)> {
    let mut locations: Vec<_> = inventory
        .entries
        .iter()
        .map(|e| (e.id, e.absolute_path.clone()))
        .collect();
    for step in steps(plan) {
        if let ExecutionAction::RenameEntry { source, target, .. } = &step.action {
            assert!(
                locations
                    .iter()
                    .any(|(id, p)| *id == step.entry_id && *p == source.path),
                "source location stale"
            );
            assert!(
                !locations.iter().any(|(_, p)| p == target),
                "would overwrite occupied target"
            );
            for (_, p) in &mut locations {
                if let Ok(relative) = p.strip_prefix(&source.path) {
                    *p = target.join(relative);
                }
            }
        }
    }
    locations
}

// Missing cycle breaking would overwrite an occupied source or omit a rename.
#[test]
fn swap_and_three_cycle_plan_without_overwrite() {
    for names in [vec!["a", "b"], vec!["a", "b", "c"]] {
        let entries: Vec<_> = names.iter().map(|n| (*n, EntryKind::File)).collect();
        let (inventory, mut batch, ns, version) = fixture(&entries);
        for i in 0..names.len() {
            target(&mut batch, i, names[(i + 1) % names.len()]);
        }
        let plan = plan(&inventory, &batch, &ns, version).unwrap();
        assert_eq!(plan.counts.blocked, 0);
        assert_eq!(steps(&plan).len(), names.len() + 1);
        let actual = simulate(&inventory, &plan);
        for (i, row) in batch.rows.iter().enumerate() {
            assert_eq!(
                actual.iter().find(|(id, _)| *id == row.entry_id).unwrap().1,
                path(names[(i + 1) % names.len()])
            );
        }
    }
}
// Case aliases must pass through a reserved, preview-frozen path.
#[test]
fn case_only_rename_uses_reserved_temp() {
    let (inventory, mut batch, mut ns, version) = fixture(&[("movie.mkv", EntryKind::File)]);
    ns.parents[0].case_sensitivity = CaseSensitivity::Insensitive;
    target(&mut batch, 0, "Movie.mkv");
    let plan = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(steps(&plan).len(), 2);
    assert!(matches!(
        &steps(&plan)[0].action,
        ExecutionAction::RenameEntry {
            temporary: true,
            ..
        }
    ));
}
// Both row final paths and compiled steps must follow moved directory identities.
#[test]
fn parent_and_child_keep_one_location_model() {
    let (inventory, mut batch, ns, version) = fixture(&[
        ("old", EntryKind::Directory),
        ("old/movie.mkv", EntryKind::File),
    ]);
    target(&mut batch, 0, "new");
    target(&mut batch, 1, "old/new.mkv");
    let plan = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(plan.counts.blocked, 0);
    assert_eq!(plan.rows[1].target_path, path("new/new.mkv"));
    assert_eq!(simulate(&inventory, &plan)[1].1, path("new/new.mkv"));
}
// Target-parent policy, not source spelling or global folding, decides collision.
#[test]
fn target_parent_case_policy_controls_collisions() {
    let (inventory, mut batch, mut ns, version) =
        fixture(&[("one", EntryKind::File), ("two", EntryKind::File)]);
    target(&mut batch, 0, "Movie");
    target(&mut batch, 1, "movie");
    let sensitive = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(sensitive.counts.changed, 2);
    assert_eq!(steps(&sensitive).len(), 2);
    ns.parents[0].case_sensitivity = CaseSensitivity::Insensitive;
    let insensitive = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(insensitive.counts.blocked, 2);
    assert!(steps(&insensitive).is_empty());
}
// Active markers and directory root entries cannot be moved.
#[test]
fn root_and_marker_protected() {
    let (inventory, mut batch, ns, version) =
        fixture(&[(".vidlib-blueprint.json", EntryKind::File)]);
    target(&mut batch, 0, "marker.json");
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
}
// Grant escape and self-descendant destinations cannot acquire execution authority.
#[test]
fn outside_destination_and_ancestor_cycle_blocked() {
    let (inventory, mut batch, ns, version) = fixture(&[("folder", EntryKind::Directory)]);
    target(&mut batch, 0, "folder/inside");
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
    batch.rows[0].target_path = path("").parent().unwrap().join("outside");
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
}
// A conflict beyond page 1 must revoke approval for the complete batch.
#[test]
fn hidden_page_conflict_blocks_entire_approval() {
    let names: Vec<_> = (0..201).map(|i| format!("movie{i}")).collect();
    let entries: Vec<_> = names
        .iter()
        .map(|s| (s.as_str(), EntryKind::File))
        .collect();
    let (inventory, mut batch, mut ns, version) = fixture(&entries);
    target(&mut batch, 200, "occupied");
    let mut occupant = ns.parents[0].entries[0].clone();
    occupant.absolute_path = path("occupied");
    occupant.name = encode_path(Path::new("occupied"));
    ns.parents[0].entries.push(occupant);
    let plan = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(plan.counts.blocked, 1);
    assert!(steps(&plan).is_empty());
    let mut store = PlanStore::new(version.session_id);
    store.publish(plan).unwrap();
    assert!(store.confirm(version, "nonce".into()).is_err());
}

// Repeated compilation must not read the clock, generate IDs or depend on capture order.
#[test]
fn independently_built_plans_are_identical_except_supplied_ref() {
    let (inventory, mut batch, mut ns, version) =
        fixture(&[("a", EntryKind::File), ("b", EntryKind::File)]);
    target(&mut batch, 0, "b");
    target(&mut batch, 1, "a");
    let first = plan(&inventory.clone(), &batch.clone(), &ns.clone(), version).unwrap();
    ns.parents[0].entries.reverse();
    let next = PlanRef {
        plan_id: PlanId::new(),
        revision: 2,
        ..version
    };
    let second = plan(&inventory, &batch, &ns, next).unwrap();
    let mut dto = second.to_dto();
    dto.version = version;
    assert_eq!(
        serde_json::to_vec(&first.to_dto()).unwrap(),
        serde_json::to_vec(&dto).unwrap()
    );
    assert_eq!(steps(&first), steps(&second));
}
// Collisions are by path, not by physical identity shared by hard links.
#[test]
fn hard_link_aliases_are_distinct_paths() {
    let (mut inventory, mut batch, mut ns, version) =
        fixture(&[("a", EntryKind::File), ("b", EntryKind::File)]);
    let identity = inventory.entries[0].stamp.physical_id.clone();
    inventory.entries[1].stamp.physical_id = identity.clone();
    batch.rows[1].stamp.physical_id = identity.clone();
    ns.parents[0].entries[1].stamp.physical_id = identity;
    target(&mut batch, 0, "c");
    target(&mut batch, 1, "d");
    let plan = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(steps(&plan).len(), 2);
    assert_eq!(
        simulate(&inventory, &plan)
            .iter()
            .map(|(_, p)| p.clone())
            .collect::<Vec<_>>(),
        vec![path("c"), path("d")]
    );
}
// A native file grant authorizes its sibling output, without authorizing sibling sources.
#[test]
fn individual_file_grant_keeps_source_scope() {
    let (mut inventory, mut batch, mut ns, version) = fixture(&[("a", EntryKind::File)]);
    inventory.roots[0].path = path("a");
    inventory.roots[0].source_grant.path = path("a");
    batch.grants.grants[0].path = path("a");
    ns.grants = batch.grants.clone();
    target(&mut batch, 0, "b");
    assert_eq!(
        steps(&plan(&inventory, &batch, &ns, version).unwrap()).len(),
        1
    );
    batch.rows[0].source_path = path("neighbor");
    assert!(plan(&inventory, &batch, &ns, version).is_err());
}
// Exact registered roots protect the root and ancestors, while contents remain usable.
#[test]
fn root_guards_do_not_exclude_normal_library_contents() {
    let (inventory, mut batch, mut ns, version) = fixture(&[
        ("library", EntryKind::Directory),
        ("library/a", EntryKind::File),
    ]);
    ns.policy.guarded_roots.push(path("library"));
    target(&mut batch, 0, "renamed");
    target(&mut batch, 1, "library/b");
    let blocked = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(blocked.rows[0].status, RowStatus::Blocked);
    batch.rows.remove(0);
    batch.recipe.selection.remove(0);
    batch.recipe.numbering_order.remove(0);
    batch.frozen_inputs.primary_order.remove(0);
    let permitted = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(permitted.counts.blocked, 0);
    assert_eq!(steps(&permitted).len(), 1);
    ns.policy.protected_roots.push(path("library"));
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
}
// Unselected descendants are visibly counted; exclusions cannot be promised to remain behind.
#[test]
fn carried_descendants_and_conflicting_exclusions() {
    let (inventory, mut batch, ns, version) = fixture(&[
        ("old", EntryKind::Directory),
        ("old/child", EntryKind::File),
    ]);
    batch.rows.pop();
    batch.recipe.selection.pop();
    batch.recipe.numbering_order.pop();
    batch.frozen_inputs.primary_order.pop();
    target(&mut batch, 0, "new");
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(result.rows[0].carried_descendants, 1);
    assert_eq!(
        serde_json::to_value(result.to_dto()).unwrap()["rows"][0]["carried_descendants"],
        "1"
    );
    let mut excluded = ns;
    excluded.policy.protected_roots.push(path("old/child"));
    assert_eq!(
        plan(&inventory, &batch, &excluded, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
}
// Suffixes must be frozen in preview, including descendants listed before their parent.
#[test]
fn directory_collision_suffix_rebases_all_descendants() {
    let (inventory, mut batch, mut ns, version) = fixture(&[
        ("old/child", EntryKind::File),
        ("old", EntryKind::Directory),
        ("occupied", EntryKind::File),
    ]);
    batch.rows.pop();
    batch.recipe.selection.pop();
    batch.recipe.numbering_order.pop();
    batch.frozen_inputs.primary_order.pop();
    batch.recipe.collision_policy = CollisionPolicy::Suffix {
        separator: "-".into(),
        max_attempts: 5,
    };
    target(&mut batch, 0, "old/new");
    target(&mut batch, 1, "occupied");
    ns.parents.reverse();
    let plan = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(plan.counts.blocked, 0);
    assert_eq!(plan.rows[1].target_path, path("occupied-1"));
    assert_eq!(plan.rows[0].target_path, path("occupied-1/new"));
    assert_eq!(simulate(&inventory, &plan)[0].1, path("occupied-1/new"));
}
// Unknown target volume is not equivalent to the source's observed volume.
#[test]
fn absent_and_mismatched_directory_volume_fail_closed() {
    let (inventory, mut batch, mut ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, "b");
    ns.parents[0]
        .stamp
        .physical_id
        .as_mut()
        .unwrap()
        .volume_identity = vec![2];
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
    ns.parents[0].stamp.physical_id = None;
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
}
// Missing selected rows and altered duplicated grant provenance must not publish authority.
#[test]
fn complete_selection_and_consistent_grants_are_required() {
    let (inventory, mut batch, ns, version) = fixture(&[("a", EntryKind::File)]);
    let mut result = plan(&inventory, &batch, &ns, version).unwrap();
    result.execution.provenance.grants.grants.clear();
    assert!(PlanStore::new(version.session_id).publish(result).is_err());
    batch.rows.clear();
    assert!(plan(&inventory, &batch, &ns, version).is_err());
}
// Source and target aliases must never be reconstructed from lossy display strings.
#[cfg(unix)]
#[test]
fn invalid_unicode_display_alias_does_not_collide_with_native_name() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let (inventory, mut batch, mut ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, "�");
    let mut extra = ns.parents[0].entries[0].clone();
    extra.absolute_path = path("").join(OsString::from_vec(vec![0xff]));
    extra.name = encode_path(Path::new(extra.absolute_path.file_name().unwrap()));
    ns.parents[0].entries.push(extra);
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(result.counts.blocked, 0);
    assert_eq!(steps(&result).len(), 1);
}
// Shared native policy owns length validation; the planner must enforce it globally.
#[test]
fn long_component_and_path_limits_block() {
    let (inventory, mut batch, mut ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, &"b".repeat(256));
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
    target(&mut batch, 0, "short");
    ns.parents[0].max_path_units = Some(5);
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
}

// A target created after preview must revoke preflight even with unchanged app epochs.
#[test]
fn target_created_after_preview_fails_pure_preflight() {
    let (inventory, mut batch, ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, "b");
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    assert!(vidlib_rename::validate::preflight(&result.execution, &ns).is_ok());
    let mut fresh = ns.clone();
    let mut extra = fresh.parents[0].entries[0].clone();
    extra.absolute_path = path("b");
    extra.name = encode_path(Path::new("b"));
    fresh.parents[0].entries.push(extra);
    assert!(vidlib_rename::validate::preflight(&result.execution, &fresh).is_err());
}
// Identical directory contents do not excuse changed identities, limits or policy epochs.
#[test]
fn changed_namespace_identity_policy_and_epochs_revoke_preflight() {
    let (inventory, mut batch, ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, "b");
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    let mut fresh = ns.clone();
    fresh.parents[0]
        .stamp
        .physical_id
        .as_mut()
        .unwrap()
        .file_identity
        .push(2);
    assert!(vidlib_rename::validate::preflight(&result.execution, &fresh).is_err());
    let mut fresh = ns.clone();
    fresh.parents[0].max_component_units = Some(1);
    assert!(vidlib_rename::validate::preflight(&result.execution, &fresh).is_err());
    let mut fresh = ns.clone();
    fresh.policy_epoch += 1;
    assert!(vidlib_rename::validate::preflight(&result.execution, &fresh).is_err());
}
// A moving directory needs affirmative complete subtree coverage, not absence of errors.
#[test]
fn missing_subtree_coverage_blocks_directory_move() {
    let (mut inventory, mut batch, ns, version) = fixture(&[("a", EntryKind::Directory)]);
    target(&mut batch, 0, "b");
    inventory.coverage.clear();
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
}
// Two entry IDs cannot claim the same observed source path through overlapping roots.
#[test]
fn overlapping_source_paths_are_rejected() {
    let (inventory, mut batch, ns, version) = fixture(&[("a", EntryKind::File)]);
    let mut inventory = inventory;
    let mut alias = inventory.entries[0].clone();
    alias.id = EntryId::new();
    let mut row = batch.rows[0].clone();
    row.entry_id = alias.id;
    row.primary_id = alias.id;
    row.group_id = GroupId::from_uuid(*alias.id.as_uuid());
    batch.recipe.selection.push(alias.id);
    batch.recipe.numbering_order.push(alias.id);
    batch.frozen_inputs.primary_order.push(alias.id);
    batch.rows.push(row);
    inventory.entries.push(alias);
    target(&mut batch, 0, "b");
    target(&mut batch, 1, "c");
    assert!(plan(&inventory, &batch, &ns, version).is_err());
}

// A selected registered root itself is protected even when its parent grant is wider.
#[test]
fn selected_root_entry_and_ancestor_of_guarded_root_are_blocked() {
    let (mut inventory, mut batch, mut ns, version) = fixture(&[("library", EntryKind::Directory)]);
    target(&mut batch, 0, "renamed");
    inventory.roots[0].path = path("library");
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
    inventory.roots[0].path = path("");
    ns.policy.guarded_roots.push(path("library/nested"));
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
}
// A legal suffix can still make a descendant's full path exceed its target policy.
#[test]
fn suffix_must_revalidate_descendant_path_lengths() {
    let (inventory, mut batch, mut ns, version) = fixture(&[
        ("old", EntryKind::Directory),
        ("old/child", EntryKind::File),
        ("x", EntryKind::File),
    ]);
    batch.rows.pop();
    batch.recipe.selection.pop();
    batch.recipe.numbering_order.pop();
    batch.frozen_inputs.primary_order.pop();
    target(&mut batch, 0, "x");
    target(&mut batch, 1, "old/child");
    batch.recipe.collision_policy = CollisionPolicy::Suffix {
        separator: "-".into(),
        max_attempts: 2,
    };
    let len = encode_path(&path("x/child"));
    ns.parents
        .iter_mut()
        .find(|f| f.parent_path == path("old"))
        .unwrap()
        .max_path_units = Some((len.bytes.len() / if cfg!(windows) { 2 } else { 1 }) as u64);
    assert!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked
            > 0
    );
}
// A directed chain must vacate its tail first and expose dependencies between groups.
#[test]
fn acyclic_dependency_groups_are_ordered_and_do_not_need_temporaries() {
    let (inventory, mut batch, ns, version) = fixture(&[
        ("a", EntryKind::File),
        ("b", EntryKind::File),
        ("c", EntryKind::File),
    ]);
    target(&mut batch, 0, "b");
    target(&mut batch, 1, "c");
    target(&mut batch, 2, "d");
    let plan = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(plan.execution.groups.len(), 3);
    assert_eq!(
        plan.execution.groups[1].depends_on,
        vec![plan.execution.groups[0].id]
    );
    assert_eq!(
        plan.execution.groups[2].depends_on,
        vec![plan.execution.groups[1].id]
    );
    assert_eq!(steps(&plan).len(), 3);
    simulate(&inventory, &plan);
}
// The same directory cycle must keep all carried identities correctly located.
#[test]
fn directory_swap_and_child_rename_use_receipts_and_current_locations() {
    let (inventory, mut batch, ns, version) = fixture(&[
        ("a", EntryKind::Directory),
        ("b", EntryKind::Directory),
        ("a/child", EntryKind::File),
        ("b/other", EntryKind::File),
    ]);
    target(&mut batch, 0, "b");
    target(&mut batch, 1, "a");
    target(&mut batch, 2, "a/new");
    let plan = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(plan.counts.blocked, 0);
    assert_eq!(plan.execution.groups.len(), 1);
    let locations = simulate(&inventory, &plan);
    assert_eq!(locations[2].1, path("b/new"));
    assert_eq!(locations[3].1, path("a/other"));
    let step_list = steps(&plan);
    assert!(step_list.iter().any(|s| matches!(
        &s.action,
        ExecutionAction::RenameEntry {
            source: EntryPrestate {
                identity: ExpectedIdentity::FromReceipt { .. },
                ..
            },
            ..
        }
    )));
}
// Unexpected metadata cannot silently disappear from an otherwise valid rename plan.
#[test]
fn unsupported_compiled_effects_fail_closed() {
    let (inventory, mut batch, ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, "b");
    batch
        .recipe
        .metadata_changes
        .push(MetadataChange::Timestamp {
            field: TimestampField::Modified,
            value_ns: 123,
        });
    assert!(plan(&inventory, &batch, &ns, version).is_err());
}

// Windows-only path semantics are compiled here and run only on actual Windows.
#[cfg(windows)]
#[test]
fn windows_ads_devices_trailing_names_and_unc_long_paths() {
    let (inventory, mut batch, mut ns, version) = fixture(&[("a", EntryKind::File)]);
    ns.parents[0].reserved_names = vec!["CON".into(), "NUL".into(), "COM1".into()];
    for name in [
        "new:stream",
        "CON.txt",
        "NUL",
        "COM1.mkv",
        "trailing.",
        "trailing ",
    ] {
        target(&mut batch, 0, name);
        assert_eq!(
            plan(&inventory, &batch, &ns, version)
                .unwrap()
                .counts
                .blocked,
            1,
            "{name}"
        );
    }
    let facts = &ns.parents[0];
    assert!(facts
        .validate_path(Path::new(r"\\.\PhysicalDrive0"))
        .is_err());
    assert!(facts
        .validate_path(Path::new(r"\\?\GLOBALROOT\Device\HarddiskVolume1\name"))
        .is_err());
    assert!(facts
        .validate_path(Path::new(r"C:\folder:stream\name"))
        .is_err());
    let long = Path::new(r"\\?\UNC\server\share")
        .join("deep".repeat(40))
        .join("sub".repeat(40))
        .join("name.mkv");
    assert!(facts.validate_path(&long).is_ok());
}

// A child's temporary receipt must never be relocated by a later ancestor step.
#[test]
fn descendant_first_selection_keeps_receipt_paths_resolvable() {
    let (inventory, mut batch, mut ns, version) = fixture(&[
        ("old/movie", EntryKind::File),
        ("old", EntryKind::Directory),
    ]);
    target(&mut batch, 0, "old/Movie");
    target(&mut batch, 1, "OLD");
    for facts in &mut ns.parents {
        facts.case_sensitivity = CaseSensitivity::Insensitive;
    }
    let plan = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(plan.counts.blocked, 0);
    let mut outputs = std::collections::BTreeMap::new();
    for step in steps(&plan) {
        if let ExecutionAction::RenameEntry { source, target, .. } = &step.action {
            if let ExpectedIdentity::FromReceipt { step_id } = source.identity {
                assert_eq!(
                    outputs[&step_id], source.path,
                    "receipt path was carried after its producing step"
                );
            }
            outputs.insert(step.id, target.clone());
        }
    }
    assert_eq!(simulate(&inventory, &plan)[0].1, path("OLD/Movie"));
}

// Suffixing a directory must not redirect a different move into an existing directory.
#[test]
fn directory_suffix_preserves_unrelated_destination_identity() {
    let (inventory, mut batch, ns, version) = fixture(&[
        ("old", EntryKind::Directory),
        ("elsewhere/file", EntryKind::File),
        ("occupied", EntryKind::Directory),
        ("occupied/keep", EntryKind::File),
    ]);
    batch.rows.truncate(2);
    batch.recipe.selection.truncate(2);
    batch.recipe.numbering_order.truncate(2);
    batch.frozen_inputs.primary_order.truncate(2);
    target(&mut batch, 0, "occupied");
    target(&mut batch, 1, "occupied/new");
    batch.recipe.collision_policy = CollisionPolicy::Suffix {
        separator: "-".into(),
        max_attempts: 2,
    };
    let plan = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(plan.counts.blocked, 0);
    assert_eq!(plan.rows[0].target_path, path("occupied-1"));
    assert_eq!(plan.rows[1].target_path, path("occupied/new"));
    assert_eq!(simulate(&inventory, &plan)[1].1, path("occupied/new"));
}

// Our own observation/content reads may advance atime without changing authority.
#[test]
fn preflight_ignores_only_directory_and_entry_access_time() {
    let (inventory, mut batch, ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, "b");
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    let mut fresh = ns.clone();
    fresh.parents[0].stamp.accessed_ns = Some(500);
    fresh.parents[0].entries[0].stamp.accessed_ns = Some(600);
    fresh.parents[0].window.started_ns += 1;
    assert!(vidlib_rename::validate::preflight(&result.execution, &fresh).is_ok());
    assert_eq!(
        result.execution.expected_namespace.parents[0]
            .stamp
            .accessed_ns,
        None
    );
    assert_eq!(
        result.execution.expected_namespace.parents[0].entries[0]
            .stamp
            .accessed_ns,
        None
    );
}
// The initial inventory-to-namespace source check needs the same material semantics.
#[test]
fn preview_source_access_time_drift_remains_usable() {
    let (inventory, mut batch, mut ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, "b");
    ns.parents[0].entries[0].stamp.accessed_ns = Some(700);
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(result.counts.blocked, 0);
    assert_eq!(steps(&result).len(), 1);
}
// Creation time absent in the expected observation is optional, matching check_stamp.
#[test]
fn newly_available_creation_time_does_not_revoke_preflight() {
    let (inventory, mut batch, ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, "b");
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    let mut fresh = ns.clone();
    fresh.parents[0].stamp.created_ns = Some(900);
    fresh.parents[0].entries[0].stamp.created_ns = Some(900);
    assert!(vidlib_rename::validate::preflight(&result.execution, &fresh).is_ok());
}
// Ignoring atime must not weaken identity, size, mtime, supplied birth time or attributes.
#[test]
fn material_stamp_changes_still_revoke_preflight() {
    let (mut inventory, mut batch, mut ns, version) = fixture(&[("a", EntryKind::File)]);
    target(&mut batch, 0, "b");
    inventory.entries[0].stamp.created_ns = Some(100);
    batch.rows[0].stamp.created_ns = Some(100);
    ns.parents[0].stamp.created_ns = Some(100);
    ns.parents[0].entries[0].stamp.created_ns = Some(100);
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    for directory in [true, false] {
        for change in 0..5 {
            let mut fresh = ns.clone();
            let stamp = if directory {
                &mut fresh.parents[0].stamp
            } else {
                &mut fresh.parents[0].entries[0].stamp
            };
            match change {
                0 => stamp.physical_id.as_mut().unwrap().file_identity.push(42),
                1 => stamp.size = Some(42),
                2 => stamp.modified_ns = Some(42),
                3 => stamp.created_ns = None,
                _ => stamp.windows_attributes = Some(42),
            }
            assert!(
                vidlib_rename::validate::preflight(&result.execution, &fresh).is_err(),
                "directory={directory}, change={change}"
            );
        }
    }
}

// Model namespace writes changing parent directory freshness independently of the
// planner. Both Observed and FromReceipt prestates must remain consumable.
fn simulate_material_prestates(inventory: &Inventory, plan: &RenamePlan) -> Vec<PathBuf> {
    let mut state: Vec<_> = inventory
        .entries
        .iter()
        .map(|e| (e.id, e.absolute_path.clone(), e.kind, e.stamp.clone()))
        .collect();
    let mut receipts = std::collections::BTreeMap::<StepId, (PathBuf, FileStamp)>::new();
    for (ordinal, step) in steps(plan).iter().enumerate() {
        let ExecutionAction::RenameEntry { source, target, .. } = &step.action else {
            panic!("expected pure rename graph");
        };
        let index = state
            .iter()
            .position(|e| e.0 == step.entry_id && e.1 == source.path)
            .expect("current source identity/path");
        assert!(
            !state.iter().any(|e| &e.1 == target),
            "target must be absent"
        );
        let expected = match &source.identity {
            ExpectedIdentity::DirectoryAfterEffects { .. } => {
                panic!("ordinary rename graph cannot refresh copy-directory material")
            }
            ExpectedIdentity::Observed(stamp) => stamp,
            ExpectedIdentity::FromReceipt { step_id } => {
                let (recorded_path, stamp) = &receipts[step_id];
                assert_eq!(
                    recorded_path, &source.path,
                    "receipt path must remain exact"
                );
                stamp
            }
        };
        let actual = &state[index].3;
        assert!(expected.physical_id.is_some());
        assert_eq!(actual.physical_id, expected.physical_id);
        assert_eq!(actual.size, expected.size);
        assert_eq!(
            actual.modified_ns,
            expected.modified_ns,
            "directory prestate invalidated by an earlier child namespace mutation: {}",
            source.path.display()
        );
        if let Some(created) = expected.created_ns {
            assert_eq!(actual.created_ns, Some(created));
        }
        assert_eq!(actual.windows_attributes, expected.windows_attributes);
        // Removing/adding a direct child changes the owning directory's mtime.
        // This remains true while that directory lives at a temporary path.
        for (_, path, kind, stamp) in &mut state {
            if *kind == EntryKind::Directory
                && (Some(path.as_path()) == source.path.parent()
                    || Some(path.as_path()) == target.parent())
            {
                stamp.modified_ns = Some(10_000 + ordinal as i128);
            }
        }
        for (_, path, _, _) in &mut state {
            if let Ok(relative) = path.strip_prefix(&source.path) {
                *path = target.join(relative);
            }
        }
        receipts.insert(step.id, (target.clone(), state[index].3.clone()));
    }
    state.into_iter().map(|e| e.1).collect()
}

// The ready escape must wait through both parking and finalization of A. Moving
// x after A is parked would stale the receipt consumed by A's final rename too.
#[test]
fn ancestor_finishes_before_descendant_escape_changes_its_material_stamp() {
    let (inventory, mut batch, ns, version) = fixture(&[
        ("A", EntryKind::Directory),
        ("B", EntryKind::File),
        ("A/x", EntryKind::File),
    ]);
    target(&mut batch, 0, "B");
    target(&mut batch, 1, "A/x");
    target(&mut batch, 2, "outside");
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(result.counts.blocked, 0);
    assert_eq!(
        simulate_material_prestates(&inventory, &result),
        vec![path("B"), path("B/x"), path("outside")]
    );
    let compiled = steps(&result);
    let ancestor_final = compiled.iter().position(|s| s.entry_id == batch.rows[0].entry_id && matches!(&s.action, ExecutionAction::RenameEntry { target, temporary: false, .. } if target == &path("B"))).unwrap();
    let escape = compiled
        .iter()
        .position(|s| s.entry_id == batch.rows[2].entry_id)
        .unwrap();
    assert!(
        ancestor_final < escape,
        "parking alone does not preserve a consumed receipt's material stamp"
    );
}

// Selection order can choose a different reserved node, but must not weaken the
// material-prestate invariant or change the intended final locations.
#[test]
fn descendant_escape_material_prestates_survive_selection_permutations() {
    for order in [[2, 0, 1], [1, 2, 0], [0, 2, 1]] {
        let (inventory, mut batch, ns, version) = fixture(&[
            ("A", EntryKind::Directory),
            ("B", EntryKind::File),
            ("A/x", EntryKind::File),
        ]);
        target(&mut batch, 0, "B");
        target(&mut batch, 1, "A/x");
        target(&mut batch, 2, "outside");
        batch.rows = order.iter().map(|&i| batch.rows[i].clone()).collect();
        batch.recipe.numbering_order = batch.rows.iter().map(|r| r.entry_id).collect();
        batch.frozen_inputs.primary_order = batch.recipe.numbering_order.clone();
        let result = plan(&inventory, &batch, &ns, version).unwrap();
        assert_eq!(result.counts.blocked, 0);
        assert_eq!(
            simulate_material_prestates(&inventory, &result),
            vec![path("B"), path("B/x"), path("outside")]
        );
    }
}

// Operation-owned cycle paths must be excluded by the ordinary inventory and
// Rename policy, while remaining frozen logical reservations in compiled steps.
#[test]
fn cycle_temps_use_protected_operation_staging_and_cannot_be_selected() {
    let (inventory, mut batch, mut ns, version) = fixture(&[("a", EntryKind::File)]);
    ns.parents[0].case_sensitivity = CaseSensitivity::Insensitive;
    target(&mut batch, 0, "A");
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    let ExecutionAction::RenameEntry {
        target: reserved,
        temporary: true,
        ..
    } = &steps(&result)[0].action
    else {
        panic!("case-only cycle needs a reservation");
    };
    assert!(
        ns.policy.excludes(reserved),
        "normal inventory must exclude operation temps"
    );
    assert!(reserved
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with(".vidlib-staging-rename-"));
    assert!(reserved
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .contains(&batch.rows[0].entry_id.to_string()));
    let mut ordinary = batch.clone();
    ordinary.rows[0].target_path = reserved.clone();
    assert_eq!(
        plan(&inventory, &ordinary, &ns, version)
            .unwrap()
            .counts
            .blocked,
        1
    );
    let (temp_inventory, mut temp_batch, temp_ns, temp_version) = fixture(&[(
        reserved.file_name().unwrap().to_str().unwrap(),
        EntryKind::File,
    )]);
    target(&mut temp_batch, 0, "ordinary");
    assert_eq!(
        plan(&temp_inventory, &temp_batch, &temp_ns, temp_version)
            .unwrap()
            .counts
            .blocked,
        1
    );
}

// Internal staging authorization must not bypass exact marker/root guards or
// additional configured exclusion prefixes when choosing a reserved name.
#[test]
fn internal_temp_reservations_still_honor_marker_root_and_custom_policy() {
    let (inventory, mut batch, mut ns, version) = fixture(&[("a", EntryKind::File)]);
    ns.parents[0].case_sensitivity = CaseSensitivity::Insensitive;
    target(&mut batch, 0, "A");
    let blocked_name = format!(".vidlib-staging-rename-{}-0", batch.rows[0].entry_id);
    ns.policy.marker_name = blocked_name.clone();
    ns.policy.guarded_roots.push(path(&format!(
        ".vidlib-staging-rename-{}-1",
        batch.rows[0].entry_id
    )));
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(result.counts.blocked, 0);
    let ExecutionAction::RenameEntry {
        target,
        temporary: true,
        ..
    } = &steps(&result)[0].action
    else {
        panic!("expected reservation");
    };
    assert_eq!(
        target,
        &path(&format!(
            ".vidlib-staging-rename-{}-2",
            batch.rows[0].entry_id
        ))
    );
    ns.policy
        .staging_prefixes
        .push(".vidlib-staging-rename-".into());
    let blocked = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(blocked.counts.blocked, 1);
    assert!(blocked.execution.groups.is_empty());
    assert!(blocked
        .issues
        .iter()
        .any(|issue| issue.message == "no legal temporary reservation available"));
}

// A directory's own target can fit while an unselected carried child's final
// path exceeds the original child-parent policy. Approval must cover both.
#[test]
fn direct_directory_growth_validates_unselected_carried_child_path() {
    let (inventory, mut batch, mut ns, version) = fixture(&[
        ("old", EntryKind::Directory),
        ("old/child", EntryKind::File),
    ]);
    batch.rows.truncate(1);
    batch.recipe.selection.truncate(1);
    batch.recipe.numbering_order.truncate(1);
    batch.frozen_inputs.primary_order.truncate(1);
    let native = encode_path(&path("old/child"));
    ns.parents
        .iter_mut()
        .find(|facts| facts.parent_path == path("old"))
        .unwrap()
        .max_path_units = Some((native.bytes.len() / if cfg!(windows) { 2 } else { 1 }) as u64);
    target(&mut batch, 0, "new");
    assert_eq!(
        plan(&inventory, &batch, &ns, version)
            .unwrap()
            .counts
            .blocked,
        0
    );
    target(&mut batch, 0, "longer");
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(result.counts.blocked, 1);
    assert_eq!(result.rows[0].status, RowStatus::Blocked);
    assert!(result.execution.groups.is_empty());
    assert!(result
        .issues
        .iter()
        .any(|issue| issue.entry_id == Some(batch.rows[0].entry_id)
            && issue.code == RenameErrorCode::InvalidPath));
}

// Final suffixes can grow the inherited prefix even though the carried child
// has no PlanRow and consumes no numbering ordinal.
#[test]
fn directory_suffix_growth_validates_unselected_carried_child_path() {
    let (inventory, mut batch, mut ns, version) = fixture(&[
        ("old", EntryKind::Directory),
        ("old/child", EntryKind::File),
        ("new", EntryKind::File),
    ]);
    batch.rows.truncate(1);
    batch.recipe.selection.truncate(1);
    batch.recipe.numbering_order.truncate(1);
    batch.frozen_inputs.primary_order.truncate(1);
    let native = encode_path(&path("new/child"));
    ns.parents
        .iter_mut()
        .find(|facts| facts.parent_path == path("old"))
        .unwrap()
        .max_path_units = Some((native.bytes.len() / if cfg!(windows) { 2 } else { 1 }) as u64);
    batch.recipe.collision_policy = CollisionPolicy::Suffix {
        separator: "-".into(),
        max_attempts: 2,
    };
    target(&mut batch, 0, "new");
    let result = plan(&inventory, &batch, &ns, version).unwrap();
    assert_eq!(result.rows[0].target_path, path("new-1"));
    assert_eq!(result.counts.blocked, 1);
    assert_eq!(result.rows[0].status, RowStatus::Blocked);
    assert!(result.execution.groups.is_empty());
}

// Complete inventory coverage is not a substitute for captured native parent
// identity and policy facts used to validate carried descendants' final paths.
#[test]
fn unavailable_carried_parent_facts_block_responsible_directory() {
    for missing in 0..4 {
        let (inventory, mut batch, mut ns, version) = fixture(&[
            ("old", EntryKind::Directory),
            ("old/child", EntryKind::File),
        ]);
        batch.rows.truncate(1);
        batch.recipe.selection.truncate(1);
        batch.recipe.numbering_order.truncate(1);
        batch.frozen_inputs.primary_order.truncate(1);
        target(&mut batch, 0, "new");
        let parent = ns
            .parents
            .iter_mut()
            .find(|facts| facts.parent_path == path("old"))
            .unwrap();
        match missing {
            0 => ns.parents.retain(|facts| facts.parent_path != path("old")),
            1 => parent.stamp.physical_id = None,
            2 => parent.children_complete = false,
            _ => parent.name_policy = NamePolicy::Unknown,
        }
        let result = plan(&inventory, &batch, &ns, version).unwrap();
        assert_eq!(result.counts.blocked, 1, "unavailable fact case {missing}");
        assert!(result.execution.groups.is_empty());
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.entry_id == Some(batch.rows[0].entry_id)
                && issue.code == RenameErrorCode::InvalidNamespace));
    }
}

#[test]
fn cycle_receipt_paths_preserve_exact_native_units_without_trailing_separator() {
    let (inventory, mut batch, namespace, version) =
        fixture(&[("a", EntryKind::File), ("b", EntryKind::File)]);
    target(&mut batch, 0, "b");
    target(&mut batch, 1, "a");
    let planned = plan(&inventory, &batch, &namespace, version).unwrap();
    let steps = steps(&planned);
    for step in &steps {
        if let ExecutionAction::RenameEntry {
            source:
                EntryPrestate {
                    path,
                    identity: ExpectedIdentity::FromReceipt { step_id },
                },
            ..
        } = &step.action
        {
            let creator = steps.iter().find(|s| s.id == *step_id).unwrap();
            let ExecutionAction::RenameEntry { target, .. } = &creator.action else {
                panic!("rename expected")
            };
            assert_eq!(
                encode_path(path),
                encode_path(target),
                "receipt paths must preserve exact native units, including separators"
            );
        }
    }
}
