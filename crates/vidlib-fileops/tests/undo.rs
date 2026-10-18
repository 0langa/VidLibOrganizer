#![cfg(target_os = "linux")]
#[path = "support/legacy_schema.rs"]
mod legacy_schema;
use std::{fs, path::PathBuf};
use vidlib_fileops::{
    coordinator::{Coordinator, LeaseKind},
    journal::{Journal, OperationRecord, OperationState},
    recovery::{apply_recovery, reconcile, RecoveryChoice},
    undo::plan_undo,
    *,
};
use vidlib_inventory::{ids::*, *};
use vidlib_rename::*;

struct Fixture {
    _dir: tempfile::TempDir,
    media: PathBuf,
    coordinator: Coordinator,
    plan: RenamePlan,
}
impl Fixture {
    fn new(names: &[(&str, &str)]) -> Self {
        let dir = match std::env::var("VIDLIB_UNDO_ROOT") {
            Ok(root) => tempfile::tempdir_in(root).unwrap(),
            Err(_) => tempfile::tempdir().unwrap(),
        };
        let media = dir.path().join("media");
        fs::create_dir(&media).unwrap();
        for (name, _) in names {
            if names
                .iter()
                .any(|(other, _)| other.starts_with(&format!("{name}/")))
            {
                fs::create_dir_all(media.join(name)).unwrap();
            } else {
                fs::create_dir_all(media.join(name).parent().unwrap()).unwrap();
                fs::write(media.join(name), name.as_bytes()).unwrap();
            }
        }
        let coordinator = Coordinator::open(&dir.path().join("app")).unwrap();
        let inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(media.clone()).unwrap()]),
            &JobControl::default(),
        )
        .unwrap();
        let selection: Vec<_> = names
            .iter()
            .map(|(name, _)| {
                inventory
                    .entries
                    .iter()
                    .find(|e| e.absolute_path == media.join(name))
                    .unwrap()
                    .id
            })
            .collect();
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
            generation: inventory.generation,
            now_ns: 0,
            timezone: FrozenTimezone::Utc,
            primary_order: selection,
            numbering_scope: Default::default(),
            values: vec![],
        };
        let mut batch = evaluate(&inventory, &recipe, &frozen).unwrap();
        for row in &mut batch.rows {
            row.target_path = media.join(
                names
                    .iter()
                    .find(|(name, _)| row.source_path == media.join(name))
                    .unwrap()
                    .1,
            );
            row.status = RowStatus::Changed;
        }
        let ns = PlatformFs::default()
            .capture_namespace(
                &batch.grants,
                &batch
                    .rows
                    .iter()
                    .map(|r| r.source_path.clone())
                    .chain(
                        inventory
                            .coverage
                            .iter()
                            .map(|c| c.directory.join("capture")),
                    )
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let plan = plan(&inventory, &batch, &ns, version()).unwrap();
        Self {
            _dir: dir,
            media,
            coordinator,
            plan,
        }
    }
    fn journal(&self) -> Journal {
        Journal::open(&self.coordinator.database_path()).unwrap()
    }
    fn complete(&self) -> OperationRecord {
        let lease = self.coordinator.try_acquire(LeaseKind::Write).unwrap();
        let mut journal = self.journal();
        let report = execute(
            &self.plan,
            &lease,
            &mut journal,
            &mut PlatformFs::default(),
            &JobControl::default(),
        )
        .unwrap();
        assert_eq!(report.state, OperationState::Completed);
        journal.operation(report.operation_id.unwrap()).unwrap()
    }
    fn namespace(&self) -> NamespaceSnapshot {
        let inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(self.media.clone()).unwrap()]),
            &JobControl::default(),
        )
        .unwrap();
        let mut ns = PlatformFs::default()
            .capture_namespace(
                &self.plan.grants,
                &inventory
                    .coverage
                    .iter()
                    .map(|c| c.directory.join("capture"))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let lease = self.coordinator.try_acquire(LeaseKind::Read).unwrap();
        let epochs = lease.status().unwrap().epochs;
        ns.namespace_epoch = epochs.namespace;
        ns.policy_epoch = epochs.policy;
        ns
    }
}
fn version() -> PlanRef {
    PlanRef {
        session_id: SessionId::new(),
        plan_id: PlanId::new(),
        revision: 1,
    }
}
#[test]
fn undo_requires_current_poststate() {
    let f = Fixture::new(&[("a", "b")]);
    let op = f.complete();
    fs::write(f.media.join("b"), b"changed externally").unwrap();
    let result = plan_undo(&op, &f.namespace(), version());
    assert!(
        result.is_ok(),
        "undo must produce a reviewable blocked plan: {result:?}"
    );
    assert_eq!(result.unwrap().rows[0].status, RowStatus::Blocked);
}
#[test]
fn recreated_original_blocks_inverse() {
    let f = Fixture::new(&[("a", "b")]);
    let op = f.complete();
    fs::write(f.media.join("a"), b"unrelated").unwrap();
    let inverse = plan_undo(&op, &f.namespace(), version());
    assert!(inverse.is_ok(), "collision must be reviewable: {inverse:?}");
    assert_eq!(inverse.unwrap().rows[0].status, RowStatus::Blocked);
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"unrelated");
}
#[test]
fn interrupted_undo_is_recoverable() {
    let f = Fixture::new(&[("a", "b")]);
    let original = f.complete();
    let inverse = plan_undo(&original, &f.namespace(), version());
    assert!(
        inverse.is_ok(),
        "completed rename must have an inverse: {inverse:?}"
    );
    let inverse = inverse.unwrap();
    let mut journal = f.journal();
    let op = journal.begin(&inverse.execution).unwrap();
    let step = &inverse.execution.groups[0].steps[0];
    journal.intent(op, step).unwrap();
    let ExecutionAction::RenameEntry { source, target, .. } = &step.action else {
        panic!()
    };
    let ExpectedIdentity::Observed(stamp) = &source.identity else {
        panic!()
    };
    let mut native = PlatformFs::default();
    let sp = native
        .inspect(source.path.parent().unwrap())
        .unwrap()
        .stamp
        .physical_id
        .unwrap();
    let tp = native
        .inspect(target.parent().unwrap())
        .unwrap()
        .stamp
        .physical_id
        .unwrap();
    native
        .rename_reviewed(&platform::RenameRequest {
            controlled_timestamps: &[],
            source: &source.path,
            target,
            expected: stamp,
            source_parent: &sp,
            target_parent: &tp,
        })
        .unwrap();
    drop(journal);
    let mut journal = f.journal();
    let pending = journal.pending().unwrap().pop().unwrap();
    let preview = reconcile(&pending, &mut native).unwrap();
    let nonce = preview.review_nonce();
    let lease = f.coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let report = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Resume,
        &lease,
        &mut journal,
        &mut native,
        &JobControl::default(),
    )
    .unwrap();
    assert_eq!(report.state, OperationState::Completed);
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"a");
    assert!(!f.media.join("b").exists());
    assert_eq!(
        journal.operation(original.operation_id).unwrap().steps[0].receipts,
        original.steps[0].receipts
    );
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 2);
}
#[test]
fn unfinished_history_cannot_be_removed() {
    let f = Fixture::new(&[("a", "b")]);
    let mut journal = f.journal();
    let op = journal.begin(&f.plan.execution).unwrap();
    let delete_pending = journal.remove_history(op);
    assert!(
        delete_pending.is_err(),
        "pending evidence must stay visible and retained"
    );
    assert_eq!(
        delete_pending.unwrap_err().code,
        ErrorCode::RecoveryRequired
    );
}
#[test]
fn history_pages_are_stable() {
    let f = Fixture::new(&[("a", "b")]);
    let mut journal = f.journal();
    let mut ids = Vec::new();
    for _ in 0..205 {
        let id = journal.begin(&f.plan.execution).unwrap();
        journal.set_state(id, OperationState::Cancelled).unwrap();
        ids.push(id);
    }
    let page = journal.history(None).unwrap();
    assert_eq!(page.items.len(), 200);
    journal.remove_history(ids[0]).unwrap();
    let late = journal.begin(&f.plan.execution).unwrap();
    journal.set_state(late, OperationState::Cancelled).unwrap();
    let next = journal.history(page.next_cursor).unwrap();
    assert_eq!(
        next.items
            .iter()
            .map(|o| o.operation_id)
            .collect::<Vec<_>>(),
        ids[200..]
    );
    assert!(next.next_cursor.is_none());
}
#[test]
fn inverse_executes_through_confirmation_and_preserves_facts() {
    let f = Fixture::new(&[("a", "b"), ("b", "a")]);
    let original = f.complete();
    let inverse = plan_undo(&original, &f.namespace(), version());
    assert!(inverse.is_ok(), "cycle must have inverse: {inverse:?}");
    let inverse = inverse.unwrap();
    let mut store = PlanStore::new(inverse.version.session_id);
    let version = inverse.version;
    store.publish(inverse).unwrap();
    store.confirm(version, "undo-confirmed".into()).unwrap();
    let inverse = store
        .consume_confirmation(version, "undo-confirmed")
        .unwrap();
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut journal = f.journal();
    let result = execute(
        inverse,
        &lease,
        &mut journal,
        &mut PlatformFs {
            namespace_epoch: inverse.execution.provenance.namespace_epoch,
            policy_epoch: inverse.execution.provenance.policy_epoch,
            ..Default::default()
        },
        &JobControl::default(),
    )
    .unwrap();
    assert_eq!(result.state, OperationState::Completed);
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"a");
    assert_eq!(fs::read(f.media.join("b")).unwrap(), b"b");
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 6);
}
#[test]
fn removing_completed_history_preserves_outbox() {
    let f = Fixture::new(&[("a", "b")]);
    let original = f.complete();
    let mut journal = f.journal();
    assert_eq!(journal.history(None).unwrap().items.len(), 1);
    assert!(plan_undo(&original, &f.namespace(), version()).is_ok());
    let before = journal.events_after(0, 200).unwrap();
    journal.remove_history(original.operation_id).unwrap();
    assert!(journal.history(None).unwrap().items.is_empty());
    assert_eq!(journal.events_after(0, 200).unwrap().items, before.items);
    assert!(plan_undo(
        &journal.operation(original.operation_id).unwrap(),
        &f.namespace(),
        version()
    )
    .is_err());
}

