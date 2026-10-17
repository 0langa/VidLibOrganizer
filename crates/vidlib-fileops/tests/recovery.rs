#![cfg(target_os = "linux")]
#[path = "support/crash_child.rs"]
mod crash_child;
#[path = "support/legacy_schema.rs"]
mod legacy_schema;
use crash_child::{CrashFs, Fixture};
use std::{fs, path::PathBuf, process::Command};
use vidlib_fileops::{
    coordinator::{Coordinator, LeaseKind},
    journal::{Journal, OperationState},
    recovery::{apply_recovery, reconcile, RecoveryChoice, RecoveryDisposition},
    *,
};
use vidlib_inventory::JobControl;

#[test]
fn crash_child_entry() {
    if std::env::var_os("VIDLIB_CRASH_ROOT").is_none() {
        return;
    }
    let case = std::env::var("VIDLIB_CRASH_CYCLE").unwrap();
    let mut f = Fixture::new(if case == "groups" {
        &[("a", "new"), ("b", "other")]
    } else if case == "1" {
        &[("a", "b"), ("b", "a")]
    } else {
        &[("a", "new")]
    });
    let phase = std::env::var("VIDLIB_CRASH_BOUNDARY").unwrap();
    if phase == "prepared" {
        let _lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
        let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
        journal.begin(&f.plan.execution).unwrap();
        std::process::exit(86);
    }
    if [
        "stage",
        "publish",
        "remove_source",
        "cleanup",
        "directory",
        "directory_cleanup",
        "metadata",
    ]
    .contains(&phase.as_str())
    {
        crash_child::persisted_phase_child(
            &mut f,
            &phase,
            std::env::var("VIDLIB_CRASH_INDEX").unwrap() == "1",
        );
    }
    if std::env::var("VIDLIB_CRASH_CYCLE").unwrap() == "hierarchy" {
        fs::create_dir(f.media.join("A")).unwrap();
        fs::write(f.media.join("A/x"), b"child").unwrap();
        fs::write(f.media.join("B"), b"blocker").unwrap();
        let root = f.media.clone();
        crash_child::replan(
            &mut f,
            root,
            &[("A", "B"), ("B", "A/x"), ("A/x", "outside")],
        );
    }
    if std::env::var("VIDLIB_CRASH_BOUNDARY").unwrap() == "published" {
        use std::io::Write;
        use vidlib_fileops::{
            events::{CompletedPhase, ContentDigest},
            journal::StepReceipt,
        };
        use vidlib_inventory::{encode_path, ids::StepId};
        use vidlib_rename::{
            EntryPrestate, ExecutionAction, ExecutionStep, ExpectedIdentity, OwnedEntryPrestate,
        };
        let original = f.plan.execution.groups[0].steps[0].clone();
        let ExecutionAction::RenameEntry { source, target, .. } = original.action else {
            unreachable!()
        };
        let staging = f.media.join(".vidlib-staging-copy");
        let stage = ExecutionStep {
            id: StepId::new(),
            entry_id: original.entry_id,
            group_id: original.group_id,
            action: ExecutionAction::StageCopy {
                source: source.clone(),
                staging: staging.clone(),
            },
        };
        let publish = ExecutionStep {
            id: StepId::new(),
            entry_id: original.entry_id,
            group_id: original.group_id,
            action: ExecutionAction::PublishCopy {
                staging: OwnedEntryPrestate {
                    entry: EntryPrestate {
                        path: staging.clone(),
                        identity: ExpectedIdentity::FromReceipt { step_id: stage.id },
                    },
                    created_by: stage.id,
                },
                target,
                verified_by: stage.id,
            },
        };
        f.plan.execution.groups[0].steps = vec![stage.clone(), publish.clone()];
        let _lease = f.coordinator.try_acquire(LeaseKind::Write).unwrap();
        let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
        let mut native = PlatformFs::default();
        let op = journal.begin(&f.plan.execution).unwrap();
        journal.intent(op, &stage).unwrap();
        let mut copy = native.create_staging(&staging).unwrap();
        copy.file.write_all(b"fixture").unwrap();
        let durability = native.refresh_staging(&mut copy).unwrap();
        let stamp = native.inspect(&source.path).unwrap().stamp;
        journal
            .complete(
                op,
                &StepReceipt {
                    step_id: stage.id,
                    phase: CompletedPhase::CopyStaged,
                    before_path: Some(encode_path(&source.path)),
                    after_path: Some(encode_path(&staging)),
                    before: Some(stamp),
                    after: Some(copy.ownership.stamp.clone()),
                    digest: Some(ContentDigest {
                        algorithm: "md5".into(),
                        bytes: md5::compute(b"fixture").0.to_vec(),
                    }),
                    observed_ns: 1,
                    durability,
                },
            )
            .unwrap();
        journal.intent(op, &publish).unwrap();
        native
            .publish_staging(&copy.ownership, &f.media.join("new"))
            .unwrap();
        std::process::exit(86);
    }
    let mut adapter = CrashFs {
        outcome_durability: None,
        after_capture: None,
        native: PlatformFs::default(),
        boundary: std::env::var("VIDLIB_CRASH_BOUNDARY").unwrap(),
        index: std::env::var("VIDLIB_CRASH_INDEX")
            .unwrap()
            .parse()
            .unwrap(),
        count: 0,
    };
    f.run(&mut adapter, &JobControl::default()).unwrap();
    panic!("child missed crash boundary");
}
fn crashed(
    boundary: &str,
    index: usize,
    cycle: bool,
) -> (tempfile::TempDir, PathBuf, Coordinator, Journal) {
    crashed_case(boundary, index, if cycle { "1" } else { "0" })
}
fn crashed_case(
    boundary: &str,
    index: usize,
    case: &str,
) -> (tempfile::TempDir, PathBuf, Coordinator, Journal) {
    let dir = tempfile::tempdir().unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child_entry", "--nocapture"])
        .env("VIDLIB_CRASH_ROOT", dir.path())
        .env("VIDLIB_CRASH_BOUNDARY", boundary)
        .env("VIDLIB_CRASH_INDEX", index.to_string())
        .env("VIDLIB_CRASH_CYCLE", case)
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(86),
        "real child must terminate inside native phase"
    );
    let root = fs::read_dir(dir.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let coordinator = Coordinator::open(&root.join("app")).unwrap();
    let journal = Journal::open(&coordinator.database_path()).unwrap();
    (dir, root.join("media"), coordinator, journal)
}
#[test]
fn crash_before_and_after_each_phase_reconciles() {
    for boundary in ["before", "after"] {
        for index in 1..=3 {
            let (_dir, media, coordinator, mut journal) = crashed(boundary, index, true);
            let op = journal.pending().unwrap().remove(0);
            let preview = reconcile(&op, &mut PlatformFs::default()).unwrap();
            let again = reconcile(&op, &mut PlatformFs::default()).unwrap();
            assert_eq!(again.actions, preview.actions);
            assert_eq!(
                preview.actions.len(),
                3,
                "each compiled phase must be classified"
            );
            assert!(preview.unresolved_groups.is_empty());
            let nonce = preview.review_nonce();
            let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
            let result = apply_recovery(
                preview.review(nonce).unwrap(),
                RecoveryChoice::Resume,
                &lease,
                &mut journal,
                &mut PlatformFs::default(),
                &JobControl::default(),
            )
            .unwrap();
            assert_eq!(result.state, OperationState::Completed);
            assert_eq!(fs::read(media.join("a")).unwrap(), b"b");
            assert_eq!(fs::read(media.join("b")).unwrap(), b"fixture");
            assert!(journal.pending().unwrap().is_empty());
            assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 3);
        }
    }
}
#[test]
fn temporary_names_are_reported() {
    let (_dir, _media, _coordinator, journal) = crashed("after", 1, true);
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    assert_eq!(
        preview.temporary_names.len(),
        1,
        "compiled live temporary reservation must be visible"
    );
    assert!(preview.temporary_names[0].exists());
}
#[test]
fn unknown_target_prevents_blind_resume() {
    let (_dir, media, coordinator, mut journal) = crashed("before", 1, false);
    fs::write(media.join("new"), b"unowned").unwrap();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    assert_eq!(preview.unresolved_groups.len(), 1);
    assert_eq!(
        preview.actions[0].disposition,
        RecoveryDisposition::Unresolved
    );
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Resume,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default()
    )
    .is_err());
    assert_eq!(fs::read(media.join("new")).unwrap(), b"unowned");
    assert_eq!(fs::read(media.join("a")).unwrap(), b"fixture");
}
#[test]
fn compensation_stops_without_deleting_unowned_entries() {
    let (_dir, media, coordinator, mut journal) = crashed("after", 1, false);
    let unowned_target = media.join("a");
    fs::write(&unowned_target, b"new occupant").unwrap();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let result = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default(),
    );
    assert!(result.is_err() || result.unwrap().state == OperationState::RecoveryRequired);
    assert!(unowned_target.exists());
    assert_eq!(fs::read(unowned_target).unwrap(), b"new occupant");
    assert_eq!(
        journal.pending().unwrap().len(),
        1,
        "unresolved ownership retains recovery gate"
    );
}

