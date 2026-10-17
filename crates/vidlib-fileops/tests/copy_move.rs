#![cfg(target_os = "linux")]
use std::{fs, path::PathBuf};
use vidlib_fileops::{
    coordinator::{Coordinator, LeaseKind},
    journal::{Journal, OperationState},
    recovery::{apply_recovery, reconcile, RecoveryChoice},
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
        Self::with_source(names, None, false)
    }
    fn with_source(names: &[(&str, &str)], content: Option<&[u8]>, single_file: bool) -> Self {
        let dir = match std::env::var("VIDLIB_COPY_CRASH_ROOT") {
            Ok(root) => tempfile::Builder::new()
                .prefix("fixture-")
                .tempdir_in(root)
                .unwrap(),
            Err(_) => tempfile::tempdir().unwrap(),
        };
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
        if std::env::var("VIDLIB_COPY_BOUNDARY").as_deref() == Ok("mid-stream") {
            fs::write(media.join("a"), vec![0x5a; 16 * 1024 * 1024]).unwrap();
        }
        if let Some(content) = content {
            fs::write(media.join("a"), content).unwrap();
        }
        let coordinator = Coordinator::open(&dir.path().join("app")).unwrap();
        let inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(if single_file {
                media.join("a")
            } else {
                media.clone()
            })
            .unwrap()]),
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

// Removing the verified-copy executor must fail on real bytes and durable facts.
#[test]
fn copy_digest_and_metadata_verified() {
    let mut f = Fixture::new(&[("a", "b")]);
    let group = &mut f.plan.execution.groups[0];
    let first = group.steps[0].clone();
    let ExecutionAction::RenameEntry { source, target, .. } = first.action else {
        unreachable!()
    };
    let staging = target.with_file_name(".vidlib-staging-copy");
    let publish = StepId::new();
    let write = StepId::new();
    group.steps = vec![
        ExecutionStep {
            id: first.id,
            entry_id: first.entry_id,
            group_id: group.id,
            action: ExecutionAction::CreateStaging {
                staging: staging.clone(),
            },
        },
        ExecutionStep {
            id: write,
            entry_id: first.entry_id,
            group_id: group.id,
            action: ExecutionAction::WriteStaging {
                source,
                staging: Box::new(OwnedEntryPrestate {
                    entry: EntryPrestate {
                        path: staging.clone(),
                        identity: ExpectedIdentity::FromReceipt { step_id: first.id },
                    },
                    created_by: first.id,
                }),
            },
        },
        ExecutionStep {
            id: publish,
            entry_id: first.entry_id,
            group_id: group.id,
            action: ExecutionAction::PublishCopy {
                staging: OwnedEntryPrestate {
                    entry: EntryPrestate {
                        path: staging,
                        identity: ExpectedIdentity::FromReceipt { step_id: write },
                    },
                    created_by: first.id,
                },
                target,
                verified_by: write,
            },
        },
    ];
    let report = f.run(&mut PlatformFs::default(), &JobControl::default());
    assert!(report.is_ok(), "verified copy must execute: {report:?}");
    let report = report.unwrap();
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
    assert_eq!(fs::read(f.media.join("b")).unwrap(), b"fixture");
    let receipt = report.receipts.last().unwrap();
    assert_eq!(receipt.phase, events::CompletedPhase::CopyPublished);
    assert_eq!(receipt.digest.as_ref().unwrap().algorithm, "sha256");
    assert_eq!(
        receipt.digest.as_ref().unwrap().bytes,
        [
            0xf1, 0x6d, 0x5, 0xec, 0x6b, 0x29, 0x24, 0x8d, 0x2c, 0x61, 0xad, 0xb1, 0xe9, 0x26,
            0x3f, 0x78, 0xe4, 0xf7, 0xba, 0xce, 0x1b, 0x95, 0x50, 0x14, 0xa2, 0xd1, 0x78, 0x72,
            0xcf, 0xe4, 0x6, 0x4d
        ]
    );
    assert_eq!(
        receipt.after.as_ref().unwrap().modified_ns,
        f.plan.rows[0].stamp.modified_ns
    );
}

#[test]
fn folder_layout_preview_matches_all_descendants() {
    let mut f = Fixture::new(&[("a", "b")]);
    let dest = f.media.join("destination");
    fs::create_dir(&dest).unwrap();
    let grant = SourceGrant {
        id: GrantId::new(),
        path: dest.clone(),
        source: GrantSource::NativeSelection,
    };
    let inventory = &f.plan.execution.provenance.inventory;
    let mut recipe = f.plan.execution.provenance.recipe.clone();
    recipe.destination = Destination::CopyTo {
        grant_id: grant.id,
        layout: DestinationLayout::FolderTemplate {
            parts: vec![TemplatePart::Literal("Films/2026".into())],
        },
    };
    let evaluated = evaluate(inventory, &recipe, &f.plan.frozen_inputs).unwrap();
    let mut grants = evaluated.grants.clone();
    grants.grants.push(grant);
    let batch = resolve_destinations(evaluated, &recipe.destination, &grants).unwrap();
    assert_eq!(batch.rows[0].target_path, dest.join("Films/2026/a"));
    let ns = PlatformFs::default()
        .capture_namespace(&batch.grants, &destination::capture_candidates(&batch))
        .unwrap();
    f.plan = plan(inventory, &batch, &ns, f.plan.version).unwrap();
    assert_eq!(f.plan.counts.blocked, 0, "{:?}", f.plan.issues);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    assert_eq!(fs::read(dest.join("Films/2026/a")).unwrap(), b"fixture");
    assert!(f.media.join("a").exists());
}

