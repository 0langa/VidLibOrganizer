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
                recipe_json: None,
                rebind_selection: false,
                revision: 2,
                selected_ids: selected,
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
                selection_filter: None,
                numbering_scope: Default::default(),
                row_overrides: vec![],
                companions: None,
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
                recipe_json: None,
                rebind_selection: false,
                revision: 2,
                selected_ids: selected,
                rule: NamingRule::FixedStem,
                fixed_stem: "collision".into(),
                selection_filter: None,
                numbering_scope: Default::default(),
                row_overrides: vec![],
                companions: None,
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
                recipe_json: None,
                rebind_selection: false,
                revision: 2,
                selected_ids: vec![id],
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
                selection_filter: None,
                numbering_scope: Default::default(),
                row_overrides: vec![],
                companions: None,
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
                recipe_json: None,
                rebind_selection: false,
                revision: 2,
                selected_ids: vec![choose("private")],
                rule: NamingRule::FixedStem,
                fixed_stem: "moved".into(),
                selection_filter: None,
                numbering_scope: Default::default(),
                row_overrides: vec![],
                companions: None,
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
                recipe_json: None,
                rebind_selection: false,
                revision: 3,
                selected_ids: vec![choose("clip.mp4")],
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
                selection_filter: None,
                numbering_scope: Default::default(),
                row_overrides: vec![],
                companions: None,
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
                recipe_json: None,
                rebind_selection: false,
                revision: 2,
                selected_ids: vec![id],
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
                selection_filter: None,
                numbering_scope: Default::default(),
                row_overrides: vec![],
                companions: None,
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
                recipe_json: None,
                rebind_selection: false,
                revision: 2,
                selected_ids: selected,
                rule: NamingRule::FixedStem,
                fixed_stem: "renamed".into(),
                selection_filter: None,
                numbering_scope: Default::default(),
                row_overrides: vec![],
                companions: None,
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
                recipe_json: None,
                rebind_selection: false,
                revision: 4,
                selected_ids: vec![item],
                rule: NamingRule::FixedStem,
                fixed_stem: "latest".into(),
                selection_filter: None,
                numbering_scope: Default::default(),
                row_overrides: vec![],
                companions: None,
            },
        )
        .unwrap(),
    );
    assert!(preview_owned(
        &state,
        RenameRequestDto {
            recipe_json: None,
            rebind_selection: false,
            revision: 3,
            selected_ids: vec![item],
            rule: NamingRule::FixedStem,
            fixed_stem: "late old".into(),
            selection_filter: None,
            numbering_scope: Default::default(),
            row_overrides: vec![],
            companions: None,
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

#[test]
fn nested_rename_wire_golden_is_camel_case_and_exact() {
    let golden: Value = serde_json::from_str(include_str!(
        "../../tests/ui/fixtures/command-contracts.json"
    ))
    .unwrap();
    let literal = &golden["renameRequest"];
    let mut filter_only = literal.clone();
    filter_only["rowOverrides"] = json!([]);
    let mut override_only = literal.clone();
    override_only["selectionFilter"] = Value::Null;
    let failures = [
        ("complete", literal.clone()),
        ("filter only", filter_only),
        ("override only", override_only),
    ]
    .into_iter()
    .filter_map(|(label, payload)| {
        serde_json::from_value::<RenameRequestDto>(payload)
            .err()
            .map(|error| format!("{label}: {error}"))
    })
    .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "camelCase request variants rejected: {failures:?}"
    );
    let parsed = serde_json::from_value::<RenameRequestDto>(literal.clone());
    assert!(
        parsed.is_ok(),
        "agreed nested camelCase golden rejected: {:?}",
        parsed.err()
    );
    for (wire, expected) in [(json!({}), true), (json!({"recursive":false}), false)] {
        let mut payload = literal.clone();
        payload["selectionFilter"] = wire;
        let request: RenameRequestDto = serde_json::from_value(payload).unwrap();
        let filter: vidlib_rename::SelectionFilter = request.selection_filter.unwrap().into();
        assert_eq!(filter.recursive, expected);
    }
    let request = parsed.unwrap();
    assert_eq!(request.revision, 9_007_199_254_740_993);
    let filter: vidlib_rename::SelectionFilter = request.selection_filter.unwrap().into();
    assert!(filter.recursive);
    assert_eq!(filter.max_depth, Some(4));
    assert_eq!(filter.excluded_ids, vec![request.selected_ids[1]]);
    assert_eq!(
        filter.kinds,
        vec![
            vidlib_inventory::EntryKind::File,
            vidlib_inventory::EntryKind::ReparsePoint
        ]
    );
    assert_eq!(filter.extensions, vec!["mkv", "mp4"]);
    assert_eq!(filter.name_length.as_ref().unwrap().max, Some(256));
    assert_eq!(filter.path_length.as_ref().unwrap().min, Some(1));
    assert_eq!(
        filter.size.as_ref().unwrap().min,
        Some(9_007_199_254_740_993)
    );
    assert_eq!(filter.size.as_ref().unwrap().max, Some(u64::MAX));
    assert_eq!(
        filter.created_ns.as_ref().unwrap().min,
        Some(-9_007_199_254_740_993)
    );
    assert_eq!(
        filter.created_ns.as_ref().unwrap().max,
        Some(1_700_000_000_000_000_000_123_456_789)
    );
    assert_eq!(filter.modified_ns.as_ref().unwrap().min, Some(0));
    assert_eq!(
        filter.accessed_ns.as_ref().unwrap().max,
        Some(9_007_199_254_740_993)
    );
    assert_eq!(filter.attributes.as_ref().unwrap().required, 2);
    assert_eq!(filter.attributes.as_ref().unwrap().forbidden, 1);
    assert_eq!(
        filter.include[0].field,
        vidlib_rename::SelectionField::RelativePath
    );
    assert_eq!(
        filter.include[0].syntax,
        vidlib_rename::PatternSyntax::Wildcard
    );
    assert_eq!(filter.include[0].value, "Original.*");
    assert!(!filter.include[0].case_sensitive);
    assert_eq!(
        filter.exclude[0].field,
        vidlib_rename::SelectionField::FullPath
    );
    assert_eq!(
        filter.exclude[0].syntax,
        vidlib_rename::PatternSyntax::Regex
    );
    assert!(filter.exclude[0].case_sensitive);
    let edit: vidlib_rename::RowOverride = request.row_overrides.into_iter().next().unwrap().into();
    assert_eq!(edit.entry_id, request.selected_ids[0]);
    assert_eq!(edit.new_name, "=literal,名.mkv");
    // Desktop casing is a conversion boundary; portable recipe data stays stable.
    let core = serde_json::to_value(filter).unwrap();
    assert_eq!(core["max_depth"], 4);
    assert_eq!(core["created_ns"]["min"], "-9007199254740993");
    assert_eq!(core["include"][0]["field"], "relative_path");
    assert_eq!(core["include"][0]["case_sensitive"], false);
    assert_eq!(
        serde_json::to_value(edit).unwrap()["new_name"],
        "=literal,名.mkv"
    );
}
#[test]
fn nested_rename_wire_rejects_unknown_fields_tags_and_numeric_bounds() {
    let golden: Value = serde_json::from_str(include_str!(
        "../../tests/ui/fixtures/command-contracts.json"
    ))
    .unwrap();
    let literal = &golden["renameRequest"];
    assert!(
        serde_json::from_value::<RenameRequestDto>(literal.clone()).is_ok(),
        "valid golden must be accepted before negative checks"
    );
    for pointer in [
        "",
        "/selectionFilter",
        "/selectionFilter/include/0",
        "/selectionFilter/size",
        "/selectionFilter/attributes",
        "/rowOverrides/0",
    ] {
        let mut invalid = literal.clone();
        invalid
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), json!(true));
        assert!(
            serde_json::from_value::<RenameRequestDto>(invalid).is_err(),
            "unknown field at {pointer}"
        );
    }
    for (pointer, old, new) in [
        ("/selectionFilter", "maxDepth", "max_depth"),
        ("/selectionFilter", "excludedIds", "excluded_ids"),
        ("/selectionFilter", "createdNs", "created_ns"),
        (
            "/selectionFilter/include/0",
            "caseSensitive",
            "case_sensitive",
        ),
        ("/rowOverrides/0", "entryId", "entry_id"),
        ("/rowOverrides/0", "newName", "new_name"),
    ] {
        let mut invalid = literal.clone();
        let object = invalid
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap();
        let value = object.remove(old).unwrap();
        object.insert(new.into(), value);
        assert!(
            serde_json::from_value::<RenameRequestDto>(invalid).is_err(),
            "portable member {new} accepted on desktop wire"
        );
    }
    for (pointer, value) in [
        ("/selectionFilter/kinds/0", json!("File")),
        ("/selectionFilter/include/0/field", json!("relative_path")),
        ("/selectionFilter/include/0/syntax", json!("unknown")),
    ] {
        let mut invalid = literal.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            serde_json::from_value::<RenameRequestDto>(invalid).is_err(),
            "invalid desktop tag at {pointer}"
        );
    }
    for pointer in [
        "/selectionFilter/size/min",
        "/selectionFilter/size/max",
        "/selectionFilter/nameLength/min",
        "/selectionFilter/pathLength/min",
        "/selectionFilter/createdNs/min",
        "/selectionFilter/modifiedNs/min",
        "/selectionFilter/accessedNs/max",
    ] {
        for value in [
            json!(2),
            json!(9007199254740992u64),
            json!(9007199254740992.0),
        ] {
            let mut invalid = literal.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(
                serde_json::from_value::<RenameRequestDto>(invalid).is_err(),
                "numeric quantity accepted at {pointer}"
            );
        }
    }
}