#[test]
fn restore_reverses_only_current_cycle_group() {
    for index in 1..=3 {
        let (_dir, media, coordinator, mut journal) = crashed("after", index, true);
        let preview =
            reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        let report = apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Restore,
            &lease,
            &mut journal,
            &mut PlatformFs::default(),
            &JobControl::default(),
        )
        .expect("verified cycle must restore");
        assert_eq!(report.state, OperationState::Cancelled);
        assert_eq!(fs::read(media.join("a")).unwrap(), b"fixture");
        assert_eq!(fs::read(media.join("b")).unwrap(), b"b");
    }
}
#[test]
fn published_phase_without_receipt_is_reconciled_once() {
    let (_dir, media, coordinator, mut journal) = crashed("published", 1, false);
    let op = journal.pending().unwrap().remove(0);
    let preview = reconcile(&op, &mut PlatformFs::default()).unwrap();
    let second = reconcile(&op, &mut PlatformFs::default()).unwrap();
    assert_eq!(preview.actions, second.actions);
    assert_eq!(
        preview
            .actions
            .iter()
            .filter(|a| a.disposition == RecoveryDisposition::Applied)
            .count(),
        1,
        "verified published effect must be recognized"
    );
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let report = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Resume,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default(),
    )
    .unwrap();
    assert_eq!(report.state, OperationState::Completed);
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 2);
    assert_eq!(fs::read(media.join("a")).unwrap(), b"fixture");
    assert_eq!(fs::read(media.join("new")).unwrap(), b"fixture");
    assert!(journal.pending().unwrap().is_empty());
}

