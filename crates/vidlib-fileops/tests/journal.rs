//! Real-file journal boundary tests. Each test names the missing durability or authority guarantee.
use rusqlite::Connection;
use vidlib_fileops::{events::*, journal::*, platform::Durability, ErrorCode};
use vidlib_inventory::{ids::*, *};
use vidlib_rename::*;

fn fixture() -> (tempfile::TempDir, ExecutionPlan) {
    let dir = tempfile::tempdir().unwrap();
    let inventory = Inventory {
        id: InventoryId::new(),
        generation: u64::MAX,
        roots: vec![],
        entries: vec![],
        coverage: vec![],
        window: ObservationWindow::default(),
        issues: vec![],
    };
    let entry = EntryId::new();
    let group = GroupId::new();
    let step = ExecutionStep {
        id: StepId::new(),
        entry_id: entry,
        group_id: group,
        action: ExecutionAction::RenameEntry {
            source: EntryPrestate {
                path: dir.path().join("source"),
                identity: ExpectedIdentity::Observed(stamp()),
            },
            target: dir.path().join("target"),
            temporary: false,
        },
    };
    let plan = ExecutionPlan {
        groups: vec![ExecutionGroup {
            id: group,
            depends_on: vec![],
            steps: vec![step],
        }],
        provenance: InspectionProvenance {
            approved_plan: PlanRef {
                session_id: SessionId::new(),
                plan_id: PlanId::new(),
                revision: u64::MAX,
            },
            recipe: RecipeV1 {
                schema_version: 1,
                revision: u64::MAX,
                selection: vec![entry],
                numbering_order: vec![entry],
                rules: vec![],
                companions: CompanionPolicy::None,
                destination: Destination::SameDirectory,
                collision_policy: CollisionPolicy::Block,
                metadata_changes: vec![],
            },
            frozen_inputs: FrozenInputs {
                inventory_id: inventory.id,
                generation: u64::MAX,
                now_ns: i128::MIN,
                timezone: FrozenTimezone::Utc,
                primary_order: vec![entry],
                values: vec![],
            },
            inventory,
            grants: GrantSet::default(),
            namespace_epoch: u64::MAX,
            policy_epoch: u64::MAX,
        },
    };
    (dir, plan)
}
fn stamp() -> FileStamp {
    FileStamp {
        size: Some(u64::MAX),
        modified_ns: Some(i128::MIN),
        accessed_ns: Some(i128::MAX),
        physical_id: Some(PhysicalId {
            platform: if cfg!(windows) {
                PathPlatform::WindowsUtf16Le
            } else {
                PathPlatform::UnixBytes
            },
            volume_identity: vec![1, 0, 255],
            file_identity: vec![2, 0, 254],
        }),
        ..Default::default()
    }
}
fn receipt(step: &ExecutionStep, phase: CompletedPhase) -> StepReceipt {
    let (before, after) = match &step.action {
        ExecutionAction::RenameEntry { source, target, .. } => (Some(&source.path), Some(target)),
        ExecutionAction::CreateDirectory { target } => (None, Some(target)),
        ExecutionAction::StageCopy { source, staging } => (Some(&source.path), Some(staging)),
        ExecutionAction::PublishCopy {
            staging, target, ..
        } => (Some(&staging.entry.path), Some(target)),
        ExecutionAction::RemoveSource { source, .. } => (Some(&source.path), None),
        ExecutionAction::SetMetadata { entry, .. } => (Some(&entry.path), Some(&entry.path)),
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => (Some(&owned.entry.path), None),
    };
    StepReceipt {
        step_id: step.id,
        phase,
        before_path: before.map(|p| encode_path(p)),
        after_path: after.map(|p| encode_path(p)),
        before: before.map(|_| stamp()),
        after: after.map(|_| stamp()),
        digest: Some(ContentDigest {
            algorithm: "sha256".into(),
            bytes: vec![7; 32],
        }),
        observed_ns: i128::MAX,
        durability: Durability::Synced,
    }
}

// Losing the committed intent (or fabricating Applied after restart) must fail this.
#[test]
fn intent_survives_reopen() {
    let (dir, plan) = fixture();
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let op = journal.begin(&plan).unwrap();
    journal.intent(op, &plan.groups[0].steps[0]).unwrap();
    drop(journal);
    let reopened = Journal::open(&path).unwrap();
    let pending = reopened.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].operation_id, op);
    assert_eq!(pending[0].state, OperationState::Running);
    assert_eq!(pending[0].steps[0].state, StepState::IntentDurable);
    assert!(pending[0].steps[0].receipts.is_empty());
}