fn destination_plan(f: &mut Fixture, target: &std::path::Path, moving: bool) {
    let inventory = &f.plan.execution.provenance.inventory;
    let grant = SourceGrant {
        id: GrantId::new(),
        path: target.to_owned(),
        source: GrantSource::NativeSelection,
    };
    let mut recipe = f.plan.execution.provenance.recipe.clone();
    recipe.destination = if moving {
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
    let batch = evaluate(inventory, &recipe, &f.plan.frozen_inputs).unwrap();
    let mut grants = batch.grants.clone();
    grants.grants.push(grant);
    let batch = resolve_destinations(batch, &recipe.destination, &grants).unwrap();
    let ns = PlatformFs::default()
        .capture_namespace(&batch.grants, &destination::capture_candidates(&batch))
        .unwrap();
    f.plan = plan(inventory, &batch, &ns, f.plan.version).unwrap();
    assert_eq!(f.plan.counts.blocked, 0, "{:?}", f.plan.issues);
}
#[test]
fn cross_volume_source_removed_only_after_verified_publish() {
    let mut f = Fixture::new(&[("a", "b")]);
    let dest = tempfile::tempdir_in("/dev/shm").unwrap();
    assert_ne!(
        PlatformFs::default()
            .inspect(&f.media)
            .unwrap()
            .stamp
            .physical_id
            .unwrap()
            .volume_identity,
        PlatformFs::default()
            .inspect(dest.path())
            .unwrap()
            .stamp
            .physical_id
            .unwrap()
            .volume_identity
    );
    destination_plan(&mut f, dest.path(), true);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(report.state, OperationState::Completed, "{report:?}");
    let published = report
        .receipts
        .iter()
        .position(|r| r.phase == events::CompletedPhase::CopyPublished)
        .unwrap();
    let removed = report
        .receipts
        .iter()
        .position(|r| r.phase == events::CompletedPhase::SourceRemoved)
        .unwrap();
    assert!(published < removed);
    assert_eq!(fs::read(dest.path().join("a")).unwrap(), b"fixture");
    assert!(!f.media.join("a").exists());
}
#[test]
fn owned_partial_staging_resumes_without_inventing_verified_digest() {
    use std::io::Write;
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    destination_plan(&mut f, &target, false);
    let mut native = PlatformFs::default();
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let op;
    {
        let _lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
        op = journal.begin(&f.plan.execution).unwrap();
        let create = &f.plan.execution.groups[0].steps[0];
        let write = &f.plan.execution.groups[0].steps[1];
        let ExecutionAction::CreateStaging { staging } = &create.action else {
            panic!()
        };
        journal.intent(op, create).unwrap();
        let mut output = native.create_staging(staging).unwrap();
        let durability = native.refresh_staging(&mut output).unwrap();
        journal
            .complete(
                op,
                &journal::StepReceipt {
                    step_id: create.id,
                    phase: events::CompletedPhase::StagingCreated,
                    before_path: None,
                    before: None,
                    after_path: Some(encode_path(staging)),
                    after: Some(output.ownership.stamp.clone()),
                    digest: None,
                    observed_ns: 0,
                    durability,
                },
            )
            .unwrap();
        journal.intent(op, write).unwrap();
        output.file.write_all(b"par").unwrap();
        output.file.sync_all().unwrap();
    }
    let lease = f.coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let pending = journal.operation(op).unwrap();
    assert!(pending.steps[1].receipts.is_empty());
    let preview = reconcile(&pending, &mut native).unwrap();
    assert!(
        preview.unresolved_groups.is_empty(),
        "owned partial write must be recoverable"
    );
    let nonce = preview.review_nonce();
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
    assert_eq!(fs::read(target.join("a")).unwrap(), b"fixture");
}

#[test]
fn owned_partial_staging_restore_removes_only_recorded_object() {
    use std::io::Write;
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    destination_plan(&mut f, &target, false);
    let mut native = PlatformFs::default();
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let op;
    {
        let _lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
        op = journal.begin(&f.plan.execution).unwrap();
        let create = &f.plan.execution.groups[0].steps[0];
        let write = &f.plan.execution.groups[0].steps[1];
        let ExecutionAction::CreateStaging { staging } = &create.action else {
            panic!()
        };
        journal.intent(op, create).unwrap();
        let mut output = native.create_staging(staging).unwrap();
        let durability = native.refresh_staging(&mut output).unwrap();
        journal
            .complete(
                op,
                &journal::StepReceipt {
                    step_id: create.id,
                    phase: events::CompletedPhase::StagingCreated,
                    before_path: None,
                    before: None,
                    after_path: Some(encode_path(staging)),
                    after: Some(output.ownership.stamp.clone()),
                    digest: None,
                    observed_ns: 0,
                    durability,
                },
            )
            .unwrap();
        journal.intent(op, write).unwrap();
        output.file.write_all(b"par").unwrap();
        output.file.sync_all().unwrap();
    }
    let lease = f.coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let pending = journal.operation(op).unwrap();
    assert!(pending.steps[1].receipts.is_empty());
    let preview = reconcile(&pending, &mut native).unwrap();
    assert!(
        preview.unresolved_groups.is_empty(),
        "owned partial write must be recoverable"
    );
    let nonce = preview.review_nonce();
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
    assert!(!target.join("a").exists());
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}

#[path = "support/copy_faults.rs"]
mod copy_faults;
#[test]
fn copy_phase_crash_child() {
    let Ok(boundary) = std::env::var("VIDLIB_COPY_BOUNDARY") else {
        return;
    };
    let mut f = Fixture::new(&[("a", "b")]);
    let target = PathBuf::from(std::env::var("VIDLIB_COPY_TARGET").unwrap());
    if boundary.starts_with("undo-") {
        if boundary.starts_with("undo-folder-") {
            directory_plan(&mut f, &target, boundary.contains("move"));
        } else {
            destination_plan(&mut f, &target, boundary == "undo-move-after-remove");
        }
        let original = f
            .run(&mut PlatformFs::default(), &JobControl::default())
            .unwrap();
        assert_eq!(original.state, OperationState::Completed);
        f.plan = undo_plan(&f, original.operation_id.unwrap());
        let mut adapter = copy_faults::CopyFs::new(if boundary.starts_with("undo-folder-") {
            "after-owned-directory-remove"
        } else {
            "after-remove"
        });
        adapter.native.namespace_epoch = f.plan.execution.provenance.namespace_epoch;
        f.run(&mut adapter, &JobControl::default()).unwrap();
        panic!("inverse deletion boundary not reached");
    }
    if boundary == "after-directory-remove" || boundary.starts_with("folder-") {
        directory_plan(&mut f, &target, true);
    } else {
        destination_plan(&mut f, &target, true);
    }
    f.run(
        &mut copy_faults::CopyFs::new(boundary.strip_prefix("folder-").unwrap_or(&boundary)),
        &JobControl::default(),
    )
    .unwrap();
    panic!("requested phase boundary was not reached");
}
fn crashed_copy(
    boundary: &str,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    PathBuf,
    Coordinator,
    Journal,
) {
    let root = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir_in("/dev/shm").unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "copy_phase_crash_child", "--nocapture"])
        .env("VIDLIB_COPY_CRASH_ROOT", root.path())
        .env("VIDLIB_COPY_BOUNDARY", boundary)
        .env("VIDLIB_COPY_TARGET", target.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86), "phase {boundary}");
    let fixture = fs::read_dir(root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let coordinator = Coordinator::open(&fixture.join("app")).unwrap();
    let journal = Journal::open(&coordinator.database_path()).unwrap();
    (root, target, fixture, coordinator, journal)
}
#[test]
fn published_and_removed_copy_restore_real_phase_crashes() {
    for boundary in [
        "before-publish",
        "after-publish",
        "after-rename",
        "after-remove",
    ] {
        let (_root, target, fixture, coordinator, mut journal) = crashed_copy(boundary);
        let pending = journal.pending().unwrap().remove(0);
        let originals = pending
            .steps
            .iter()
            .flat_map(|s| s.receipts.clone())
            .collect::<Vec<_>>();
        let mut native = PlatformFs::default();
        let preview = reconcile(&pending, &mut native).unwrap();
        assert!(preview.unresolved_groups.is_empty(), "{boundary}");
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        let result = apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Restore,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default(),
        );
        assert!(result.is_ok(), "{boundary}: {result:?}");
        assert_eq!(fs::read(fixture.join("media/a")).unwrap(), b"fixture");
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
        assert_eq!(fs::read_dir(fixture.join("media")).unwrap().count(), 1);
        let restored = journal.operation(pending.operation_id).unwrap();
        for receipt in originals {
            assert!(restored
                .steps
                .iter()
                .flat_map(|s| &s.receipts)
                .any(|r| r == &receipt));
        }
    }
}

fn undo_plan(f: &Fixture, original: OperationId) -> RenamePlan {
    let journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let record = journal.operation(original).unwrap();
    let context = undo::capture_context(&record).unwrap();
    let lease = f.coordinator.try_acquire(LeaseKind::Read).unwrap();
    let epochs = lease.status().unwrap().epochs;
    let mut ns = PlatformFs::default()
        .capture_namespace(&context.grants, &context.candidates)
        .unwrap();
    ns.namespace_epoch = epochs.namespace;
    ns.policy_epoch = epochs.policy;
    undo::plan_undo(
        &record,
        &ns,
        PlanRef {
            session_id: SessionId::new(),
            plan_id: PlanId::new(),
            revision: 2,
        },
    )
    .unwrap()
}
#[test]
fn copy_undo_checks_full_digest() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    destination_plan(&mut f, &target, false);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(report.state, OperationState::Completed);
    f.plan = undo_plan(&f, report.operation_id.unwrap());
    let undone = f.run(
        &mut PlatformFs {
            namespace_epoch: f.plan.execution.provenance.namespace_epoch,
            policy_epoch: f.plan.execution.provenance.policy_epoch,
            ..Default::default()
        },
        &JobControl::default(),
    );
    assert!(undone.is_ok(), "{undone:?}");
    assert_eq!(undone.unwrap().state, OperationState::Completed);
    assert!(!target.join("a").exists());
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
}
#[test]
fn cross_volume_move_undo_copies_back_with_new_identity() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = tempfile::tempdir_in("/dev/shm").unwrap();
    destination_plan(&mut f, target.path(), true);
    let original = f.plan.rows[0].stamp.physical_id.clone();
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(report.state, OperationState::Completed);
    f.plan = undo_plan(&f, report.operation_id.unwrap());
    let undone = f.run(
        &mut PlatformFs {
            namespace_epoch: f.plan.execution.provenance.namespace_epoch,
            policy_epoch: f.plan.execution.provenance.policy_epoch,
            ..Default::default()
        },
        &JobControl::default(),
    );
    assert!(undone.is_ok(), "{undone:?}");
    assert_eq!(undone.unwrap().state, OperationState::Completed);
    assert!(!target.path().join("a").exists());
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
    assert_ne!(
        PlatformFs::default()
            .inspect(&f.media.join("a"))
            .unwrap()
            .stamp
            .physical_id,
        original
    );
}