fn run_inverse(f: &Fixture, inverse: &RenamePlan) -> OperationReport {
    let mut store = PlanStore::new(inverse.version.session_id);
    store.publish(inverse.clone()).unwrap();
    store.confirm(inverse.version, "confirmed".into()).unwrap();
    let plan = store
        .consume_confirmation(inverse.version, "confirmed")
        .unwrap();
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    execute(
        plan,
        &lease,
        &mut f.journal(),
        &mut PlatformFs {
            namespace_epoch: plan.execution.provenance.namespace_epoch,
            policy_epoch: plan.execution.provenance.policy_epoch,
            ..Default::default()
        },
        &JobControl::default(),
    )
    .unwrap()
}
#[test]
fn parent_child_undo_uses_complete_transformed_namespace() {
    let f = Fixture::new(&[("folder", "renamed"), ("folder/child", "folder/new")]);
    let original = f.complete();
    let inverse = plan_undo(&original, &f.namespace(), version()).unwrap();
    assert_eq!(inverse.counts.blocked, 0, "{:?}", inverse.issues);
    assert_eq!(run_inverse(&f, &inverse).state, OperationState::Completed);
    assert_eq!(
        fs::read(f.media.join("folder/child")).unwrap(),
        b"folder/child"
    );
    assert!(!f.media.join("renamed").exists());
}
#[test]
fn unknown_child_or_changed_material_blocks_directory_undo() {
    for unknown in [true, false] {
        let f = Fixture::new(&[("folder", "renamed"), ("folder/child", "folder/new")]);
        let original = f.complete();
        let target = if unknown {
            "renamed/unknown"
        } else {
            "renamed/new"
        };
        fs::write(f.media.join(target), b"external material").unwrap();
        let inverse = plan_undo(&original, &f.namespace(), version()).unwrap();
        assert!(inverse.counts.blocked > 0);
        assert!(inverse.execution.groups.is_empty());
        assert_eq!(
            fs::read(f.media.join(target)).unwrap(),
            b"external material"
        );
    }
}
fn durable_steps(f: &Fixture, count: usize) -> OperationId {
    let _lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut journal = f.journal();
    let op = journal.begin(&f.plan.execution).unwrap();
    let mut native = PlatformFs::default();
    for step in f
        .plan
        .execution
        .groups
        .iter()
        .flat_map(|g| &g.steps)
        .take(count)
    {
        journal.intent(op, step).unwrap();
        let ExecutionAction::RenameEntry { source, target, .. } = &step.action else {
            panic!()
        };
        let ExpectedIdentity::Observed(stamp) = &source.identity else {
            panic!()
        };
        let source_parent = native
            .inspect(source.path.parent().unwrap())
            .unwrap()
            .stamp
            .physical_id
            .unwrap();
        let target_parent = native
            .inspect(target.parent().unwrap())
            .unwrap()
            .stamp
            .physical_id
            .unwrap();
        let outcome = native
            .rename_reviewed(&platform::RenameRequest {
                controlled_timestamps: &[],
                source: &source.path,
                target,
                expected: stamp,
                source_parent: &source_parent,
                target_parent: &target_parent,
            })
            .unwrap();
        journal
            .complete(
                op,
                &journal::StepReceipt {
                    step_id: step.id,
                    phase: events::CompletedPhase::Renamed,
                    before_path: Some(encode_path(&source.path)),
                    after_path: Some(encode_path(target)),
                    before: Some(stamp.clone()),
                    after: Some(outcome.observation.stamp),
                    digest: None,
                    observed_ns: 0,
                    durability: outcome.durability,
                },
            )
            .unwrap();
    }
    op
}
fn restore(f: &Fixture, op: OperationId) -> OperationRecord {
    let mut journal = f.journal();
    let mut native = PlatformFs::default();
    let pending = journal.operation(op).unwrap();
    let preview = reconcile(&pending, &mut native).unwrap();
    let nonce = preview.review_nonce();
    let lease = f.coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut native,
        &JobControl::default(),
    )
    .unwrap();
    journal.operation(op).unwrap()
}
#[test]
fn cancelled_operation_retains_undo_for_completed_earlier_group() {
    let f = Fixture::new(&[("a", "new"), ("b", "other")]);
    let op = durable_steps(&f, 1);
    let record = restore(&f, op);
    assert_eq!(record.state, OperationState::Cancelled);
    assert!(record.recovery_effects.is_empty());
    let inverse = plan_undo(&record, &f.namespace(), version()).unwrap();
    assert_eq!(inverse.rows.len(), 1);
    assert_eq!(inverse.counts.blocked, 0);
    assert_eq!(run_inverse(&f, &inverse).state, OperationState::Completed);
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"a");
    assert_eq!(fs::read(f.media.join("b")).unwrap(), b"b");
    assert!(!f.media.join("other").exists());
}
#[test]
fn compensated_original_receipts_do_not_offer_another_undo() {
    // One incomplete dependency group: the second original step never executes.
    let mut f = Fixture::new(&[("a", "new"), ("b", "other")]);
    let group = f.plan.execution.groups[0].id;
    let mut second = f.plan.execution.groups.pop().unwrap();
    for step in &mut second.steps {
        step.group_id = group;
    }
    f.plan.execution.groups[0].steps.extend(second.steps);
    f.plan.rows.iter_mut().for_each(|r| r.group_id = group);
    let op = durable_steps(&f, 1);
    let record = restore(&f, op);
    assert_eq!(record.recovery_effects.len(), 1);
    assert_eq!(record.steps[0].receipts.len(), 1);
    assert!(plan_undo(&record, &f.namespace(), version()).is_err());
    let mut journal = f.journal();
    let events = journal.events_after(0, 200).unwrap().items;
    assert_eq!(events.len(), 2);
    journal.remove_history(op).unwrap();
    assert_eq!(journal.events_after(0, 200).unwrap().items, events);
    assert_eq!(
        journal.operation(op).unwrap().recovery_effects,
        record.recovery_effects
    );
}
#[test]
fn historical_authority_is_rechecked_at_begin() {
    let f = Fixture::new(&[("a", "b")]);
    let original = f.complete();
    let inverse = plan_undo(&original, &f.namespace(), version()).unwrap();
    let mut forged = inverse.execution.clone();
    let ExecutionAction::RenameEntry { target, .. } = &mut forged.groups[0].steps[0].action else {
        panic!()
    };
    *target = f.media.join("unapproved");
    assert!(f.journal().begin(&forged).is_err());
    assert!(!f.media.join("unapproved").exists());
    f.journal().remove_history(original.operation_id).unwrap();
    assert!(f.journal().begin(&inverse.execution).is_err());
    assert_eq!(fs::read(f.media.join("b")).unwrap(), b"a");
}
#[test]
fn newly_created_original_after_preview_cannot_be_overwritten() {
    let f = Fixture::new(&[("a", "b")]);
    let original = f.complete();
    let inverse = plan_undo(&original, &f.namespace(), version()).unwrap();
    fs::write(f.media.join("a"), b"external").unwrap();
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let result = execute(
        &inverse,
        &lease,
        &mut f.journal(),
        &mut PlatformFs {
            namespace_epoch: inverse.execution.provenance.namespace_epoch,
            ..Default::default()
        },
        &JobControl::default(),
    );
    assert!(result.is_err());
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"external");
    assert_eq!(fs::read(f.media.join("b")).unwrap(), b"a");
}
#[test]
fn original_and_inverse_outboxes_survive_independent_history_removal() {
    let f = Fixture::new(&[("a", "b")]);
    let original = f.complete();
    let inverse = plan_undo(&original, &f.namespace(), version()).unwrap();
    let report = run_inverse(&f, &inverse);
    let inverse_id = report.operation_id.unwrap();
    let mut journal = f.journal();
    assert_eq!(
        journal.operation(inverse_id).unwrap().inverse_of,
        Some(original.operation_id)
    );
    let current = journal.operation(original.operation_id).unwrap();
    assert_eq!(current.active_inverses, vec![inverse_id]);
    assert!(plan_undo(&current, &f.namespace(), version()).is_err());
    let events = journal.events_after(0, 200).unwrap().items;
    assert_eq!(events.len(), 2);
    journal.remove_history(original.operation_id).unwrap();
    assert_eq!(journal.history(None).unwrap().items.len(), 1);
    assert_eq!(journal.events_after(0, 200).unwrap().items, events);
    journal.ack_events(&[events[0].event_id]).unwrap();
    journal.remove_history(inverse_id).unwrap();
    assert!(journal.history(None).unwrap().items.is_empty());
    assert_eq!(
        journal.events_after(0, 200).unwrap().items,
        vec![events[1].clone()]
    );
}

