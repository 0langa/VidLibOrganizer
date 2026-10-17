#![cfg(target_os = "linux")]
use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
};
use vidlib_fileops::{
    coordinator::{Coordinator, LeaseKind},
    journal::{Journal, OperationState, StepState},
    platform::{Durability, MetadataUpdate, MutationOutcome, OwnedEntry, StagingFile},
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
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        fs::create_dir(&media).unwrap();
        for (name, _) in names {
            fs::write(
                media.join(name),
                if *name == "a" {
                    b"fixture".as_slice()
                } else {
                    name.as_bytes()
                },
            )
            .unwrap();
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
            numbering_order: selection.clone(),
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
            values: vec![],
        };
        let mut batch = evaluate(&inventory, &recipe, &frozen).unwrap();
        for row in &mut batch.rows {
            let (_, target) = names
                .iter()
                .find(|(name, _)| row.source_path == media.join(name))
                .unwrap();
            row.target_path = media.join(target);
            row.status = RowStatus::Changed;
        }
        let namespace = PlatformFs::default()
            .capture_namespace(
                &batch.grants,
                &batch
                    .rows
                    .iter()
                    .map(|r| r.source_path.clone())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let plan = plan(
            &inventory,
            &batch,
            &namespace,
            PlanRef {
                session_id: SessionId::new(),
                plan_id: PlanId::new(),
                revision: 1,
            },
        )
        .unwrap();
        assert_eq!(plan.counts.blocked, 0, "{:?}", plan.issues);
        Self {
            _dir: dir,
            media,
            coordinator,
            plan,
        }
    }
    fn run(
        &self,
        adapter: &mut dyn Filesystem,
        control: &JobControl,
    ) -> Result<OperationReport, OpError> {
        let lease = self.coordinator.try_acquire(LeaseKind::Write).unwrap();
        let mut journal = Journal::open(&self.coordinator.database_path()).unwrap();
        execute(&self.plan, &lease, &mut journal, adapter, control)
    }
}
struct FaultFs {
    native: PlatformFs,
    database: PathBuf,
    calls: Vec<StepId>,
    fail_at: Option<usize>,
    collide: bool,
    cancel: Option<JobControl>,
    durability: Option<Durability>,
    stale_next: Option<PathBuf>,
}
impl FaultFs {
    fn new(f: &Fixture) -> Self {
        Self {
            native: PlatformFs::default(),
            database: f.coordinator.database_path(),
            calls: vec![],
            fail_at: None,
            collide: false,
            cancel: None,
            durability: None,
            stale_next: None,
        }
    }
}
impl Filesystem for FaultFs {
    fn rename_reserved(
        &mut self,
        authority: &platform::RenameReservation,
    ) -> Result<MutationOutcome, OpError> {
        let journal = Journal::open(&self.database).unwrap();
        let operation = journal.operation(authority.operation_id()).unwrap();
        let step = operation
            .steps
            .iter()
            .find(|s| s.step.id == authority.step_id())
            .unwrap();
        assert_eq!(step.state, StepState::IntentDurable);
        self.calls.push(step.step.id);
        if self.fail_at == Some(self.calls.len()) {
            return Err(OpError::new(
                ErrorCode::Io,
                Phase::Rename,
                "injected reservation failure",
            ));
        }
        let outcome = self.native.rename_reserved(authority);
        if let Some(control) = &self.cancel {
            control.cancel();
        }
        outcome
    }
    fn capture_namespace(&self, g: &GrantSet, c: &[PathBuf]) -> Result<NamespaceSnapshot, OpError> {
        self.native.capture_namespace(g, c)
    }
    fn inspect(&self, p: &Path) -> Result<FileObservation, OpError> {
        self.native.inspect(p)
    }
    fn verify(&self, p: &Path, s: &FileStamp) -> Result<FileObservation, OpError> {
        self.native.verify(p, s)
    }
    fn rename_no_replace(
        &mut self,
        s: &Path,
        t: &Path,
        e: &FileStamp,
    ) -> Result<MutationOutcome, OpError> {
        let journal = Journal::open(&self.database).unwrap();
        let pending = journal.pending().unwrap();
        let intent = pending
            .iter()
            .flat_map(|p| &p.steps)
            .find(|s| s.state == StepState::IntentDurable)
            .expect("native effect requires committed intent visible on separate connection");
        self.calls.push(intent.step.id);
        if self.fail_at == Some(self.calls.len()) {
            return Err(OpError::new(
                ErrorCode::Io,
                Phase::Rename,
                "injected filesystem failure",
            ));
        }
        if self.collide {
            fs::write(t, b"late occupant").unwrap();
        }
        let mut result = self.native.rename_no_replace(s, t, e);
        if let (Ok(outcome), Some(durability)) = (&mut result, &self.durability) {
            outcome.durability = durability.clone();
        }
        if let Some(path) = self.stale_next.take() {
            fs::write(path, b"late changed source").unwrap();
        }
        if let Some(control) = &self.cancel {
            control.cancel();
        }
        result
    }
    fn publish_staging(&mut self, o: &OwnedEntry, t: &Path) -> Result<MutationOutcome, OpError> {
        self.native.publish_staging(o, t)
    }
    fn create_directory(&mut self, p: &Path) -> Result<OwnedEntry, OpError> {
        self.native.create_directory(p)
    }
    fn create_staging(&mut self, p: &Path) -> Result<StagingFile, OpError> {
        self.native.create_staging(p)
    }
    fn refresh_staging(&self, s: &mut StagingFile) -> Result<Durability, OpError> {
        self.native.refresh_staging(s)
    }
    fn open_verified(&self, p: &Path, s: &FileStamp) -> Result<File, OpError> {
        self.native.open_verified(p, s)
    }
    fn set_metadata(
        &mut self,
        p: &Path,
        s: &FileStamp,
        u: &MetadataUpdate,
    ) -> Result<MutationOutcome, OpError> {
        self.native.set_metadata(p, s, u)
    }
    fn flush_directory(&self, p: &Path) -> Result<Durability, OpError> {
        self.native.flush_directory(p)
    }
    fn remove_owned(&mut self, o: &OwnedEntry) -> Result<Durability, OpError> {
        self.native.remove_owned(o)
    }
}
#[test]
fn executes_exact_reviewed_plan() {
    let f = Fixture::new(&[("a", "new")]);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert!(
        f.media.join("new").exists(),
        "reviewed target must be materialized"
    );
    assert_eq!(fs::read(f.media.join("new")).unwrap(), b"fixture");
    assert_eq!(report.state, OperationState::Completed);
    assert_eq!(report.receipts.len(), 1);
    assert!(!f.media.join("a").exists());
}
#[test]
fn late_stale_item_changes_nothing_in_preflight() {
    let f = Fixture::new(&[("a", "new"), ("b", "other")]);
    fs::write(f.media.join("b"), b"changed material content").unwrap();
    let result = f.run(&mut PlatformFs::default(), &JobControl::default());
    assert!(
        result.is_err(),
        "late stale item must reject whole selection"
    );
    assert!(f.media.join("a").exists());
    assert!(!f.media.join("new").exists());
    assert!(Journal::open(&f.coordinator.database_path())
        .unwrap()
        .pending()
        .unwrap()
        .is_empty());
}
#[test]
fn no_runtime_collision_suffix() {
    let f = Fixture::new(&[("a", "new")]);
    let mut adapter = FaultFs::new(&f);
    adapter.collide = true;
    let report = f.run(&mut adapter, &JobControl::default()).unwrap();
    assert_eq!(
        report.failure.as_ref().map(|e| e.code),
        Some(ErrorCode::DestinationExists)
    );
    assert_eq!(fs::read(f.media.join("new")).unwrap(), b"late occupant");
    assert!(f.media.join("a").exists());
    assert!(!f.media.join("new-1").exists());
}
#[test]
fn all_graph_steps_have_prior_durable_intent() {
    let f = Fixture::new(&[("a", "new"), ("b", "other")]);
    let mut adapter = FaultFs::new(&f);
    let report = f.run(&mut adapter, &JobControl::default()).unwrap();
    let expected: Vec<_> = f
        .plan
        .execution
        .groups
        .iter()
        .flat_map(|g| g.steps.iter().map(|s| s.id))
        .collect();
    assert_eq!(
        adapter.calls, expected,
        "every compiled step must execute after durable intent"
    );
    assert_eq!(report.receipts.len(), 2);
}
#[test]
fn failed_group_stops_following_groups() {
    let f = Fixture::new(&[("a", "new"), ("b", "other"), ("c", "last")]);
    let mut adapter = FaultFs::new(&f);
    adapter.fail_at = Some(2);
    let report = f.run(&mut adapter, &JobControl::default()).unwrap();
    assert_eq!(adapter.calls.len(), 2, "stop on failed group");
    assert_eq!(report.completed_groups.len(), 1);
    assert_eq!(report.state, OperationState::RecoveryRequired);
    assert!(f.media.join("c").exists());
    assert!(!f.media.join("last").exists());
}
#[test]
fn native_swap_consumes_only_compiled_cycle_reservations() {
    let f = Fixture::new(&[("a", "b"), ("b", "a")]);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(
        report.state,
        OperationState::Completed,
        "{:?}",
        report.failure
    );
    assert_eq!(fs::read(f.media.join("b")).unwrap(), b"fixture");
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"b");
    assert_eq!(report.receipts.len(), 3);
    assert_eq!(fs::read_dir(&f.media).unwrap().count(), 2);
}
#[test]
fn ordinary_unowned_cycle_temp_is_refused() {
    let f = Fixture::new(&[("a", "new")]);
    let mut native = PlatformFs::default();
    let source = f.media.join("a");
    let stamp = native.inspect(&source).unwrap().stamp;
    let target = f.media.join(".vidlib-staging-rename-unowned-0");
    assert_eq!(
        native
            .rename_no_replace(&source, &target, &stamp)
            .unwrap_err()
            .code,
        ErrorCode::ProtectedEntry
    );
    assert!(source.exists());
    assert!(!target.exists());
}
#[test]
fn cancellation_waits_for_group_boundary_and_reports_completed_groups() {
    let f = Fixture::new(&[("a", "new"), ("b", "other")]);
    let control = JobControl::default();
    let mut adapter = FaultFs::new(&f);
    adapter.cancel = Some(control.clone());
    let report = f.run(&mut adapter, &control).unwrap();
    assert_eq!(report.state, OperationState::Cancelled);
    assert_eq!(report.completed_groups.len(), 1);
    assert_eq!(report.receipts.len(), 1);
    assert!(f.media.join("b").exists());
    assert!(Journal::open(&f.coordinator.database_path())
        .unwrap()
        .pending()
        .unwrap()
        .is_empty());
    assert_eq!(
        f.coordinator
            .try_acquire(LeaseKind::Read)
            .unwrap()
            .epochs()
            .unwrap()
            .namespace,
        1
    );
}
#[test]
fn matching_write_lease_and_current_epochs_are_required() {
    let f = Fixture::new(&[("a", "new")]);
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let read = f.coordinator.try_acquire(LeaseKind::Read).unwrap();
    assert_eq!(
        execute(
            &f.plan,
            &read,
            &mut journal,
            &mut PlatformFs::default(),
            &JobControl::default()
        )
        .unwrap_err()
        .code,
        ErrorCode::PermissionDenied
    );
    drop(read);
    let other = tempfile::tempdir().unwrap();
    let foreign = Coordinator::open(other.path()).unwrap();
    let lease = foreign.try_acquire(LeaseKind::Write).unwrap();
    assert_eq!(
        execute(
            &f.plan,
            &lease,
            &mut journal,
            &mut PlatformFs::default(),
            &JobControl::default()
        )
        .unwrap_err()
        .code,
        ErrorCode::PermissionDenied
    );
    drop(lease);
    let lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
    lease.advance_policy().unwrap();
    assert_eq!(
        execute(
            &f.plan,
            &lease,
            &mut journal,
            &mut PlatformFs::default(),
            &JobControl::default()
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleState
    );
    assert!(f.media.join("a").exists());
}
#[test]
fn expected_namespace_is_persisted_with_native_paths() {
    let f = Fixture::new(&[("a", "new")]);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    let record = Journal::open(&f.coordinator.database_path())
        .unwrap()
        .operation(report.operation_id.unwrap())
        .unwrap();
    let stored = record
        .inspection
        .get("expected_namespace")
        .expect("expected namespace inspection must survive reopen");
    assert_eq!(
        stored["parents"][0]["parent_path"],
        serde_json::to_value(encode_path(&f.media)).unwrap()
    );
    assert_eq!(stored["namespace_epoch"], "0");
}
fn replan(f: &mut Fixture, root: PathBuf, mapping: &[(&str, &str)]) {
    let inventory = scan(
        &InventoryRequest::new(vec![RootBinding::new(root).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let selection: Vec<_> = mapping
        .iter()
        .map(|(s, _)| {
            inventory
                .entries
                .iter()
                .find(|e| e.absolute_path == f.media.join(s))
                .unwrap()
                .id
        })
        .collect();
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: selection.clone(),
        numbering_order: selection.clone(),
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
        values: vec![],
    };
    let mut batch = evaluate(&inventory, &recipe, &frozen).unwrap();
    for row in &mut batch.rows {
        let (_, t) = mapping
            .iter()
            .find(|(s, _)| row.source_path == f.media.join(s))
            .unwrap();
        row.target_path = f.media.join(t);
        row.status = RowStatus::Changed;
    }
    let candidates: Vec<_> = inventory
        .entries
        .iter()
        .map(|e| e.absolute_path.clone())
        .collect();
    let namespace = PlatformFs::default()
        .capture_namespace(&batch.grants, &candidates)
        .unwrap();
    f.plan = plan(
        &inventory,
        &batch,
        &namespace,
        PlanRef {
            session_id: SessionId::new(),
            plan_id: PlanId::new(),
            revision: 1,
        },
    )
    .unwrap();
    assert_eq!(f.plan.counts.blocked, 0, "{:?}", f.plan.issues);
}
#[test]
fn native_individual_file_grant_allows_absent_sibling_target() {
    let mut f = Fixture::new(&[("a", "new")]);
    let root = f.media.join("a");
    replan(&mut f, root, &[("a", "new")]);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(
        report.state,
        OperationState::Completed,
        "{:?}",
        report.failure
    );
    assert_eq!(fs::read(f.media.join("new")).unwrap(), b"fixture");
}
#[test]
fn individual_file_grant_never_authorizes_unrelated_sibling_source() {
    let mut f = Fixture::new(&[("a", "new"), ("b", "other")]);
    let root = f.media.join("a");
    replan(&mut f, root, &[("a", "new")]);
    let source = EntryPrestate {
        path: f.media.join("b"),
        identity: ExpectedIdentity::Observed(
            PlatformFs::default()
                .inspect(&f.media.join("b"))
                .unwrap()
                .stamp,
        ),
    };
    f.plan.execution.groups[0].steps[0].action = ExecutionAction::RenameEntry {
        source,
        target: f.media.join("other"),
        temporary: false,
    };
    let result = f.run(&mut PlatformFs::default(), &JobControl::default());
    assert!(
        result.is_err(),
        "compiled effect must retain source grant and entry authority"
    );
    assert!(f.media.join("b").exists());
    assert!(!f.media.join("other").exists());
}
#[test]
fn native_directory_move_preserves_unselected_carried_descendants() {
    let mut f = Fixture::new(&[("a", "new")]);
    fs::create_dir_all(f.media.join("folder/nested")).unwrap();
    fs::write(f.media.join("folder/nested/child"), b"carried").unwrap();
    let root = f.media.clone();
    replan(&mut f, root, &[("folder", "renamed")]);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(
        report.state,
        OperationState::Completed,
        "{:?}",
        report.failure
    );
    assert_eq!(
        fs::read(f.media.join("renamed/nested/child")).unwrap(),
        b"carried"
    );
}
#[test]
fn journal_intent_failure_prevents_effect_and_retains_gate() {
    let f = Fixture::new(&[("a", "new"), ("b", "other")]);
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_intent BEFORE UPDATE OF state ON steps WHEN NEW.state='IntentDurable' BEGIN SELECT RAISE(ABORT,'injected intent failure'); END;").unwrap();
    let mut adapter = FaultFs::new(&f);
    let error = f.run(&mut adapter, &JobControl::default()).unwrap_err();
    assert!(error.operation_id.is_some());
    assert!(
        error.step_id.is_some(),
        "original failed journal step must survive finalization"
    );
    assert!(adapter.calls.is_empty());
    assert!(f.media.join("a").exists());
    assert_eq!(
        f.coordinator
            .try_acquire(LeaseKind::Write)
            .unwrap_err()
            .code,
        ErrorCode::RecoveryRequired
    );
}
#[test]
fn receipt_failure_preserves_action_gap_and_stops_next_group() {
    let f = Fixture::new(&[("a", "new"), ("b", "other")]);
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON receipts BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;").unwrap();
    let mut adapter = FaultFs::new(&f);
    let error = f.run(&mut adapter, &JobControl::default()).unwrap_err();
    assert!(
        error.step_id.is_some(),
        "original failed receipt step must survive finalization"
    );
    assert_eq!(adapter.calls.len(), 1);
    assert_eq!(fs::read(f.media.join("new")).unwrap(), b"fixture");
    assert!(f.media.join("b").exists());
    let journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let pending = journal.pending().unwrap();
    assert_eq!(pending[0].steps[0].state, StepState::IntentDurable);
    assert!(pending[0].steps[0].receipts.is_empty());
    assert!(journal.events_after(0, 200).unwrap().items.is_empty());
    let recovery = f.coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    assert_eq!(recovery.epochs().unwrap().namespace, 1);
}
#[test]
fn durability_failure_records_observed_rename_and_stops() {
    let f = Fixture::new(&[("a", "new"), ("b", "other")]);
    let mut adapter = FaultFs::new(&f);
    adapter.durability = Some(Durability::Failed(
        "injected directory flush failure".into(),
    ));
    let report = f.run(&mut adapter, &JobControl::default()).unwrap();
    assert_eq!(report.state, OperationState::RecoveryRequired);
    let error = report.failure.as_ref().unwrap();
    assert_eq!(
        error.operation_id, report.operation_id,
        "durability failure retains operation context"
    );
    assert_eq!(error.step_id, Some(report.receipts[0].step_id));
    assert!(error.item_id.is_some());
    assert_eq!(report.receipts.len(), 1);
    assert_eq!(adapter.calls.len(), 1);
    assert_eq!(fs::read(f.media.join("new")).unwrap(), b"fixture");
    let journal = Journal::open(&f.coordinator.database_path()).unwrap();
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 1);
    assert!(f.media.join("b").exists());
}
#[test]
fn source_freshness_rechecked_after_prior_group() {
    let f = Fixture::new(&[("a", "new"), ("b", "other")]);
    let mut adapter = FaultFs::new(&f);
    adapter.stale_next = Some(f.media.join("b"));
    let report = f.run(&mut adapter, &JobControl::default()).unwrap();
    assert_eq!(report.failure.unwrap().code, ErrorCode::StaleState);
    assert_eq!(report.completed_groups.len(), 1);
    assert_eq!(adapter.calls.len(), 1);
    assert!(!f.media.join("other").exists());
}
#[test]
fn unimplemented_effects_fail_closed_before_first_rename() {
    let mut f = Fixture::new(&[("a", "new"), ("b", "other")]);
    f.plan.execution.groups[1].steps[0].action = ExecutionAction::CreateDirectory {
        target: f.media.join("unsupported"),
    };
    assert_eq!(
        f.run(&mut PlatformFs::default(), &JobControl::default())
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    assert!(f.media.join("a").exists());
    assert!(!f.media.join("new").exists());
}
#[test]
fn native_case_only_rename_preserves_content_and_modified_timestamp() {
    let f = Fixture::new(&[("a", "A")]);
    let before = PlatformFs::default()
        .inspect(&f.media.join("a"))
        .unwrap()
        .stamp;
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(report.state, OperationState::Completed);
    let after = PlatformFs::default()
        .inspect(&f.media.join("A"))
        .unwrap()
        .stamp;
    assert_eq!(before.modified_ns, after.modified_ns);
    assert_eq!(before.physical_id, after.physical_id);
    assert_eq!(fs::read(f.media.join("A")).unwrap(), b"fixture");
}
#[test]
fn namespace_inspection_roundtrips_native_bytes_and_exact_quantities() {
    use std::os::unix::ffi::OsStringExt;
    let mut f = Fixture::new(&[("a", "new")]);
    let raw = f.media.join(std::ffi::OsString::from_vec(vec![0xff, b'x']));
    let expected = &mut f.plan.execution.expected_namespace;
    expected.parents[0].parent_path = raw.clone();
    expected.parents[0].max_path_units = Some(u64::MAX);
    expected.parents[0].stamp.modified_ns = Some(i128::MIN);
    expected.parents[0].entries[0].absolute_path = raw.clone();
    expected.grants.grants[0].path = raw.clone();
    expected.policy.guarded_roots = vec![raw.clone()];
    expected.policy.protected_roots = vec![raw.clone()];
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let op = journal.begin(&f.plan.execution).unwrap();
    drop(journal);
    let record = Journal::open(&f.coordinator.database_path())
        .unwrap()
        .operation(op)
        .unwrap();
    let ns = &record.inspection["expected_namespace"];
    assert_eq!(ns["parents"][0]["max_path_units"], u64::MAX.to_string());
    assert_eq!(
        ns["parents"][0]["stamp"]["modified_ns"],
        i128::MIN.to_string()
    );
    for actual in [
        &ns["parents"][0]["parent_path"],
        &ns["parents"][0]["entries"][0]["absolute_path"],
        &ns["grants"]["grants"][0]["path"],
        &ns["policy"]["guarded_roots"][0],
        &ns["policy"]["protected_roots"][0],
    ] {
        let record: NativePathRecord = serde_json::from_value(actual.clone()).unwrap();
        assert_eq!(decode_path(&record).unwrap(), raw);
    }
}
#[test]
fn compatible_v3_migration_preserves_pending_evidence_without_inventing_namespace() {
    let f = Fixture::new(&[("a", "new")]);
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let op = journal.begin(&f.plan.execution).unwrap();
    journal
        .intent(op, &f.plan.execution.groups[0].steps[0])
        .unwrap();
    drop(journal);
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    db.execute_batch("UPDATE operations SET inspection=json_remove(inspection,'$.expected_namespace'); ALTER TABLE operations DROP COLUMN inspection_version; DELETE FROM schema_migrations WHERE version=4; PRAGMA user_version=3;").unwrap();
    let journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let record = journal.operation(op).unwrap();
    assert_eq!(record.steps[0].state, StepState::IntentDurable);
    assert!(record.inspection.get("expected_namespace").is_none());
    assert_eq!(
        db.query_row("SELECT inspection_version FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}
#[test]
fn failed_v3_migration_rolls_back_schema_and_keeps_evidence() {
    let f = Fixture::new(&[("a", "new")]);
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let op = journal.begin(&f.plan.execution).unwrap();
    drop(journal);
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    db.execute_batch("ALTER TABLE operations DROP COLUMN inspection_version; DELETE FROM schema_migrations WHERE version=4; PRAGMA user_version=3; CREATE TRIGGER reject_upgrade BEFORE INSERT ON schema_migrations WHEN NEW.version=4 BEGIN SELECT RAISE(ABORT,'injected migration failure'); END;").unwrap();
    let before: String = db
        .query_row("SELECT inspection FROM operations", [], |r| r.get(0))
        .unwrap();
    assert!(Journal::open(&f.coordinator.database_path()).is_err());
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        db.query_row(
            "SELECT inspection FROM operations WHERE id=?1",
            [op.to_string()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    assert!(db
        .prepare("SELECT inspection_version FROM operations")
        .is_err());
}
#[test]
fn epoch_failure_retains_pending_journal_after_observed_rename() {
    let f = Fixture::new(&[("a", "new")]);
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_epoch BEFORE UPDATE ON coordination BEGIN SELECT RAISE(ABORT,'injected epoch failure'); END;").unwrap();
    let error = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap_err();
    assert!(error.operation_id.is_some());
    assert_eq!(fs::read(f.media.join("new")).unwrap(), b"fixture");
    let journal = Journal::open(&f.coordinator.database_path()).unwrap();
    assert_eq!(
        journal.pending().unwrap()[0].steps[0].state,
        StepState::CompletionDurable
    );
    assert_eq!(
        f.coordinator
            .try_acquire(LeaseKind::Write)
            .unwrap_err()
            .code,
        ErrorCode::RecoveryRequired
    );
}
#[test]
fn finalization_failure_keeps_recovery_gate_and_advanced_epoch() {
    let f = Fixture::new(&[("a", "new")]);
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_final BEFORE UPDATE OF state ON operations WHEN NEW.state='Completed' BEGIN SELECT RAISE(ABORT,'injected finalization failure'); END;").unwrap();
    assert!(f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .is_err());
    let lease = f.coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    assert_eq!(lease.epochs().unwrap().namespace, 1);
    assert!(lease.status().unwrap().pending_journal);
    assert_eq!(fs::read(f.media.join("new")).unwrap(), b"fixture");
}
#[test]
fn cancellation_inside_cycle_finishes_group_before_stopping() {
    let f = Fixture::new(&[("a", "b"), ("b", "a"), ("c", "last")]);
    let control = JobControl::default();
    let mut adapter = FaultFs::new(&f);
    adapter.cancel = Some(control.clone());
    let report = f.run(&mut adapter, &control).unwrap();
    assert_eq!(report.state, OperationState::Cancelled);
    assert_eq!(report.completed_groups.len(), 1);
    assert_eq!(report.receipts.len(), 3);
    assert_eq!(fs::read(f.media.join("b")).unwrap(), b"fixture");
    assert!(f.media.join("c").exists());
    assert!(!f.media.join("last").exists());
    let ids: Vec<_> = f.plan.execution.groups[0]
        .steps
        .iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(adapter.calls, ids);
}
#[test]
fn failed_cycle_keeps_owned_temporary_receipt_for_recovery() {
    let f = Fixture::new(&[("a", "b"), ("b", "a"), ("c", "last")]);
    let mut adapter = FaultFs::new(&f);
    adapter.fail_at = Some(2);
    let report = f.run(&mut adapter, &JobControl::default()).unwrap();
    assert_eq!(report.state, OperationState::RecoveryRequired);
    assert!(report.completed_groups.is_empty());
    assert_eq!(report.receipts.len(), 1);
    let temp = decode_path(report.receipts[0].after_path.as_ref().unwrap()).unwrap();
    assert!(temp.exists());
    assert!(f.media.join("c").exists());
    assert_eq!(adapter.calls.len(), 2);
    let record = Journal::open(&f.coordinator.database_path())
        .unwrap()
        .operation(report.operation_id.unwrap())
        .unwrap();
    assert_eq!(record.steps[0].state, StepState::CompletionDurable);
    assert_eq!(record.steps[1].state, StepState::Unresolved);
}
#[test]
fn ancestor_cycle_finishes_before_child_escape_invalidates_directory_stamp() {
    let mut f = Fixture::new(&[("a", "new")]);
    fs::create_dir(f.media.join("A")).unwrap();
    fs::write(f.media.join("A/x"), b"child").unwrap();
    fs::write(f.media.join("B"), b"blocker").unwrap();
    let root = f.media.clone();
    replan(
        &mut f,
        root,
        &[("A", "B"), ("B", "A/x"), ("A/x", "outside")],
    );
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(
        report.state,
        OperationState::Completed,
        "{:?}",
        report.failure
    );
    assert_eq!(fs::read(f.media.join("B/x")).unwrap(), b"blocker");
    assert_eq!(fs::read(f.media.join("outside")).unwrap(), b"child");
}
