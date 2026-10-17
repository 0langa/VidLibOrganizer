use serde_json::{json, Value};
use std::{path::PathBuf, thread, time::Duration};
use vidlib_tauri::{app_state::AppState, commands::rename::*, selection::install_selection};
fn fixture() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("media");
    std::fs::create_dir(&media).unwrap();
    std::fs::write(media.join("original.mp4"), b"media").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    install_selection(&state, vec![media]).unwrap();
    (dir, state)
}
fn wait(state: &AppState, id: vidlib_inventory::ids::JobId) -> Value {
    for _ in 0..500 {
        let status = status_owned(state, JobRequest { job_id: id }).unwrap();
        if status.state != "running" {
            assert_eq!(status.state, "completed", "{status:?}");
            return status.result.unwrap();
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("job timeout")
}
fn history_request(state: &AppState) -> HistoryRequest {
    let revision = history_revision_owned(state, EmptyRequest {}).unwrap()["revision"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    HistoryRequest {
        query: PageQuery {
            sort: "oldest".into(),
            ..PageQuery::new(revision)
        },
    }
}
fn preview(state: &AppState) -> UiPlanRef {
    wait(
        state,
        inventory_owned(state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    let page = inventory_page_owned(
        state,
        InventoryPageRequest {
            query: PageQuery::new(1),
        },
    )
    .unwrap();
    let selected = page
        .items
        .iter()
        .map(|v| serde_json::from_value(v["itemId"].clone()).unwrap())
        .collect();
    let result = wait(
        state,
        preview_owned(
            state,
            RenameRequestDto {
                revision: 2,
                selected_ids: selected,
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
            },
        )
        .unwrap(),
    );
    serde_json::from_value(result["planRef"].clone()).unwrap()
}
#[test]
fn stale_nonce_cannot_apply() {
    let (dir, state) = fixture();
    let plan_ref = preview(&state);
    let confirmation = confirm_owned(&state, PlanRequest { plan_ref }).unwrap();
    update_inputs_owned(&state, InputRevision { revision: 3 }).unwrap();
    assert!(
        apply_owned(
            &state,
            ApplyRequest {
                plan_ref,
                nonce: confirmation.nonce
            }
        )
        .is_err(),
        "editing recipe retained apply authority"
    );
    assert!(dir.path().join("media/original.mp4").exists());
    assert!(!dir.path().join("media/renamed.mp4").exists());
}
#[test]
fn old_session_plan_is_invalid() {
    let (dir, state) = fixture();
    let plan_ref = preview(&state);
    drop(state);
    let restarted =
        AppState::open(&dir.path().join("app"), PathBuf::from("libraries.json")).unwrap();
    assert!(page_owned(
        &restarted,
        PageRequest {
            plan_ref,
            query: PageQuery::new(2)
        }
    )
    .is_err());
}
#[test]
fn page_never_returns_native_paths() {
    let (dir, state) = fixture();
    let plan_ref = preview(&state);
    let page = page_owned(
        &state,
        PageRequest {
            plan_ref,
            query: PageQuery {
                limit: 1000,
                ..PageQuery::new(2)
            },
        },
    )
    .unwrap();
    let value = serde_json::to_value(page).unwrap();
    assert_eq!(value["items"][0]["newName"], "renamed.mp4");
    fn keys(v: &Value) {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    assert!(
                        ![
                            "sourcePath",
                            "targetPath",
                            "absolutePath",
                            "bytes",
                            "platform"
                        ]
                        .contains(&k.as_str()),
                        "native path authority leaked: {k}"
                    );
                    keys(v)
                }
            }
            Value::Array(a) => a.iter().for_each(keys),
            _ => (),
        }
    }
    keys(&value);
    assert!(dir.path().join("media/original.mp4").exists());
}
#[test]
fn desktop_apply_moves_real_file_and_records_factual_history() {
    let (dir, state) = fixture();
    let plan_ref = preview(&state);
    let confirmation = confirm_owned(&state, PlanRequest { plan_ref }).unwrap();
    let outcome = wait(
        &state,
        apply_owned(
            &state,
            ApplyRequest {
                plan_ref,
                nonce: confirmation.nonce,
            },
        )
        .unwrap(),
    );
    assert_eq!(outcome["state"], "completed");
    assert_eq!(
        std::fs::read(dir.path().join("media/renamed.mp4")).unwrap(),
        b"media"
    );
    assert!(!dir.path().join("media/original.mp4").exists());
    let history = history_owned(&state, history_request(&state)).unwrap();
    assert_eq!(history["items"][0]["completedSteps"], "1");
}
#[test]
fn owned_dto_matches_browser_golden_and_rejects_rounded_numbers() {
    let golden: Value = serde_json::from_str(include_str!(
        "../../tests/ui/fixtures/command-contracts.json"
    ))
    .unwrap();
    let reference: UiPlanRef = serde_json::from_value(golden["planRef"].clone()).unwrap();
    assert_eq!(serde_json::to_value(reference).unwrap(), golden["planRef"]);
    let query: PageQuery = serde_json::from_value(golden["pageQuery"].clone()).unwrap();
    assert_eq!(serde_json::to_value(query).unwrap(), golden["pageQuery"]);
    assert!(serde_json::from_value::<UiPlanRef>(json!({"sessionId":reference.session_id,"planId":reference.plan_id,"revision":9007199254740992u64})).is_err());
}
#[test]
fn undo_uses_transformed_individual_file_grant_and_common_confirmation() {
    let (dir, state) = fixture();
    install_selection(&state, vec![dir.path().join("media/original.mp4")]).unwrap();
    let reference = preview(&state);
    let review = confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference,
        },
    )
    .unwrap();
    let done = wait(
        &state,
        apply_owned(
            &state,
            ApplyRequest {
                plan_ref: reference,
                nonce: review.nonce,
            },
        )
        .unwrap(),
    );
    let op = serde_json::from_value(done["operationId"].clone()).unwrap();
    let start = undo_owned(
        &state,
        UndoRequest {
            operation_id: op,
            revision: 3,
        },
    );
    assert!(
        start.is_ok(),
        "retained file effects must offer an Undo preview: {start:?}"
    );
    let result = wait(&state, start.unwrap());
    let reference = serde_json::from_value(result["planRef"].clone()).unwrap();
    let review = confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference,
        },
    )
    .unwrap();
    wait(
        &state,
        apply_owned(
            &state,
            ApplyRequest {
                plan_ref: reference,
                nonce: review.nonce,
            },
        )
        .unwrap(),
    );
    assert_eq!(
        std::fs::read(dir.path().join("media/original.mp4")).unwrap(),
        b"media"
    );
    let history = history_owned(&state, history_request(&state)).unwrap();
    assert_eq!(
        history["items"][0]["undoAvailable"], false,
        "an active inverse must revoke repeated original Undo"
    );
}
#[test]
fn recovery_uses_opaque_consumed_review_and_real_journal() {
    use vidlib_fileops::{coordinator::LeaseKind, journal::Journal};
    let (dir, state) = fixture();
    let reference = preview(&state);
    let plan = {
        let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
        state
            .plans
            .lock()
            .unwrap()
            .current(reference.into(), &lease)
            .unwrap()
            .clone()
    };
    let op = {
        let _lease = state.coordinator.try_acquire(LeaseKind::Write).unwrap();
        Journal::open(&state.operations)
            .unwrap()
            .begin(&plan.execution)
            .unwrap()
    };
    let history = history_owned(&state, history_request(&state)).unwrap();
    assert_eq!(history["pendingOperations"][0], json!(op));
    assert!(inventory_owned(&state, InventoryRequestDto { revision: 3 }).is_err());
    let review = recovery_preview_owned(&state, OperationRequest { operation_id: op }).unwrap();
    assert_eq!(review["choices"], json!(["resume", "restore"]));
    let request = RecoveryRequest {
        recovery_ref: serde_json::from_value(review["recoveryRef"].clone()).unwrap(),
        nonce: serde_json::from_value(review["nonce"].clone()).unwrap(),
        choice: Choice::Resume,
    };
    wait(&state, recovery_owned(&state, request.clone()).unwrap());
    assert!(recovery_owned(&state, request).is_err());
    assert!(dir.path().join("media/renamed.mp4").exists());
}
#[test]
fn empty_pending_list_never_clears_legacy_uncertainty() {
    use vidlib_fileops::coordinator::LeaseKind;
    let (_dir, state) = fixture();
    let lease = state.coordinator.try_acquire(LeaseKind::Write).unwrap();
    {
        let _unfinished_marker = lease.begin_legacy_mutation().unwrap();
    }
    drop(lease);
    let history = history_owned(&state, history_request(&state)).unwrap();
    assert_eq!(history["pendingOperations"], json!([]));
    assert_eq!(history["legacyUnresolved"], true);
    assert!(inventory_owned(&state, InventoryRequestDto { revision: 1 }).is_err());
    assert!(state.coordinator.try_acquire(LeaseKind::Write).is_err());
}
#[test]
fn changed_selection_cannot_reuse_old_inventory_ids() {
    let (dir, state) = fixture();
    let reference = preview(&state);
    let review = confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference,
        },
    )
    .unwrap();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    install_selection(&state, vec![elsewhere]).unwrap();
    assert!(apply_owned(
        &state,
        ApplyRequest {
            plan_ref: reference,
            nonce: review.nonce
        }
    )
    .is_err());
    assert!(
        inventory_page_owned(
            &state,
            InventoryPageRequest {
                query: PageQuery::new(2)
            }
        )
        .is_err(),
        "old inventory survived source selection change"
    );
}
#[test]
fn all_selected_conflicts_block_apply_beyond_first_page() {
    let (dir, state) = fixture();
    let media = dir.path().join("media");
    for index in 0..204 {
        std::fs::write(media.join(format!("clip{index:03}.mp4")), b"same").unwrap();
    }
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    let first = inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery {
                limit: usize::MAX,
                ..PageQuery::new(1)
            },
        },
    )
    .unwrap();
    assert_eq!(first.items.len(), 200);
    assert!(first.next_cursor.is_some());
    let second = inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery {
                cursor: first.next_cursor.clone(),
                ..PageQuery::new(1)
            },
        },
    )
    .unwrap();
    assert_eq!(second.items.len(), 5);
    let selected = first
        .items
        .iter()
        .chain(&second.items)
        .map(|v| serde_json::from_value(v["itemId"].clone()).unwrap())
        .collect();
    let result = wait(
        &state,
        preview_owned(
            &state,
            RenameRequestDto {
                revision: 2,
                selected_ids: selected,
                rule: NamingRule::FixedStem,
                fixed_stem: "collision".into(),
            },
        )
        .unwrap(),
    );
    assert_eq!(result["counts"]["blocked"], "205");
    let reference = serde_json::from_value(result["planRef"].clone()).unwrap();
    let page = page_owned(
        &state,
        PageRequest {
            plan_ref: reference,
            query: PageQuery {
                limit: usize::MAX,
                ..PageQuery::new(2)
            },
        },
    )
    .unwrap();
    assert_eq!(page.items.len(), 200);
    assert!(confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference
        }
    )
    .is_err());
    assert!(media.join("original.mp4").exists());
}
#[test]
fn directory_capture_preserves_unselected_nested_children_for_undo() {
    use vidlib_fileops::coordinator::LeaseKind;
    let (dir, state) = fixture();
    let nested = dir.path().join("media/folder/nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("deep.mkv"), b"deep media").unwrap();
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    let page = inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery::new(1),
        },
    )
    .unwrap();
    let id = serde_json::from_value(
        page.items
            .iter()
            .find(|v| v["displayName"] == "folder")
            .unwrap()["itemId"]
            .clone(),
    )
    .unwrap();
    let result = wait(
        &state,
        preview_owned(
            &state,
            RenameRequestDto {
                revision: 2,
                selected_ids: vec![id],
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
            },
        )
        .unwrap(),
    );
    assert_eq!(result["counts"]["blocked"], "0");
    let reference: UiPlanRef = serde_json::from_value(result["planRef"].clone()).unwrap();
    {
        let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
        let mut registry = state.plans.lock().unwrap();
        let plan = registry.current(reference.into(), &lease).unwrap();
        assert!(plan
            .execution
            .expected_namespace
            .parents
            .iter()
            .any(|p| p.parent_path == nested));
    }
    let review = confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference,
        },
    )
    .unwrap();
    let done = wait(
        &state,
        apply_owned(
            &state,
            ApplyRequest {
                plan_ref: reference,
                nonce: review.nonce,
            },
        )
        .unwrap(),
    );
    let op = serde_json::from_value(done["operationId"].clone()).unwrap();
    let inverse = wait(
        &state,
        undo_owned(
            &state,
            UndoRequest {
                operation_id: op,
                revision: 3,
            },
        )
        .unwrap(),
    );
    assert_eq!(inverse["counts"]["blocked"], "0");
    let reference = serde_json::from_value(inverse["planRef"].clone()).unwrap();
    let review = confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference,
        },
    )
    .unwrap();
    wait(
        &state,
        apply_owned(
            &state,
            ApplyRequest {
                plan_ref: reference,
                nonce: review.nonce,
            },
        )
        .unwrap(),
    );
    assert_eq!(
        std::fs::read(nested.join("deep.mkv")).unwrap(),
        b"deep media"
    );
}
#[test]
fn managed_databases_are_excluded_from_rename_inventory() {
    let (dir, state) = fixture();
    install_selection(&state, vec![dir.path().join("app")]).unwrap();
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    let page = inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery::new(1),
        },
    )
    .unwrap();
    assert!(
        page.items.is_empty(),
        "managed database namespace must be protected from Rename"
    );
}
#[test]
fn carrying_private_namespace_is_blocked_but_unrelated_descendants_work() {
    let dir = tempfile::tempdir().unwrap();
    let private = dir.path().join("container/private/app");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::create_dir_all(dir.path().join("container/public")).unwrap();
    std::fs::write(dir.path().join("container/public/clip.mp4"), b"media").unwrap();
    let state = AppState::open(&private, dir.path().join("libraries.json")).unwrap();
    install_selection(&state, vec![dir.path().join("container")]).unwrap();
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    let entries = inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery::new(1),
        },
    )
    .unwrap();
    assert!(!entries
        .items
        .iter()
        .any(|v| v["displayName"] == "operations.sqlite3"));
    let choose = |name: &str| {
        serde_json::from_value(
            entries
                .items
                .iter()
                .find(|v| v["displayName"] == name)
                .unwrap()["itemId"]
                .clone(),
        )
        .unwrap()
    };
    let denied = wait(
        &state,
        preview_owned(
            &state,
            RenameRequestDto {
                revision: 2,
                selected_ids: vec![choose("private")],
                rule: NamingRule::FixedStem,
                fixed_stem: "moved".into(),
            },
        )
        .unwrap(),
    );
    assert_eq!(denied["counts"]["blocked"], "1");
    assert!(private.join("operations.sqlite3").exists());
    let allowed = wait(
        &state,
        preview_owned(
            &state,
            RenameRequestDto {
                revision: 3,
                selected_ids: vec![choose("clip.mp4")],
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
            },
        )
        .unwrap(),
    );
    assert_eq!(allowed["counts"]["blocked"], "0");
    let reference = serde_json::from_value(allowed["planRef"].clone()).unwrap();
    let confirmation = confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference,
        },
    )
    .unwrap();
    wait(
        &state,
        apply_owned(
            &state,
            ApplyRequest {
                plan_ref: reference,
                nonce: confirmation.nonce,
            },
        )
        .unwrap(),
    );
    assert!(dir.path().join("container/public/renamed.mp4").exists());
    assert!(private.join("operations.sqlite3").exists());
    install_selection(&state, vec![private.join("operations.sqlite3")]).unwrap();
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 4 }).unwrap(),
    );
    assert!(inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery::new(4)
        }
    )
    .unwrap()
    .items
    .is_empty());
}
#[test]
fn empty_directory_rename_retains_complete_namespace_for_undo() {
    let (dir, state) = fixture();
    std::fs::create_dir(dir.path().join("media/empty")).unwrap();
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    let page = inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery::new(1),
        },
    )
    .unwrap();
    let id = serde_json::from_value(
        page.items
            .iter()
            .find(|v| v["displayName"] == "empty")
            .unwrap()["itemId"]
            .clone(),
    )
    .unwrap();
    let result = wait(
        &state,
        preview_owned(
            &state,
            RenameRequestDto {
                revision: 2,
                selected_ids: vec![id],
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
            },
        )
        .unwrap(),
    );
    let reference = serde_json::from_value(result["planRef"].clone()).unwrap();
    let review = confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference,
        },
    )
    .unwrap();
    let done = wait(
        &state,
        apply_owned(
            &state,
            ApplyRequest {
                plan_ref: reference,
                nonce: review.nonce,
            },
        )
        .unwrap(),
    );
    let result = wait(
        &state,
        undo_owned(
            &state,
            UndoRequest {
                operation_id: serde_json::from_value(done["operationId"].clone()).unwrap(),
                revision: 3,
            },
        )
        .unwrap(),
    );
    assert_eq!(
        result["counts"]["blocked"], "0",
        "fresh empty directory cannot replace missing historical listing"
    );
}
#[test]
fn native_picker_result_cannot_replace_newer_inputs() {
    use vidlib_tauri::selection::install_selection_at;
    let (dir, state) = fixture();
    let _reference = preview(&state);
    let replacement = dir.path().join("replacement");
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(replacement.join("other.mp4"), b"other").unwrap();
    assert!(install_selection_at(&state, vec![replacement], Some(1)).is_err());
    let page = inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery::new(2),
        },
    )
    .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0]["displayName"], "original.mp4");
    let session = session_owned(&state, EmptyRequest {}).unwrap();
    update_inputs_owned(
        &state,
        InputRevision {
            revision: 9_007_199_254_740_993,
        },
    )
    .unwrap();
    assert_eq!(
        session_owned(&state, EmptyRequest {}).unwrap()["revision"],
        "9007199254740993"
    );
    let restarted =
        AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    assert_ne!(
        session["sessionId"],
        session_owned(&restarted, EmptyRequest {}).unwrap()["sessionId"]
    );
}
fn prepare_pending(state: &AppState) -> vidlib_inventory::ids::OperationId {
    use vidlib_fileops::{coordinator::LeaseKind, journal::Journal};
    let reference = preview(state);
    let plan = {
        let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
        state
            .plans
            .lock()
            .unwrap()
            .current(reference.into(), &lease)
            .unwrap()
            .clone()
    };
    let _lease = state.coordinator.try_acquire(LeaseKind::Write).unwrap();
    Journal::open(&state.operations)
        .unwrap()
        .begin(&plan.execution)
        .unwrap()
}
#[test]
fn recovery_reconciliation_cannot_republish_after_edit() {
    let (dir, state) = fixture();
    let op = prepare_pending(&state);
    let inspection = prepare_recovery_owned(&state, OperationRequest { operation_id: op }).unwrap();
    update_inputs_owned(&state, InputRevision { revision: 3 }).unwrap();
    let result = publish_recovery_owned(&state, inspection);
    assert!(
        result.is_err(),
        "edit-raced reconciliation republished an executable review: {result:?}"
    );
    assert!(dir.path().join("media/original.mp4").exists());
}
#[test]
fn recovery_reconciliation_cannot_republish_after_native_replacement() {
    let (dir, state) = fixture();
    let op = prepare_pending(&state);
    let inspection = prepare_recovery_owned(&state, OperationRequest { operation_id: op }).unwrap();
    let replacement = dir.path().join("replacement");
    std::fs::create_dir(&replacement).unwrap();
    install_selection(&state, vec![replacement]).unwrap();
    let result = publish_recovery_owned(&state, inspection);
    assert!(
        result.is_err(),
        "native-selection-raced reconciliation republished an executable review: {result:?}"
    );
    assert!(dir.path().join("media/original.mp4").exists());
}
#[test]
fn history_accepts_the_owned_page_query_contract() {
    let golden: Value = serde_json::from_str(include_str!(
        "../../tests/ui/fixtures/command-contracts.json"
    ))
    .unwrap();
    let request = serde_json::from_value::<HistoryRequest>(golden["historyRequest"].clone());
    assert!(
        request.is_ok(),
        "approved history PageQuery was rejected: {:?}",
        request.err()
    );
    let request: HistoryRequest = serde_json::from_value(golden["historyRequest"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        golden["historyRequest"]
    );
    let response: HistoryResponse =
        serde_json::from_value(golden["historyResponse"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        golden["historyResponse"]
    );
    let request: RecoveryPageRequest =
        serde_json::from_value(golden["recoveryPageRequest"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        golden["recoveryPageRequest"]
    );
}
#[test]
fn recovery_all_actions_are_reachable_through_one_review() {
    use vidlib_fileops::{coordinator::LeaseKind, journal::Journal};
    let (dir, state) = fixture();
    for index in 0..204 {
        std::fs::write(
            dir.path().join(format!("media/clip{index:03}.e{index:03}")),
            b"media",
        )
        .unwrap();
    }
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    let first = inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery::new(1),
        },
    )
    .unwrap();
    let last = inventory_page_owned(
        &state,
        InventoryPageRequest {
            query: PageQuery {
                cursor: first.next_cursor,
                ..PageQuery::new(1)
            },
        },
    )
    .unwrap();
    let selected = first
        .items
        .iter()
        .chain(&last.items)
        .map(|v| serde_json::from_value(v["itemId"].clone()).unwrap())
        .collect();
    let result = wait(
        &state,
        preview_owned(
            &state,
            RenameRequestDto {
                revision: 2,
                selected_ids: selected,
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
            },
        )
        .unwrap(),
    );
    let reference: UiPlanRef = serde_json::from_value(result["planRef"].clone()).unwrap();
    let plan = {
        let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
        state
            .plans
            .lock()
            .unwrap()
            .current(reference.into(), &lease)
            .unwrap()
            .clone()
    };
    let op = {
        let _lease = state.coordinator.try_acquire(LeaseKind::Write).unwrap();
        Journal::open(&state.operations)
            .unwrap()
            .begin(&plan.execution)
            .unwrap()
    };
    let review = recovery_preview_owned(&state, OperationRequest { operation_id: op }).unwrap();
    assert_eq!(review["actionCount"], "205");
    assert!(
        review["page"]["nextCursor"].is_string(),
        "review permanently truncated actions beyond the first 200"
    );
    let reference = serde_json::from_value(review["recoveryRef"].clone()).unwrap();
    let page = recovery_page_owned(
        &state,
        RecoveryPageRequest {
            recovery_ref: reference,
            query: PageQuery {
                cursor: serde_json::from_value(review["page"]["nextCursor"].clone()).unwrap(),
                ..PageQuery::new(2)
            },
        },
    )
    .unwrap();
    assert_eq!(page.items.len(), 5);
    assert!(page.next_cursor.is_none());
    let mut ids: Vec<_> = review["page"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(&page.items)
        .map(|a| a["stepId"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 205);
    let encoded = serde_json::to_string(&page).unwrap();
    for native in [
        "beforePath",
        "afterPath",
        "platform",
        "bytes",
        "sourcePath",
        "targetPath",
    ] {
        assert!(
            !encoded.contains(&format!("\"{native}\"")),
            "native recovery path encoding crossed IPC"
        );
    }
    update_inputs_owned(&state, InputRevision { revision: 3 }).unwrap();
    assert!(recovery_page_owned(
        &state,
        RecoveryPageRequest {
            recovery_ref: reference,
            query: PageQuery::new(2)
        }
    )
    .is_err());
    let fresh = recovery_preview_owned(&state, OperationRequest { operation_id: op }).unwrap();
    let fresh_ref = serde_json::from_value(fresh["recoveryRef"].clone()).unwrap();
    assert!(recovery_page_owned(
        &state,
        RecoveryPageRequest {
            recovery_ref: fresh_ref,
            query: PageQuery::new(4)
        }
    )
    .is_err());
    let replacement = dir.path().join("replacement");
    std::fs::create_dir(&replacement).unwrap();
    install_selection(&state, vec![replacement]).unwrap();
    assert!(recovery_page_owned(
        &state,
        RecoveryPageRequest {
            recovery_ref: fresh_ref,
            query: PageQuery::new(3)
        }
    )
    .is_err());
    assert!(recovery_owned(
        &state,
        RecoveryRequest {
            recovery_ref: fresh_ref,
            choice: Choice::Resume,
            nonce: serde_json::from_value(fresh["nonce"].clone()).unwrap()
        }
    )
    .is_err());
}
#[test]
fn history_query_pages_filter_sort_and_reject_changed_data_revision() {
    use vidlib_fileops::{
        coordinator::LeaseKind,
        journal::{Journal, OperationState},
    };
    let (_dir, state) = fixture();
    let reference = preview(&state);
    let plan = {
        let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
        state
            .plans
            .lock()
            .unwrap()
            .snapshot(reference.into(), &lease)
            .unwrap()
    };
    let mut ids = Vec::new();
    {
        let _lease = state.coordinator.try_acquire(LeaseKind::Write).unwrap();
        let mut journal = Journal::open(&state.operations).unwrap();
        for _ in 0..205 {
            let op = journal.begin(&plan.execution).unwrap();
            journal.set_state(op, OperationState::Cancelled).unwrap();
            ids.push(op);
        }
    }
    let request = history_request(&state);
    let revision = request.query.expected_revision;
    let first = history_owned(
        &state,
        HistoryRequest {
            query: PageQuery {
                limit: usize::MAX,
                ..request.query.clone()
            },
        },
    )
    .unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 200);
    assert_eq!(first["items"][0]["operationId"], json!(ids[0]));
    assert_eq!(first["revision"], revision.to_string());
    let cursor: Option<String> = serde_json::from_value(first["nextCursor"].clone()).unwrap();
    let last = history_owned(
        &state,
        HistoryRequest {
            query: PageQuery {
                cursor: cursor.clone(),
                ..request.query.clone()
            },
        },
    )
    .unwrap();
    assert_eq!(last["items"].as_array().unwrap().len(), 5);
    assert_eq!(last["items"][4]["operationId"], json!(ids[204]));
    let filtered = history_owned(
        &state,
        HistoryRequest {
            query: PageQuery {
                limit: 1,
                filter: ids[7].to_string(),
                sort: "state".into(),
                ..request.query.clone()
            },
        },
    )
    .unwrap();
    assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["items"][0]["operationId"], json!(ids[7]));
    {
        let _lease = state.coordinator.try_acquire(LeaseKind::Write).unwrap();
        Journal::open(&state.operations)
            .unwrap()
            .remove_history(ids[0])
            .unwrap();
    }
    assert_eq!(
        history_owned(&state, request.clone()).unwrap_err().code,
        "staleState"
    );
    let current = history_request(&state);
    assert!(current.query.expected_revision > revision);
    let continuation = history_owned(
        &state,
        HistoryRequest {
            query: PageQuery {
                cursor,
                ..current.query
            },
        },
    )
    .unwrap();
    assert_eq!(
        continuation["items"].as_array().unwrap().len(),
        5,
        "history removal shifted the stable keyset range"
    );
}
#[test]
fn delayed_older_preview_dispatch_cannot_replace_current_backend_plan() {
    let (_dir, state) = fixture();
    let reference = preview(&state);
    let first = page_owned(
        &state,
        PageRequest {
            plan_ref: reference,
            query: PageQuery::new(2),
        },
    )
    .unwrap();
    let item = serde_json::from_value(first.items[0]["itemId"].clone()).unwrap();
    let latest = wait(
        &state,
        preview_owned(
            &state,
            RenameRequestDto {
                revision: 4,
                selected_ids: vec![item],
                rule: NamingRule::FixedStem,
                fixed_stem: "latest".into(),
            },
        )
        .unwrap(),
    );
    assert!(preview_owned(
        &state,
        RenameRequestDto {
            revision: 3,
            selected_ids: vec![item],
            rule: NamingRule::FixedStem,
            fixed_stem: "late old".into()
        }
    )
    .is_err());
    let reference = serde_json::from_value(latest["planRef"].clone()).unwrap();
    assert!(confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference
        }
    )
    .is_ok());
}