#[allow(dead_code)]
#[path = "support/crash_child.rs"]
mod crash_support;

#[test]
fn undo_crash_child() {
    let Ok(boundary) = std::env::var("VIDLIB_UNDO_CRASH") else {
        return;
    };
    let f = Fixture::new(&[("a", "b"), ("b", "a")]);
    let original = f.complete();
    let inverse = plan_undo(&original, &f.namespace(), version()).unwrap();
    let mut adapter = crash_support::CrashFs {
        native: PlatformFs {
            namespace_epoch: inverse.execution.provenance.namespace_epoch,
            ..Default::default()
        },
        boundary,
        index: 1,
        count: 0,
        outcome_durability: None,
        after_capture: None,
    };
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let _ = execute(
        &inverse,
        &lease,
        &mut f.journal(),
        &mut adapter,
        &JobControl::default(),
    );
    panic!("child did not terminate at inverse boundary");
}
#[test]
fn interrupted_inverse_process_recovers_after_original_history_removed() {
    for boundary in ["before", "after"] {
        let root = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "undo_crash_child", "--nocapture"])
            .env("VIDLIB_UNDO_ROOT", root.path())
            .env("VIDLIB_UNDO_CRASH", boundary)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));
        let fixture = fs::read_dir(root.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let coordinator = Coordinator::open(&fixture.join("app")).unwrap();
        let mut journal = Journal::open(&coordinator.database_path()).unwrap();
        let pending = journal.pending().unwrap().pop().unwrap();
        let original = pending.inverse_of.unwrap();
        let original_record = journal.operation(original).unwrap();
        let original_receipts: Vec<_> = original_record
            .steps
            .iter()
            .flat_map(|s| s.receipts.clone())
            .collect();
        journal.remove_history(original).unwrap();
        assert_eq!(journal.pending().unwrap(), vec![pending.clone()]);
        let mut native = PlatformFs::default();
        let preview = reconcile(&pending, &mut native).unwrap();
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        let report = apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Resume,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default(),
        )
        .unwrap();
        assert_eq!(report.state, OperationState::Completed);
        assert!(journal.pending().unwrap().is_empty());
        assert_eq!(fs::read(fixture.join("media/a")).unwrap(), b"a");
        assert_eq!(fs::read(fixture.join("media/b")).unwrap(), b"b");
        assert_eq!(
            journal
                .operation(original)
                .unwrap()
                .steps
                .iter()
                .flat_map(|s| s.receipts.clone())
                .collect::<Vec<_>>(),
            original_receipts
        );
        assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 6);
        journal.remove_history(pending.operation_id).unwrap();
        assert!(journal.history(None).unwrap().items.is_empty());
        assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 6);
    }
}
#[test]
fn failed_v6_inverse_migration_retains_original_evidence() {
    let f = Fixture::new(&[("a", "b")]);
    let original = f.complete();
    drop(f.journal());
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    legacy_schema::remove_copy_protocol(&db);
    db.execute_batch("DROP INDEX IF EXISTS history_by_state; DROP TABLE IF EXISTS history_revision; DROP TABLE operation_inverses; DELETE FROM schema_migrations WHERE version>=7; PRAGMA user_version=6; CREATE TABLE operation_inverses(evidence TEXT); INSERT INTO operation_inverses VALUES('retain me');").unwrap();
    assert!(Journal::open(&f.coordinator.database_path()).is_err());
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        6
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM receipts WHERE operation_id=?1",
            [original.operation_id.to_string()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT evidence FROM operation_inverses", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "retain me"
    );
}
#[test]
fn v6_history_migrates_without_inventing_inverse_authority() {
    let f = Fixture::new(&[("a", "b")]);
    let original = f.complete();
    let events = f.journal().events_after(0, 200).unwrap().items;
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    legacy_schema::remove_copy_protocol(&db);
    db.execute_batch("DROP INDEX IF EXISTS history_by_state; DROP TABLE IF EXISTS history_revision; DROP TABLE operation_inverses; DELETE FROM schema_migrations WHERE version>=7; PRAGMA user_version=6;").unwrap();
    drop(db);
    let journal = f.journal();
    assert_eq!(journal.operation(original.operation_id).unwrap(), original);
    assert_eq!(journal.events_after(0, 200).unwrap().items, events);
    assert_eq!(
        plan_undo(&original, &f.namespace(), version())
            .unwrap()
            .counts
            .blocked,
        0
    );
}

