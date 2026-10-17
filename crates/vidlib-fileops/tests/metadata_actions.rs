#![cfg(any(target_os = "linux", windows))]
use std::{fs, path::PathBuf};
use vidlib_fileops::{
    coordinator::{Coordinator, LeaseKind},
    journal::{Journal, OperationState},
    *,
};
use vidlib_inventory::{ids::*, *};
use vidlib_rename::*;

struct Fixture {
    _dir: tempfile::TempDir,
    media: PathBuf,
    coordinator: Coordinator,
    inventory: Inventory,
    recipe: RecipeV1,
    frozen: FrozenInputs,
}
impl Fixture {
    fn new() -> Self {
        let dir = match std::env::var("VIDLIB_METADATA_ROOT") {
            Ok(root) => tempfile::tempdir_in(root).unwrap(),
            Err(_) => tempfile::tempdir().unwrap(),
        };
        let media = dir.path().join("media");
        fs::create_dir(&media).unwrap();
        fs::write(media.join("a"), b"original bytes").unwrap();
        fs::write(media.join("b"), b"second bytes").unwrap();
        let coordinator = Coordinator::open(&dir.path().join("app")).unwrap();
        let inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(media.clone()).unwrap()]),
            &JobControl::default(),
        )
        .unwrap();
        let ids: Vec<_> = ["b", "a"]
            .iter()
            .map(|name| {
                inventory
                    .entries
                    .iter()
                    .find(|e| e.display_name == *name)
                    .unwrap()
                    .id
            })
            .collect();
        let recipe = RecipeV1 {
            schema_version: 1,
            revision: 1,
            selection: ids.clone(),
            selection_filter: None,
            numbering_order: ids.clone(),
            numbering_scope: Default::default(),
            rules: vec![],
            companions: CompanionPolicy::None,
            destination: Destination::SameDirectory,
            collision_policy: CollisionPolicy::Block,
            metadata_changes: vec![],
        };
        let frozen = FrozenInputs {
            inventory_id: inventory.id,
            generation: inventory.generation,
            now_ns: 0,
            timezone: FrozenTimezone::Utc,
            primary_order: ids,
            numbering_scope: Default::default(),
            values: vec![],
        };
        Self {
            _dir: dir,
            media,
            coordinator,
            inventory,
            recipe,
            frozen,
        }
    }
    fn plan(&self) -> Result<RenamePlan, RenameError> {
        let batch = evaluate(&self.inventory, &self.recipe, &self.frozen)?;
        let namespace = PlatformFs::default()
            .capture_namespace(
                &batch.grants,
                &batch
                    .rows
                    .iter()
                    .map(|r| r.source_path.clone())
                    .chain(
                        self.inventory
                            .coverage
                            .iter()
                            .map(|c| c.directory.join(".capture")),
                    )
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        plan(
            &self.inventory,
            &batch,
            &namespace,
            PlanRef {
                session_id: SessionId::new(),
                plan_id: PlanId::new(),
                revision: 1,
            },
        )
    }
    fn run(&self, plan: &RenamePlan) -> OperationReport {
        let lease = self.coordinator.try_acquire(LeaseKind::Write).unwrap();
        let mut journal = Journal::open(&self.coordinator.database_path()).unwrap();
        execute(
            plan,
            &lease,
            &mut journal,
            &mut PlatformFs {
                namespace_epoch: plan.execution.provenance.namespace_epoch,
                policy_epoch: plan.execution.provenance.policy_epoch,
                ..Default::default()
            },
            &JobControl::default(),
        )
        .unwrap()
    }
}

// A Keep-only recipe must never introduce a field effect or summary.
#[test]
fn all_defaults_leave_metadata_unchanged() {
    let f = Fixture::new();
    let before = PlatformFs::default()
        .inspect(&f.media.join("a"))
        .unwrap()
        .stamp;
    let plan = f.plan().unwrap();
    assert_eq!(plan.counts.changed, 0);
    assert!(plan.execution.groups.is_empty());
    assert!(plan.rows.iter().all(|r| r.explanations.is_empty()));
    assert_eq!(
        PlatformFs::default()
            .inspect(&f.media.join("a"))
            .unwrap()
            .stamp,
        before
    );
}

// Removing metadata compilation must block explicit fields, including paths unchanged.
#[test]
fn each_windows_attribute_and_timestamp_can_be_explicit() {
    for field in [
        TimestampField::Created,
        TimestampField::Modified,
        TimestampField::Accessed,
    ] {
        let mut f = Fixture::new();
        f.recipe.metadata_changes = vec![MetadataChange::Timestamp {
            field,
            value_ns: 1_600_000_000_000_000_000,
        }];
        let result = f.plan();
        assert!(
            result.is_ok(),
            "explicit timestamp must compile: {result:?}"
        );
        let plan = result.unwrap();
        assert_eq!(plan.rows[0].status, RowStatus::Changed);
        assert_eq!(
            plan.execution
                .groups
                .iter()
                .map(|g| g.steps.len())
                .sum::<usize>(),
            2
        );
    }
    for bit in [1, 2, 4, 32] {
        let mut f = Fixture::new();
        for entry in &mut f.inventory.entries {
            entry.stamp.windows_attributes = Some(0x400);
        }
        f.recipe.metadata_changes = vec![MetadataChange::WindowsAttributes { set: bit, clear: 0 }];
        let batch = evaluate(&f.inventory, &f.recipe, &f.frozen);
        assert!(batch.is_ok(), "explicit attribute must evaluate: {batch:?}");
        assert!(batch
            .unwrap()
            .rows
            .iter()
            .all(|r| r.status == RowStatus::Changed));
    }
}

// Literal values derive from b,a primary ordering, never display sorting.
#[test]
fn offset_increment_uses_frozen_item_order() {
    let mut f = Fixture::new();
    let parsed = serde_json::from_value::<MetadataChange>(
        serde_json::json!({"type":"timestamp_increment", "field":"modified", "start_ns":"1600000000000000000", "step_ns":"2000000000"}),
    );
    assert!(
        parsed.is_ok(),
        "explicit increment must be accepted: {parsed:?}"
    );
    f.recipe.metadata_changes = vec![parsed.unwrap()];
    let plan = f.plan().unwrap();
    let report = f.run(&plan);
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    assert_eq!(
        PlatformFs::default()
            .inspect(&f.media.join("b"))
            .unwrap()
            .stamp
            .modified_ns,
        Some(1_600_000_000_000_000_000)
    );
    assert_eq!(
        PlatformFs::default()
            .inspect(&f.media.join("a"))
            .unwrap()
            .stamp
            .modified_ns,
        Some(1_600_000_002_000_000_000)
    );
}

#[test]
fn native_timestamp_set_is_exact() {
    let mut f = Fixture::new();
    f.recipe.metadata_changes = vec![MetadataChange::Timestamp {
        field: TimestampField::Modified,
        value_ns: 1_600_000_000_123_456_700,
    }];
    let result = f.plan();
    assert!(
        result.is_ok(),
        "metadata-only preview must compile: {result:?}"
    );
    let report = f.run(&result.unwrap());
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    assert!(report
        .receipts
        .iter()
        .all(|r| r.after.as_ref().unwrap().modified_ns == Some(1_600_000_000_123_456_700)));
}

#[cfg(target_os = "linux")]
#[path = "support/copy_faults.rs"]
mod copy_faults;