#[test]
fn copy_undo_refuses_changed_digest_even_with_original_size_and_time() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    destination_plan(&mut f, &target, false);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(report.state, OperationState::Completed);
    let p = target.join("a");
    let metadata = fs::metadata(&p).unwrap();
    fs::write(&p, b"changed").unwrap();
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(metadata.modified().unwrap())
                .set_accessed(metadata.accessed().unwrap()),
        )
        .unwrap();
    f.plan = undo_plan(&f, report.operation_id.unwrap());
    let result = f.run(
        &mut PlatformFs {
            namespace_epoch: f.plan.execution.provenance.namespace_epoch,
            ..Default::default()
        },
        &JobControl::default(),
    );
    assert!(!matches!(result,Ok(ref r) if r.state==OperationState::Completed));
    assert_eq!(fs::read(&p).unwrap(), b"changed");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
}
#[test]
fn created_directory_undo_requires_empty() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    let grant = SourceGrant {
        id: GrantId::new(),
        path: target.clone(),
        source: GrantSource::NativeSelection,
    };
    let inventory = &f.plan.execution.provenance.inventory;
    let mut recipe = f.plan.execution.provenance.recipe.clone();
    recipe.destination = Destination::CopyTo {
        grant_id: grant.id,
        layout: DestinationLayout::FolderTemplate {
            parts: vec![TemplatePart::Literal("Films/2026".into())],
        },
    };
    let batch = evaluate(inventory, &recipe, &f.plan.frozen_inputs).unwrap();
    let mut grants = batch.grants.clone();
    grants.grants.push(grant);
    let batch = resolve_destinations(batch, &recipe.destination, &grants).unwrap();
    let ns = PlatformFs::default()
        .capture_namespace(&grants, &destination::capture_candidates(&batch))
        .unwrap();
    f.plan = plan(inventory, &batch, &ns, f.plan.version).unwrap();
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(report.state, OperationState::Completed);
    f.plan = undo_plan(&f, report.operation_id.unwrap());
    let result = f.run(
        &mut PlatformFs {
            namespace_epoch: f.plan.execution.provenance.namespace_epoch,
            ..Default::default()
        },
        &JobControl::default(),
    );
    assert!(
        matches!(result,Ok(ref r) if r.state==OperationState::Completed),
        "{result:?}"
    );
    assert!(!target.join("Films").exists());
}

fn directory_plan(f: &mut Fixture, target: &std::path::Path, moving: bool) {
    directory_plan_unchecked(f, target, moving);
    assert_eq!(f.plan.counts.blocked, 0, "{:?}", f.plan.issues);
}
fn directory_plan_unchecked(f: &mut Fixture, target: &std::path::Path, moving: bool) {
    fs::create_dir_all(f.media.join("Tree/sub")).unwrap();
    fs::write(f.media.join("Tree/sub/child.mkv"), b"child bytes").unwrap();
    let inventory = scan(
        &InventoryRequest::new(vec![RootBinding::new(f.media.clone()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let id = inventory
        .entries
        .iter()
        .find(|e| e.absolute_path == f.media.join("Tree"))
        .unwrap()
        .id;
    let grant = SourceGrant {
        id: GrantId::new(),
        path: target.to_owned(),
        source: GrantSource::NativeSelection,
    };
    let mut recipe = f.plan.execution.provenance.recipe.clone();
    recipe.selection = vec![id];
    recipe.numbering_order = vec![id];
    recipe.destination = if moving {
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
    let frozen = FrozenInputs {
        inventory_id: inventory.id,
        generation: inventory.generation,
        primary_order: vec![id],
        ..f.plan.frozen_inputs.clone()
    };
    let batch = evaluate(&inventory, &recipe, &frozen).unwrap();
    let mut grants = batch.grants.clone();
    grants.grants.push(grant);
    let batch = resolve_destinations(batch, &recipe.destination, &grants).unwrap();
    let ns = PlatformFs::default()
        .capture_namespace(&grants, &destination::capture_candidates(&batch))
        .unwrap();
    f.plan = plan(&inventory, &batch, &ns, f.plan.version).unwrap();
}
#[test]
fn selected_folder_copy_preserves_all_descendants_and_owned_undo() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    directory_plan(&mut f, &target, false);
    assert_eq!(f.plan.rows[0].carried_descendants, 2);
    let result = f.run(&mut PlatformFs::default(), &JobControl::default());
    assert!(
        matches!(result,Ok(ref r) if r.state==OperationState::Completed),
        "{result:?}"
    );
    assert_eq!(
        fs::read(target.join("Tree/sub/child.mkv")).unwrap(),
        b"child bytes"
    );
    assert_eq!(
        fs::read(f.media.join("Tree/sub/child.mkv")).unwrap(),
        b"child bytes"
    );
    f.plan = undo_plan(&f, result.unwrap().operation_id.unwrap());
    let result = f.run(
        &mut PlatformFs {
            namespace_epoch: f.plan.execution.provenance.namespace_epoch,
            ..Default::default()
        },
        &JobControl::default(),
    );
    assert!(
        matches!(result,Ok(ref r) if r.state==OperationState::Completed),
        "{result:?}"
    );
    assert!(!target.join("Tree").exists());
}
#[test]
fn selected_folder_cross_volume_move_removes_empty_sources_and_undo_restores_tree() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = tempfile::tempdir_in("/dev/shm").unwrap();
    directory_plan(&mut f, target.path(), true);
    let result = f.run(&mut PlatformFs::default(), &JobControl::default());
    assert!(
        matches!(result,Ok(ref r) if r.state==OperationState::Completed),
        "{result:?}"
    );
    assert!(!f.media.join("Tree").exists());
    assert_eq!(
        fs::read(target.path().join("Tree/sub/child.mkv")).unwrap(),
        b"child bytes"
    );
    f.plan = undo_plan(&f, result.unwrap().operation_id.unwrap());
    let result = f.run(
        &mut PlatformFs {
            namespace_epoch: f.plan.execution.provenance.namespace_epoch,
            ..Default::default()
        },
        &JobControl::default(),
    );
    assert!(
        matches!(result,Ok(ref r) if r.state==OperationState::Completed),
        "{result:?}"
    );
    assert!(!target.path().join("Tree").exists());
    assert_eq!(
        fs::read(f.media.join("Tree/sub/child.mkv")).unwrap(),
        b"child bytes"
    );
}
#[test]
fn same_volume_move_uses_native_rename_preserving_identity() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    destination_plan(&mut f, &target, true);
    let identity = f.plan.rows[0].stamp.physical_id.clone();
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(report.state, OperationState::Completed);
    assert_eq!(
        PlatformFs::default()
            .inspect(&target.join("a"))
            .unwrap()
            .stamp
            .physical_id,
        identity
    );
    assert_eq!(report.receipts.len(), 1);
}

// A parent replacement after executor preflight must never acquire new authority.
#[test]
fn staging_creation_binds_reviewed_native_parent() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    destination_plan(&mut f, &target, false);
    let mut adapter = copy_faults::CopyFs::new("replace-parent-before-create");
    let _ = f.run(&mut adapter, &JobControl::default());
    assert_eq!(
        adapter.creates.get(),
        0,
        "replacement parent must receive no mutation"
    );
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
}

#[test]
fn copied_directory_metadata_is_applied_after_descendants() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    directory_plan(&mut f, &target, false);
    let expected = f
        .plan
        .execution
        .provenance
        .inventory
        .entries
        .iter()
        .find(|e| e.absolute_path == f.media.join("Tree/sub"))
        .unwrap()
        .stamp
        .clone();
    let result = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(result.state, OperationState::Completed, "{result:?}");
    let actual = PlatformFs::default()
        .inspect(&target.join("Tree/sub"))
        .unwrap()
        .stamp;
    assert_eq!(
        actual.modified_ns, expected.modified_ns,
        "directory timestamp must survive child creation"
    );
    assert_eq!(
        fs::read(target.join("Tree/sub/child.mkv")).unwrap(),
        b"child bytes"
    );
}

