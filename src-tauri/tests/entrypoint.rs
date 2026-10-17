use serde_json::Value;
use vidlib_tauri::{app_state::AppState, commands::rename::*};
// Exercise the real picker/drop/argv producer, not the retained legacy subtree seam.
fn install_selection(
    state: &AppState,
    paths: Vec<std::path::PathBuf>,
) -> Result<Option<vidlib_tauri::selection::SelectionDto>, CommandError> {
    let revision = session_owned(state, EmptyRequest {}).unwrap()["revision"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    vidlib_tauri::selection::complete_picker_at(state, Some(paths), revision)
}
#[track_caller]
fn wait(state: &AppState, id: vidlib_inventory::ids::JobId) -> Value {
    for _ in 0..500 {
        let status = status_owned(state, JobRequest { job_id: id }).unwrap();
        if status.state != "running" {
            assert_eq!(status.state, "completed", "{status:?}");
            return status.result.unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timeout")
}
#[test]
fn native_picker_preserves_paths_and_selected_folder_root() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Tree 名");
    std::fs::create_dir_all(folder.join("nested")).unwrap();
    std::fs::write(folder.join("nested/video.mp4"), b"payload").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    install_selection(
        &state,
        vec![folder.clone(), folder.join("nested"), folder.clone()],
    )
    .unwrap();
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
    assert_eq!(
        page.items.len(),
        3,
        "explicit folder root and all descendants, without overlap duplicates"
    );
    assert!(page.items.iter().any(|row| row["displayName"] == "Tree 名"));
    assert_eq!(
        std::fs::read(folder.join("nested/video.mp4")).unwrap(),
        b"payload"
    );
}
fn apply(state: &AppState, result: &Value) -> Value {
    let plan_ref = serde_json::from_value(result["planRef"].clone()).unwrap();
    let review = confirm_owned(state, PlanRequest { plan_ref }).unwrap_or_else(|error| {
        panic!(
            "confirm {result}: {error:?}; rows {:?}",
            page_owned(
                state,
                PageRequest {
                    plan_ref,
                    query: PageQuery::new(
                        result["inputRevision"].as_str().unwrap().parse().unwrap()
                    )
                }
            )
            .unwrap()
            .items
        )
    });
    wait(
        state,
        apply_owned(
            state,
            ApplyRequest {
                plan_ref,
                nonce: review.nonce,
            },
        )
        .unwrap(),
    )
}
#[test]
fn selected_folder_root_preview_execution_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("media/Tree");
    std::fs::create_dir_all(folder.join("nested")).unwrap();
    std::fs::write(folder.join("nested/video.mp4"), b"payload").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    install_selection(&state, vec![folder.clone(), folder.join("nested")]).unwrap();
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
    let selected = page
        .items
        .iter()
        .filter(|row| row["kind"] == "directory")
        .map(|row| row["itemId"].clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    let request=serde_json::from_value(serde_json::json!({"revision":"2","selectedIds":selected,"rule":"fixedStem","fixedStem":"Moved","selectionFilter":{"kinds":["directory"]}})).unwrap();
    let result = wait(&state, preview_owned(&state, request).unwrap());
    assert_eq!(result["counts"]["blocked"], "0", "{result}");
    assert_eq!(result["summary"]["carriedDescendants"], "2");
    let done = apply(&state, &result);
    assert_eq!(
        std::fs::read(dir.path().join("media/Moved/Moved/video.mp4")).unwrap(),
        b"payload"
    );
    let undo = wait(
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
    apply(&state, &undo);
    assert_eq!(
        std::fs::read(folder.join("nested/video.mp4")).unwrap(),
        b"payload"
    );
    assert!(!dir.path().join("media/Moved").exists());
}
#[test]
fn explorer_arguments_select_without_applying() {
    use std::ffi::OsString;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("quoted ' & $(touch PWNED) 名.mp4");
    std::fs::write(&path, b"unchanged").unwrap();
    let parsed = vidlib_tauri::entrypoint::parse_selection_arguments([
        OsString::from("--select"),
        OsString::from("--"),
        path.clone().into_os_string(),
    ])
    .unwrap();
    assert_eq!(parsed, Some(vec![path.clone()]));
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    let id = vidlib_tauri::entrypoint::queue_native_paths(&state, parsed.unwrap()).unwrap();
    assert_eq!(
        session_owned(&state, EmptyRequest {}).unwrap()["revision"],
        "0"
    );
    vidlib_tauri::entrypoint::accept_owned(
        &state,
        vidlib_tauri::entrypoint::AcceptSelectionRequest {
            selection_id: id,
            revision: 0,
        },
    )
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"unchanged");
    assert!(!dir.path().join("PWNED").exists());
}
#[test]
fn malicious_quoted_path_is_data_and_control_arguments_are_rejected() {
    use std::ffi::OsString;
    for args in [
        vec!["--apply"],
        vec!["--select", "relative"],
        vec!["--select", "--"],
        vec!["--select", "--", "../outside"],
    ] {
        assert!(vidlib_tauri::entrypoint::parse_selection_arguments(
            args.into_iter().map(OsString::from)
        )
        .is_err());
    }
    assert!(
        vidlib_tauri::entrypoint::parse_selection_arguments(Vec::new())
            .unwrap()
            .is_none()
    );
}
#[test]
fn cancelled_picker_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("original.mp4");
    std::fs::write(&path, b"content").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    let selected = vidlib_tauri::selection::complete_picker_at(&state, Some(vec![path.clone()]), 0)
        .unwrap()
        .unwrap();
    assert_eq!(selected.revision, "1");
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
    let request=serde_json::from_value(serde_json::json!({"revision":"2","selectedIds":[page.items[0]["itemId"]],"rule":"fixedStem","fixedStem":"changed"})).unwrap();
    let result = wait(&state, preview_owned(&state, request).unwrap());
    let plan_ref = serde_json::from_value(result["planRef"].clone()).unwrap();
    let review = confirm_owned(&state, PlanRequest { plan_ref }).unwrap();
    assert!(vidlib_tauri::selection::complete_picker_at(&state, None, 2)
        .unwrap()
        .is_none());
    assert_eq!(
        session_owned(&state, EmptyRequest {}).unwrap()["revision"],
        "2"
    );
    wait(
        &state,
        apply_owned(
            &state,
            ApplyRequest {
                plan_ref,
                nonce: review.nonce,
            },
        )
        .unwrap(),
    );
    assert_eq!(
        std::fs::read(dir.path().join("changed.mp4")).unwrap(),
        b"content"
    );
}
#[test]
fn configured_library_roots_and_carrying_ancestors_remain_guarded() {
    for select_ancestor in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media/Tree");
        std::fs::create_dir_all(root.join("child")).unwrap();
        std::fs::write(root.join("child/video.mp4"), b"content").unwrap();
        let prefs = dir.path().join("libraries.json");
        std::fs::write(&prefs, serde_json::to_vec(&vec![&root]).unwrap()).unwrap();
        let state = AppState::open(&dir.path().join("app"), prefs).unwrap();
        if select_ancestor {
            install_selection(&state, vec![root.parent().unwrap().to_path_buf()]).unwrap();
        } else {
            vidlib_tauri::selection::select_library_roots_at(&state, 0).unwrap();
        }
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
        let name = if select_ancestor { "media" } else { "Tree" };
        let id = page
            .items
            .iter()
            .find(|v| v["displayName"] == name)
            .unwrap()["itemId"]
            .clone();
        let result=wait(&state,preview_owned(&state,serde_json::from_value(serde_json::json!({"revision":"2","selectedIds":[id],"rule":"fixedStem","fixedStem":"Moved"})).unwrap()).unwrap());
        assert_eq!(result["counts"]["blocked"], "1");
        let reference = serde_json::from_value(result["planRef"].clone()).unwrap();
        assert!(confirm_owned(
            &state,
            PlanRequest {
                plan_ref: reference
            }
        )
        .is_err());
        assert_eq!(
            std::fs::read(root.join("child/video.mp4")).unwrap(),
            b"content"
        );
    }
}
#[test]
fn native_selection_process_child() {
    let Some(app) = std::env::var_os("VIDLIB_SELECTION_CHILD_APP") else {
        return;
    };
    let ready = std::path::PathBuf::from(std::env::var_os("VIDLIB_SELECTION_CHILD_READY").unwrap());
    let args = std::env::args_os().skip_while(|arg| arg != "--select");
    let paths = vidlib_tauri::entrypoint::parse_selection_arguments(args)
        .unwrap()
        .unwrap();
    let original = std::fs::read(&paths[0]).unwrap();
    let state = AppState::open(
        std::path::Path::new(&app),
        std::path::PathBuf::from(&app).join("libraries.json"),
    )
    .unwrap();
    let id = vidlib_tauri::entrypoint::queue_native_paths(&state, paths.clone()).unwrap();
    assert!(
        vidlib_tauri::entrypoint::pending_owned(&state, EmptyRequest {}).is_err(),
        "another process owns writer"
    );
    std::fs::write(&ready, b"queued").unwrap();
    let release = ready.with_extension("release");
    for _ in 0..1000 {
        if release.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(release.exists());
    assert_eq!(
        vidlib_tauri::entrypoint::pending_owned(&state, EmptyRequest {})
            .unwrap()
            .unwrap()
            .selection_id,
        id
    );
    vidlib_tauri::entrypoint::accept_owned(
        &state,
        vidlib_tauri::entrypoint::AcceptSelectionRequest {
            selection_id: id,
            revision: 0,
        },
    )
    .unwrap();
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    assert_eq!(std::fs::read(&paths[0]).unwrap(), original);
    assert_eq!(
        session_owned(&state, EmptyRequest {}).unwrap()["revision"],
        "1"
    );
}
#[test]
fn multiple_instances_forward_selection_safely_after_coordinator_allows_review() {
    use std::process::{Command, Stdio};
    use vidlib_fileops::coordinator::LeaseKind;
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    let state = AppState::open(&app, dir.path().join("libraries.json")).unwrap();
    let writer = state.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut children = Vec::new();
    for n in 0..2 {
        let path = dir.path().join(format!("instance {n} ' & $() 名.mp4"));
        std::fs::write(&path, b"not applied").unwrap();
        let ready = dir.path().join(format!("ready-{n}"));
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native_selection_process_child",
                "--nocapture",
                "--",
                "--select",
                "--",
            ])
            .arg(&path)
            .env("VIDLIB_SELECTION_CHILD_APP", &app)
            .env("VIDLIB_SELECTION_CHILD_READY", &ready)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        children.push((child, ready, path));
    }
    for (_, ready, _) in &children {
        for _ in 0..500 {
            if ready.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ready.exists(), "real child did not enqueue selection");
    }
    drop(writer);
    for (_, ready, _) in &children {
        std::fs::write(ready.with_extension("release"), b"go").unwrap();
    }
    for (child, _, path) in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read(path).unwrap(), b"not applied");
    }
    assert_eq!(
        session_owned(&state, EmptyRequest {}).unwrap()["revision"],
        "0",
        "other instances cannot replace this process selection"
    );
}
#[cfg(target_os = "linux")]
#[path = "../../crates/vidlib-fileops/tests/support/copy_faults.rs"]
mod copy_faults;
#[cfg(target_os = "linux")]
#[test]
fn selected_root_crash_child() {
    use vidlib_fileops::{coordinator::LeaseKind, journal::Journal};
    let Some(root) = std::env::var_os("VIDLIB_SELECTED_ROOT_CRASH") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let folder = root.join("media/Tree");
    let state = AppState::open(&root.join("app"), root.join("libraries.json")).unwrap();
    install_selection(&state, vec![folder.clone(), folder.join("nested")]).unwrap();
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
    let selected = page
        .items
        .iter()
        .filter(|v| v["kind"] == "directory")
        .map(|v| v["itemId"].clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    let mut request = serde_json::json!({"revision":"2","selectedIds":selected,"rule":"fixedStem","fixedStem":"Moved","selectionFilter":{"kinds":["directory"]}});
    if std::env::var_os("VIDLIB_SELECTED_ROOT_METADATA").is_some() {
        request["metadataChanges"] =
            serde_json::json!({"modified":{"type":"set","valueNs":"1600000000123456700"}});
    }
    if let Some(destination) = std::env::var_os("VIDLIB_SELECTED_ROOT_DESTINATION") {
        let grant =
            vidlib_tauri::selection::install_destination_at(&state, destination.into(), Some(1))
                .unwrap();
        request["destination"] = serde_json::json!({"type":"moveTo","grantId":grant.grant_id,"layout":{"type":"preserveHierarchy"}});
    }
    let result = wait(
        &state,
        preview_owned(&state, serde_json::from_value(request).unwrap()).unwrap(),
    );
    assert_eq!(result["counts"]["blocked"], "0", "{result}");
    let plan_ref: UiPlanRef = serde_json::from_value(result["planRef"].clone()).unwrap();
    let plan = {
        let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
        state
            .plans
            .lock()
            .unwrap()
            .current(plan_ref.into(), &lease)
            .unwrap()
            .clone()
    };
    // The exact parent is observed, never inserted as a source or metadata grant.
    assert!(plan
        .grants
        .grants
        .iter()
        .all(|g| g.path != folder.parent().unwrap()));
    assert!(plan.execution.groups.iter().flat_map(|g|&g.steps).all(|s|!matches!(&s.action,vidlib_rename::ExecutionAction::SetMetadata{entry,..} if entry.path==folder.parent().unwrap())));
    let lease = state.coordinator.try_acquire(LeaseKind::Write).unwrap();
    let mut journal = Journal::open(&state.operations).unwrap();
    let mut fs = copy_faults::CopyFs::new(&std::env::var("VIDLIB_SELECTED_ROOT_BOUNDARY").unwrap());
    fs.native.policy = plan.execution.expected_namespace.policy.clone();
    vidlib_fileops::rename::execute(
        &plan,
        &lease,
        &mut journal,
        &mut fs,
        &vidlib_inventory::JobControl::default(),
    )
    .unwrap();
    panic!("actual native mutation boundary was not reached")
}
#[cfg(target_os = "linux")]
#[test]
fn selected_root_actual_rename_and_move_crashes_recover_and_undo() {
    use vidlib_fileops::{coordinator::LeaseKind, journal::Journal};
    for (mode, boundary, restore) in [
        ("rename", "after-rename", false),
        ("rename", "after-rename", true),
        ("move", "after-directory-quarantine", true),
        ("move", "after-remove", false),
        ("metadata", "after-metadata-2", false),
        ("metadata", "after-metadata-2", true),
    ] {
        eprintln!("native root crash case {mode}/{boundary}, restore={restore}");
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("media/Tree");
        std::fs::create_dir_all(folder.join("nested")).unwrap();
        std::fs::write(folder.join("nested/video.mp4"), b"payload").unwrap();
        let original_modified = std::fs::metadata(&folder).unwrap().modified().unwrap();
        let destination = tempfile::tempdir_in("/dev/shm").unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "selected_root_crash_child", "--nocapture"])
            .env("VIDLIB_SELECTED_ROOT_CRASH", dir.path())
            .env("VIDLIB_SELECTED_ROOT_BOUNDARY", boundary);
        if mode == "move" {
            child.env("VIDLIB_SELECTED_ROOT_DESTINATION", destination.path());
        }
        if mode == "metadata" {
            child.env("VIDLIB_SELECTED_ROOT_METADATA", "1");
        }
        let output = child.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(86),
            "{mode}/{boundary}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let state =
            AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
        let pending = Journal::open(&state.operations)
            .unwrap()
            .pending()
            .unwrap()
            .remove(0);
        let op = pending.operation_id;
        assert!(state.coordinator.try_acquire(LeaseKind::Read).is_err());
        let review = recovery_preview_owned(&state, OperationRequest { operation_id: op }).unwrap();
        assert_eq!(
            review["unresolvedGroups"], "0",
            "{mode}/{boundary}: {review}"
        );
        wait(
            &state,
            recovery_owned(
                &state,
                RecoveryRequest {
                    recovery_ref: serde_json::from_value(review["recoveryRef"].clone()).unwrap(),
                    nonce: serde_json::from_value(review["nonce"].clone()).unwrap(),
                    choice: if restore {
                        Choice::Restore
                    } else {
                        Choice::Resume
                    },
                },
            )
            .unwrap(),
        );
        assert!(Journal::open(&state.operations)
            .unwrap()
            .pending()
            .unwrap()
            .is_empty());
        if !restore {
            let target = if mode == "move" {
                destination.path().join("Moved/Moved/video.mp4")
            } else {
                dir.path().join("media/Moved/Moved/video.mp4")
            };
            assert_eq!(std::fs::read(target).unwrap(), b"payload");
            let undo = wait(
                &state,
                undo_owned(
                    &state,
                    UndoRequest {
                        operation_id: op,
                        revision: 1,
                    },
                )
                .unwrap(),
            );
            apply(&state, &undo);
        }
        assert_eq!(
            std::fs::read(folder.join("nested/video.mp4")).unwrap(),
            b"payload"
        );
        if mode == "metadata" {
            assert_eq!(
                std::fs::metadata(&folder).unwrap().modified().unwrap(),
                original_modified
            );
        }
        assert!(!dir.path().join("media/Moved").exists());
        assert_eq!(std::fs::read_dir(destination.path()).unwrap().count(), 0);
    }
}
#[test]
fn configured_root_added_after_selection_is_guarded_on_fresh_preview() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("media/Tree");
    std::fs::create_dir_all(&folder).unwrap();
    let prefs = dir.path().join("libraries.json");
    let state = AppState::open(&dir.path().join("app"), prefs.clone()).unwrap();
    install_selection(&state, vec![folder.clone()]).unwrap();
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
    std::fs::write(prefs, serde_json::to_vec(&vec![folder]).unwrap()).unwrap();
    let result=wait(&state,preview_owned(&state,serde_json::from_value(serde_json::json!({"revision":"2","selectedIds":[page.items[0]["itemId"]],"rule":"fixedStem","fixedStem":"Moved"})).unwrap()).unwrap());
    assert_eq!(
        result["counts"]["blocked"], "1",
        "new configured library roots must guard an already selected native source"
    );
}
#[test]
fn native_replacement_cancels_old_producer_and_starts_new_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("media");
    std::fs::create_dir(&folder).unwrap();
    for i in 0..1500 {
        std::fs::write(folder.join(format!("clip{i}.mp4")), b"content").unwrap();
    }
    let other = dir.path().join("other.mp4");
    std::fs::write(&other, b"other").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    install_selection(&state, vec![folder]).unwrap();
    let old = inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap();
    vidlib_tauri::selection::complete_picker_at(&state, Some(vec![other]), 1).unwrap();
    let new = inventory_owned(&state, InventoryRequestDto { revision: 2 });
    assert!(
        new.is_ok(),
        "cancelled old source work must not strand the new selection: {new:?}"
    );
    wait(&state, new.unwrap());
    for _ in 0..500 {
        let status = status_owned(&state, JobRequest { job_id: old }).unwrap();
        if status.state != "running" {
            assert_eq!(status.state, "cancelled");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("old source work did not stop");
}
#[test]
fn library_save_revokes_confirmed_plans_across_instances_before_publishing() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("media/Tree");
    std::fs::create_dir_all(&folder).unwrap();
    let prefs = dir.path().join("libraries.json");
    let app = dir.path().join("app");
    let state = AppState::open(&app, prefs.clone()).unwrap();
    install_selection(&state, vec![folder.clone()]).unwrap();
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
    let result=wait(&state,preview_owned(&state,serde_json::from_value(serde_json::json!({"revision":"2","selectedIds":[page.items[0]["itemId"]],"rule":"fixedStem","fixedStem":"Moved"})).unwrap()).unwrap());
    let plan_ref = serde_json::from_value(result["planRef"].clone()).unwrap();
    let confirmation = confirm_owned(&state, PlanRequest { plan_ref }).unwrap();
    let other = AppState::open(&app, prefs.clone()).unwrap();
    vidlib_tauri::save_libraries_owned(&other, vec![folder.to_str().unwrap().into()]).unwrap();
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&std::fs::read(prefs).unwrap()).unwrap(),
        vec![folder.to_str().unwrap()]
    );
    assert!(apply_owned(
        &state,
        ApplyRequest {
            plan_ref,
            nonce: confirmation.nonce
        }
    )
    .is_err());
    assert!(folder.exists());
}
#[cfg(unix)]
#[test]
fn exact_native_bytes_survive_picker_and_argv_without_display_roundtrip() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join(OsString::from_vec(vec![b'a', 0xff, b'.', b'm', b'p', b'4']));
    std::fs::write(&path, b"native").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    vidlib_tauri::selection::complete_picker_at(&state, Some(vec![path.clone()]), 0).unwrap();
    let args = vec![
        OsString::from("--select"),
        OsString::from("--"),
        path.clone().into_os_string(),
    ];
    assert_eq!(
        vidlib_tauri::entrypoint::parse_selection_arguments(args).unwrap(),
        Some(vec![path.clone()])
    );
    let mut root = vidlib_inventory::RootBinding::new(path.clone()).unwrap();
    root.source_grant.source = vidlib_inventory::GrantSource::NativeSelectedEntry;
    let inventory = vidlib_inventory::scan(
        &vidlib_inventory::InventoryRequest::new(vec![root]),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(inventory.entries[0].absolute_path, path);
    assert_eq!(std::fs::read(&path).unwrap(), b"native");
}
#[cfg(windows)]
#[test]
fn windows_argv_retains_unc_and_unpaired_native_utf16_units() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    let mut units: Vec<_> = "\\\\server\\share\\名".encode_utf16().collect();
    units.push(0xd800);
    units.extend(".mp4".encode_utf16());
    let native = OsString::from_wide(&units);
    let parsed = vidlib_tauri::entrypoint::parse_selection_arguments([
        OsString::from("--select"),
        OsString::from("--"),
        native.clone(),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(parsed[0].as_os_str(), native);
}
#[cfg(target_os = "linux")]
#[test]
fn selected_root_keeps_verified_content_and_helper_ancestry_and_rejects_siblings() {
    use vidlib_inventory::*;
    let dir = tempfile::tempdir().unwrap();
    let selected = dir.path().join("Tree");
    std::fs::create_dir_all(selected.join("nested")).unwrap();
    std::fs::write(selected.join("nested/video.mp4"), b"abc").unwrap();
    std::fs::write(dir.path().join("sibling.mp4"), b"foreign").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    install_selection(&state, vec![selected.clone()]).unwrap();
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
    let id = page.items.iter().find(|e| e["kind"] == "file").unwrap()["itemId"].clone();
    let result=wait(&state,preview_owned(&state,serde_json::from_value(serde_json::json!({"revision":"2","selectedIds":[id],"rule":"fixedStem","fixedStem":"renamed"})).unwrap()).unwrap());
    let plan_ref: UiPlanRef = serde_json::from_value(result["planRef"].clone()).unwrap();
    let inv = {
        let lease = state
            .coordinator
            .try_acquire(vidlib_fileops::coordinator::LeaseKind::Read)
            .unwrap();
        state
            .plans
            .lock()
            .unwrap()
            .current(plan_ref.into(), &lease)
            .unwrap()
            .execution
            .provenance
            .inventory
            .clone()
    };
    let file = inv
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::File)
        .unwrap();
    let content = with_verified_content(&inv, file.id, |reader| {
        let mut value = Vec::new();
        reader.read_to_end(&mut value)?;
        Ok(value)
    })
    .unwrap();
    assert_eq!(content, b"abc");
    let helper = with_verified_helper_path(&inv, file.id, |path| std::fs::read(path)).unwrap();
    assert_eq!(helper, b"abc");
    let mut forged = inv.clone();
    forged
        .entries
        .iter_mut()
        .find(|e| e.id == file.id)
        .unwrap()
        .absolute_path = dir.path().join("sibling.mp4");
    assert!(with_verified_content(&forged, file.id, |_| Ok(())).is_err());
    std::fs::rename(&selected, dir.path().join("original")).unwrap();
    std::fs::create_dir_all(selected.join("nested")).unwrap();
    std::fs::write(selected.join("nested/video.mp4"), b"abc").unwrap();
    assert!(with_verified_content(&inv, file.id, |_| Ok(())).is_err());
    assert!(with_verified_helper_path(&inv, file.id, |_| Ok(())).is_err());
}
#[test]
fn older_subtree_grants_do_not_acquire_selected_root_entry_authority() {
    use vidlib_inventory::*;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("video.mp4"), b"content").unwrap();
    for source in [GrantSource::NativeSelection, GrantSource::ValidatedManual] {
        let mut root = RootBinding::new(dir.path().to_path_buf()).unwrap();
        root.source_grant.source = source;
        let inv = scan(&InventoryRequest::new(vec![root]), &Default::default()).unwrap();
        assert_eq!(inv.entries.len(), 1);
        assert_eq!(inv.entries[0].kind, EntryKind::File);
    }
}
#[test]
fn failed_library_publication_preserves_preferences_and_revokes_old_epoch() {
    use vidlib_fileops::coordinator::LeaseKind;
    let dir = tempfile::tempdir().unwrap();
    let prefs = dir.path().join("libraries.json");
    std::fs::write(&prefs, b"[]").unwrap();
    let state = AppState::open(&dir.path().join("app"), prefs.clone()).unwrap();
    let before = state
        .coordinator
        .try_acquire(LeaseKind::Read)
        .unwrap()
        .epochs()
        .unwrap();
    std::fs::create_dir(prefs.with_extension("pending")).unwrap();
    assert!(
        vidlib_tauri::save_libraries_owned(&state, vec![dir.path().to_str().unwrap().into()])
            .is_err()
    );
    assert_eq!(std::fs::read(prefs).unwrap(), b"[]");
    let after = state
        .coordinator
        .try_acquire(LeaseKind::Read)
        .unwrap()
        .epochs()
        .unwrap();
    assert!(
        after.policy > before.policy,
        "a failed publication may revoke conservatively, never publish without revocation"
    );
}
#[cfg(target_os = "linux")]
#[test]
fn selected_root_recovery_does_not_bless_unknown_carried_children() {
    use vidlib_fileops::journal::Journal;
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("media/Tree");
    std::fs::create_dir_all(folder.join("nested")).unwrap();
    std::fs::write(folder.join("nested/video.mp4"), b"payload").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "selected_root_crash_child", "--nocapture"])
        .env("VIDLIB_SELECTED_ROOT_CRASH", dir.path())
        .env("VIDLIB_SELECTED_ROOT_BOUNDARY", "after-rename")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(86));
    let foreign = dir.path().join("media/Moved/foreign.txt");
    std::fs::write(&foreign, b"foreign").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    let op = Journal::open(&state.operations)
        .unwrap()
        .pending()
        .unwrap()
        .remove(0);
    let review = recovery_preview_owned(
        &state,
        OperationRequest {
            operation_id: op.operation_id,
        },
    )
    .unwrap();
    assert_ne!(
        review["unresolvedGroups"], "0",
        "fresh listing cannot authorize an unknown carried child"
    );
    assert_eq!(std::fs::read(foreign).unwrap(), b"foreign");
}
#[test]
fn discard_waiting_native_paths_never_changes_current_selection_or_revision() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    let file = dir.path().join("video.mp4");
    std::fs::write(&file, b"unchanged").unwrap();
    let id = vidlib_tauri::entrypoint::queue_native_paths(&state, vec![file.clone()]).unwrap();
    let writer = state
        .coordinator
        .try_acquire(vidlib_fileops::coordinator::LeaseKind::Write)
        .unwrap();
    vidlib_tauri::entrypoint::discard_owned(&state, EmptyRequest {}).unwrap();
    drop(writer);
    assert!(
        vidlib_tauri::entrypoint::pending_owned(&state, EmptyRequest {})
            .unwrap()
            .is_none()
    );
    assert!(vidlib_tauri::entrypoint::accept_owned(
        &state,
        vidlib_tauri::entrypoint::AcceptSelectionRequest {
            selection_id: id,
            revision: 0
        }
    )
    .is_err());
    assert_eq!(
        session_owned(&state, EmptyRequest {}).unwrap()["revision"],
        "0"
    );
    assert_eq!(std::fs::read(file).unwrap(), b"unchanged");
}
#[test]
fn removed_library_guard_allows_fresh_selection_but_retains_independent_guard() {
    for independent in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("media/Tree");
        std::fs::create_dir_all(&folder).unwrap();
        let state =
            AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
        if independent {
            *state.rename.lock().unwrap() = RenameState::new(vidlib_inventory::ProtectedPolicy {
                guarded_roots: vec![folder.clone()],
                ..Default::default()
            });
        }
        vidlib_tauri::save_libraries_owned(&state, vec![folder.to_str().unwrap().into()]).unwrap();
        install_selection(&state, vec![folder.clone()]).unwrap();
        vidlib_tauri::save_libraries_owned(&state, vec![]).unwrap();
        install_selection(&state, vec![folder.clone()]).unwrap();
        wait(
            &state,
            inventory_owned(&state, InventoryRequestDto { revision: 2 }).unwrap(),
        );
        let page = inventory_page_owned(
            &state,
            InventoryPageRequest {
                query: PageQuery::new(2),
            },
        )
        .unwrap();
        let result=wait(&state,preview_owned(&state,serde_json::from_value(serde_json::json!({"revision":"3","selectedIds":[page.items[0]["itemId"]],"rule":"fixedStem","fixedStem":"Moved"})).unwrap()).unwrap());
        assert_eq!(
            result["counts"]["blocked"],
            if independent { "1" } else { "0" },
            "removed library ownership must not survive or erase independent ownership"
        );
    }
}
#[test]
fn native_absolute_volume_root_is_a_valid_selection_source() {
    #[cfg(unix)]
    let path = std::path::PathBuf::from("/");
    #[cfg(windows)]
    let path = std::path::PathBuf::from("C:\\");
    assert!(
        vidlib_tauri::entrypoint::validate_paths(std::slice::from_ref(&path)).is_ok(),
        "a volume root may be inventoried without becoming a renamable entry"
    );
}
#[test]
fn selected_root_undo_refuses_changed_external_sibling_material() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("media/Tree");
    std::fs::create_dir_all(&folder).unwrap();
    let sibling = dir.path().join("media/sibling.mp4");
    std::fs::write(&sibling, b"original").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    install_selection(&state, vec![folder.clone()]).unwrap();
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
    let preview=wait(&state,preview_owned(&state,serde_json::from_value(serde_json::json!({"revision":"2","selectedIds":[page.items[0]["itemId"]],"rule":"fixedStem","fixedStem":"Moved"})).unwrap()).unwrap());
    let done = apply(&state, &preview);
    std::fs::write(&sibling, b"externally changed material").unwrap();
    let undo = wait(
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
    assert_ne!(
        undo["counts"]["blocked"], "0",
        "historical sibling material must not be replaced by fresh-listing authority"
    );
    assert_eq!(
        std::fs::read(&sibling).unwrap(),
        b"externally changed material"
    );
    assert!(dir.path().join("media/Moved").exists());
    assert!(!folder.exists());
}
#[test]
fn selected_root_metadata_rename_and_undo_never_grant_parent_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("media/Tree");
    std::fs::create_dir_all(folder.join("nested")).unwrap();
    std::fs::write(folder.join("nested/video.mp4"), b"payload").unwrap();
    let original = std::fs::metadata(&folder).unwrap().modified().unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    install_selection(&state, vec![folder.clone()]).unwrap();
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
    let id = page
        .items
        .iter()
        .find(|v| v["displayName"] == "Tree")
        .unwrap()["itemId"]
        .clone();
    let result=wait(&state,preview_owned(&state,serde_json::from_value(serde_json::json!({"revision":"2","selectedIds":[id],"rule":"fixedStem","fixedStem":"Moved","metadataChanges":{"modified":{"type":"set","valueNs":"1600000000123456700"}}})).unwrap()).unwrap());
    assert_eq!(result["counts"]["blocked"], "0");
    let plan_ref: UiPlanRef = serde_json::from_value(result["planRef"].clone()).unwrap();
    {
        let lease = state
            .coordinator
            .try_acquire(vidlib_fileops::coordinator::LeaseKind::Read)
            .unwrap();
        let mut registry = state.plans.lock().unwrap();
        let plan = registry.current(plan_ref.into(), &lease).unwrap();
        assert!(plan
            .grants
            .grants
            .iter()
            .all(|g| g.path != folder.parent().unwrap()));
        let metadata: Vec<_> = plan
            .execution
            .groups
            .iter()
            .flat_map(|g| &g.steps)
            .filter_map(|s| {
                if let vidlib_rename::ExecutionAction::SetMetadata { entry, .. } = &s.action {
                    Some(entry)
                } else {
                    None
                }
            })
            .collect();
        assert!(!metadata.is_empty());
        assert!(metadata
            .iter()
            .all(|entry| entry.path != folder.parent().unwrap()));
    }
    let done = apply(&state, &result);
    assert_eq!(done["state"], "completed", "{done}");
    let moved = dir.path().join("media/Moved");
    assert_eq!(
        std::fs::metadata(&moved)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        1600000000123456700
    );
    let undo = wait(
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
    apply(&state, &undo);
    assert_eq!(
        std::fs::metadata(&folder).unwrap().modified().unwrap(),
        original
    );
    assert_eq!(
        std::fs::read(folder.join("nested/video.mp4")).unwrap(),
        b"payload"
    );
}