#[cfg(target_os = "linux")]
#[test]
fn partial_set_failure_is_reported_and_recoverable() {
    let mut f = Fixture::new();
    f.recipe.metadata_changes = vec![
        MetadataChange::Timestamp {
            field: TimestampField::Modified,
            value_ns: 1_600_000_000_000_000_000,
        },
        MetadataChange::Timestamp {
            field: TimestampField::Accessed,
            value_ns: 1_600_000_002_000_000_000,
        },
    ];
    let plan = f.plan().unwrap();
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let report = execute(
        &plan,
        &lease,
        &mut journal,
        &mut copy_faults::CopyFs::new("metadata-second-fail"),
        &JobControl::default(),
    )
    .unwrap();
    assert_eq!(
        report.state,
        OperationState::Failed,
        "verified first field must be compensated: {report:?}"
    );
    assert!(report.failure.is_some());
    assert!(journal.pending().unwrap().is_empty());
    for entry in &f.inventory.entries {
        let restored = PlatformFs::default()
            .inspect(&entry.absolute_path)
            .unwrap()
            .stamp;
        assert_eq!(restored.modified_ns, entry.stamp.modified_ns);
        assert_eq!(restored.accessed_ns, entry.stamp.accessed_ns);
    }
}

fn undo_plan(f: &Fixture, op: OperationId) -> Result<RenamePlan, OpError> {
    let lease = f.coordinator.try_acquire(LeaseKind::Read).unwrap();
    let journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let record = journal.operation(op).unwrap();
    let context = vidlib_fileops::undo::capture_context(&record).unwrap();
    let epochs = lease.epochs().unwrap();
    let adapter = PlatformFs {
        namespace_epoch: epochs.namespace,
        policy_epoch: epochs.policy,
        ..Default::default()
    };
    let ns = adapter
        .capture_namespace(&context.grants, &context.candidates)
        .unwrap();
    vidlib_fileops::undo::plan_undo(
        &record,
        &ns,
        PlanRef {
            session_id: SessionId::new(),
            plan_id: PlanId::new(),
            revision: 1,
        },
    )
}

#[test]
fn undo_refuses_changed_poststate() {
    let mut f = Fixture::new();
    f.recipe.metadata_changes = vec![MetadataChange::Timestamp {
        field: TimestampField::Modified,
        value_ns: 1_600_000_000_000_000_000,
    }];
    let report = f.run(&f.plan().unwrap());
    assert_eq!(report.state, OperationState::Completed);
    let undo = undo_plan(&f, report.operation_id.unwrap());
    assert!(
        undo.is_ok(),
        "unchanged metadata must have an inverse: {undo:?}"
    );
    assert_eq!(undo.unwrap().counts.changed, 2);
    fs::write(f.media.join("a"), b"externally changed").unwrap();
    let rejected = undo_plan(&f, report.operation_id.unwrap());
    assert!(rejected.is_err() || rejected.as_ref().unwrap().counts.blocked > 0);
}

impl Fixture {
    fn single(&mut self) {
        let id = self
            .inventory
            .entries
            .iter()
            .find(|e| e.display_name == "a")
            .unwrap()
            .id;
        self.recipe.selection = vec![id];
        self.recipe.numbering_order = vec![id];
        self.frozen.primary_order = vec![id];
    }
    fn modified_and_accessed(&mut self) {
        self.recipe.metadata_changes = vec![
            MetadataChange::Timestamp {
                field: TimestampField::Modified,
                value_ns: 1_600_000_000_000_000_000,
            },
            MetadataChange::Timestamp {
                field: TimestampField::Accessed,
                value_ns: 1_600_000_002_000_000_000,
            },
        ];
    }
    fn renamed(&mut self) {
        self.recipe.rules = vec![Rule {
            id: "rename".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::FixedStem {
                value: "renamed".into(),
            },
        }];
    }
    fn destination(&mut self, dest: &std::path::Path, moving: bool) -> RenamePlan {
        let grant = RootBinding::new(dest.to_owned()).unwrap().source_grant;
        self.recipe.destination = if moving {
            Destination::MoveTo {
                grant_id: grant.id,
                layout: DestinationLayout::PreserveHierarchy,
            }
        } else {
            Destination::CopyTo {
                grant_id: grant.id,
                layout: DestinationLayout::PreserveHierarchy,
            }
        };
        let batch = evaluate(&self.inventory, &self.recipe, &self.frozen).unwrap();
        let mut grants = batch.grants.clone();
        grants.grants.push(grant);
        let batch = resolve_destinations(batch, &self.recipe.destination, &grants).unwrap();
        let candidates = destination::capture_candidates(&batch);
        let ns = PlatformFs::default()
            .capture_namespace(&batch.grants, &candidates)
            .unwrap();
        plan(
            &self.inventory,
            &batch,
            &ns,
            PlanRef {
                session_id: SessionId::new(),
                plan_id: PlanId::new(),
                revision: 1,
            },
        )
        .unwrap()
    }
}

#[test]
fn rename_metadata_undo_restores_original_fields_and_path() {
    let mut f = Fixture::new();
    f.single();
    f.renamed();
    f.modified_and_accessed();
    let original = PlatformFs::default()
        .inspect(&f.media.join("a"))
        .unwrap()
        .stamp;
    let report = f.run(&f.plan().unwrap());
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    let undo = undo_plan(&f, report.operation_id.unwrap()).unwrap();
    let reversed = f.run(&undo);
    assert_eq!(reversed.state, OperationState::Completed, "{reversed:?}");
    assert!(!f.media.join("renamed").exists());
    let after = PlatformFs::default()
        .inspect(&f.media.join("a"))
        .unwrap()
        .stamp;
    assert_eq!(after.modified_ns, original.modified_ns);
    assert_eq!(after.accessed_ns, original.accessed_ns);
}

#[cfg(target_os = "linux")]
#[test]
fn copy_and_cross_volume_move_metadata_undo_use_latest_poststate() {
    for moving in [false, true] {
        let mut f = Fixture::new();
        f.single();
        f.modified_and_accessed();
        let original = PlatformFs::default()
            .inspect(&f.media.join("a"))
            .unwrap()
            .stamp;
        let dest = tempfile::tempdir_in("/dev/shm").unwrap();
        let plan = f.destination(dest.path(), moving);
        let report = f.run(&plan);
        assert_eq!(report.state, OperationState::Completed, "{report:?}");
        let target = &plan.rows[0].target_path;
        assert_eq!(
            PlatformFs::default()
                .inspect(target)
                .unwrap()
                .stamp
                .modified_ns,
            Some(1_600_000_000_000_000_000)
        );
        if !moving {
            assert_eq!(
                PlatformFs::default()
                    .inspect(&f.media.join("a"))
                    .unwrap()
                    .stamp
                    .modified_ns,
                original.modified_ns
            );
        }
        let undo = undo_plan(&f, report.operation_id.unwrap());
        assert!(undo.is_ok(), "metadata copy/move inverse: {undo:?}");
        let reversed = f.run(&undo.unwrap());
        assert_eq!(reversed.state, OperationState::Completed, "{reversed:?}");
        assert!(!target.exists());
        assert_eq!(
            PlatformFs::default()
                .inspect(&f.media.join("a"))
                .unwrap()
                .stamp
                .modified_ns,
            original.modified_ns
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn metadata_crash_child() {
    if std::env::var("VIDLIB_METADATA_ROOT").is_err() {
        return;
    }
    let mut f = Fixture::new();
    let folder = std::env::var("VIDLIB_METADATA_FOLDER").is_ok();
    let native_folder = std::env::var("VIDLIB_METADATA_NATIVE_FOLDER").ok();
    if folder || native_folder.is_some() {
        f.folder();
    } else {
        f.single();
        f.renamed();
    }
    if let Some(kind) = native_folder.as_deref() {
        if kind == "direct" {
            fs::rename(f.media.join("Tree/sub/movie"), f.media.join("Tree/movie")).unwrap();
        }
        f.inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(f.media.clone()).unwrap()]),
            &JobControl::default(),
        )
        .unwrap();
        let child = if kind == "direct" {
            "Tree/movie"
        } else {
            "Tree/sub/movie"
        };
        let ids: Vec<_> = ["Tree", child]
            .iter()
            .map(|p| {
                f.inventory
                    .entries
                    .iter()
                    .find(|e| e.absolute_path == f.media.join(p))
                    .unwrap()
                    .id
            })
            .collect();
        f.recipe.selection = ids.clone();
        f.recipe.numbering_order = ids.clone();
        f.frozen.inventory_id = f.inventory.id;
        f.frozen.generation = f.inventory.generation;
        f.frozen.primary_order = ids;
        f.renamed();
        fs::write(
            f._dir.path().join("original-tree.json"),
            serde_json::to_vec(
                &PlatformFs::default()
                    .inspect(&f.media.join("Tree"))
                    .unwrap()
                    .stamp,
            )
            .unwrap(),
        )
        .unwrap();
    }
    f.modified_and_accessed();
    fs::write(
        f._dir.path().join("original.json"),
        serde_json::to_vec(
            &PlatformFs::default()
                .inspect(&f.media.join("a"))
                .unwrap()
                .stamp,
        )
        .unwrap(),
    )
    .unwrap();
    let plan = if folder {
        let dest = std::path::PathBuf::from(std::env::var("VIDLIB_METADATA_DEST").unwrap());
        f.destination(&dest, true)
    } else {
        f.plan().unwrap()
    };
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    execute(
        &plan,
        &lease,
        &mut journal,
        &mut copy_faults::CopyFs::new(&std::env::var("VIDLIB_METADATA_BOUNDARY").unwrap()),
        &JobControl::default(),
    )
    .unwrap();
    panic!("requested crash boundary not reached");
}