#[test]
fn folder_restore_recreates_parents_before_children_and_finalizes_metadata() {
    let (_root, target, fixture, coordinator, mut journal) = crashed_copy("after-directory-remove");
    let pending = journal.pending().unwrap().remove(0);
    assert!(!fixture.join("media/Tree/sub").exists());
    let expected = pending
        .steps
        .iter()
        .find_map(|s| match &s.step.action {
            ExecutionAction::RenameEntry {
                source:
                    EntryPrestate {
                        path,
                        identity:
                            ExpectedIdentity::DirectoryAfterEffects {
                                original,
                                created_by: None,
                                ..
                            },
                    },
                ..
            } if path.ends_with("Tree/sub") => Some(original.clone()),
            _ => None,
        })
        .unwrap();
    let mut native = PlatformFs::default();
    let preview = reconcile(&pending, &mut native).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let result = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut native,
        &JobControl::default(),
    );
    if result.is_err() {
        for effect in journal
            .operation(pending.operation_id)
            .unwrap()
            .recovery_effects
        {
            eprintln!(
                "inverse {:?}: {:?}",
                effect.inverse.state, effect.inverse.step.action
            );
        }
    }
    assert!(result.is_ok(), "directory Restore must execute: {result:?}");
    assert_eq!(
        fs::read(fixture.join("media/Tree/sub/child.mkv")).unwrap(),
        b"child bytes"
    );
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    let actual = native
        .inspect(&fixture.join("media/Tree/sub"))
        .unwrap()
        .stamp;
    assert_eq!(actual.modified_ns, expected.modified_ns);
    assert_ne!(actual.physical_id, expected.physical_id);
}

#[test]
fn copy_restore_crash_child() {
    let Ok(fixture) = std::env::var("VIDLIB_RESTORE_FIXTURE") else {
        return;
    };
    let coordinator = Coordinator::open(&PathBuf::from(fixture).join("app")).unwrap();
    let mut journal = Journal::open(&coordinator.database_path()).unwrap();
    let mut adapter = copy_faults::CopyFs::new(&std::env::var("VIDLIB_RESTORE_BOUNDARY").unwrap());
    let op = journal.pending().unwrap().remove(0);
    let preview = reconcile(&op, &mut adapter).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut adapter,
        &JobControl::default(),
    )
    .unwrap();
    panic!("restore crash boundary was not reached");
}
#[test]
fn folder_restore_resumes_durable_group_graph_after_real_inverse_crashes() {
    for boundary in [
        "before-create",
        "before-stream",
        "after-publish",
        "before-metadata",
        "after-metadata",
    ] {
        let (_root, target, fixture, coordinator, mut journal) =
            crashed_copy("after-directory-remove");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "copy_restore_crash_child", "--nocapture"])
            .env("VIDLIB_RESTORE_FIXTURE", &fixture)
            .env("VIDLIB_RESTORE_BOUNDARY", boundary)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86), "{boundary}");
        let op = journal.pending().unwrap().remove(0);
        assert!(!op.recovery_effects.is_empty());
        assert!(op.restore_context.is_some());
        let mut native = PlatformFs::default();
        let preview = reconcile(&op, &mut native).unwrap();
        assert!(
            preview.unresolved_groups.is_empty(),
            "{boundary}: {:?}",
            preview
                .actions
                .iter()
                .filter(|a| a.disposition == recovery::RecoveryDisposition::Unresolved)
                .map(|a| (a.step_id, a.phase))
                .collect::<Vec<_>>()
        );
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        let result = apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Restore,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default(),
        );
        assert!(result.is_ok(), "{boundary}: {result:?}");
        assert_eq!(
            fs::read(fixture.join("media/Tree/sub/child.mkv")).unwrap(),
            b"child bytes"
        );
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
        assert!(journal
            .operation(op.operation_id)
            .unwrap()
            .steps
            .iter()
            .all(|s| matches!(
                s.state,
                journal::StepState::Planned | journal::StepState::Compensated
            )));
    }
}

#[test]
fn recovery_group_missing_canonical_dependency_is_rejected() {
    let (_root, _target, fixture, coordinator, journal) = crashed_copy("after-directory-remove");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "copy_restore_crash_child"])
        .env("VIDLIB_RESTORE_FIXTURE", &fixture)
        .env("VIDLIB_RESTORE_BOUNDARY", "before-create")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let original = journal.pending().unwrap().remove(0);
    let db = rusqlite::Connection::open(coordinator.database_path()).unwrap();
    db.execute("DELETE FROM recovery_dependencies WHERE rowid = (SELECT max(rowid) FROM recovery_dependencies)", []).unwrap();
    assert!(
        journal.operation(original.operation_id).is_err(),
        "missing inverse dependency must invalidate durable graph authority"
    );
}

#[test]
fn interrupted_copy_and_move_undo_restore_deleted_published_copy() {
    for boundary in ["undo-copy-after-remove", "undo-move-after-remove"] {
        let (_root, target, fixture, coordinator, mut journal) = crashed_copy(boundary);
        let pending = journal.pending().unwrap().remove(0);
        assert!(!target.path().join("a").exists());
        let mut native = PlatformFs::default();
        let preview = reconcile(&pending, &mut native).unwrap();
        assert!(preview.unresolved_groups.is_empty());
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        let result = apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Restore,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default(),
        );
        assert!(result.is_ok(), "{boundary}: {result:?}");
        assert_eq!(fs::read(target.path().join("a")).unwrap(), b"fixture");
        if boundary == "undo-copy-after-remove" {
            assert_eq!(fs::read(fixture.join("media/a")).unwrap(), b"fixture");
        } else {
            assert!(!fixture.join("media/a").exists());
        }
    }
}

#[test]
fn interrupted_folder_undo_restore_recreates_deleted_destination_tree() {
    for boundary in ["undo-folder-copy", "undo-folder-move"] {
        let (_root, target, fixture, coordinator, mut journal) = crashed_copy(boundary);
        let pending = journal.pending().unwrap().remove(0);
        assert!(!target.path().join("Tree/sub").exists());
        let mut native = PlatformFs::default();
        let preview = reconcile(&pending, &mut native).unwrap();
        assert!(preview.unresolved_groups.is_empty());
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        let result = apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Restore,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default(),
        );
        assert!(result.is_ok(), "{boundary}: {result:?}");
        assert_eq!(
            fs::read(target.path().join("Tree/sub/child.mkv")).unwrap(),
            b"child bytes"
        );
        if boundary == "undo-folder-copy" {
            assert!(fixture.join("media/Tree/sub/child.mkv").exists());
        } else {
            assert!(!fixture.join("media/Tree").exists());
        }
    }
}

#[test]
fn same_volume_move_with_new_parent_preserves_file_and_subtree_identities() {
    for directory in [false, true] {
        let mut f = Fixture::new(&[("a", "b")]);
        let target = f._dir.path().join("target");
        fs::create_dir(&target).unwrap();
        if directory {
            directory_plan(&mut f, &target, true);
        } else {
            destination_plan(&mut f, &target, true);
        }
        let mut recipe = f.plan.execution.provenance.recipe.clone();
        let Destination::MoveTo { layout, .. } = &mut recipe.destination else {
            panic!()
        };
        *layout = DestinationLayout::FolderTemplate {
            parts: vec![TemplatePart::Literal("Films/2026".into())],
        };
        let inventory = &f.plan.execution.provenance.inventory;
        let batch = resolve_destinations(
            evaluate(inventory, &recipe, &f.plan.frozen_inputs).unwrap(),
            &recipe.destination,
            &f.plan.grants,
        )
        .unwrap();
        let ns = PlatformFs::default()
            .capture_namespace(&f.plan.grants, &destination::capture_candidates(&batch))
            .unwrap();
        f.plan = plan(inventory, &batch, &ns, f.plan.version).unwrap();
        let original = f.plan.rows[0].stamp.physical_id.clone();
        let target = f.plan.rows[0].target_path.clone();
        let result = f
            .run(&mut PlatformFs::default(), &JobControl::default())
            .unwrap();
        assert_eq!(result.state, OperationState::Completed, "{result:?}");
        assert_eq!(
            PlatformFs::default()
                .inspect(&target)
                .unwrap()
                .stamp
                .physical_id,
            original,
            "same-volume move must preserve native identity through new parents"
        );
        f.plan = undo_plan(&f, result.operation_id.unwrap());
        let result = f
            .run(
                &mut PlatformFs {
                    namespace_epoch: f.plan.execution.provenance.namespace_epoch,
                    ..Default::default()
                },
                &JobControl::default(),
            )
            .unwrap();
        assert_eq!(result.state, OperationState::Completed, "{result:?}");
        assert!(!f._dir.path().join("target/Films").exists());
        assert_eq!(
            PlatformFs::default()
                .inspect(&f.media.join(if directory { "Tree" } else { "a" }))
                .unwrap()
                .stamp
                .physical_id,
            original
        );
    }
}