#[test]
fn saved_ordered_recipe_preview_rebinds_explicitly_and_revokes_after_edit() {
    use vidlib_tauri::rename_preferences::*;
    let (_dir, state) = fixture();
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
    let selected = vec![page.items[0]["itemId"].clone()];
    let recipe = json!({"schema_version":1,"revision":"9007199254740993","selection":[],"numbering_order":[],"numbering_scope":"files","rules":[
        {"id":"first","enabled":true,"segment":{"type":"stem"},"parameters":{"type":"fixed_stem","value":"one"}},
        {"id":"second","enabled":true,"segment":{"type":"stem"},"parameters":{"type":"text","operation":{"type":"fixed","value":"two"}}},
        {"id":"disabled","enabled":false,"segment":{"type":"stem"},"parameters":{"type":"fixed_stem","value":"wrong"}}
    ],"companions":{"type":"none"},"destination":{"type":"same_directory"},"collision_policy":{"type":"block"},"metadata_changes":[]});
    let saved = save_owned(
        &state,
        SavePresetRequest {
            preset_id: None,
            expected_revision: None,
            recipe_json: recipe.to_string(),
        },
    )
    .unwrap();
    let loaded = load_owned(
        &state,
        LoadPresetRequest {
            preset_id: saved.preset_id,
            revision: Some("1".into()),
        },
    )
    .unwrap();
    let parsed = serde_json::from_value::<RenameRequestDto>(
        json!({"revision":"2","selectedIds":selected,"rule":"keep","fixedStem":"","recipeJson":loaded.recipe_json,"rebindSelection":true}),
    );
    assert!(
        parsed.is_ok(),
        "ordered recipe DTO rejected: {:?}",
        parsed.err()
    );
    let result = wait(&state, preview_owned(&state, parsed.unwrap()).unwrap());
    let reference: UiPlanRef = serde_json::from_value(result["planRef"].clone()).unwrap();
    let rows = page_owned(
        &state,
        PageRequest {
            plan_ref: reference,
            query: PageQuery::new(2),
        },
    )
    .unwrap();
    assert_eq!(rows.items[0]["newName"], "two.mp4");
    let confirmation = confirm_owned(
        &state,
        PlanRequest {
            plan_ref: reference,
        },
    )
    .unwrap();
    update_inputs_owned(&state, InputRevision { revision: 3 }).unwrap();
    assert!(apply_owned(
        &state,
        ApplyRequest {
            plan_ref: reference,
            nonce: confirmation.nonce
        }
    )
    .is_err());
    let unknown = "00000000-0000-4000-8000-000000000099";
    let parsed:RenameRequestDto=serde_json::from_value(json!({"revision":"4","selectedIds":[unknown],"recipeJson":loaded.recipe_json,"rebindSelection":true})).unwrap();
    let job = preview_owned(&state, parsed).unwrap();
    for _ in 0..500 {
        let result = status_owned(&state, JobRequest { job_id: job }).unwrap();
        if result.state != "running" {
            assert_eq!(result.error.unwrap().code, "unknownEntry");
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("job timeout");
}

#[test]
fn recipe_document_boundaries_and_retained_exclusions_fail_closed() {
    let (_dir, state) = fixture();
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
    let selected = vec![page.items[0]["itemId"].clone()];
    let empty = json!({"schema_version":1,"revision":"1","selection":[],"numbering_order":[],"rules":[],"companions":{"type":"none"},"destination":{"type":"same_directory"},"collision_policy":{"type":"block"},"metadata_changes":[]});
    for (label, document, want) in [
        ("oversize", " ".repeat(1024 * 1024 + 1), "importLimit"),
        (
            "version",
            empty
                .to_string()
                .replace("\"schema_version\":1", "\"schema_version\":2"),
            "unknownRecipeVersion",
        ),
        (
            "unknown tag",
            empty.to_string().replace("same_directory", "script"),
            "invalidRecipe",
        ),
    ] {
        let request:RenameRequestDto=serde_json::from_value(json!({"revision":"2","selectedIds":selected,"recipeJson":document,"rebindSelection":true})).unwrap();
        match preview_owned(&state, request) {
            Err(error) => assert_eq!(error.code, want, "{label}"),
            Ok(job) => {
                let mut completed = false;
                for _ in 0..500 {
                    let status = status_owned(&state, JobRequest { job_id: job }).unwrap();
                    if status.state != "running" {
                        assert_eq!(status.error.unwrap().code, want, "{label}");
                        completed = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(completed, "{label} timed out");
            }
        }
    }
    for (label, document, rebind, want) in [
        ("no implicit rebind", empty.clone(), false, "staleState"),
        (
            "unknown exclusion",
            {
                let mut value = empty.clone();
                value["selection_filter"] =
                    json!({"excluded_ids":["00000000-0000-4000-8000-000000000099"]});
                value
            },
            true,
            "unknownEntry",
        ),
    ] {
        let request:RenameRequestDto=serde_json::from_value(json!({"revision":"2","selectedIds":selected,"recipeJson":document.to_string(),"rebindSelection":rebind})).unwrap();
        let job = preview_owned(&state, request).unwrap();
        let mut finished = false;
        for _ in 0..500 {
            let status = status_owned(&state, JobRequest { job_id: job }).unwrap();
            if status.state != "running" {
                assert_eq!(status.error.unwrap().code, want, "{label}");
                finished = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(finished, "{label} timed out");
    }
}

#[test]
fn preset_document_wire_matches_shared_camel_case_golden_without_schema_rewriting() {
    use vidlib_tauri::rename_preferences::*;
    let golden: Value = serde_json::from_str(include_str!(
        "../../tests/ui/fixtures/command-contracts.json"
    ))
    .unwrap();
    let request: SavePresetRequest =
        serde_json::from_value(golden["presetSaveRequest"].clone()).unwrap();
    let document: RecipeDocumentRequest =
        serde_json::from_value(golden["recipeDocumentRequest"].clone()).unwrap();
    assert_eq!(request.recipe_json, document.recipe_json);
    let recipe =
        vidlib_rename::presets::PresetStore::import(document.recipe_json.as_bytes()).unwrap();
    assert_eq!(recipe.revision, 9007199254740993);
    assert_eq!(
        recipe.numbering_scope,
        vidlib_rename::NumberingScope::Directories
    );
    assert!(!recipe.rules[1].enabled);
    assert!(matches!(
        recipe.rules[1].segment,
        vidlib_rename::SegmentSelector::Between { occurrence: 2, .. }
    ));
    let dto = PresetDto {
        preset_id: serde_json::from_value(golden["presetDto"]["presetId"].clone()).unwrap(),
        revision: "9007199254740993".into(),
        recipe_json: document.recipe_json,
    };
    assert_eq!(serde_json::to_value(dto).unwrap(), golden["presetDto"]);
    for bad in [
        json!({"recipe_json":"{}"}),
        json!({"recipeJson":{}}),
        json!({"recipeJson":"{}","path":"/media"}),
    ] {
        assert!(serde_json::from_value::<RecipeDocumentRequest>(bad).is_err());
    }
}

#[test]
fn real_metadata_reaches_ordered_and_legacy_template_preview() {
    use vidlib_rename::*;
    use vidlib_tauri::metadata::MediaInfoProvider;
    let (dir, mut state) = fixture();
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/media/metadata.mp4"),
        dir.path().join("media/original.mp4"),
    )
    .unwrap();
    #[cfg(not(windows))]
    let (exe, version) = (PathBuf::from("/usr/bin/mediainfo"), "25.04");
    #[cfg(windows)]
    let (exe, version) = (
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/mediainfo/windows-x86_64/MediaInfo.exe"),
        "25.10",
    );
    state.metadata_provider =
        std::sync::Arc::new(MediaInfoProvider::from_executable(exe, version.into()));
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
    let selected = vec![serde_json::from_value(page.items[0]["itemId"].clone()).unwrap()];
    let parts = vec![
        TemplatePart::Field {
            key: FieldKey::Media {
                property: MediaProperty::Title,
            },
            format: ValueFormat::Text,
            fallback: None,
        },
        TemplatePart::Literal("-".into()),
        TemplatePart::Field {
            key: FieldKey::Media {
                property: MediaProperty::Width,
            },
            format: ValueFormat::Text,
            fallback: None,
        },
    ];
    for (revision, ordered) in [(2, false), (3, true)] {
        let recipe = RecipeV1 {
            schema_version: 1,
            revision,
            selection: vec![],
            selection_filter: None,
            numbering_order: vec![],
            numbering_scope: Default::default(),
            rules: vec![
                Rule {
                    id: "first".into(),
                    enabled: true,
                    segment: SegmentSelector::Stem,
                    parameters: RuleParameters::FixedStem {
                        value: "discarded".into(),
                    },
                },
                Rule {
                    id: "metadata".into(),
                    enabled: true,
                    segment: SegmentSelector::Stem,
                    parameters: RuleParameters::Template {
                        parts: parts.clone(),
                    },
                },
            ],
            companions: CompanionPolicy::None,
            destination: Destination::SameDirectory,
            collision_policy: CollisionPolicy::Block,
            metadata_changes: vec![],
        };
        let request = RenameRequestDto {
            revision,
            selected_ids: selected.clone(),
            rule: NamingRule::Template {
                parts: parts.clone(),
            },
            fixed_stem: String::new(),
            recipe_json: ordered.then(|| serde_json::to_string(&recipe).unwrap()),
            rebind_selection: ordered,
            selection_filter: None,
            numbering_scope: Default::default(),
            row_overrides: vec![],
            companions: None,
        };
        let result = wait(&state, preview_owned(&state, request).unwrap());
        let reference = serde_json::from_value(result["planRef"].clone()).unwrap();
        let rows = page_owned(
            &state,
            PageRequest {
                plan_ref: reference,
                query: PageQuery::new(revision),
            },
        )
        .unwrap();
        assert_eq!(
            rows.items[0]["newName"], "VidLib fixture-64.mp4",
            "actual backend metadata pipeline, ordered={ordered}: {:?}",
            rows.items
        );
    }
    let request = RenameRequestDto {
        revision: 4,
        selected_ids: selected.clone(),
        rule: NamingRule::Template {
            parts: vec![TemplatePart::Field {
                key: FieldKey::Date {
                    source: DateSource::Recorded,
                },
                format: ValueFormat::Date {
                    pattern: "%Y%m%d".into(),
                },
                fallback: None,
            }],
        },
        fixed_stem: String::new(),
        recipe_json: None,
        rebind_selection: false,
        selection_filter: None,
        numbering_scope: Default::default(),
        row_overrides: vec![],
        companions: None,
    };
    let result = wait(&state, preview_owned(&state, request).unwrap());
    let reference = serde_json::from_value(result["planRef"].clone()).unwrap();
    let rows = page_owned(
        &state,
        PageRequest {
            plan_ref: reference,
            query: PageQuery::new(4),
        },
    )
    .unwrap();
    assert_eq!(rows.items[0]["status"], "blocked");
    assert!(
        rows.items[0]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("timezone is unknown")),
        "civil recording time must not silently become UTC"
    );
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 5,
        selection: vec![],
        selection_filter: None,
        numbering_order: vec![],
        numbering_scope: Default::default(),
        rules: vec![Rule {
            id: "date".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::Date {
                operation: DateOperation::Insert {
                    source: DateSource::Recorded,
                    at: InsertPosition::End,
                    prefix: "-".into(),
                    suffix: String::new(),
                    rule: DateRule {
                        input_format: None,
                        output_format: "%Y%m%d".into(),
                        timezone: None,
                        assume_timezone: Some(FrozenTimezone::Utc),
                        ambiguity: DstAmbiguity::Reject,
                        nonexistent: NonexistentTime::Reject,
                    },
                },
            },
        }],
        companions: CompanionPolicy::None,
        destination: Destination::SameDirectory,
        collision_policy: CollisionPolicy::Block,
        metadata_changes: vec![],
    };
    let request = RenameRequestDto {
        revision: 5,
        selected_ids: selected,
        rule: NamingRule::Keep,
        fixed_stem: String::new(),
        recipe_json: Some(serde_json::to_string(&recipe).unwrap()),
        rebind_selection: true,
        selection_filter: None,
        numbering_scope: Default::default(),
        row_overrides: vec![],
        companions: None,
    };
    let result = wait(&state, preview_owned(&state, request).unwrap());
    let reference = serde_json::from_value(result["planRef"].clone()).unwrap();
    let rows = page_owned(
        &state,
        PageRequest {
            plan_ref: reference,
            query: PageQuery::new(5),
        },
    )
    .unwrap();
    assert_eq!(rows.items[0]["newName"], "original-20240203.mp4");
}

#[test]
fn companion_wire_is_strict_camel_case() {
    let primary = vidlib_inventory::ids::EntryId::new();
    let asset = vidlib_inventory::ids::EntryId::new();
    let base = json!({"revision":"1","selectedIds":[primary,asset],"companions":{"type":"matchExact","associations":[{"primaryId":primary,"companionIds":[asset]}]}});
    assert!(
        serde_json::from_value::<RenameRequestDto>(base.clone()).is_ok(),
        "valid camelCase policy must be accepted"
    );
    for bad in [
        json!({"type":"match_exact","associations":[]}),
        json!({"type":"matchExact","associations":[],"path":"/outside"}),
        json!({"type":"explicit","associations":[{"primary_id":primary,"companionIds":[asset]}]}),
        json!({"type":"explicit","associations":[{"primaryId":primary,"companion_ids":[asset]}]}),
        json!({"type":"explicit","associations":[{"primaryId":primary,"companionIds":[asset],"target":"outside"}]}),
    ] {
        let mut payload = base.clone();
        payload["companions"] = bad;
        assert!(serde_json::from_value::<RenameRequestDto>(payload).is_err());
    }
}

#[test]
fn companions_reach_legacy_and_ordered_preview_before_metadata_and_counter_freezing() {
    use vidlib_rename::*;
    use vidlib_tauri::metadata::MediaInfoProvider;
    let (dir, mut state) = fixture();
    let media = dir.path().join("media");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/media/metadata.mp4");
    for name in ["original.mp4", "second.mp4"] {
        std::fs::copy(&fixture, media.join(name)).unwrap();
    }
    std::fs::write(media.join("original.en.forced.srt"), b"subtitle, not media").unwrap();
    std::fs::write(media.join("second.fr.srt"), b"second subtitle").unwrap();
    #[cfg(not(windows))]
    let (exe, version) = (PathBuf::from("/usr/bin/mediainfo"), "25.04");
    #[cfg(windows)]
    let (exe, version) = (
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/mediainfo/windows-x86_64/MediaInfo.exe"),
        "25.10",
    );
    state.metadata_provider =
        std::sync::Arc::new(MediaInfoProvider::from_executable(exe, version.into()));
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
    let id = |name: &str| {
        page.items
            .iter()
            .find(|r| r["name"] == name || r["displayName"] == name)
            .map(|r| r["itemId"].clone())
            .unwrap_or_else(|| panic!("missing {name}: {:?}", page.items))
    };
    let primary = id("original.mp4");
    let subtitle = id("original.en.forced.srt");
    let selected = vec![
        subtitle.clone(),
        primary.clone(),
        id("second.fr.srt"),
        id("second.mp4"),
    ];
    let parts = vec![
        TemplatePart::Field {
            key: FieldKey::Media {
                property: MediaProperty::Title,
            },
            format: ValueFormat::Text,
            fallback: None,
        },
        TemplatePart::Literal("-".into()),
        TemplatePart::Field {
            key: FieldKey::Counter,
            format: ValueFormat::Text,
            fallback: None,
        },
    ];
    for (revision, ordered) in [(2, false), (3, true)] {
        let mut payload = json!({"revision":revision.to_string(),"selectedIds":selected,"companions":{"type":"matchExact","associations":[]},"rule":{"template":{"parts":parts}}});
        if ordered {
            let recipe = json!({"schema_version":1,"revision":"1","selection":[],"numbering_order":[],"rules":[{"id":"title-counter","enabled":true,"segment":{"type":"stem"},"parameters":{"type":"template","parts":parts}},{"id":"last","enabled":true,"segment":{"type":"stem"},"parameters":{"type":"text","operation":{"type":"suffix","text":"-final"}}}],"companions":{"type":"match_exact","associations":[]},"destination":{"type":"same_directory"},"collision_policy":{"type":"block"},"metadata_changes":[]});
            payload["recipeJson"] = Value::String(recipe.to_string());
            payload["rebindSelection"] = json!(true);
            payload.as_object_mut().unwrap().remove("companions");
            payload.as_object_mut().unwrap().remove("rule");
        }
        let parsed = serde_json::from_value::<RenameRequestDto>(payload);
        assert!(parsed.is_ok(), "{:?}", parsed.err());
        let result = wait(&state, preview_owned(&state, parsed.unwrap()).unwrap());
        assert_eq!(result["counts"]["blocked"], "0", "{result}");
        let plan_ref = serde_json::from_value(result["planRef"].clone()).unwrap();
        let page = page_owned(
            &state,
            PageRequest {
                plan_ref,
                query: PageQuery::new(revision),
            },
        )
        .unwrap();
        let row = |id: &Value| page.items.iter().find(|r| &r["itemId"] == id).unwrap();
        let suffix = if ordered { "-final" } else { "" };
        assert_eq!(
            row(&primary)["newName"],
            format!("VidLib fixture-1{suffix}.mp4")
        );
        assert_eq!(
            row(&subtitle)["newName"],
            format!("VidLib fixture-1{suffix}.en.forced.srt")
        );
        {
            let lease = state
                .coordinator
                .try_acquire(vidlib_fileops::coordinator::LeaseKind::Read)
                .unwrap();
            let stored = state
                .plans
                .lock()
                .unwrap()
                .snapshot(plan_ref.into(), &lease)
                .unwrap();
            let primary_id = serde_json::from_value(primary.clone()).unwrap();
            let second_id = serde_json::from_value(id("second.mp4")).unwrap();
            assert_eq!(
                stored.frozen_inputs.primary_order,
                vec![primary_id, second_id]
            );
            assert!(stored
                .frozen_inputs
                .values
                .iter()
                .all(|v| v.entry_id == primary_id || v.entry_id == second_id));
        }
        assert!(row(&subtitle)["explanations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["ruleId"] == "companion"));
        assert!(row(&subtitle)["explanations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e.get("rule_id").is_none()));
        assert_eq!(row(&subtitle)["association"]["primaryId"], primary);
        assert_eq!(
            row(&subtitle)["association"]["groupId"],
            row(&primary)["association"]["groupId"]
        );
        assert_eq!(
            row(&id("second.mp4"))["newName"],
            format!("VidLib fixture-2{suffix}.mp4")
        );
    }
}

#[test]
fn manual_companion_binding_requires_current_selected_grants() {
    let (dir, state) = fixture();
    let media = dir.path().join("media");
    std::fs::create_dir(media.join("subtitles")).unwrap();
    std::fs::write(media.join("subtitles/unrelated.EN.srt"), b"subtitle").unwrap();
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
    let primary = page
        .items
        .iter()
        .find(|r| r["displayName"] == "original.mp4")
        .unwrap()["itemId"]
        .clone();
    let asset = page
        .items
        .iter()
        .find(|r| r["displayName"] == "unrelated.EN.srt")
        .unwrap()["itemId"]
        .clone();
    let mut payload = json!({"revision":"2","selectedIds":[primary,asset],"rule":"fixedStem","fixedStem":"Final","companions":{"type":"explicit","associations":[{"primaryId":primary,"companionIds":[asset]}]}});
    let request = serde_json::from_value::<RenameRequestDto>(payload.clone()).unwrap();
    let result = wait(&state, preview_owned(&state, request).unwrap());
    let plan_ref = serde_json::from_value(result["planRef"].clone()).unwrap();
    let rows = page_owned(
        &state,
        PageRequest {
            plan_ref,
            query: PageQuery::new(2),
        },
    )
    .unwrap();
    let row = rows.items.iter().find(|r| r["itemId"] == asset).unwrap();
    assert_eq!(row["newName"], "Final.EN.srt");
    assert_eq!(row["association"]["primaryId"], primary);
    assert!(row["newDisplay"].as_str().unwrap().contains("subtitles"));
    payload["revision"] = json!("3");
    payload["selectedIds"] = json!([primary]);
    let job = preview_owned(&state, serde_json::from_value(payload).unwrap()).unwrap();
    let mut completed = false;
    for _ in 0..500 {
        let status = status_owned(&state, JobRequest { job_id: job }).unwrap();
        if status.state != "running" {
            assert_eq!(status.error.unwrap().code, "unselectedEntry");
            completed = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(completed);
    assert!(confirm_owned(&state, PlanRequest { plan_ref }).is_err());
    assert!(media.join("subtitles/unrelated.EN.srt").exists());
}