#[cfg(target_os = "linux")]
#[test]
fn metadata_process_crashes_resume_and_restore_each_field_boundary() {
    use vidlib_fileops::recovery::{apply_recovery, reconcile, RecoveryChoice};
    for boundary in [
        "before-metadata",
        "after-metadata",
        "before-second-metadata",
    ] {
        for restore in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "metadata_crash_child", "--nocapture"])
                .env("VIDLIB_METADATA_ROOT", root.path())
                .env("VIDLIB_METADATA_BOUNDARY", boundary)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86));
            let base = fs::read_dir(root.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let coordinator = Coordinator::open(&base.join("app")).unwrap();
            let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
            let mut journal = Journal::open(&coordinator.database_path()).unwrap();
            let pending = journal.pending().unwrap().pop().unwrap();
            let original_facts: Vec<_> = pending
                .steps
                .iter()
                .flat_map(|s| s.receipts.clone())
                .collect();
            let mut adapter = PlatformFs::default();
            let preview = reconcile(&pending, &mut adapter).unwrap();
            let nonce = preview.review_nonce();
            let report = apply_recovery(
                preview.review(nonce).unwrap(),
                if restore {
                    RecoveryChoice::Restore
                } else {
                    RecoveryChoice::Resume
                },
                &lease,
                &mut journal,
                &mut adapter,
                &JobControl::default(),
            )
            .unwrap();
            assert!(
                journal.pending().unwrap().is_empty(),
                "{boundary},restore={restore}: {report:?}"
            );
            let path = base
                .join("media")
                .join(if restore { "a" } else { "renamed" });
            let current = adapter.inspect(&path).unwrap().stamp;
            let original: FileStamp =
                serde_json::from_slice(&fs::read(base.join("original.json")).unwrap()).unwrap();
            assert_eq!(
                current.modified_ns,
                if restore {
                    original.modified_ns
                } else {
                    Some(1_600_000_000_000_000_000)
                }
            );
            assert_eq!(
                current.accessed_ns,
                if restore {
                    original.accessed_ns
                } else {
                    Some(1_600_000_002_000_000_000)
                }
            );
            let retained = journal.operation(pending.operation_id).unwrap();
            for fact in original_facts {
                assert!(retained
                    .steps
                    .iter()
                    .flat_map(|s| &s.receipts)
                    .any(|r| *r == fact));
            }
        }
    }
}

impl Fixture {
    fn folder(&mut self) {
        fs::create_dir_all(self.media.join("Tree/sub")).unwrap();
        fs::write(self.media.join("Tree/sub/movie"), b"folder bytes").unwrap();
        self.inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(self.media.clone()).unwrap()]),
            &JobControl::default(),
        )
        .unwrap();
        let ids: Vec<_> = ["Tree", "Tree/sub/movie"]
            .iter()
            .map(|p| {
                self.inventory
                    .entries
                    .iter()
                    .find(|e| e.absolute_path == self.media.join(p))
                    .unwrap()
                    .id
            })
            .collect();
        self.recipe.selection = ids.clone();
        self.recipe.numbering_order = ids.clone();
        self.frozen.inventory_id = self.inventory.id;
        self.frozen.generation = self.inventory.generation;
        self.frozen.primary_order = ids;
    }
}