#[test]
fn publication_then_remove_failure_preserves_fact() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = tempfile::tempdir_in("/dev/shm").unwrap();
    destination_plan(&mut f, target.path(), true);
    let report = f
        .run(
            &mut copy_faults::CopyFs::new("remove-permission"),
            &JobControl::default(),
        )
        .unwrap();
    assert_ne!(report.state, OperationState::Completed);
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
    let journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let record = journal.operation(report.operation_id.unwrap()).unwrap();
    assert!(record
        .steps
        .iter()
        .flat_map(|s| &s.receipts)
        .any(|r| r.phase == events::CompletedPhase::CopyPublished
            && r.digest.as_ref().unwrap().algorithm == "sha256"));
    assert!(!record
        .steps
        .iter()
        .flat_map(|s| &s.receipts)
        .any(|r| r.phase == events::CompletedPhase::SourceRemoved));
    assert!(journal
        .events_after(0, 200)
        .unwrap()
        .items
        .iter()
        .any(|e| e.phase == events::CompletedPhase::CopyPublished));
}
#[test]
fn disk_full_cleans_only_owned_staging() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join(".vidlib-staging-unknown"), b"unknown owner").unwrap();
    destination_plan(&mut f, &target, false);
    let report = f
        .run(
            &mut copy_faults::CopyFs::new("disk-full"),
            &JobControl::default(),
        )
        .unwrap();
    assert_eq!(report.state, OperationState::Failed);
    assert!(report
        .failure
        .as_ref()
        .unwrap()
        .message
        .contains("No space left on device"));
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
    assert_eq!(
        fs::read(target.join(".vidlib-staging-unknown")).unwrap(),
        b"unknown owner"
    );
    assert_eq!(fs::read_dir(target).unwrap().count(), 1);
}
#[test]
fn publication_conflict_and_permission_failure_preserve_source_and_external_winner() {
    for boundary in ["publication-race", "publication-permission"] {
        let mut f = Fixture::new(&[("a", "b")]);
        let target = f._dir.path().join("target");
        fs::create_dir(&target).unwrap();
        destination_plan(&mut f, &target, false);
        let report = f
            .run(
                &mut copy_faults::CopyFs::new(boundary),
                &JobControl::default(),
            )
            .unwrap();
        assert_ne!(report.state, OperationState::Completed);
        assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
        if boundary == "publication-race" {
            assert_eq!(fs::read(target.join("a")).unwrap(), b"external winner");
        } else {
            assert!(!target.join("a").exists());
        }
    }
}
#[test]
fn actual_mid_stream_crash_resumes_or_restores_only_owned_bytes() {
    for choice in [RecoveryChoice::Resume, RecoveryChoice::Restore] {
        let (_root, target, fixture, coordinator, mut journal) = crashed_copy("mid-stream");
        let pending = journal.pending().unwrap().remove(0);
        let staging = fs::read_dir(target.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let length = fs::metadata(&staging).unwrap().len();
        assert!(
            length > 0 && length < 16 * 1024 * 1024,
            "must terminate during actual writes: {length}"
        );
        let mut native = PlatformFs::default();
        let preview = reconcile(&pending, &mut native).unwrap();
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        apply_recovery(
            preview.review(nonce).unwrap(),
            choice,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default(),
        )
        .unwrap();
        let bytes = vec![0x5a; 16 * 1024 * 1024];
        if choice == RecoveryChoice::Resume {
            assert_eq!(fs::read(target.path().join("a")).unwrap(), bytes);
            assert!(!fixture.join("media/a").exists());
        } else {
            assert_eq!(fs::read(fixture.join("media/a")).unwrap(), bytes);
            assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
        }
    }
}
#[test]
fn cancellation_at_group_barrier_retains_completed_group() {
    let mut f = Fixture::new(&[("a", "x"), ("b", "y")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    destination_plan(&mut f, &target, false);
    assert_eq!(f.plan.execution.groups.len(), 2);
    let control = JobControl::default();
    let mut adapter = copy_faults::CopyFs::new("cancel-after-publish");
    adapter.cancel = Some(control.clone());
    let report = f.run(&mut adapter, &control).unwrap();
    assert_eq!(report.state, OperationState::Cancelled);
    assert_eq!(report.completed_groups.len(), 1);
    assert_eq!(fs::read_dir(target).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&f.media).unwrap().count(), 2);
}

#[test]
fn directory_after_effects_refuses_unrelated_attribute_changes() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    directory_plan(&mut f, &target, false);
    let result = f
        .run(
            &mut copy_faults::CopyFs::new("changed-directory-attributes"),
            &JobControl::default(),
        )
        .unwrap();
    assert_ne!(
        result.state,
        OperationState::Completed,
        "child receipts do not authorize changed directory attributes"
    );
}

#[test]
fn cancellation_during_actual_streaming_restores_owned_partial_staging() {
    let bytes = vec![0x5a; 16 * 1024 * 1024];
    let mut f = Fixture::with_source(&[("a", "b")], Some(&bytes), false);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    destination_plan(&mut f, &target, false);
    let control = JobControl::default();
    let mut adapter = copy_faults::CopyFs::new("cancel-mid-stream");
    adapter.cancel = Some(control.clone());
    let report = f.run(&mut adapter, &control).unwrap();
    assert!(control.is_cancelled());
    assert_eq!(report.state, OperationState::Cancelled, "{report:?}");
    assert!(report.completed_groups.is_empty());
    assert_eq!(fs::read(f.media.join("a")).unwrap(), bytes);
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}
#[test]
fn individual_file_grant_restore_never_sets_parent_metadata() {
    let mut f = Fixture::with_source(&[("a", "b")], None, true);
    let target = tempfile::tempdir_in("/dev/shm").unwrap();
    destination_plan(&mut f, target.path(), true);
    let report = f
        .run(
            &mut copy_faults::CopyFs::new("remove-permission"),
            &JobControl::default(),
        )
        .unwrap();
    assert_ne!(report.state, OperationState::Completed);
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
    let pending = Journal::open(&f.coordinator.database_path())
        .unwrap()
        .operation(report.operation_id.unwrap())
        .unwrap();
    assert!(pending.recovery_effects.iter().all(|e| !matches!(&e.inverse.step.action, ExecutionAction::SetMetadata { entry, .. } if entry.path == f.media)));
}
#[test]
fn unknown_after_create_is_preserved_and_never_invented_as_owned() {
    let (_root, target, fixture, coordinator, mut journal) = crashed_copy("after-create");
    let pending = journal.pending().unwrap().remove(0);
    assert!(pending.steps.iter().all(|s| s.receipts.is_empty()));
    let unknown = fs::read_dir(target.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let identity = PlatformFs::default()
        .inspect(&unknown)
        .unwrap()
        .stamp
        .physical_id;
    for choice in [RecoveryChoice::Resume, RecoveryChoice::Restore] {
        let mut native = PlatformFs::default();
        let preview = reconcile(
            &journal.operation(pending.operation_id).unwrap(),
            &mut native,
        )
        .unwrap();
        assert!(!preview.unresolved_groups.is_empty());
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        assert!(apply_recovery(
            preview.review(nonce).unwrap(),
            choice,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default()
        )
        .is_err());
        assert_eq!(
            PlatformFs::default()
                .inspect(&unknown)
                .unwrap()
                .stamp
                .physical_id,
            identity
        );
        assert_eq!(fs::read(fixture.join("media/a")).unwrap(), b"fixture");
    }
}
#[test]
fn owned_before_stream_and_verified_write_crashes_resume_or_restore() {
    for boundary in ["before-stream", "after-stream", "before-publish"] {
        for choice in [RecoveryChoice::Resume, RecoveryChoice::Restore] {
            let (_root, target, fixture, coordinator, mut journal) = crashed_copy(boundary);
            let pending = journal.pending().unwrap().remove(0);
            assert!(pending.steps.iter().any(|s| s
                .receipts
                .iter()
                .any(|r| r.phase == events::CompletedPhase::StagingCreated)));
            let mut native = PlatformFs::default();
            let preview = reconcile(&pending, &mut native).unwrap();
            let nonce = preview.review_nonce();
            let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
            apply_recovery(
                preview.review(nonce).unwrap(),
                choice,
                &lease,
                &mut journal,
                &mut native,
                &JobControl::default(),
            )
            .unwrap();
            if choice == RecoveryChoice::Resume {
                assert_eq!(fs::read(target.path().join("a")).unwrap(), b"fixture");
                assert!(!fixture.join("media/a").exists());
            } else {
                assert_eq!(fs::read(fixture.join("media/a")).unwrap(), b"fixture");
                assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
            }
        }
    }
}

#[test]
fn inverse_directory_creation_gap_preserves_unknown_native_object() {
    let (_root, target, fixture, coordinator, mut journal) = crashed_copy("after-directory-remove");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "copy_restore_crash_child"])
        .env("VIDLIB_RESTORE_FIXTURE", &fixture)
        .env("VIDLIB_RESTORE_BOUNDARY", "after-directory")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let op = journal.pending().unwrap().remove(0);
    let created = fixture.join("media/Tree/sub");
    let id = PlatformFs::default()
        .inspect(&created)
        .unwrap()
        .stamp
        .physical_id;
    let mut native = PlatformFs::default();
    let preview = reconcile(&op, &mut native).unwrap();
    assert!(!preview.unresolved_groups.is_empty());
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut native,
        &JobControl::default()
    )
    .is_err());
    assert_eq!(native.inspect(&created).unwrap().stamp.physical_id, id);
    assert_eq!(
        fs::read(target.path().join("Tree/sub/child.mkv")).unwrap(),
        b"child bytes"
    );
    assert!(journal
        .operation(op.operation_id)
        .unwrap()
        .steps
        .iter()
        .any(|s| s.state == journal::StepState::CompletionDurable));
}
#[test]
fn directory_finalization_refuses_unknown_missing_or_replaced_children() {
    for mutation in ["unknown", "missing", "replaced"] {
        let (_root, _target, fixture, coordinator, mut journal) =
            crashed_copy("after-directory-remove");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "copy_restore_crash_child"])
            .env("VIDLIB_RESTORE_FIXTURE", &fixture)
            .env("VIDLIB_RESTORE_BOUNDARY", "before-metadata")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));
        let directory = fixture.join("media/Tree/sub");
        let child = directory.join("child.mkv");
        match mutation {
            "unknown" => fs::write(directory.join("external"), b"unowned").unwrap(),
            "missing" => fs::remove_file(&child).unwrap(),
            _ => {
                fs::rename(&child, directory.join("old")).unwrap();
                fs::write(&child, b"external replacement").unwrap();
            }
        }
        let op = journal.pending().unwrap().remove(0);
        let mut native = PlatformFs::default();
        let preview = reconcile(&op, &mut native).unwrap();
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        assert!(
            apply_recovery(
                preview.review(nonce).unwrap(),
                RecoveryChoice::Restore,
                &lease,
                &mut journal,
                &mut native,
                &JobControl::default()
            )
            .is_err(),
            "{mutation}"
        );
        if mutation == "unknown" {
            assert_eq!(fs::read(directory.join("external")).unwrap(), b"unowned");
        }
        if mutation == "replaced" {
            assert_eq!(fs::read(&child).unwrap(), b"external replacement");
        }
    }
}
#[test]
fn changed_source_staging_or_publication_never_authorizes_source_removal() {
    for object in ["source", "staging", "publication", "identity"] {
        let boundary = if object == "publication" {
            "after-publish"
        } else {
            "before-publish"
        };
        let (_root, target, fixture, coordinator, mut journal) = crashed_copy(boundary);
        let changed = match object {
            "source" | "identity" => fixture.join("media/a"),
            "publication" => target.path().join("a"),
            _ => fs::read_dir(target.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        };
        let mut native = PlatformFs::default();
        let old = native.inspect(&changed).unwrap().stamp;
        if object == "identity" {
            fs::rename(&changed, changed.with_extension("old")).unwrap();
        }
        fs::write(&changed, b"changed").unwrap();
        fs::File::options()
            .write(true)
            .open(&changed)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(
                std::time::UNIX_EPOCH
                    + std::time::Duration::from_nanos(old.modified_ns.unwrap().try_into().unwrap()),
            ))
            .unwrap();
        let op = journal.pending().unwrap().remove(0);
        let preview = reconcile(&op, &mut native).unwrap();
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        assert!(
            apply_recovery(
                preview.review(nonce).unwrap(),
                RecoveryChoice::Resume,
                &lease,
                &mut journal,
                &mut native,
                &JobControl::default()
            )
            .is_err(),
            "{object}"
        );
        let op = journal.operation(op.operation_id).unwrap();
        assert!(op.steps.iter().all(|s| s
            .receipts
            .iter()
            .all(|r| r.phase != events::CompletedPhase::SourceRemoved)));
        assert!(
            op.steps
                .iter()
                .flat_map(|s| &s.receipts)
                .filter_map(|r| r.after_path.as_ref())
                .filter_map(|p| decode_path(p).ok())
                .chain(std::iter::once(changed.clone()))
                .any(|p| fs::read(p).is_ok_and(|b| b == b"changed")),
            "changed bytes must survive: {object}"
        );
    }
}
#[test]
fn owned_directory_with_unrelated_child_survives_copy_undo() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    directory_plan(&mut f, &target, false);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    f.plan = undo_plan(&f, report.operation_id.unwrap());
    fs::write(target.join("Tree/sub/external"), b"unowned").unwrap();
    let mut native = PlatformFs {
        namespace_epoch: f.plan.execution.provenance.namespace_epoch,
        ..Default::default()
    };
    assert!(
        !matches!(f.run(&mut native,&JobControl::default()),Ok(r) if r.state == OperationState::Completed)
    );
    assert_eq!(
        fs::read(target.join("Tree/sub/external")).unwrap(),
        b"unowned"
    );
}