#[test]
fn recovery_crash_child_entry() {
    let Ok(root) = std::env::var("VIDLIB_RECOVERY_ROOT") else {
        return;
    };
    let coordinator = Coordinator::open(&PathBuf::from(root).join("app")).unwrap();
    let mut journal = Journal::open(&coordinator.database_path()).unwrap();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let boundary = std::env::var("VIDLIB_CRASH_BOUNDARY").unwrap();
    let reset = boundary == "post_reset";
    let mut native = CrashFs {
        outcome_durability: None,
        after_capture: None,
        native: PlatformFs::default(),
        boundary: if reset || boundary == "resolved_gap" {
            format!("{boundary}:{}", coordinator.database_path().display())
        } else {
            boundary
        },
        index: std::env::var("VIDLIB_CRASH_INDEX")
            .unwrap()
            .parse()
            .unwrap(),
        count: 0,
    };
    apply_recovery(
        preview.review(nonce).unwrap(),
        if reset {
            RecoveryChoice::Resume
        } else {
            RecoveryChoice::Restore
        },
        &lease,
        &mut journal,
        &mut native,
        &JobControl::default(),
    )
    .unwrap();
    panic!("missed inverse crash");
}
#[test]
fn crashes_during_restore_do_not_repeat_an_inverse() {
    for boundary in ["before", "after"] {
        for index in 1..=3 {
            let (_dir, media, coordinator, mut journal) = crashed("after", 3, true);
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "recovery_crash_child_entry", "--nocapture"])
                .env("VIDLIB_RECOVERY_ROOT", media.parent().unwrap())
                .env("VIDLIB_CRASH_BOUNDARY", boundary)
                .env("VIDLIB_CRASH_INDEX", index.to_string())
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86));
            let preview =
                reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
            let nonce = preview.review_nonce();
            let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
            let report = apply_recovery(
                preview.review(nonce).unwrap(),
                RecoveryChoice::Restore,
                &lease,
                &mut journal,
                &mut PlatformFs::default(),
                &JobControl::default(),
            )
            .unwrap();
            assert_eq!(report.state, OperationState::Cancelled);
            assert_eq!(fs::read(media.join("a")).unwrap(), b"fixture");
            assert_eq!(fs::read(media.join("b")).unwrap(), b"b");
            assert_eq!(
                journal.events_after(0, 200).unwrap().items.len(),
                6,
                "each forward/inverse has exactly one fact"
            );
            if boundary == "before" {
                let db = rusqlite::Connection::open(coordinator.database_path()).unwrap();
                assert!(
                    db.query_row(
                        "SELECT count(*) FROM recovery_audit WHERE resolution='NotApplied'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap()
                        > 0,
                    "inverse retry requires durable fresh non-application evidence"
                );
            }
        }
    }
}
fn crashed_hierarchy() -> (tempfile::TempDir, PathBuf, Coordinator, Journal) {
    let dir = tempfile::tempdir().unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child_entry", "--nocapture"])
        .env("VIDLIB_CRASH_ROOT", dir.path())
        .env("VIDLIB_CRASH_BOUNDARY", "after")
        .env("VIDLIB_CRASH_INDEX", "4")
        .env("VIDLIB_CRASH_CYCLE", "hierarchy")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let root = fs::read_dir(dir.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let media = root.join("media");
    let coordinator = Coordinator::open(&root.join("app")).unwrap();
    let journal = Journal::open(&coordinator.database_path()).unwrap();
    (dir, media, coordinator, journal)
}
#[test]
fn directory_parent_restores_after_child_namespace_changes() {
    let (_dir, media, coordinator, mut journal) = crashed_hierarchy();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let report = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default(),
    )
    .expect("child effects require verified material refresh for parent inverse");
    assert_eq!(report.state, OperationState::Cancelled);
    assert_eq!(fs::read(media.join("A/x")).unwrap(), b"child");
    assert_eq!(fs::read(media.join("B")).unwrap(), b"blocker");
    assert!(!media.join("outside").exists());
}
#[test]
fn lost_drive_is_unresolved_and_reconnect_requires_fresh_review() {
    let (_dir, media, coordinator, mut journal) = crashed("after", 1, false);
    let parked = media.with_file_name("disconnected");
    fs::rename(&media, &parked).unwrap();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    assert_eq!(preview.unresolved_groups.len(), 1);
    fs::rename(&parked, &media).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Resume,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default()
    )
    .is_err());
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    assert_eq!(
        apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Resume,
            &lease,
            &mut journal,
            &mut PlatformFs::default(),
            &JobControl::default()
        )
        .unwrap()
        .state,
        OperationState::Completed
    );
}