#[test]
fn selected_root_apply_refuses_target_clobber_and_replaced_parent() {
    for replace_parent in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        let folder = media.join("Tree");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("video.mp4"), b"original").unwrap();
        let state =
            AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
        install_selection(&state, vec![folder.clone()]).unwrap();
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
        let id = page
            .items
            .iter()
            .find(|v| v["displayName"] == "Tree")
            .unwrap()["itemId"]
            .clone();
        let result = wait(&state, preview_owned(&state, serde_json::from_value(serde_json::json!({"revision":"2","selectedIds":[id],"rule":"fixedStem","fixedStem":"Moved"})).unwrap()).unwrap());
        let plan_ref = serde_json::from_value(result["planRef"].clone()).unwrap();
        let review = confirm_owned(&state, PlanRequest { plan_ref }).unwrap();
        if replace_parent {
            std::fs::rename(&media, dir.path().join("original-media")).unwrap();
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("video.mp4"), b"foreign").unwrap();
        } else {
            std::fs::create_dir(media.join("Moved")).unwrap();
            std::fs::write(media.join("Moved/foreign.mp4"), b"foreign").unwrap();
        }
        let job = apply_owned(
            &state,
            ApplyRequest {
                plan_ref,
                nonce: review.nonce,
            },
        )
        .unwrap();
        let mut terminal = None;
        for _ in 0..500 {
            let status = status_owned(&state, JobRequest { job_id: job }).unwrap();
            if status.state != "running" {
                terminal = Some(status);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_ne!(terminal.expect("apply finished").state, "completed");
        if replace_parent {
            assert_eq!(std::fs::read(folder.join("video.mp4")).unwrap(), b"foreign");
            assert_eq!(
                std::fs::read(dir.path().join("original-media/Tree/video.mp4")).unwrap(),
                b"original"
            );
            assert!(!media.join("Moved").exists());
        } else {
            assert_eq!(
                std::fs::read(folder.join("video.mp4")).unwrap(),
                b"original"
            );
            assert_eq!(
                std::fs::read(media.join("Moved/foreign.mp4")).unwrap(),
                b"foreign"
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn restarted_pending_recovery_loads_current_library_guards() {
    use vidlib_fileops::journal::Journal;
    for restore in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("media/Tree");
        std::fs::create_dir_all(folder.join("nested")).unwrap();
        std::fs::write(folder.join("nested/video.mp4"), b"payload").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "selected_root_crash_child", "--nocapture"])
            .env("VIDLIB_SELECTED_ROOT_CRASH", dir.path())
            .env("VIDLIB_SELECTED_ROOT_BOUNDARY", "after-rename")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(86));
        let protected = dir.path().join("media/Moved");
        let prefs = dir.path().join("libraries.json");
        std::fs::write(&prefs, serde_json::to_vec(&vec![&protected]).unwrap()).unwrap();
        let state = AppState::open(&dir.path().join("app"), prefs).unwrap();
        let op = Journal::open(&state.operations)
            .unwrap()
            .pending()
            .unwrap()
            .remove(0);
        let review = recovery_preview_owned(
            &state,
            OperationRequest {
                operation_id: op.operation_id,
            },
        )
        .unwrap();
        assert_eq!(
            review["unresolvedGroups"], "0",
            "diagnostic ownership inspection remains available"
        );
        let job = recovery_owned(
            &state,
            RecoveryRequest {
                recovery_ref: serde_json::from_value(review["recoveryRef"].clone()).unwrap(),
                nonce: serde_json::from_value(review["nonce"].clone()).unwrap(),
                choice: if restore {
                    Choice::Restore
                } else {
                    Choice::Resume
                },
            },
        )
        .unwrap();
        let mut terminal = None;
        for _ in 0..500 {
            let status = status_owned(&state, JobRequest { job_id: job }).unwrap();
            if status.state != "running" {
                terminal = Some(status);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let status = terminal.expect("recovery finished");
        if restore {
            assert_eq!(
                status.state, "failed",
                "actual current protected-root relocation must be refused: {status:?}"
            );
            assert_eq!(status.error.unwrap().code, "protectedEntry");
            let pending = Journal::open(&state.operations)
                .unwrap()
                .pending()
                .unwrap()
                .remove(0);
            assert_eq!(pending.operation_id, op.operation_id);
            assert_eq!(
                pending.steps.iter().map(|s| &s.step).collect::<Vec<_>>(),
                op.steps.iter().map(|s| &s.step).collect::<Vec<_>>()
            );
            assert_eq!(
                std::fs::read(protected.join("nested/video.mp4")).unwrap(),
                b"payload"
            );
        } else {
            assert_eq!(status.state,"completed","receipt-only root reconciliation and descendant rename remain permitted: {status:?}");
            assert_eq!(
                std::fs::read(protected.join("Moved/video.mp4")).unwrap(),
                b"payload"
            );
            assert!(Journal::open(&state.operations)
                .unwrap()
                .pending()
                .unwrap()
                .is_empty());
        }
        assert!(protected.exists());
        assert!(!folder.exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn selected_root_metadata_rejects_wrong_receipt_paths_step_and_identity() {
    use vidlib_fileops::{journal::Journal, Filesystem};
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("media/Tree");
    std::fs::create_dir_all(folder.join("nested")).unwrap();
    std::fs::write(folder.join("nested/video.mp4"), b"payload").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "selected_root_crash_child", "--nocapture"])
        .env("VIDLIB_SELECTED_ROOT_CRASH", dir.path())
        .env("VIDLIB_SELECTED_ROOT_BOUNDARY", "after-metadata-2")
        .env("VIDLIB_SELECTED_ROOT_METADATA", "1")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(86),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    let op = Journal::open(&state.operations)
        .unwrap()
        .pending()
        .unwrap()
        .remove(0);
    let foreign = dir.path().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    let foreign_id = vidlib_fileops::PlatformFs::default()
        .inspect(&foreign)
        .unwrap()
        .stamp
        .physical_id;
    for defect in ["path", "step", "identity"] {
        let mut forged = op.clone();
        let root=forged.steps.iter_mut().find(|s|matches!(&s.step.action,vidlib_rename::ExecutionAction::RenameEntry{source,..} if source.path==folder)).unwrap();
        let fact = root.receipts.last_mut().unwrap();
        match defect {
            "path" => fact.after_path = Some(vidlib_inventory::encode_path(&foreign)),
            "step" => fact.step_id = vidlib_inventory::ids::StepId::new(),
            _ => {
                fact.before.as_mut().unwrap().physical_id = foreign_id.clone();
                fact.after.as_mut().unwrap().physical_id = foreign_id.clone();
            }
        }
        let preview = vidlib_fileops::recovery::reconcile(
            &forged,
            &mut vidlib_fileops::PlatformFs::default(),
        )
        .unwrap();
        assert!(
            !preview.unresolved_groups.is_empty(),
            "{defect} must not authorize a rebased selected-root metadata proof"
        );
    }
    assert_eq!(
        std::fs::read(dir.path().join("media/Moved/Moved/video.mp4")).unwrap(),
        b"payload"
    );
    assert!(foreign.exists());
}