#[test]
fn inverse_preview_keeps_stable_ids_for_unselected_namespace_entries() {
    let f = Fixture::new(&[("a", "b")]);
    let original = f.complete();
    fs::write(f.media.join("unrelated"), b"keep").unwrap();
    let ns = f.namespace();
    let version = version();
    let first = plan_undo(&original, &ns, version).unwrap();
    let second = plan_undo(&original, &ns, version).unwrap();
    assert_eq!(
        first
            .execution
            .provenance
            .inventory
            .entries
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>(),
        second
            .execution
            .provenance
            .inventory
            .entries
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>(),
        "the same historical receipt and namespace must yield stable inverse inventory identities"
    );
}

#[test]
fn interrupted_undo_restore_preserves_original_facts_and_allows_fresh_retry() {
    let root = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "undo_crash_child", "--nocapture"])
        .env("VIDLIB_UNDO_ROOT", root.path())
        .env("VIDLIB_UNDO_CRASH", "after")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let fixture = fs::read_dir(root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let coordinator = Coordinator::open(&fixture.join("app")).unwrap();
    let mut journal = Journal::open(&coordinator.database_path()).unwrap();
    let pending = journal.pending().unwrap().pop().unwrap();
    let original = pending.inverse_of.unwrap();
    let prior = journal.operation(original).unwrap();
    let mut native = PlatformFs::default();
    let preview = reconcile(&pending, &mut native).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let report = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut native,
        &JobControl::default(),
    )
    .unwrap();
    assert_eq!(report.state, OperationState::Cancelled);
    drop(lease);
    assert_eq!(fs::read(fixture.join("media/a")).unwrap(), b"b");
    assert_eq!(fs::read(fixture.join("media/b")).unwrap(), b"a");
    let restored = journal.operation(original).unwrap();
    assert!(restored.active_inverses.is_empty());
    assert_eq!(restored.steps, prior.steps);
    let grants = GrantSet {
        grants: restored.inspection["grants"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| SourceGrant {
                id: serde_json::from_value(g["id"].clone()).unwrap(),
                source: serde_json::from_value(g["source"].clone()).unwrap(),
                path: decode_path(&serde_json::from_value(g["path"].clone()).unwrap()).unwrap(),
            })
            .collect(),
    };
    let lease = coordinator.try_acquire(LeaseKind::Read).unwrap();
    let epochs = lease.epochs().unwrap();
    let native = PlatformFs {
        namespace_epoch: epochs.namespace,
        policy_epoch: epochs.policy,
        ..Default::default()
    };
    let ns = native
        .capture_namespace(&grants, &[fixture.join("media/capture")])
        .unwrap();
    assert_eq!(
        plan_undo(&restored, &ns, version()).unwrap().counts.blocked,
        0
    );
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 5);
}