#[cfg(target_os = "linux")]
#[test]
fn folder_copy_move_and_undo_keep_directory_proof_with_explicit_metadata() {
    for moving in [false, true] {
        let mut f = Fixture::new();
        f.folder();
        f.modified_and_accessed();
        let original = PlatformFs::default()
            .inspect(&f.media.join("Tree"))
            .unwrap()
            .stamp;
        let dest = tempfile::tempdir_in("/dev/shm").unwrap();
        let plan = f.destination(dest.path(), moving);
        let report = f.run(&plan);
        assert_eq!(
            report.state,
            OperationState::Completed,
            "moving={moving}: {report:?}"
        );
        assert_eq!(
            PlatformFs::default()
                .inspect(&dest.path().join("Tree"))
                .unwrap()
                .stamp
                .modified_ns,
            Some(1_600_000_000_000_000_000)
        );
        let undo = undo_plan(&f, report.operation_id.unwrap());
        assert!(undo.is_ok(), "folder metadata undo: {undo:?}");
        let report = f.run(&undo.unwrap());
        assert_eq!(report.state, OperationState::Completed, "{report:?}");
        assert!(!dest.path().join("Tree").exists());
        assert_eq!(
            PlatformFs::default()
                .inspect(&f.media.join("Tree"))
                .unwrap()
                .stamp
                .modified_ns,
            original.modified_ns
        );
        assert_eq!(
            fs::read(f.media.join("Tree/sub/movie")).unwrap(),
            b"folder bytes"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn unsupported_fields_return_typed_failure_without_effect_or_recovery_gate() {
    for changes in [
        vec![MetadataChange::WindowsAttributes { set: 2, clear: 0 }],
        vec![MetadataChange::Timestamp {
            field: TimestampField::Created,
            value_ns: 1_600_000_000_000_000_000,
        }],
    ] {
        let mut f = Fixture::new();
        f.single();
        f.recipe.metadata_changes = changes;
        let original = PlatformFs::default()
            .inspect(&f.media.join("a"))
            .unwrap()
            .stamp;
        let report = f.run(&f.plan().unwrap());
        assert_eq!(
            report.failure.as_ref().unwrap().code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            report.state,
            OperationState::Failed,
            "non-applied unsupported field must clear recovery gate"
        );
        assert_eq!(
            PlatformFs::default()
                .inspect(&f.media.join("a"))
                .unwrap()
                .stamp,
            original
        );
        assert!(Journal::open(&f.coordinator.database_path())
            .unwrap()
            .pending()
            .unwrap()
            .is_empty());
    }
}

#[test]
fn offset_is_original_based_and_overflow_is_a_row_issue() {
    let mut f = Fixture::new();
    f.recipe.metadata_changes = vec![MetadataChange::TimestampOffset {
        field: TimestampField::Modified,
        offset_ns: -1_000_000_000,
    }];
    let plan = f.plan().unwrap();
    assert_eq!(f.run(&plan).state, OperationState::Completed);
    for e in &f.inventory.entries {
        assert_eq!(
            PlatformFs::default()
                .inspect(&e.absolute_path)
                .unwrap()
                .stamp
                .modified_ns,
            e.stamp.modified_ns.map(|n| n - 1_000_000_000)
        );
    }
    let mut f = Fixture::new();
    f.recipe.metadata_changes = vec![MetadataChange::TimestampIncrement {
        field: TimestampField::Modified,
        start_ns: i128::MAX,
        step_ns: 1,
    }];
    let plan = f.plan().unwrap();
    assert!(plan.counts.blocked > 0);
    assert!(plan.execution.groups.is_empty());
    f.recipe.metadata_changes = vec![MetadataChange::TimestampOffset {
        field: TimestampField::Created,
        offset_ns: 1,
    }];
    for e in &mut f.inventory.entries {
        e.stamp.created_ns = None;
    }
    let batch = evaluate(&f.inventory, &f.recipe, &f.frozen).unwrap();
    assert!(batch.rows.iter().all(|r| r.status == RowStatus::Blocked));
}

#[cfg(target_os = "linux")]
#[test]
fn folder_metadata_crash_restore_and_resume_preserve_complete_lineage() {
    use vidlib_fileops::recovery::{apply_recovery, reconcile, RecoveryChoice};
    for restore in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir_in("/dev/shm").unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "metadata_crash_child", "--nocapture"])
            .env("VIDLIB_METADATA_ROOT", root.path())
            .env("VIDLIB_METADATA_BOUNDARY", "after-metadata-5")
            .env("VIDLIB_METADATA_FOLDER", "1")
            .env("VIDLIB_METADATA_DEST", dest.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));
        let base = fs::read_dir(root.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let coordinator = Coordinator::open(&base.join("app")).unwrap();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        let mut journal = Journal::open(&coordinator.database_path()).unwrap();
        let pending = journal.pending().unwrap().pop().unwrap();
        let mut adapter = PlatformFs::default();
        let preview = reconcile(&pending, &mut adapter).unwrap();
        let gap = pending
            .steps
            .iter()
            .find(|s| s.state == vidlib_fileops::journal::StepState::IntentDurable)
            .unwrap();
        assert_eq!(
            preview
                .actions
                .iter()
                .find(|a| a.step_id == gap.step.id)
                .unwrap()
                .disposition,
            vidlib_fileops::recovery::RecoveryDisposition::Applied,
            "directory setter already ran; refreshed live state must not become its own prestate"
        );
        let nonce = preview.review_nonce();
        let recovered = apply_recovery(
            preview.review(nonce).unwrap(),
            if restore {
                RecoveryChoice::Restore
            } else {
                RecoveryChoice::Resume
            },
            &lease,
            &mut journal,
            &mut adapter,
            &JobControl::default(),
        );
        assert!(recovered.is_ok(), "folder restore={restore}: {recovered:?}");
        assert!(journal.pending().unwrap().is_empty());
        let path = if restore {
            base.join("media/Tree/sub/movie")
        } else {
            dest.path().join("Tree/sub/movie")
        };
        assert_eq!(fs::read(path).unwrap(), b"folder bytes");
        if restore {
            assert!(!dest.path().join("Tree").exists());
        } else {
            assert!(!base.join("media/Tree").exists());
        }
    }
}