// Separate receipt/event transactions would leave an orphan on the forced event failure.
#[test]
fn receipt_and_event_commit_together() {
    let (dir, plan) = fixture();
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let op = journal.begin(&plan).unwrap();
    let step = &plan.groups[0].steps[0];
    journal.intent(op, step).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_event BEFORE INSERT ON operation_events BEGIN SELECT RAISE(ABORT, 'fixture event failure'); END;").unwrap();
    assert!(journal
        .complete(op, &receipt(step, CompletedPhase::Renamed))
        .is_err());
    assert_eq!(
        db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(journal.events_after(0, 200).unwrap().items.is_empty());
    assert_eq!(
        journal.pending().unwrap()[0].steps[0].state,
        StepState::IntentDurable
    );
}

// A deferred FK fails at COMMIT, not at INSERT; continuing after it would authorize an unjournaled next effect.
#[test]
fn commit_error_prevents_next_step() {
    let (dir, mut plan) = fixture();
    let mut second = plan.groups[0].steps[0].clone();
    second.id = StepId::new();
    plan.groups[0].steps.push(second.clone());
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let op = journal.begin(&plan).unwrap();
    let first = &plan.groups[0].steps[0];
    journal.intent(op, first).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE fixture_parent(id INTEGER PRIMARY KEY); CREATE TABLE fixture_child(id INTEGER REFERENCES fixture_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER fail_commit AFTER INSERT ON operation_events BEGIN INSERT INTO fixture_child VALUES(42); END;").unwrap();
    assert_eq!(
        journal
            .complete(op, &receipt(first, CompletedPhase::Renamed))
            .unwrap_err()
            .code,
        ErrorCode::RecoveryRequired
    );
    assert_eq!(
        journal.intent(op, &second).unwrap_err().code,
        ErrorCode::RecoveryRequired
    );
    assert!(journal.events_after(0, 200).unwrap().items.is_empty());
    drop(journal);
    let reopened = Journal::open(&path).unwrap();
    assert_eq!(
        reopened.pending().unwrap()[0].steps[1].state,
        StepState::Planned
    );
    assert_eq!(
        reopened.pending().unwrap()[0].steps[0].state,
        StepState::IntentDurable
    );
}

// Publication must remain a fact even when source removal never completes.
#[test]
fn published_copy_without_source_removal_is_distinct_fact() {
    let (dir, mut plan) = fixture();
    let group = &mut plan.groups[0];
    let source = EntryPrestate {
        path: dir.path().join("source"),
        identity: ExpectedIdentity::Observed(stamp()),
    };
    let stage = group.steps[0].id;
    group.steps[0].action = ExecutionAction::StageCopy {
        source: source.clone(),
        staging: dir.path().join("staging"),
    };
    let publish = ExecutionStep {
        id: StepId::new(),
        entry_id: group.steps[0].entry_id,
        group_id: group.id,
        action: ExecutionAction::PublishCopy {
            staging: OwnedEntryPrestate {
                entry: EntryPrestate {
                    path: dir.path().join("staging"),
                    identity: ExpectedIdentity::FromReceipt { step_id: stage },
                },
                created_by: stage,
            },
            target: dir.path().join("published"),
            verified_by: stage,
        },
    };
    let remove = ExecutionStep {
        id: StepId::new(),
        entry_id: publish.entry_id,
        group_id: group.id,
        action: ExecutionAction::RemoveSource {
            source,
            published_by: publish.id,
        },
    };
    group.steps.extend([publish, remove]);
    let mut journal = Journal::open(&dir.path().join("operations.sqlite3")).unwrap();
    let op = journal.begin(&plan).unwrap();
    for (step, phase) in plan.groups[0]
        .steps
        .iter()
        .zip([CompletedPhase::CopyStaged, CompletedPhase::CopyPublished])
    {
        journal.intent(op, step).unwrap();
        let mut fact = receipt(step, phase);
        fact.after
            .as_mut()
            .unwrap()
            .physical_id
            .as_mut()
            .unwrap()
            .file_identity = vec![9];
        if phase == CompletedPhase::CopyPublished {
            fact.before
                .as_mut()
                .unwrap()
                .physical_id
                .as_mut()
                .unwrap()
                .file_identity = vec![9];
        }
        journal.complete(op, &fact).unwrap();
    }
    let first_page = journal.events_after(0, 1).unwrap();
    let events = journal.events_after(first_page.next_cursor, 200).unwrap();
    assert_eq!(events.items[0].phase, CompletedPhase::CopyPublished);
    assert!(!events
        .items
        .iter()
        .any(|e| e.phase == CompletedPhase::SourceRemoved));
    journal.intent(op, &plan.groups[0].steps[2]).unwrap();
    journal
        .set_state(op, OperationState::RecoveryRequired)
        .unwrap();
    assert_eq!(
        journal
            .events_after(first_page.next_cursor, 200)
            .unwrap()
            .items,
        events.items
    );
}

// Opening a newer schema must not change even the original file's bytes/PRAGMAs.
#[test]
fn unknown_schema_preserves_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("operations.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA user_version=999; CREATE TABLE future_evidence(note TEXT); INSERT INTO future_evidence VALUES('keep me');").unwrap();
    drop(db);
    let original = std::fs::read(&path).unwrap();
    assert!(Journal::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

// An existing unrelated/malformed schema cannot be treated as a fresh journal or overwritten.
#[test]
fn invalid_migration_preserves_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("operations.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, checksum TEXT); INSERT INTO schema_migrations VALUES(1, 'wrong'); PRAGMA user_version=1;").unwrap();
    drop(db);
    let original = std::fs::read(&path).unwrap();
    assert!(Journal::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

// Lossy display paths or SQLite signed integers would corrupt these exact values.
#[test]
fn exact_numeric_and_native_paths_survive_reopen() {
    let (dir, mut plan) = fixture();
    #[cfg(unix)]
    let native = {
        use std::os::unix::ffi::OsStringExt;
        dir.path()
            .join(std::ffi::OsString::from_vec(vec![b'x', 0xff]))
    };
    #[cfg(windows)]
    let native = {
        use std::os::windows::ffi::OsStringExt;
        dir.path()
            .join(std::ffi::OsString::from_wide(&[120, 0xd800]))
    };
    if let ExecutionAction::RenameEntry { target, .. } = &mut plan.groups[0].steps[0].action {
        *target = native.clone();
    }
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let op = journal.begin(&plan).unwrap();
    journal.intent(op, &plan.groups[0].steps[0]).unwrap();
    let mut fact = receipt(&plan.groups[0].steps[0], CompletedPhase::Renamed);
    fact.after_path = Some(encode_path(&native));
    journal.complete(op, &fact).unwrap();
    drop(journal);
    let journal = Journal::open(&path).unwrap();
    let record = journal.operation(op).unwrap();
    assert_eq!(record.steps[0].step, plan.groups[0].steps[0]);
    assert_eq!(
        record.steps[0].receipts[0].after.as_ref().unwrap().size,
        Some(u64::MAX)
    );
    assert_eq!(
        record.steps[0].receipts[0]
            .after
            .as_ref()
            .unwrap()
            .modified_ns,
        Some(i128::MIN)
    );
    let event = &journal.events_after(0, 200).unwrap().items[0];
    assert_eq!(event.observed_ns, i128::MAX);
    assert_eq!(event.after_path, Some(encode_path(&native)));
    let json = serde_json::to_value(event).unwrap();
    assert_eq!(json["observed_ns"], i128::MAX.to_string());
    assert_eq!(json["after"]["size"], u64::MAX.to_string());
    assert_eq!(record.inspection["namespace_epoch"], u64::MAX.to_string());
    assert_eq!(
        record.inspection["frozen_inputs"]["now_ns"],
        i128::MIN.to_string()
    );
}

// Stable IDs, cursor ordering and acknowledgments must survive restart without losing audit history.
#[test]
fn stable_outbox_ack_and_terminal_state_survive_reopen() {
    let (dir, plan) = fixture();
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let op = journal.begin(&plan).unwrap();
    let step = &plan.groups[0].steps[0];
    assert!(journal.set_state(op, OperationState::Completed).is_err());
    journal.intent(op, step).unwrap();
    journal
        .complete(op, &receipt(step, CompletedPhase::Renamed))
        .unwrap();
    journal.set_state(op, OperationState::Completed).unwrap();
    let first = journal.events_after(0, 200).unwrap();
    assert_eq!(first.items.len(), 1);
    drop(journal);
    let mut journal = Journal::open(&path).unwrap();
    assert_eq!(journal.events_after(0, 200).unwrap().items, first.items);
    assert!(journal
        .ack_events(&[first.items[0].event_id, EventId::new()])
        .is_err());
    assert_eq!(journal.events_after(0, 200).unwrap().items, first.items);
    journal.ack_events(&[first.items[0].event_id]).unwrap();
    journal.ack_events(&[first.items[0].event_id]).unwrap();
    drop(journal);
    let journal = Journal::open(&path).unwrap();
    assert!(journal.pending().unwrap().is_empty());
    assert!(journal.events_after(0, 200).unwrap().items.is_empty());
    assert_eq!(journal.operation(op).unwrap().steps[0].receipts.len(), 1);
}

// A caller cannot substitute a different path, skip a predecessor or complete without an intent.
#[test]
fn rejects_forged_steps_wrong_phases_and_out_of_order_actions() {
    let (dir, mut plan) = fixture();
    let mut second = plan.groups[0].steps[0].clone();
    second.id = StepId::new();
    plan.groups[0].steps.push(second.clone());
    let mut journal = Journal::open(&dir.path().join("operations.sqlite3")).unwrap();
    let op = journal.begin(&plan).unwrap();
    let first = &plan.groups[0].steps[0];
    assert!(journal.intent(op, &second).is_err());
    assert!(journal
        .complete(op, &receipt(first, CompletedPhase::Renamed))
        .is_err());
    let mut forged = first.clone();
    forged.action = ExecutionAction::CreateDirectory {
        target: dir.path().join("forged"),
    };
    assert!(journal.intent(op, &forged).is_err());
    journal.intent(op, first).unwrap();
    assert!(journal.intent(op, first).is_err());
    assert!(journal
        .complete(op, &receipt(first, CompletedPhase::SourceRemoved))
        .is_err());
    journal
        .complete(op, &receipt(first, CompletedPhase::Renamed))
        .unwrap();
    assert!(journal
        .complete(op, &receipt(first, CompletedPhase::Renamed))
        .is_err());
    journal.intent(op, &second).unwrap();
}

// A pending writer must produce Busy, retain recovery evidence and block subsequent intents on this handle.
#[test]
fn busy_writer_preserves_pending_operation() {
    let (dir, plan) = fixture();
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let op = journal.begin(&plan).unwrap();
    let lock = Connection::open(&path).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        journal
            .intent(op, &plan.groups[0].steps[0])
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
    lock.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        journal
            .intent(op, &plan.groups[0].steps[0])
            .unwrap_err()
            .code,
        ErrorCode::RecoveryRequired
    );
    assert_eq!(
        journal.pending().unwrap()[0].steps[0].state,
        StepState::Planned
    );
}

// Every required primitive, receipt reference, temporary reservation and exact prestate must round trip.
#[test]
fn complete_action_family_round_trips_and_missing_references_are_rejected() {
    let (dir, mut plan) = fixture();
    let group = &mut plan.groups[0];
    let entry_id = group.steps[0].entry_id;
    let make = |action| ExecutionStep {
        id: StepId::new(),
        entry_id,
        group_id: group.id,
        action,
    };
    let observed = EntryPrestate {
        path: dir.path().join("source"),
        identity: ExpectedIdentity::Observed(stamp()),
    };
    let create = make(ExecutionAction::CreateDirectory {
        target: dir.path().join("folder"),
    });
    let stage = make(ExecutionAction::StageCopy {
        source: observed.clone(),
        staging: dir.path().join("stage"),
    });
    let owned = OwnedEntryPrestate {
        entry: EntryPrestate {
            path: dir.path().join("stage"),
            identity: ExpectedIdentity::FromReceipt { step_id: stage.id },
        },
        created_by: stage.id,
    };
    let publish = make(ExecutionAction::PublishCopy {
        staging: owned.clone(),
        target: dir.path().join("target"),
        verified_by: stage.id,
    });
    let remove = make(ExecutionAction::RemoveSource {
        source: observed.clone(),
        published_by: publish.id,
    });
    let metadata = make(ExecutionAction::SetMetadata {
        entry: observed,
        changes: vec![
            MetadataChange::Timestamp {
                field: TimestampField::Modified,
                value_ns: i128::MIN,
            },
            MetadataChange::WindowsAttributes { set: 2, clear: 1 },
        ],
    });
    let clean = make(ExecutionAction::RemoveOwnedCopy { owned });
    let clean_dir = make(ExecutionAction::RemoveOwnedEmptyDirectory {
        owned: OwnedEntryPrestate {
            entry: EntryPrestate {
                path: dir.path().join("folder"),
                identity: ExpectedIdentity::FromReceipt { step_id: create.id },
            },
            created_by: create.id,
        },
    });
    if let ExecutionAction::RenameEntry { temporary, .. } = &mut group.steps[0].action {
        *temporary = true;
    }
    group
        .steps
        .extend([create, stage, publish, remove, metadata, clean, clean_dir]);
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let op = journal.begin(&plan).unwrap();
    drop(journal);
    let mut journal = Journal::open(&path).unwrap();
    let record = journal.operation(op).unwrap();
    assert_eq!(record.steps.len(), 8);
    for (actual, expected) in record.steps.iter().zip(&plan.groups[0].steps) {
        assert_eq!(&actual.step, expected);
    }
    if let ExecutionAction::PublishCopy { verified_by, .. } = &mut plan.groups[0].steps[3].action {
        *verified_by = StepId::new();
    }
    assert!(journal.begin(&plan).is_err());
    assert_eq!(journal.pending().unwrap().len(), 1);
}

// A crash gap must be explicitly unresolved; terminal states must not hide its recovery gate.
#[test]
fn unresolved_step_cannot_be_cancelled_or_retried() {
    let (dir, plan) = fixture();
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let op = journal.begin(&plan).unwrap();
    let step = &plan.groups[0].steps[0];
    journal.intent(op, step).unwrap();
    journal.mark_unresolved(op, step.id).unwrap();
    assert!(journal.set_state(op, OperationState::Cancelled).is_err());
    drop(journal);
    let mut journal = Journal::open(&path).unwrap();
    assert_eq!(
        journal.pending().unwrap()[0].steps[0].state,
        StepState::Unresolved
    );
    assert_eq!(
        journal.intent(op, step).unwrap_err().code,
        ErrorCode::RecoveryRequired
    );
}

// A source removal may only reference publication in a declared dependency, never an unrelated or future effect.
#[test]
fn rejects_forward_wrong_phase_and_cross_group_receipt_references() {
    let (dir, mut plan) = fixture();
    let mut journal = Journal::open(&dir.path().join("operations.sqlite3")).unwrap();
    let original = plan.groups[0].steps[0].clone();
    let from = EntryPrestate {
        path: dir.path().join("target"),
        identity: ExpectedIdentity::FromReceipt {
            step_id: original.id,
        },
    };
    let next = ExecutionStep {
        id: StepId::new(),
        entry_id: original.entry_id,
        group_id: GroupId::new(),
        action: ExecutionAction::SetMetadata {
            entry: from,
            changes: vec![MetadataChange::Timestamp {
                field: TimestampField::Modified,
                value_ns: i128::MIN,
            }],
        },
    };
    plan.groups.push(ExecutionGroup {
        id: next.group_id,
        depends_on: vec![],
        steps: vec![next.clone()],
    });
    assert!(journal.begin(&plan).is_err());
    plan.groups[1].depends_on.push(original.group_id);
    let op = journal.begin(&plan).unwrap();
    assert_eq!(
        journal.operation(op).unwrap().groups[1].depends_on,
        vec![original.group_id]
    );
    plan.groups[0].steps[0].action = ExecutionAction::SetMetadata {
        entry: EntryPrestate {
            path: dir.path().join("target"),
            identity: ExpectedIdentity::FromReceipt { step_id: next.id },
        },
        changes: vec![],
    };
    assert!(journal.begin(&plan).is_err());
    plan.groups[0].steps[0] = original.clone();
    plan.groups[1].steps[0].action = ExecutionAction::RemoveSource {
        source: EntryPrestate {
            path: dir.path().join("source"),
            identity: ExpectedIdentity::Observed(stamp()),
        },
        published_by: original.id,
    };
    assert!(journal.begin(&plan).is_err());
    assert_eq!(journal.pending().unwrap().len(), 1);
}

// Changed native paths or physical identity cannot create an authoritative completion fact.
#[test]
fn mismatched_receipt_has_no_event_and_valid_receipt_can_still_commit() {
    let (dir, plan) = fixture();
    let mut journal = Journal::open(&dir.path().join("operations.sqlite3")).unwrap();
    let op = journal.begin(&plan).unwrap();
    let step = &plan.groups[0].steps[0];
    journal.intent(op, step).unwrap();
    let mut fact = receipt(step, CompletedPhase::Renamed);
    fact.after
        .as_mut()
        .unwrap()
        .physical_id
        .as_mut()
        .unwrap()
        .file_identity = vec![99];
    let error = journal.complete(op, &fact).unwrap_err();
    assert_eq!(error.operation_id, Some(op));
    assert_eq!(error.step_id, Some(step.id));
    assert_eq!(error.item_id, Some(step.entry_id));
    assert!(journal.events_after(0, 200).unwrap().items.is_empty());
    fact = receipt(step, CompletedPhase::Renamed);
    fact.after_path = Some(encode_path(&dir.path().join("other")));
    assert!(journal.complete(op, &fact).is_err());
    journal
        .complete(op, &receipt(step, CompletedPhase::Renamed))
        .unwrap();
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 1);
}

// A durable fact about an effect with failed filesystem flushing must keep recovery pending.
#[test]
fn failed_filesystem_durability_records_fact_and_stops_next_intent() {
    let (dir, mut plan) = fixture();
    let mut next = plan.groups[0].steps[0].clone();
    next.id = StepId::new();
    plan.groups[0].steps.push(next.clone());
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let op = journal.begin(&plan).unwrap();
    let first = &plan.groups[0].steps[0];
    journal.intent(op, first).unwrap();
    let mut fact = receipt(first, CompletedPhase::Renamed);
    fact.durability = Durability::Failed("fixture flush error".into());
    journal.complete(op, &fact).unwrap();
    drop(journal);
    let mut journal = Journal::open(&path).unwrap();
    assert_eq!(
        journal.pending().unwrap()[0].state,
        OperationState::RecoveryRequired
    );
    assert_eq!(
        journal.operation(op).unwrap().steps[0].receipts[0].durability,
        Durability::Failed("fixture flush error".into())
    );
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 1);
    assert_eq!(
        journal.intent(op, &next).unwrap_err().code,
        ErrorCode::RecoveryRequired
    );
}

// Shared transitive ancestors are not duplicate direct dependencies.
#[test]
fn transitive_group_dependencies_preserve_declared_order() {
    let (dir, mut plan) = fixture();
    let first = plan.groups[0].id;
    let make = |depends_on: Vec<GroupId>| {
        let id = GroupId::new();
        ExecutionGroup {
            id,
            depends_on,
            steps: vec![ExecutionStep {
                id: StepId::new(),
                entry_id: EntryId::new(),
                group_id: id,
                action: ExecutionAction::CreateDirectory {
                    target: dir.path().join(id.to_string()),
                },
            }],
        }
    };
    let second = make(vec![first]);
    let third = make(vec![second.id, first]);
    plan.groups.extend([second, third]);
    let mut journal = Journal::open(&dir.path().join("operations.sqlite3")).unwrap();
    let op = journal.begin(&plan).unwrap();
    assert_eq!(
        journal.operation(op).unwrap().groups[2].depends_on,
        vec![plan.groups[1].id, first]
    );
    plan.groups[2].depends_on.push(first);
    assert!(journal.begin(&plan).is_err());
}

// Compensation of a later group must never remove ownership belonging to an earlier completed group.
#[test]
fn compensation_cannot_claim_an_earlier_dependency_groups_creation() {
    let (dir, mut plan) = fixture();
    let creator = plan.groups[0].steps[0].id;
    plan.groups[0].steps[0].action = ExecutionAction::CreateDirectory {
        target: dir.path().join("owned-folder"),
    };
    let group = GroupId::new();
    plan.groups.push(ExecutionGroup {
        id: group,
        depends_on: vec![plan.groups[0].id],
        steps: vec![ExecutionStep {
            id: StepId::new(),
            group_id: group,
            entry_id: EntryId::new(),
            action: ExecutionAction::RemoveOwnedEmptyDirectory {
                owned: OwnedEntryPrestate {
                    entry: EntryPrestate {
                        path: dir.path().join("owned-folder"),
                        identity: ExpectedIdentity::FromReceipt { step_id: creator },
                    },
                    created_by: creator,
                },
            },
        }],
    });
    let mut journal = Journal::open(&dir.path().join("operations.sqlite3")).unwrap();
    assert!(journal.begin(&plan).is_err());
    assert!(journal.pending().unwrap().is_empty());
}

// Matching a source stamp cannot claim a newly copied object's creation ownership.
#[test]
fn owned_cleanup_requires_the_creators_actual_native_identity() {
    let (dir, mut plan) = fixture();
    let creator = plan.groups[0].steps[0].id;
    plan.groups[0].steps[0].action = ExecutionAction::StageCopy {
        source: EntryPrestate {
            path: dir.path().join("source"),
            identity: ExpectedIdentity::Observed(stamp()),
        },
        staging: dir.path().join("stage"),
    };
    let clean = ExecutionStep {
        id: StepId::new(),
        entry_id: plan.groups[0].steps[0].entry_id,
        group_id: plan.groups[0].id,
        action: ExecutionAction::RemoveOwnedCopy {
            owned: OwnedEntryPrestate {
                entry: EntryPrestate {
                    path: dir.path().join("stage"),
                    identity: ExpectedIdentity::Observed(stamp()),
                },
                created_by: creator,
            },
        },
    };
    plan.groups[0].steps.push(clean.clone());
    let mut journal = Journal::open(&dir.path().join("operations.sqlite3")).unwrap();
    let op = journal.begin(&plan).unwrap();
    let step = &plan.groups[0].steps[0];
    journal.intent(op, step).unwrap();
    let mut fact = receipt(step, CompletedPhase::CopyStaged);
    fact.after
        .as_mut()
        .unwrap()
        .physical_id
        .as_mut()
        .unwrap()
        .file_identity = vec![9];
    journal.complete(op, &fact).unwrap();
    assert_eq!(
        journal.intent(op, &clean).unwrap_err().code,
        ErrorCode::RecoveryRequired
    );
    assert_eq!(
        journal.operation(op).unwrap().steps[1].state,
        StepState::Planned
    );
    assert_eq!(journal.events_after(0, 200).unwrap().items.len(), 1);
}

// Indexed mutation reads must retain exact provenance and ordering after reopen,
// even with a large inspection snapshot and unrelated receipts in the same store.
#[test]
fn many_steps_keep_exact_provenance_and_order_after_reopen() {
    let (dir, mut plan) = fixture();
    plan.provenance.inventory.issues.push(InventoryIssue {
        code: IssueCode::StatFailed,
        relative_path: None,
        message: "inspection".repeat(32768),
    });
    let template = plan.groups[0].steps[0].clone();
    for _ in 1..24 {
        let mut step = template.clone();
        step.id = StepId::new();
        plan.groups[0].steps.push(step);
    }
    let path = dir.path().join("operations.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let other = journal.begin(&plan).unwrap();
    journal.intent(other, &template).unwrap();
    journal
        .complete(other, &receipt(&template, CompletedPhase::Renamed))
        .unwrap();
    let op = journal.begin(&plan).unwrap();
    drop(journal);
    let mut journal = Journal::open(&path).unwrap();
    for (ordinal, step) in plan.groups[0].steps.iter().enumerate() {
        if let Some(next) = plan.groups[0].steps.get(ordinal + 1) {
            assert!(journal.intent(op, next).is_err());
        }
        journal.intent(op, step).unwrap();
        journal
            .complete(op, &receipt(step, CompletedPhase::Renamed))
            .unwrap();
    }
    journal.set_state(op, OperationState::Completed).unwrap();
    let events = journal.events_after(0, 200).unwrap().items;
    assert_eq!(events.len(), 25);
    for (event, step) in events
        .iter()
        .filter(|e| e.operation_id == op)
        .zip(&plan.groups[0].steps)
    {
        assert_eq!(event.step_id, step.id);
        assert_eq!(event.approved_plan, plan.provenance.approved_plan);
    }
    assert_eq!(journal.pending().unwrap().len(), 1);
}