#[test]
fn recovery_reset_is_audited_without_fake_completion() {
    let (_dir, media, coordinator, mut journal) = crashed("before", 1, false);
    let db = rusqlite::Connection::open(coordinator.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER stop_resumed_intent BEFORE UPDATE OF state ON steps WHEN NEW.state='IntentDurable' BEGIN SELECT RAISE(ABORT,'stop after non-application resolution'); END;").unwrap();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Resume,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default()
    )
    .is_err());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM recovery_audit WHERE resolution='NotApplied'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(journal.events_after(0, 200).unwrap().items.is_empty());
    assert_eq!(fs::read(media.join("a")).unwrap(), b"fixture");
    assert!(!media.join("new").exists());
}
#[test]
fn failed_recovery_audit_never_authorizes_a_filesystem_write() {
    let (_dir, media, coordinator, mut journal) = crashed("before", 1, false);
    let db = rusqlite::Connection::open(coordinator.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_resolution BEFORE INSERT ON recovery_audit BEGIN SELECT RAISE(ABORT,'resolution durability failure'); END;").unwrap();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Resume,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default()
    )
    .is_err());
    assert_eq!(fs::read(media.join("a")).unwrap(), b"fixture");
    assert!(!media.join("new").exists());
    assert!(journal.events_after(0, 200).unwrap().items.is_empty());
    assert_eq!(
        journal.pending().unwrap()[0].steps[0].state,
        vidlib_fileops::journal::StepState::IntentDurable
    );
}
#[test]
fn recovery_requires_its_own_exclusive_lease_and_nonce() {
    let (_dir, media, _coordinator, mut journal) = crashed("after", 1, false);
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    assert!(preview
        .review(vidlib_inventory::ids::StepId::new())
        .is_err());
    let other = tempfile::tempdir().unwrap();
    let other_coordinator = Coordinator::open(other.path()).unwrap();
    let lease = other_coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default()
    )
    .is_err());
    assert!(!media.join("a").exists());
    assert_eq!(fs::read(media.join("new")).unwrap(), b"fixture");
}