#[test]
fn destination_companion_collision_and_undo_share_final_primary_group() {
    let mut f = Fixture::new(&[("old.mkv", "x"), ("old.en.idx", "y"), ("old.en.sub", "z")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("Final.en.idx"), b"external collision").unwrap();
    let inventory = &f.plan.execution.provenance.inventory;
    let mut recipe = f.plan.execution.provenance.recipe.clone();
    recipe.companions = CompanionPolicy::MatchExact {
        associations: vec![],
    };
    recipe.rules = vec![Rule {
        id: "final".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::FixedStem {
            value: "Final".into(),
        },
    }];
    recipe.collision_policy = CollisionPolicy::Suffix {
        separator: "-".into(),
        max_attempts: 5,
    };
    let grant = SourceGrant {
        id: GrantId::new(),
        path: target.clone(),
        source: GrantSource::NativeSelection,
    };
    recipe.destination = Destination::CopyTo {
        grant_id: grant.id,
        layout: DestinationLayout::PreserveHierarchy,
    };
    prepare_associations(inventory, &mut recipe).unwrap();
    let mut frozen = f.plan.frozen_inputs.clone();
    frozen.primary_order = recipe.numbering_order.clone();
    let batch = evaluate(inventory, &recipe, &frozen).unwrap();
    let mut grants = batch.grants.clone();
    grants.grants.push(grant);
    let batch = resolve_destinations(batch, &recipe.destination, &grants).unwrap();
    let ns = PlatformFs::default()
        .capture_namespace(&grants, &destination::capture_candidates(&batch))
        .unwrap();
    f.plan = plan(inventory, &batch, &ns, f.plan.version).unwrap();
    assert_eq!(f.plan.counts.blocked, 0, "{:?}", f.plan.issues);
    assert_eq!(f.plan.execution.groups.len(), 1);
    assert_eq!(f.plan.frozen_inputs.primary_order.len(), 1);
    let report = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(report.state, OperationState::Completed);
    for (source, destination) in [
        ("old.mkv", "Final-1.mkv"),
        ("old.en.idx", "Final-1.en.idx"),
        ("old.en.sub", "Final-1.en.sub"),
    ] {
        assert_eq!(
            fs::read(target.join(destination)).unwrap(),
            fs::read(f.media.join(source)).unwrap()
        );
    }
    f.plan = undo_plan(&f, report.operation_id.unwrap());
    assert!(f.plan.rows.iter().all(|row| row
        .explanations
        .iter()
        .any(|e| e.rule_id == "undo-delete-created-copy")));
    let mut native = PlatformFs {
        namespace_epoch: f.plan.execution.provenance.namespace_epoch,
        ..Default::default()
    };
    assert_eq!(
        f.run(&mut native, &JobControl::default()).unwrap().state,
        OperationState::Completed
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(
        fs::read(target.join("Final.en.idx")).unwrap(),
        b"external collision"
    );
}
#[test]
fn folder_copy_blocks_special_descendants_and_protected_children() {
    for special in ["symlink", "fifo", "protected"] {
        let mut f = Fixture::new(&[("a", "b")]);
        let target = tempfile::tempdir_in("/dev/shm").unwrap();
        fs::create_dir_all(f.media.join("Tree/sub")).unwrap();
        let entry = f.media.join("Tree/sub").join(if special == "protected" {
            ".vidlib-staging-external"
        } else {
            "special"
        });
        match special {
            "symlink" => std::os::unix::fs::symlink(f.media.join("a"), &entry).unwrap(),
            "fifo" => {
                let status = std::process::Command::new("mkfifo")
                    .arg(&entry)
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            _ => fs::write(&entry, b"protected").unwrap(),
        }
        directory_plan_unchecked(&mut f, target.path(), true);
        assert!(
            f.plan.counts.blocked > 0 || f.plan.execution.groups.is_empty(),
            "{special}: {:?}",
            f.plan.issues
        );
        assert!(fs::symlink_metadata(&entry).is_ok());
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    }
}

#[test]
fn recovery_group_canonical_effect_set_cannot_be_rewritten() {
    for mutation in [
        "drop-effect",
        "rewrite-metadata",
        "drop-manifest",
        "wrong-parent",
        "cycle",
    ] {
        let (_root, _target, fixture, coordinator, journal) =
            crashed_copy("after-directory-remove");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "copy_restore_crash_child"])
            .env("VIDLIB_RESTORE_FIXTURE", &fixture)
            .env("VIDLIB_RESTORE_BOUNDARY", "before-create")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));
        let op = journal.pending().unwrap().remove(0);
        let db = rusqlite::Connection::open(coordinator.database_path()).unwrap();
        match mutation {
            "drop-effect" => {
                db.execute("DELETE FROM recovery_effects WHERE rowid=(SELECT max(rowid) FROM recovery_effects)",[]).unwrap();
            }
            "drop-manifest" => {
                db.execute("DELETE FROM recovery_groups", []).unwrap();
            }
            "rewrite-metadata" => {
                let effect = op
                    .recovery_effects
                    .iter()
                    .rev()
                    .find(|e| matches!(e.inverse.step.action, ExecutionAction::SetMetadata { .. }))
                    .unwrap();
                let payload: String = db
                    .query_row(
                        "SELECT compiled FROM steps WHERE id=?1",
                        [effect.inverse.step.id.to_string()],
                        |r| r.get(0),
                    )
                    .unwrap();
                let mut value: serde_json::Value = serde_json::from_str(&payload).unwrap();
                value["action"]["SetMetadata"]["changes"] = serde_json::json!([]);
                db.execute(
                    "UPDATE steps SET compiled=?1 WHERE id=?2",
                    rusqlite::params![value.to_string(), effect.inverse.step.id.to_string()],
                )
                .unwrap();
            }
            "wrong-parent" => {
                let (id,payload):(String,String) = db.query_row("SELECT inverse_step,parents FROM recovery_effects WHERE parents LIKE '%created_by%' LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                let mut value: serde_json::Value = serde_json::from_str(&payload).unwrap();
                for parent in value.as_array_mut().unwrap() {
                    if parent.get("created_by").is_some() {
                        parent["created_by"] = serde_json::json!(op.steps[0].step.id);
                    }
                }
                db.execute(
                    "UPDATE recovery_effects SET parents=?1 WHERE inverse_step=?2",
                    rusqlite::params![value.to_string(), id],
                )
                .unwrap();
            }
            _ => {
                let id = op.recovery_effects[0].inverse.step.id;
                db.execute(
                    "INSERT INTO recovery_dependencies VALUES(?1,?2,?2)",
                    rusqlite::params![op.operation_id.to_string(), id.to_string()],
                )
                .unwrap();
            }
        }
        assert!(
            journal.operation(op.operation_id).is_err(),
            "accepted altered compiled group: {mutation}"
        );
    }
}