#[test]
fn missing_history_visibility_never_hides_pending_recovery_evidence() {
    let f = Fixture::new(&[("a", "b")]);
    let mut journal = f.journal();
    let op = journal.begin(&f.plan.execution).unwrap();
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    db.execute(
        "DELETE FROM history_visibility WHERE operation_id=?1",
        [op.to_string()],
    )
    .unwrap();
    let pending = journal.pending();
    assert!(
        pending.is_ok(),
        "pending recovery is independent of history support: {pending:?}"
    );
    assert_eq!(pending.unwrap()[0].operation_id, op);
    assert!(!journal.operation(op).unwrap().history_visible);
}
#[test]
fn missing_historical_nested_namespace_cannot_bless_current_child_material() {
    let mut f = Fixture::new(&[
        ("folder", "renamed"),
        ("folder/child", "folder/new"),
        ("folder/nested/deep", "folder/nested/deep"),
    ]);
    // The forward planner has a complete inventory, but its target namespace
    // need not contain the unselected nested directory's child listing.
    f.plan
        .execution
        .expected_namespace
        .parents
        .retain(|p| p.parent_path != f.media.join("folder/nested"));
    let original = f.complete();
    fs::write(
        f.media.join("renamed/nested/deep"),
        b"unrecorded material change",
    )
    .unwrap();
    let inverse = plan_undo(&original, &f.namespace(), version()).unwrap();
    assert!(
        inverse.counts.blocked > 0,
        "a current complete scan cannot replace missing historical child evidence"
    );
    assert!(inverse.execution.groups.is_empty());
    assert_eq!(
        fs::read(f.media.join("renamed/nested/deep")).unwrap(),
        b"unrecorded material change"
    );
}