#[test]
fn every_persisted_phase_reports_evidence_or_explicit_unknown_ownership() {
    for phase in [
        "stage",
        "publish",
        "remove_source",
        "cleanup",
        "directory",
        "directory_cleanup",
        "metadata",
    ] {
        for after in [false, true] {
            let (_dir, media, _coordinator, journal) = crashed(phase, usize::from(after), false);
            let op = journal.pending().unwrap().remove(0);
            let preview = reconcile(&op, &mut PlatformFs::default()).unwrap();
            let expected = if !after {
                RecoveryDisposition::NotApplied
            } else if matches!(phase, "stage" | "directory") {
                RecoveryDisposition::Unresolved
            } else {
                RecoveryDisposition::Applied
            };
            assert_eq!(
                preview.actions.last().unwrap().disposition,
                expected,
                "phase={phase}, after={after}"
            );
            assert_eq!(
                preview.unresolved_groups.len(),
                usize::from(expected == RecoveryDisposition::Unresolved)
            );
            if phase == "stage" && after {
                assert!(
                    preview
                        .temporary_names
                        .contains(&media.join(".vidlib-staging-copy")),
                    "unknown staging must still be reported by exact name"
                );
                assert_eq!(
                    fs::read(media.join(".vidlib-staging-copy")).unwrap(),
                    b"fixture"
                );
            }
            if phase == "directory" && after {
                assert!(media.join("created").is_dir());
            }
        }
    }
}
#[test]
fn observed_removal_metadata_and_publication_commit_one_fact() {
    for phase in [
        "publish",
        "remove_source",
        "cleanup",
        "directory_cleanup",
        "metadata",
    ] {
        let (_dir, _media, coordinator, mut journal) = crashed(phase, 1, false);
        let op = journal.pending().unwrap().remove(0);
        let count = op.steps.len();
        let preview = reconcile(&op, &mut PlatformFs::default()).unwrap();
        let nonce = preview.review_nonce();
        let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
        let report = apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Resume,
            &lease,
            &mut journal,
            &mut PlatformFs::default(),
            &JobControl::default(),
        )
        .unwrap();
        assert_eq!(report.state, OperationState::Completed, "{phase}");
        assert_eq!(journal.events_after(0, 200).unwrap().items.len(), count);
        assert!(journal.pending().unwrap().is_empty());
    }
}

#[test]
fn legacy_interruption_is_read_only_unresolved_without_item_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let coordinator = Coordinator::open(dir.path()).unwrap();
    {
        let lease = coordinator.try_acquire(LeaseKind::Write).unwrap();
        let _interrupted = lease.begin_legacy_mutation().unwrap();
    }
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let inspection = vidlib_fileops::recovery::inspect_legacy_interruption(&lease).unwrap();
    assert!(
        inspection.is_some_and(|record| record.unresolved),
        "unknown legacy interruption must be visible without inventing compiled steps"
    );
    let journal = Journal::open(&coordinator.database_path()).unwrap();
    assert!(journal.pending().unwrap().is_empty());
    assert!(journal.events_after(0, 200).unwrap().items.is_empty());
    assert!(lease.status().unwrap().interrupted_legacy_write);
}

#[test]
fn directory_namespace_change_after_capture_cannot_mint_fresh_authority() {
    let (_dir, media, coordinator, mut journal) = crashed_hierarchy();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let mut adapter = CrashFs {
        outcome_durability: None,
        native: PlatformFs::default(),
        boundary: "none".into(),
        index: 0,
        count: 0,
        after_capture: Some(media.join("B/intruder")),
    };
    let result = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut adapter,
        &JobControl::default(),
    );
    assert!(
        result.is_err(),
        "changed directory stamp after verified capture must not become fresh inverse authority"
    );
    assert_eq!(
        fs::read(media.join("B/intruder")).unwrap(),
        b"unowned intruder"
    );
    assert!(!journal.pending().unwrap().is_empty());
}
#[test]
fn unowned_child_blocks_directory_material_refresh() {
    let (_dir, media, coordinator, mut journal) = crashed_hierarchy();
    fs::write(media.join("B/intruder"), b"unowned intruder").unwrap();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default()
    )
    .is_err());
    assert_eq!(
        fs::read(media.join("B/intruder")).unwrap(),
        b"unowned intruder"
    );
}