#[test]
fn copy_closes_writer_before_independent_hash_and_publication() {
    let mut f = Fixture::new(&[("a", "b")]);
    let target = f._dir.path().join("target");
    fs::create_dir(&target).unwrap();
    destination_plan(&mut f, &target, false);
    assert_eq!(
        f.run(
            &mut copy_faults::CopyFs::new("closed-writer"),
            &JobControl::default()
        )
        .unwrap()
        .state,
        OperationState::Completed
    );
}
#[test]
fn forward_directory_creation_gap_is_unknown_and_preserved() {
    let (_root, target, _fixture, coordinator, mut journal) =
        crashed_copy("folder-after-directory");
    let op = journal.pending().unwrap().remove(0);
    let unknown = target.path().join("Tree");
    let mut native = PlatformFs::default();
    let expected = native.inspect(&unknown).unwrap().stamp.physical_id;
    for choice in [RecoveryChoice::Resume, RecoveryChoice::Restore] {
        let preview = reconcile(&journal.operation(op.operation_id).unwrap(), &mut native).unwrap();
        assert!(!preview.unresolved_groups.is_empty());
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        assert!(apply_recovery(
            preview.review(nonce).unwrap(),
            choice,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default()
        )
        .is_err());
        assert_eq!(
            native.inspect(&unknown).unwrap().stamp.physical_id,
            expected
        );
    }
}
#[test]
fn restoration_refuses_recreated_source_file_or_directory() {
    for folder in [false, true] {
        let (_root, _target, fixture, coordinator, mut journal) = crashed_copy(if folder {
            "after-directory-remove"
        } else {
            "after-remove"
        });
        let recreated = fixture.join(if folder { "media/Tree/sub" } else { "media/a" });
        if folder {
            fs::create_dir(&recreated).unwrap();
            fs::write(recreated.join("external"), b"external").unwrap();
        } else {
            fs::write(&recreated, b"external").unwrap();
        }
        let op = journal.pending().unwrap().remove(0);
        let mut native = PlatformFs::default();
        let identity = native.inspect(&recreated).unwrap().stamp.physical_id;
        let preview = reconcile(&op, &mut native).unwrap();
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        assert!(apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Restore,
            &lease,
            &mut journal,
            &mut native,
            &JobControl::default()
        )
        .is_err());
        assert_eq!(
            native.inspect(&recreated).unwrap().stamp.physical_id,
            identity
        );
        assert_eq!(
            fs::read(if folder {
                recreated.join("external")
            } else {
                recreated
            })
            .unwrap(),
            b"external"
        );
    }
}

#[test]
fn individual_file_grant_cross_volume_move_undo_restores_original_path() {
    let mut f = Fixture::with_source(&[("a", "b")], None, true);
    let target = tempfile::tempdir_in("/dev/shm").unwrap();
    destination_plan(&mut f, target.path(), true);
    let completed = f
        .run(&mut PlatformFs::default(), &JobControl::default())
        .unwrap();
    assert_eq!(completed.state, OperationState::Completed);
    f.plan = undo_plan(&f, completed.operation_id.unwrap());
    let mut native = PlatformFs {
        namespace_epoch: f.plan.execution.provenance.namespace_epoch,
        ..Default::default()
    };
    assert_eq!(
        f.run(&mut native, &JobControl::default()).unwrap().state,
        OperationState::Completed
    );
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
    assert!(!target.path().join("a").exists());
}

#[test]
fn directory_proof_refuses_changed_descendant_attributes_or_unstatable_children() {
    for boundary in ["changed-descendant-attributes", "unstatable-child"] {
        let mut f = Fixture::new(&[("a", "b")]);
        let target = f._dir.path().join("target");
        fs::create_dir(&target).unwrap();
        directory_plan(&mut f, &target, false);
        let report = f
            .run(
                &mut copy_faults::CopyFs::new(boundary),
                &JobControl::default(),
            )
            .unwrap();
        assert_ne!(report.state, OperationState::Completed, "{boundary}");
        assert_eq!(
            fs::read(f.media.join("Tree/sub/child.mkv")).unwrap(),
            b"child bytes"
        );
    }
}