#[test]
#[ignore = "explicit real-files inverse/journal scale workload"]
fn benchmark_large_inverse_journal_begin() {
    use std::time::Instant;
    let count: usize = std::env::var("VIDLIB_INVERSE_ENTRIES")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(1000);
    assert!((1..=2000).contains(&count));
    let names = (0..count)
        .map(|i| (format!("before-{i:05}"), format!("after-{i:05}")))
        .collect::<Vec<_>>();
    let pairs = names
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect::<Vec<_>>();
    let started = Instant::now();
    let fixture = Fixture::new(&pairs);
    let prepare_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let original = fixture.complete();
    let execute_ms = started.elapsed().as_millis();
    let namespace = fixture.namespace();
    let started = Instant::now();
    let inverse = plan_undo(&original, &namespace, version()).unwrap();
    let plan_ms = started.elapsed().as_millis();
    assert_eq!(inverse.rows.len(), count);
    assert_eq!(inverse.counts.blocked, 0);
    let mut journal = fixture.journal();
    let started = Instant::now();
    let id = journal.begin(&inverse.execution).unwrap();
    let begin_ms = started.elapsed().as_millis();
    assert_eq!(
        journal.operation(id).unwrap().groups.len(),
        inverse.execution.groups.len()
    );
    assert_eq!(
        journal.operation(original.operation_id).unwrap().steps,
        original.steps
    );
    for (_, target) in &names {
        assert!(fixture.media.join(target).exists());
    }
    println!("BENCH inverse entries={count} prepare_ms={prepare_ms} execute_ms={execute_ms} plan_ms={plan_ms} journal_begin_ms={begin_ms}");
}