#[test]
fn prepared_process_crash_resumes_without_invented_effects() {
    let (_dir, media, coordinator, mut journal) = crashed("prepared", 0, false);
    assert!(journal.events_after(0, 200).unwrap().items.is_empty());
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    assert_eq!(preview.actions[0].disposition, RecoveryDisposition::Planned);
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    assert_eq!(
        apply_recovery(
            preview.review(nonce).unwrap(),
            RecoveryChoice::Resume,
            &lease,
            &mut journal,
            &mut PlatformFs::default(),
            &JobControl::default()
        )
        .unwrap()
        .state,
        OperationState::Completed
    );
    assert_eq!(fs::read(media.join("new")).unwrap(), b"fixture");
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 1);
}
#[test]
fn failed_v4_recovery_migration_rolls_back_without_losing_evidence() {
    let f = Fixture::new(&[("a", "new")]);
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let op = journal.begin(&f.plan.execution).unwrap();
    journal
        .intent(op, &f.plan.execution.groups[0].steps[0])
        .unwrap();
    drop(journal);
    let db = rusqlite::Connection::open(f.coordinator.database_path()).unwrap();
    legacy_schema::remove_copy_protocol(&db);
    db.execute_batch("DROP INDEX IF EXISTS history_by_state; DROP TABLE IF EXISTS history_revision; DROP TABLE operation_inverses; DROP TABLE recovery_restore; DROP TABLE recovery_effects; DROP TABLE recovery_audit; DELETE FROM schema_migrations WHERE version>=5; PRAGMA user_version=4; CREATE TABLE recovery_effects(conflicting_evidence TEXT); INSERT INTO recovery_effects VALUES('retain original evidence');").unwrap();
    assert!(Journal::open(&f.coordinator.database_path()).is_err());
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='recovery_audit'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        db.query_row(
            "SELECT conflicting_evidence FROM recovery_effects",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "retain original evidence"
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM steps WHERE operation_id=?1",
            [op.to_string()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "IntentDurable"
    );
}

#[test]
fn verified_flush_audit_survives_a_later_inverse_durability_gap() {
    let f = Fixture::new(&[("a", "new")]);
    let mut adapter = CrashFs {
        native: PlatformFs::default(),
        boundary: "none".into(),
        index: 0,
        count: 0,
        after_capture: None,
        outcome_durability: Some(vidlib_fileops::platform::Durability::Unsupported),
    };
    assert_eq!(
        f.run(&mut adapter, &JobControl::default()).unwrap().state,
        OperationState::RecoveryRequired
    );
    let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
    let lease = f.coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut adapter,
        &JobControl::default()
    )
    .is_err());
    assert_eq!(fs::read(f.media.join("a")).unwrap(), b"fixture");
    assert!(!f.media.join("new").exists());
    let preview = reconcile(&journal.pending().unwrap()[0], &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let report = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default(),
    )
    .expect("prior durability proof must survive a subsequent inverse gap");
    assert_eq!(report.state, OperationState::Cancelled);
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 2);
}