#[test]
fn metadata_prestate_migration_failure_keeps_original_facts() {
    let mut f = Fixture::new();
    f.single();
    f.recipe.metadata_changes = vec![MetadataChange::Timestamp {
        field: TimestampField::Modified,
        value_ns: 1_600_000_000_000_000_000,
    }];
    let report = f.run(&f.plan().unwrap());
    let database = f.coordinator.database_path();
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute_batch("DROP TABLE IF EXISTS metadata_prestates; DELETE FROM schema_migrations WHERE version=12; PRAGMA user_version=11; CREATE TRIGGER fail_metadata_migration BEFORE INSERT ON schema_migrations WHEN NEW.version=12 BEGIN SELECT RAISE(ABORT,'migration blocked'); END;").unwrap();
    let facts: String = db
        .query_row("SELECT payload FROM receipts LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert!(
        Journal::open(&database).is_err(),
        "failed metadata migration must roll back"
    );
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert_eq!(
        db.query_row("SELECT payload FROM receipts LIMIT 1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        facts
    );
    db.execute_batch("DROP TRIGGER fail_metadata_migration")
        .unwrap();
    drop(db);
    let journal = Journal::open(&database).unwrap();
    assert_eq!(
        journal
            .operation(report.operation_id.unwrap())
            .unwrap()
            .steps[0]
            .receipts[0],
        report.receipts[0]
    );
}

#[test]
fn failed_metadata_prestate_transaction_never_reaches_native_setter() {
    let mut f = Fixture::new();
    f.single();
    f.modified_and_accessed();
    let plan = f.plan().unwrap();
    let original = PlatformFs::default()
        .inspect(&f.media.join("a"))
        .unwrap()
        .stamp;
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_prestate BEFORE INSERT ON metadata_prestates BEGIN SELECT RAISE(ABORT,'prestate blocked'); END;").unwrap();
    let result = execute(
        &plan,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default(),
    );
    assert!(result.is_err() || result.as_ref().unwrap().state != OperationState::Completed);
    assert_eq!(
        PlatformFs::default()
            .inspect(&f.media.join("a"))
            .unwrap()
            .stamp,
        original
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM metadata_prestates", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM steps WHERE state='IntentDurable'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn metadata_undo_refuses_changed_explicit_access_time() {
    let mut f = Fixture::new();
    f.single();
    f.modified_and_accessed();
    let report = f.run(&f.plan().unwrap());
    let current = PlatformFs::default()
        .inspect(&f.media.join("a"))
        .unwrap()
        .stamp;
    PlatformFs::default()
        .set_metadata(
            &f.media.join("a"),
            &current,
            &vidlib_fileops::platform::MetadataUpdate {
                accessed_ns: Some(1_650_000_000_000_000_000),
                ..Default::default()
            },
        )
        .unwrap();
    let rejected = undo_plan(&f, report.operation_id.unwrap());
    assert!(rejected.is_err() || rejected.as_ref().unwrap().counts.blocked > 0);
}

#[test]
fn native_directory_rename_metadata_undo_restores_after_child_paths() {
    let mut f = Fixture::new();
    f.folder();
    fs::rename(f.media.join("Tree/sub/movie"), f.media.join("Tree/movie")).unwrap();
    f.inventory = scan(
        &InventoryRequest::new(vec![RootBinding::new(f.media.clone()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let ids: Vec<_> = ["Tree", "Tree/movie"]
        .iter()
        .map(|p| {
            f.inventory
                .entries
                .iter()
                .find(|e| e.absolute_path == f.media.join(p))
                .unwrap()
                .id
        })
        .collect();
    f.recipe.selection = ids.clone();
    f.recipe.numbering_order = ids.clone();
    f.frozen.inventory_id = f.inventory.id;
    f.frozen.generation = f.inventory.generation;
    f.frozen.primary_order = ids;
    f.renamed();
    f.modified_and_accessed();
    let original = PlatformFs::default()
        .inspect(&f.media.join("Tree"))
        .unwrap()
        .stamp;
    let report = f.run(&f.plan().unwrap());
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    let inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
    let reversed = f.run(&inverse);
    assert_eq!(reversed.state, OperationState::Completed, "{reversed:?}");
    assert_eq!(
        PlatformFs::default()
            .inspect(&f.media.join("Tree"))
            .unwrap()
            .stamp
            .modified_ns,
        original.modified_ns
    );
    assert_eq!(
        fs::read(f.media.join("Tree/movie")).unwrap(),
        b"folder bytes"
    );
}

#[test]
fn selected_companions_inherit_primary_metadata_increment() {
    let mut f = Fixture::new();
    for name in ["Film.mkv", "Film.en.srt", "Other.mkv", "unselected.nfo"] {
        fs::write(f.media.join(name), name).unwrap();
    }
    f.inventory = scan(
        &InventoryRequest::new(vec![RootBinding::new(f.media.clone()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let id = |name| {
        f.inventory
            .entries
            .iter()
            .find(|e| e.display_name == name)
            .unwrap()
            .id
    };
    let (film, sidecar, other) = (id("Film.mkv"), id("Film.en.srt"), id("Other.mkv"));
    let untouched = PlatformFs::default()
        .inspect(&f.media.join("unselected.nfo"))
        .unwrap()
        .stamp;
    f.recipe.selection = vec![sidecar, film, other];
    f.recipe.numbering_order = vec![other, film];
    f.recipe.companions = CompanionPolicy::Explicit {
        associations: vec![CompanionAssociation {
            primary: film,
            companions: vec![sidecar],
        }],
    };
    f.frozen.inventory_id = f.inventory.id;
    f.frozen.generation = f.inventory.generation;
    f.frozen.primary_order = vec![other, film];
    f.recipe.metadata_changes = vec![MetadataChange::TimestampIncrement {
        field: TimestampField::Modified,
        start_ns: 1_600_000_000_000_000_000,
        step_ns: 2_000_000_000,
    }];
    let plan = f.plan().unwrap();
    assert_eq!(f.run(&plan).state, OperationState::Completed);
    for name in ["Film.mkv", "Film.en.srt"] {
        assert_eq!(
            PlatformFs::default()
                .inspect(&f.media.join(name))
                .unwrap()
                .stamp
                .modified_ns,
            Some(1_600_000_002_000_000_000)
        );
    }
    assert_eq!(
        PlatformFs::default()
            .inspect(&f.media.join("Other.mkv"))
            .unwrap()
            .stamp
            .modified_ns,
        Some(1_600_000_000_000_000_000)
    );
    assert_eq!(
        PlatformFs::default()
            .inspect(&f.media.join("unselected.nfo"))
            .unwrap()
            .stamp,
        untouched
    );
}

#[cfg(target_os = "linux")]
#[test]
fn metadata_restore_crash_child() {
    use vidlib_fileops::recovery::{apply_recovery, reconcile, RecoveryChoice};
    let Ok(base) = std::env::var("VIDLIB_METADATA_RESTORE") else {
        return;
    };
    let base = PathBuf::from(base);
    let coordinator = Coordinator::open(&base.join("app")).unwrap();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let mut journal = Journal::open(&coordinator.database_path()).unwrap();
    let pending = journal.pending().unwrap().pop().unwrap();
    let mut adapter = copy_faults::CopyFs::new(&std::env::var("VIDLIB_METADATA_BOUNDARY").unwrap());
    let preview = reconcile(&pending, &mut adapter).unwrap();
    let nonce = preview.review_nonce();
    apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut adapter,
        &JobControl::default(),
    )
    .unwrap();
    panic!("inverse metadata boundary not reached");
}

#[cfg(target_os = "linux")]
#[test]
fn metadata_inverse_process_crashes_keep_original_facts_and_scope() {
    use vidlib_fileops::recovery::{apply_recovery, reconcile, RecoveryChoice};
    for boundary in ["before-metadata", "after-metadata"] {
        let root = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "metadata_crash_child", "--nocapture"])
            .env("VIDLIB_METADATA_ROOT", root.path())
            .env("VIDLIB_METADATA_BOUNDARY", "before-second-metadata")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));
        let base = fs::read_dir(root.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let db_path = base.join("app/operations.sqlite3");
        let prior = Journal::open(&db_path)
            .unwrap()
            .pending()
            .unwrap()
            .pop()
            .unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "metadata_restore_crash_child", "--nocapture"])
            .env("VIDLIB_METADATA_RESTORE", &base)
            .env("VIDLIB_METADATA_BOUNDARY", boundary)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));
        let coordinator = Coordinator::open(&base.join("app")).unwrap();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        let mut journal = Journal::open(&db_path).unwrap();
        let pending = journal.pending().unwrap().pop().unwrap();
        assert!(pending.restore_context.is_some());
        assert!(pending
            .recovery_effects
            .iter()
            .any(|e| e.inverse.metadata_prestate.is_some()));
        let mut native = PlatformFs::default();
        let preview = reconcile(&pending, &mut native).unwrap();
        let nonce = preview.review_nonce();
        apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Restore,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default(),
        )
        .unwrap();
        assert!(journal.pending().unwrap().is_empty());
        let final_op = journal.operation(prior.operation_id).unwrap();
        for step in prior.steps {
            assert_eq!(
                final_op
                    .steps
                    .iter()
                    .find(|s| s.step.id == step.step.id)
                    .unwrap()
                    .receipts,
                step.receipts
            );
        }
        let original: FileStamp =
            serde_json::from_slice(&fs::read(base.join("original.json")).unwrap()).unwrap();
        let current = native.inspect(&base.join("media/a")).unwrap().stamp;
        assert_eq!(current.modified_ns, original.modified_ns);
        assert_eq!(current.accessed_ns, original.accessed_ns);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn copy_undo_refuses_changed_explicit_access_time() {
    let mut f = Fixture::new();
    f.single();
    f.modified_and_accessed();
    let dest = tempfile::tempdir_in("/dev/shm").unwrap();
    let plan = f.destination(dest.path(), false);
    let report = f.run(&plan);
    assert_eq!(report.state, OperationState::Completed);
    let path = dest.path().join("a");
    let current = PlatformFs::default().inspect(&path).unwrap().stamp;
    PlatformFs::default()
        .set_metadata(
            &path,
            &current,
            &vidlib_fileops::platform::MetadataUpdate {
                accessed_ns: Some(1_650_000_000_000_000_000),
                ..Default::default()
            },
        )
        .unwrap();
    let inverse = undo_plan(&f, report.operation_id.unwrap());
    assert!(
        inverse.is_err() || inverse.as_ref().unwrap().counts.blocked > 0,
        "external explicit poststate change must refuse owned-copy deletion"
    );
}

#[test]
fn directory_explicit_original_set_survives_child_rename_and_undo() {
    for direct in [false, true] {
        let mut f = Fixture::new();
        f.folder();
        if direct {
            fs::rename(f.media.join("Tree/sub/movie"), f.media.join("Tree/movie")).unwrap();
        }
        f.inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(f.media.clone()).unwrap()]),
            &JobControl::default(),
        )
        .unwrap();
        let child = if direct {
            "Tree/movie"
        } else {
            "Tree/sub/movie"
        };
        let ids: Vec<_> = ["Tree", child]
            .iter()
            .map(|p| {
                f.inventory
                    .entries
                    .iter()
                    .find(|e| e.absolute_path == f.media.join(p))
                    .unwrap()
                    .id
            })
            .collect();
        f.recipe.selection = ids.clone();
        f.recipe.numbering_order = ids.clone();
        f.frozen.inventory_id = f.inventory.id;
        f.frozen.generation = f.inventory.generation;
        f.frozen.primary_order = ids;
        f.renamed();
        let original = PlatformFs::default()
            .inspect(&f.media.join("Tree"))
            .unwrap()
            .stamp;
        f.recipe.metadata_changes = vec![MetadataChange::Timestamp {
            field: TimestampField::Modified,
            value_ns: original.modified_ns.unwrap(),
        }];
        let plan = f.plan().unwrap();
        assert!(plan.execution.groups.iter().flat_map(|g| &g.steps).any(|s| matches!(&s.action, ExecutionAction::SetMetadata {entry,..} if entry.path == f.media.join("renamed"))));
        let report = f.run(&plan);
        assert_eq!(report.state, OperationState::Completed);
        assert_eq!(
            PlatformFs::default()
                .inspect(&f.media.join("renamed"))
                .unwrap()
                .stamp
                .modified_ns,
            original.modified_ns
        );
        let inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
        assert_eq!(f.run(&inverse).state, OperationState::Completed);
        assert_eq!(
            PlatformFs::default()
                .inspect(&f.media.join("Tree"))
                .unwrap()
                .stamp
                .modified_ns,
            original.modified_ns
        );
        assert_eq!(fs::read(f.media.join(child)).unwrap(), b"folder bytes");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn native_directory_metadata_crash_resume_restore_direct_and_nested() {
    use vidlib_fileops::recovery::{apply_recovery, reconcile, RecoveryChoice};
    for kind in ["direct", "nested"] {
        for restore in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "metadata_crash_child", "--nocapture"])
                .env("VIDLIB_METADATA_ROOT", root.path())
                .env("VIDLIB_METADATA_NATIVE_FOLDER", kind)
                .env("VIDLIB_METADATA_BOUNDARY", "after-metadata-3")
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86));
            let base = fs::read_dir(root.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let coordinator = Coordinator::open(&base.join("app")).unwrap();
            let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
            let mut journal = Journal::open(&coordinator.database_path()).unwrap();
            let pending = journal.pending().unwrap().pop().unwrap();
            let mut adapter = PlatformFs::default();
            let preview = reconcile(&pending, &mut adapter).unwrap();
            let nonce = preview.review_nonce();
            let report = apply_recovery(
                preview.review(nonce).unwrap(),
                if restore {
                    RecoveryChoice::Restore
                } else {
                    RecoveryChoice::Resume
                },
                &lease,
                &mut journal,
                &mut adapter,
                &JobControl::default(),
            );
            assert!(report.is_ok(), "{kind}, restore={restore}: {report:?}");
            assert!(
                journal.pending().unwrap().is_empty(),
                "{kind}, restore={restore}: {report:?}"
            );
            let tree = base
                .join("media")
                .join(if restore { "Tree" } else { "renamed" });
            let original: FileStamp =
                serde_json::from_slice(&fs::read(base.join("original-tree.json")).unwrap())
                    .unwrap();
            assert_eq!(
                adapter.inspect(&tree).unwrap().stamp.modified_ns,
                if restore {
                    original.modified_ns
                } else {
                    Some(1_600_000_000_000_000_000)
                }
            );
            let child = if restore { "movie" } else { "renamed" };
            assert_eq!(
                fs::read(if kind == "direct" {
                    tree.join(child)
                } else {
                    tree.join("sub").join(child)
                })
                .unwrap(),
                b"folder bytes"
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn metadata_undo_crash_child() {
    if std::env::var("VIDLIB_METADATA_UNDO").is_err() {
        return;
    }
    let mut f = Fixture::new();
    f.single();
    f.modified_and_accessed();
    let original = PlatformFs::default()
        .inspect(&f.media.join("a"))
        .unwrap()
        .stamp;
    fs::write(
        f._dir.path().join("original.json"),
        serde_json::to_vec(&original).unwrap(),
    )
    .unwrap();
    let forward = if std::env::var("VIDLIB_METADATA_COPY_UNDO").is_ok() {
        let dest = f._dir.path().join("destination");
        fs::create_dir(&dest).unwrap();
        f.destination(&dest, false)
    } else {
        f.plan().unwrap()
    };
    let report = f.run(&forward);
    assert_eq!(report.state, OperationState::Completed);
    let inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let mut adapter = copy_faults::CopyFs::new(&std::env::var("VIDLIB_METADATA_BOUNDARY").unwrap());
    adapter.native.namespace_epoch = inverse.execution.provenance.namespace_epoch;
    adapter.native.policy_epoch = inverse.execution.provenance.policy_epoch;
    execute(
        &inverse,
        &lease,
        &mut journal,
        &mut adapter,
        &JobControl::default(),
    )
    .unwrap();
    panic!("Undo metadata boundary not reached");
}

#[cfg(target_os = "linux")]
#[test]
fn metadata_undo_process_crashes_resume_or_restore_immutable_history() {
    use vidlib_fileops::recovery::{apply_recovery, reconcile, RecoveryChoice};
    for boundary in [
        "before-metadata",
        "after-metadata",
        "before-second-metadata",
    ] {
        for restore in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "metadata_undo_crash_child", "--nocapture"])
                .env("VIDLIB_METADATA_ROOT", root.path())
                .env("VIDLIB_METADATA_UNDO", "1")
                .env("VIDLIB_METADATA_BOUNDARY", boundary)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86));
            let base = fs::read_dir(root.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let coordinator = Coordinator::open(&base.join("app")).unwrap();
            let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
            let mut journal = Journal::open(&coordinator.database_path()).unwrap();
            let pending = journal.pending().unwrap().pop().unwrap();
            let original_id = pending.inverse_of.unwrap();
            let original_facts: Vec<_> = journal
                .operation(original_id)
                .unwrap()
                .steps
                .iter()
                .flat_map(|s| s.receipts.clone())
                .collect();
            let mut native = PlatformFs::default();
            let preview = reconcile(&pending, &mut native).unwrap();
            let nonce = preview.review_nonce();
            let result = apply_recovery(
                preview.review(nonce).unwrap(),
                if restore {
                    RecoveryChoice::Restore
                } else {
                    RecoveryChoice::Resume
                },
                &lease,
                &mut journal,
                &mut native,
                &JobControl::default(),
            );
            assert!(result.is_ok(), "{boundary}, restore={restore}: {result:?}");
            assert!(journal.pending().unwrap().is_empty());
            assert_eq!(
                journal
                    .operation(original_id)
                    .unwrap()
                    .steps
                    .iter()
                    .flat_map(|s| s.receipts.clone())
                    .collect::<Vec<_>>(),
                original_facts
            );
            let original: FileStamp =
                serde_json::from_slice(&fs::read(base.join("original.json")).unwrap()).unwrap();
            let current = native.inspect(&base.join("media/a")).unwrap().stamp;
            assert_eq!(
                current.modified_ns,
                if restore {
                    Some(1_600_000_000_000_000_000)
                } else {
                    original.modified_ns
                }
            );
            assert_eq!(
                current.accessed_ns,
                if restore {
                    Some(1_600_000_002_000_000_000)
                } else {
                    original.accessed_ns
                }
            );
        }
    }
}