#[test]
fn selected_parent_and_child_copy_move_keep_final_hierarchy_and_undo() {
    for moving in [false, true] {
        let mut f = Fixture::new(&[("a", "b")]);
        let target = tempfile::tempdir_in("/dev/shm").unwrap();
        directory_plan(&mut f, target.path(), moving);
        let inventory = f.plan.execution.provenance.inventory.clone();
        let mut recipe = f.plan.execution.provenance.recipe.clone();
        let child = inventory
            .entries
            .iter()
            .find(|e| e.absolute_path == f.media.join("Tree/sub/child.mkv"))
            .unwrap()
            .id;
        recipe.selection.push(child);
        recipe.numbering_order.push(child);
        recipe.rules = vec![Rule {
            id: "final".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::FixedStem {
                value: "Renamed".into(),
            },
        }];
        let mut frozen = f.plan.frozen_inputs.clone();
        frozen.primary_order = recipe.numbering_order.clone();
        let batch = evaluate(&inventory, &recipe, &frozen).unwrap();
        let batch = resolve_destinations(batch, &recipe.destination, &f.plan.grants).unwrap();
        let ns = PlatformFs::default()
            .capture_namespace(&batch.grants, &destination::capture_candidates(&batch))
            .unwrap();
        f.plan = plan(&inventory, &batch, &ns, f.plan.version).unwrap();
        assert_eq!(f.plan.counts.blocked, 0, "{:?}", f.plan.issues);
        assert_eq!(
            f.plan
                .rows
                .iter()
                .find(|r| r.entry_id == child)
                .unwrap()
                .target_path,
            target.path().join("Renamed/sub/Renamed.mkv")
        );
        let report = f
            .run(&mut PlatformFs::default(), &JobControl::default())
            .unwrap();
        assert_eq!(report.state, OperationState::Completed);
        assert_eq!(
            fs::read(target.path().join("Renamed/sub/Renamed.mkv")).unwrap(),
            b"child bytes"
        );
        assert!(!target.path().join("Tree").exists());
        assert_eq!(f.media.join("Tree").exists(), !moving);
        f.plan = undo_plan(&f, report.operation_id.unwrap());
        let mut native = PlatformFs {
            namespace_epoch: f.plan.execution.provenance.namespace_epoch,
            ..Default::default()
        };
        assert_eq!(
            f.run(&mut native, &JobControl::default()).unwrap().state,
            OperationState::Completed
        );
        assert!(!target.path().join("Renamed").exists());
        assert_eq!(
            fs::read(f.media.join("Tree/sub/child.mkv")).unwrap(),
            b"child bytes"
        );
    }
}

#[test]
fn selected_nested_directories_carry_children_after_parent_collision() {
    for moving in [false, true] {
        let mut f = Fixture::new(&[("a", "b")]);
        let target = tempfile::tempdir_in("/dev/shm").unwrap();
        directory_plan(&mut f, target.path(), moving);
        fs::write(target.path().join("Renamed"), b"external collision").unwrap();
        let inventory = f.plan.execution.provenance.inventory.clone();
        let mut recipe = f.plan.execution.provenance.recipe.clone();
        let sub = inventory
            .entries
            .iter()
            .find(|e| e.absolute_path == f.media.join("Tree/sub"))
            .unwrap()
            .id;
        recipe.selection.push(sub);
        recipe.numbering_order.push(sub);
        recipe.rules = vec![Rule {
            id: "final".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::FixedStem {
                value: "Renamed".into(),
            },
        }];
        recipe.collision_policy = CollisionPolicy::Suffix {
            separator: "-".into(),
            max_attempts: 5,
        };
        let mut frozen = f.plan.frozen_inputs.clone();
        frozen.primary_order = recipe.numbering_order.clone();
        let batch = evaluate(&inventory, &recipe, &frozen).unwrap();
        let batch = resolve_destinations(batch, &recipe.destination, &f.plan.grants).unwrap();
        let ns = PlatformFs::default()
            .capture_namespace(&batch.grants, &destination::capture_candidates(&batch))
            .unwrap();
        f.plan = plan(&inventory, &batch, &ns, f.plan.version).unwrap();
        assert_eq!(f.plan.counts.blocked, 0, "{:?}", f.plan.issues);
        assert_eq!(
            f.plan
                .rows
                .iter()
                .find(|r| r.entry_id == sub)
                .unwrap()
                .target_path,
            target.path().join("Renamed-1/Renamed")
        );
        let report = f
            .run(&mut PlatformFs::default(), &JobControl::default())
            .unwrap();
        assert_eq!(report.state, OperationState::Completed);
        assert_eq!(
            fs::read(target.path().join("Renamed-1/Renamed/child.mkv")).unwrap(),
            b"child bytes"
        );
        assert!(!target.path().join("Renamed-1/sub").exists());
        assert_eq!(f.media.join("Tree").exists(), !moving);
        f.plan = undo_plan(&f, report.operation_id.unwrap());
        let mut native = PlatformFs {
            namespace_epoch: f.plan.execution.provenance.namespace_epoch,
            ..Default::default()
        };
        assert_eq!(
            f.run(&mut native, &JobControl::default()).unwrap().state,
            OperationState::Completed
        );
        assert!(!target.path().join("Renamed-1").exists());
        assert_eq!(
            fs::read(target.path().join("Renamed")).unwrap(),
            b"external collision"
        );
        assert_eq!(
            fs::read(f.media.join("Tree/sub/child.mkv")).unwrap(),
            b"child bytes"
        );
    }
}

fn recover_folder_crash(boundary: &str, choice: RecoveryChoice) {
    let (_root, target, fixture, coordinator, mut journal) = crashed_copy(boundary);
    let op = journal.pending().unwrap().remove(0);
    let originals = op
        .steps
        .iter()
        .flat_map(|s| s.receipts.clone())
        .collect::<Vec<_>>();
    let mut native = PlatformFs::default();
    let preview = reconcile(&op, &mut native).unwrap();
    assert!(
        preview.unresolved_groups.is_empty(),
        "{boundary}: {:?}",
        preview.actions
    );
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let report = apply_recovery(
        preview.review(nonce).unwrap(),
        choice,
        &lease,
        &mut journal,
        &mut native,
        &JobControl::default(),
    )
    .unwrap_or_else(|e| panic!("{boundary} {choice:?}: {e:?}"));
    let completed = journal.operation(op.operation_id).unwrap();
    for fact in originals {
        assert!(completed
            .steps
            .iter()
            .flat_map(|s| &s.receipts)
            .any(|r| r == &fact));
    }
    assert!(journal.pending().unwrap().is_empty());
    match choice {
        RecoveryChoice::Resume => {
            assert_eq!(report.state, OperationState::Completed);
            assert!(!fixture.join("media/Tree").exists());
            assert_eq!(
                fs::read(target.path().join("Tree/sub/child.mkv")).unwrap(),
                b"child bytes"
            );
        }
        RecoveryChoice::Restore => {
            assert_eq!(report.state, OperationState::Cancelled);
            assert_eq!(
                fs::read(fixture.join("media/Tree/sub/child.mkv")).unwrap(),
                b"child bytes"
            );
            assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
        }
    }
}
#[test]
fn folder_directory_quarantine_receipt_gap_restores() {
    recover_folder_crash("folder-after-directory-quarantine", RecoveryChoice::Restore);
}
#[test]
fn folder_move_resume_after_child_effects() {
    for boundary in ["folder-after-publish", "folder-after-remove"] {
        recover_folder_crash(boundary, RecoveryChoice::Resume);
    }
}
#[test]
fn folder_move_resume_after_source_directory_removal() {
    recover_folder_crash("after-directory-remove", RecoveryChoice::Resume);
}
#[test]
fn folder_inverse_directory_removal_receipt_gap_restores() {
    let (_root, target, fixture, coordinator, mut journal) = crashed_copy("after-directory-remove");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "copy_restore_crash_child", "--nocapture"])
        .env("VIDLIB_RESTORE_FIXTURE", &fixture)
        .env("VIDLIB_RESTORE_BOUNDARY", "after-owned-directory-remove")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let op = journal.pending().unwrap().remove(0);
    assert!(op.recovery_effects.iter().any(|e| e.inverse.state
        == journal::StepState::IntentDurable
        && matches!(
            e.inverse.step.action,
            ExecutionAction::RemoveOwnedEmptyDirectory { .. }
        )));
    let mut native = PlatformFs::default();
    let preview = reconcile(&op, &mut native).unwrap();
    assert!(
        preview.unresolved_groups.is_empty(),
        "owned directory removal must be provable: {:?}",
        preview.actions
    );
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
    assert!(journal.pending().unwrap().is_empty());
    assert_eq!(
        fs::read(fixture.join("media/Tree/sub/child.mkv")).unwrap(),
        b"child bytes"
    );
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
}