fn assert_restore_preserves_completed_group(
    media: &std::path::Path,
    coordinator: &Coordinator,
    journal: &mut Journal,
) {
    use vidlib_fileops::journal::StepState;
    let before = journal.pending().unwrap().remove(0);
    assert_eq!(before.groups.len(), 2);
    assert_eq!(before.steps[0].state, StepState::CompletionDurable);
    assert_eq!(before.steps[1].state, StepState::Planned);
    let events = journal.events_after(0, 200).unwrap().items;
    assert_eq!(events.len(), 1);
    let preview = reconcile(&before, &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let epoch = lease.epochs().unwrap().namespace;
    let report = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        journal,
        &mut PlatformFs::default(),
        &JobControl::default(),
    )
    .unwrap();
    assert!(
        !media.join("a").exists(),
        "fully durable G1 must not be restored from recency"
    );
    assert_eq!(fs::read(media.join("new")).unwrap(), b"fixture");
    assert_eq!(fs::read(media.join("b")).unwrap(), b"b");
    assert!(!media.join("other").exists(), "Restore must not execute G2");
    let after = journal.operation(before.operation_id).unwrap();
    assert_eq!(
        after.steps, before.steps,
        "original states and factual receipts must remain unchanged"
    );
    assert!(
        after.recovery_effects.is_empty(),
        "no unreviewed inverse may be compiled"
    );
    assert_eq!(journal.events_after(0, 200).unwrap().items, events);
    assert_eq!(report.state, OperationState::Cancelled);
    assert_eq!(report.completed_groups, vec![before.groups[0].id]);
    assert!(journal.pending().unwrap().is_empty());
    assert!(!lease.status().unwrap().pending_journal);
    assert_eq!(lease.epochs().unwrap().namespace, epoch + 1);
}
#[test]
fn restore_after_receipt_before_next_intent_preserves_completed_group() {
    let (_dir, media, coordinator, mut journal) = crashed_case("receipt", 1, "groups");
    assert_restore_preserves_completed_group(&media, &coordinator, &mut journal);
}
#[test]
fn restore_after_non_application_reset_crash_preserves_completed_group() {
    let (_dir, media, coordinator, mut journal) = crashed_case("before", 2, "groups");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "recovery_crash_child_entry", "--nocapture"])
        .env("VIDLIB_RECOVERY_ROOT", media.parent().unwrap())
        .env("VIDLIB_CRASH_BOUNDARY", "post_reset")
        .env("VIDLIB_CRASH_INDEX", "0")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let db = rusqlite::Connection::open(coordinator.database_path()).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM recovery_audit WHERE resolution='NotApplied'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_restore_preserves_completed_group(&media, &coordinator, &mut journal);
}
#[test]
fn zero_effect_restore_keeps_gate_if_epoch_finalization_fails() {
    let (_dir, media, coordinator, mut journal) = crashed_case("receipt", 1, "groups");
    let before = journal.pending().unwrap().remove(0);
    let events = journal.events_after(0, 200).unwrap().items;
    let db = rusqlite::Connection::open(coordinator.database_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_recovery_epoch BEFORE UPDATE OF namespace_epoch ON coordination BEGIN SELECT RAISE(ABORT,'injected recovery epoch failure'); END;").unwrap();
    let preview = reconcile(&before, &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let epoch = lease.epochs().unwrap().namespace;
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default()
    )
    .is_err());
    assert!(
        lease.status().unwrap().pending_journal,
        "earlier effects remain gated until the epoch advances"
    );
    assert_eq!(lease.epochs().unwrap().namespace, epoch);
    let after = journal.operation(before.operation_id).unwrap();
    assert_eq!(after.steps, before.steps);
    assert!(after.recovery_effects.is_empty());
    assert_eq!(journal.events_after(0, 200).unwrap().items, events);
    assert_eq!(fs::read(media.join("new")).unwrap(), b"fixture");
    assert!(!media.join("a").exists());
    assert_eq!(fs::read(media.join("b")).unwrap(), b"b");
    assert!(!media.join("other").exists());
    db.execute_batch("DROP TRIGGER fail_recovery_epoch;")
        .unwrap();
    drop(lease);
    assert_restore_preserves_completed_group(&media, &coordinator, &mut journal);
}

#[test]
fn restore_scope_survives_final_gap_receipt_before_inverse_intent() {
    use vidlib_fileops::journal::StepState;
    let (_dir, media, coordinator, mut journal) = crashed("after", 3, true);
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "recovery_crash_child_entry", "--nocapture"])
        .env("VIDLIB_RECOVERY_ROOT", media.parent().unwrap())
        .env("VIDLIB_CRASH_BOUNDARY", "resolved_gap")
        .env("VIDLIB_CRASH_INDEX", "0")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let before = journal.pending().unwrap().remove(0);
    assert!(before
        .steps
        .iter()
        .all(|s| s.state == StepState::CompletionDurable));
    assert!(before.recovery_effects.is_empty());
    assert_eq!(
        before.restore_context.as_ref().unwrap().group_id,
        Some(before.groups[0].id)
    );
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 3);
    let lease = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let preview = reconcile(&before, &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    assert!(apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Resume,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default()
    )
    .is_err());
    assert_eq!(journal.operation(before.operation_id).unwrap(), before);
    let preview = reconcile(&before, &mut PlatformFs::default()).unwrap();
    let nonce = preview.review_nonce();
    let report = apply_recovery(
        preview.review(nonce).unwrap(),
        RecoveryChoice::Restore,
        &lease,
        &mut journal,
        &mut PlatformFs::default(),
        &JobControl::default(),
    )
    .unwrap();
    assert_eq!(report.state, OperationState::Cancelled);
    assert_eq!(fs::read(media.join("a")).unwrap(), b"fixture");
    assert_eq!(fs::read(media.join("b")).unwrap(), b"b");
    let after = journal.operation(before.operation_id).unwrap();
    assert!(after
        .steps
        .iter()
        .all(|s| s.state == StepState::Compensated));
    assert_eq!(after.recovery_effects.len(), 3);
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 6);
    assert!(!lease.status().unwrap().pending_journal);
}