#[test]
fn same_volume_folder_move_metadata_undo_restores_original() {
    let mut f = Fixture::new();
    f.folder();
    f.modified_and_accessed();
    let original = PlatformFs::default()
        .inspect(&f.media.join("Tree"))
        .unwrap()
        .stamp;
    let dest = tempfile::tempdir().unwrap();
    let plan = f.destination(dest.path(), true);
    let report = f.run(&plan);
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    let inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
    let reversed = f.run(&inverse);
    assert_eq!(reversed.state, OperationState::Completed, "{reversed:?}");
    assert_eq!(
        PlatformFs::default()
            .inspect(&f.media.join("Tree"))
            .unwrap()
            .stamp
            .modified_ns,
        original.modified_ns
    );
    assert_eq!(
        fs::read(f.media.join("Tree/sub/movie")).unwrap(),
        b"folder bytes"
    );
}

#[test]
fn carried_selected_file_metadata_shares_ancestor_compensation_group() {
    let mut f = Fixture::new();
    f.folder();
    f.modified_and_accessed();
    f.recipe.rules = vec![Rule {
        id: "parent-only".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::Text {
            operation: TextOperation::Replace {
                replacements: vec![LiteralReplacement {
                    from: "Tree".into(),
                    to: "RenamedTree".into(),
                }],
                scope: MatchScope::All,
                case_sensitive: true,
            },
        },
    }];
    let plan = f.plan().unwrap();
    assert_eq!(
        plan.execution.groups.len(),
        1,
        "carried metadata must compensate with ancestor path effects"
    );
    let report = f.run(&plan);
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    assert_eq!(
        PlatformFs::default()
            .inspect(&f.media.join("RenamedTree/sub/movie"))
            .unwrap()
            .stamp
            .modified_ns,
        Some(1_600_000_000_000_000_000)
    );
    let inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
    assert_eq!(f.run(&inverse).state, OperationState::Completed);
    assert_eq!(
        fs::read(f.media.join("Tree/sub/movie")).unwrap(),
        b"folder bytes"
    );
}

// This is a portable observation fault, never Windows native setter evidence.
#[cfg(target_os = "linux")]
#[test]
fn directory_created_field_gap_accepts_only_requested_poststate() {
    use vidlib_fileops::recovery::{reconcile, RecoveryDisposition};
    let mut f = Fixture::new();
    f.folder();
    let id = f.recipe.selection[0];
    f.recipe.selection = vec![id];
    f.recipe.numbering_order = vec![id];
    f.frozen.primary_order = vec![id];
    f.recipe.metadata_changes = vec![MetadataChange::Timestamp {
        field: TimestampField::Created,
        value_ns: 1_600_000_000_000_000_000,
    }];
    let plan = f.plan().unwrap();
    let step = &plan.execution.groups[0].steps[0];
    let original = PlatformFs::default()
        .inspect(&f.media.join("Tree"))
        .unwrap()
        .stamp;
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let op = journal.begin(&plan.execution).unwrap();
    journal.intent_metadata(op, step, &original).unwrap();
    let pending = journal.operation(op).unwrap();
    let mut post = original.clone();
    post.created_ns = Some(1_600_000_000_000_000_000);
    let mut adapter = copy_faults::CopyFs::new("");
    adapter.metadata_observation = Some((f.media.join("Tree"), post.clone()));
    let review = reconcile(&pending, &mut adapter).unwrap();
    assert_eq!(
        review.actions[0].disposition,
        RecoveryDisposition::Applied,
        "exact created-time gap retains directory proof"
    );
    post.windows_attributes = Some(2);
    adapter.metadata_observation = Some((f.media.join("Tree"), post));
    let rejected = reconcile(&pending, &mut adapter).unwrap();
    assert_eq!(
        rejected.actions[0].disposition,
        RecoveryDisposition::Unresolved,
        "unrequested attributes remain blockers"
    );
    assert_eq!(
        journal.operation(op).unwrap().steps[0].metadata_prestate,
        Some(original)
    );
}

// Authored qualification only: this is not executed by Linux fault fixtures.
#[cfg(windows)]
#[test]
fn windows_metadata_execution_and_undo_restore_each_explicit_field() {
    for bit in [1, 2, 4, 32] {
        let mut f = Fixture::new();
        f.single();
        let original = PlatformFs::default()
            .inspect(&f.media.join("a"))
            .unwrap()
            .stamp;
        let initially_set = original.windows_attributes.unwrap() & bit != 0;
        f.recipe.metadata_changes = vec![MetadataChange::WindowsAttributes {
            set: if initially_set { 0 } else { bit },
            clear: if initially_set { bit } else { 0 },
        }];
        let report = f.run(&f.plan().unwrap());
        assert_eq!(report.state, OperationState::Completed, "{report:?}");
        assert_ne!(
            PlatformFs::default()
                .inspect(&f.media.join("a"))
                .unwrap()
                .stamp
                .windows_attributes
                .unwrap()
                & bit
                != 0,
            initially_set
        );
        let inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
        assert_eq!(f.run(&inverse).state, OperationState::Completed);
        assert_eq!(
            PlatformFs::default()
                .inspect(&f.media.join("a"))
                .unwrap()
                .stamp
                .windows_attributes,
            original.windows_attributes
        );
    }
    for field in [
        TimestampField::Created,
        TimestampField::Modified,
        TimestampField::Accessed,
    ] {
        let mut f = Fixture::new();
        f.single();
        let original = PlatformFs::default()
            .inspect(&f.media.join("a"))
            .unwrap()
            .stamp;
        f.recipe.metadata_changes = vec![MetadataChange::Timestamp {
            field,
            value_ns: 1_600_000_000_123_456_700,
        }];
        let report = f.run(&f.plan().unwrap());
        assert_eq!(report.state, OperationState::Completed, "{report:?}");
        let inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
        assert_eq!(f.run(&inverse).state, OperationState::Completed);
        assert_eq!(
            vidlib_rename::metadata::timestamp(
                &PlatformFs::default()
                    .inspect(&f.media.join("a"))
                    .unwrap()
                    .stamp,
                field
            ),
            vidlib_rename::metadata::timestamp(&original, field)
        );
    }
}