#[test]
fn v5_migration_preserves_receipts_without_inventing_restore_scope() {
    let (_dir, media, coordinator, mut journal) = crashed_case("receipt", 1, "groups");
    let before = journal.pending().unwrap().remove(0);
    let events = journal.events_after(0, 200).unwrap().items;
    drop(journal);
    let db = rusqlite::Connection::open(coordinator.database_path()).unwrap();
    legacy_schema::remove_copy_protocol(&db);
    db.execute_batch("DROP INDEX IF EXISTS history_by_state; DROP TABLE IF EXISTS history_revision; DROP TABLE operation_inverses; DROP TABLE recovery_restore; DELETE FROM schema_migrations WHERE version>=6; PRAGMA user_version=5;").unwrap();
    journal = Journal::open(&coordinator.database_path()).unwrap();
    assert_eq!(journal.pending().unwrap(), vec![before]);
    assert_eq!(journal.events_after(0, 200).unwrap().items, events);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_restore_preserves_completed_group(&media, &coordinator, &mut journal);
}

#[test]
fn failed_v5_restore_migration_retains_original_evidence() {
    let (_dir, _media, coordinator, journal) = crashed_case("receipt", 1, "groups");
    drop(journal);
    let db = rusqlite::Connection::open(coordinator.database_path()).unwrap();
    let states = |db: &rusqlite::Connection| {
        db.prepare("SELECT state FROM steps ORDER BY sequence")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let before = states(&db);
    legacy_schema::remove_copy_protocol(&db);
    db.execute_batch("DROP INDEX IF EXISTS history_by_state; DROP TABLE IF EXISTS history_revision; DROP TABLE operation_inverses; DROP TABLE recovery_restore; DELETE FROM schema_migrations WHERE version>=6; PRAGMA user_version=5; CREATE TABLE recovery_restore(evidence TEXT); INSERT INTO recovery_restore VALUES('retain original evidence');").unwrap();
    assert!(Journal::open(&coordinator.database_path()).is_err());
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        db.query_row("SELECT evidence FROM recovery_restore", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "retain original evidence"
    );
    assert_eq!(states(&db), before);
    assert_eq!(
        db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM operation_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn directory_after_effects_rename_gap_requires_complete_surviving_namespace() {
    use vidlib_rename::{ExecutionAction, ExpectedIdentity};
    for tamper in ["none", "unknown-child", "missing-child", "replacement"] {
        let mut f = Fixture::new(&[("a", "new")]);
        fs::create_dir(f.media.join("Tree")).unwrap();
        fs::write(f.media.join("Tree/child"), b"owned bytes").unwrap();
        let media = f.media.clone();
        crash_child::replan(&mut f, media.clone(), &[("Tree", "Moved")]);
        let step = &mut f.plan.execution.groups[0].steps[0];
        let ExecutionAction::RenameEntry { source, .. } = &mut step.action else {
            unreachable!()
        };
        let ExpectedIdentity::Observed(original) = &source.identity else {
            unreachable!()
        };
        source.identity = ExpectedIdentity::DirectoryAfterEffects {
            original: original.clone(),
            created_by: None,
            effects: vec![],
        };
        let step = step.clone();
        let mut journal = Journal::open(&f.coordinator.database_path()).unwrap();
        let op = journal.begin(&f.plan.execution).unwrap();
        journal.intent(op, &step).unwrap();
        fs::rename(media.join("Tree"), media.join("Moved")).unwrap();
        match tamper {
            "unknown-child" => fs::write(media.join("Moved/foreign"), b"external").unwrap(),
            "missing-child" => fs::remove_file(media.join("Moved/child")).unwrap(),
            "replacement" => {
                fs::rename(media.join("Moved"), media.join("Parked")).unwrap();
                fs::create_dir(media.join("Moved")).unwrap();
            }
            _ => {}
        }
        let pending = journal.operation(op).unwrap();
        let preview = reconcile(&pending, &mut PlatformFs::default()).unwrap();
        assert_eq!(
            preview.actions[0].disposition,
            if tamper == "none" {
                RecoveryDisposition::Applied
            } else {
                RecoveryDisposition::Unresolved
            },
            "{tamper}"
        );
        assert!(
            journal.operation(op).unwrap().steps[0].receipts.is_empty(),
            "read-only proof must not mint a durable fact"
        );
        if tamper == "unknown-child" {
            assert_eq!(fs::read(media.join("Moved/foreign")).unwrap(), b"external");
        }
    }
}