#[test]
fn directory_undo_refuses_external_controlled_timestamps() {
    directory_controlled_timestamp_refusal(false);
}
#[test]
fn directory_child_undo_refuses_external_modified_time() {
    directory_controlled_timestamp_refusal(true);
}
fn directory_controlled_timestamp_refusal(with_child: bool) {
    {
        let mut f = Fixture::new();
        f.folder();
        if !with_child {
            let id = f.recipe.selection[0];
            f.recipe.selection = vec![id];
            f.recipe.numbering_order = vec![id];
            f.frozen.primary_order = vec![id];
        }
        if with_child {
            f.renamed();
        }
        let field = if with_child {
            TimestampField::Modified
        } else {
            TimestampField::Accessed
        };
        f.recipe.metadata_changes = vec![MetadataChange::Timestamp {
            field,
            value_ns: 1_600_000_000_000_000_000,
        }];
        let report = f.run(&f.plan().unwrap());
        assert_eq!(report.state, OperationState::Completed);
        let path = f.media.join(if with_child { "renamed" } else { "Tree" });
        let current = PlatformFs::default().inspect(&path).unwrap().stamp;
        let external = 1_650_000_000_000_000_000;
        PlatformFs::default()
            .set_metadata(
                &path,
                &current,
                &vidlib_fileops::platform::MetadataUpdate {
                    modified_ns: with_child.then_some(external),
                    accessed_ns: (!with_child).then_some(external),
                    ..Default::default()
                },
            )
            .unwrap();
        let inverse = undo_plan(&f, report.operation_id.unwrap());
        assert!(
            inverse.is_err() || inverse.as_ref().unwrap().counts.blocked > 0,
            "controlled directory field adopted: child={with_child}"
        );
        assert_eq!(
            vidlib_rename::metadata::timestamp(
                &PlatformFs::default().inspect(&path).unwrap().stamp,
                field
            ),
            Some(external)
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn copy_undo_rechecks_controlled_access_time_after_preview() {
    let mut f = Fixture::new();
    f.single();
    f.modified_and_accessed();
    let dest = tempfile::tempdir_in("/dev/shm").unwrap();
    let plan = f.destination(dest.path(), false);
    let report = f.run(&plan);
    assert_eq!(report.state, OperationState::Completed);
    let inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
    let path = dest.path().join("a");
    let expected = PlatformFs::default().inspect(&path).unwrap().stamp;
    let external = 1_650_000_000_000_000_000;
    PlatformFs::default()
        .set_metadata(
            &path,
            &expected,
            &vidlib_fileops::platform::MetadataUpdate {
                accessed_ns: Some(external),
                ..Default::default()
            },
        )
        .unwrap();
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let mut adapter = PlatformFs {
        namespace_epoch: inverse.execution.provenance.namespace_epoch,
        policy_epoch: inverse.execution.provenance.policy_epoch,
        ..Default::default()
    };
    let outcome = execute(
        &inverse,
        &lease,
        &mut journal,
        &mut adapter,
        &JobControl::default(),
    );
    assert!(
        outcome.is_err() || outcome.as_ref().unwrap().state != OperationState::Completed,
        "copy removed despite changed controlled poststate"
    );
    assert!(path.exists());
    assert_eq!(
        adapter.inspect(&path).unwrap().stamp.accessed_ns,
        Some(external)
    );
}

#[test]
fn native_directory_move_into_missing_parent_undo_restores_metadata() {
    let mut f = Fixture::new();
    f.folder();
    f.modified_and_accessed();
    let id = f.recipe.selection[0];
    f.recipe.selection = vec![id];
    f.recipe.numbering_order = vec![id];
    f.frozen.primary_order = vec![id];
    let original = f
        .inventory
        .entries
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .stamp
        .clone();
    let dest = tempfile::tempdir().unwrap();
    let grant = RootBinding::new(dest.path().to_owned())
        .unwrap()
        .source_grant;
    f.recipe.destination = Destination::MoveTo {
        grant_id: grant.id,
        layout: DestinationLayout::FolderTemplate {
            parts: vec![TemplatePart::Literal("new-parent".into())],
        },
    };
    let batch = evaluate(&f.inventory, &f.recipe, &f.frozen).unwrap();
    let mut grants = batch.grants.clone();
    grants.grants.push(grant);
    let batch = resolve_destinations(batch, &f.recipe.destination, &grants).unwrap();
    let ns = PlatformFs::default()
        .capture_namespace(&batch.grants, &destination::capture_candidates(&batch))
        .unwrap();
    let plan = plan(
        &f.inventory,
        &batch,
        &ns,
        PlanRef {
            session_id: SessionId::new(),
            plan_id: PlanId::new(),
            revision: 1,
        },
    )
    .unwrap();
    assert!(plan
        .execution
        .groups
        .iter()
        .flat_map(|g| &g.steps)
        .any(|s| matches!(s.action, ExecutionAction::CreateDirectory { .. })));
    assert!(plan
        .execution
        .groups
        .iter()
        .flat_map(|g| &g.steps)
        .any(|s| matches!(s.action, ExecutionAction::RenameEntry { .. })));
    let report = f.run(&plan);
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    let inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
    let reversed = f.run(&inverse);
    assert_eq!(reversed.state, OperationState::Completed, "{reversed:?}");
    let restored = PlatformFs::default()
        .inspect(&f.media.join("Tree"))
        .unwrap()
        .stamp;
    assert_eq!(
        (restored.modified_ns, restored.accessed_ns),
        (original.modified_ns, original.accessed_ns)
    );
    assert!(
        !dest.path().join("new-parent").exists(),
        "owned empty parent must be cleaned"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn copy_undo_field_authority_survives_native_boundaries_and_canonical_validation() {
    for boundary in [
        "change-atime-before-rename",
        "change-atime-before-cleanup",
        "change-atime-after-rename",
        "strip-authority",
    ] {
        let mut f = Fixture::new();
        f.single();
        f.modified_and_accessed();
        let dest = tempfile::tempdir().unwrap();
        let forward = f.destination(dest.path(), false);
        let report = f.run(&forward);
        assert_eq!(report.state, OperationState::Completed);
        let mut inverse = undo_plan(&f, report.operation_id.unwrap()).unwrap();
        if boundary == "strip-authority" {
            for step in inverse
                .execution
                .groups
                .iter_mut()
                .flat_map(|g| &mut g.steps)
            {
                if let ExecutionAction::RenameEntry { source, .. } = &mut step.action {
                    if let ExpectedIdentity::MetadataChecked { stamp, .. } = &source.identity {
                        source.identity = ExpectedIdentity::Observed(stamp.clone());
                    }
                }
            }
        }
        let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
        let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
        let mut adapter = copy_faults::CopyFs::new(boundary);
        adapter.native.namespace_epoch = inverse.execution.provenance.namespace_epoch;
        adapter.native.policy_epoch = inverse.execution.provenance.policy_epoch;
        let result = execute(
            &inverse,
            &lease,
            &mut journal,
            &mut adapter,
            &JobControl::default(),
        );
        assert!(
            result.is_err() || result.as_ref().unwrap().state != OperationState::Completed,
            "{boundary}: {result:?}"
        );
        let survivors: Vec<_> = fs::read_dir(dest.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(survivors.len(), 1, "copy must survive {boundary}");
        if boundary != "strip-authority" {
            assert_eq!(
                adapter.inspect(&survivors[0]).unwrap().stamp.accessed_ns,
                Some(1_650_000_000_000_000_000)
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn copy_undo_field_authority_survives_crash_reopen_and_recovery() {
    use vidlib_fileops::recovery::{
        apply_recovery, reconcile, RecoveryChoice, RecoveryDisposition,
    };
    for boundary in ["before-rename", "after-rename"] {
        for changed in [false, true] {
            for restore in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "metadata_undo_crash_child", "--nocapture"])
                    .env("VIDLIB_METADATA_ROOT", root.path())
                    .env("VIDLIB_METADATA_UNDO", "1")
                    .env("VIDLIB_METADATA_COPY_UNDO", "1")
                    .env("VIDLIB_METADATA_BOUNDARY", boundary)
                    .status()
                    .unwrap();
                assert_eq!(status.code(), Some(86));
                let base = fs::read_dir(root.path())
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                let coordinator = Coordinator::open(&base.join("app")).unwrap();
                let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
                let mut journal = Journal::open(&coordinator.database_path()).unwrap();
                let pending = journal.pending().unwrap().pop().unwrap();
                assert!(pending.steps.iter().any(|s| matches!(&s.step.action, ExecutionAction::RenameEntry { source: EntryPrestate { identity: ExpectedIdentity::MetadataChecked { fields, .. }, .. }, .. } if fields.contains(&TimestampField::Accessed))));
                let survivor = fs::read_dir(base.join("destination"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                let mut native = PlatformFs::default();
                if changed {
                    fs::File::open(&survivor)
                        .unwrap()
                        .set_times(fs::FileTimes::new().set_accessed(
                            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_650_000_000),
                        ))
                        .unwrap();
                }
                let preview = reconcile(&pending, &mut native).unwrap();
                if changed {
                    assert!(preview
                        .actions
                        .iter()
                        .any(|a| a.disposition == RecoveryDisposition::Unresolved));
                }
                let nonce = preview.review_nonce();
                let result = apply_recovery(
                    preview.review(nonce).unwrap(),
                    if restore {
                        RecoveryChoice::Restore
                    } else {
                        RecoveryChoice::Resume
                    },
                    &lease,
                    &mut journal,
                    &mut native,
                    &JobControl::default(),
                );
                if changed {
                    assert!(result.is_err());
                    assert!(survivor.exists());
                    assert_eq!(
                        native.inspect(&survivor).unwrap().stamp.accessed_ns,
                        Some(1_650_000_000_000_000_000)
                    );
                } else {
                    assert!(result.is_ok(), "{boundary},restore={restore}: {result:?}");
                    assert!(journal.pending().unwrap().is_empty());
                    assert_eq!(base.join("destination/a").exists(), restore);
                }
            }
        }
    }
}
