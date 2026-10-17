pub mod app_state;
pub mod commands;
pub mod jobs;
mod preferences;
pub mod rename_preferences;
pub mod selection;

use app_state::AppState;
use jobs::{Jobs, Status};
use std::{
    path::Path,
    sync::MutexGuard,
    time::{Duration, Instant},
};
use tauri::{Manager, State};
use vidlib_engine::{ScanMode, ScanResult};
use vidlib_fileops::{ActionReport, ErrorCode, FileAction, OpError, Phase};

fn lock_jobs(state: &AppState) -> Result<MutexGuard<'_, Jobs>, String> {
    state
        .jobs
        .lock()
        .map_err(|_| "The scan state could not be accessed. Restart the app.".into())
}

#[tauri::command]
fn load_libraries(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let _guard = state.preferences_lock.lock().map_err(|e| e.to_string())?;
    preferences::load(&state.preferences)
}

#[tauri::command]
fn save_libraries(paths: Vec<String>, state: State<'_, AppState>) -> Result<(), String> {
    let _guard = state.preferences_lock.lock().map_err(|e| e.to_string())?;
    preferences::save(&state.preferences, paths)
}

#[tauri::command]
async fn pick_folder() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .set_title("Choose a library or destination folder")
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().into_owned())
}

#[tauri::command]
fn start_scan(
    roots: Vec<String>,
    mode: ScanMode,
    state: State<'_, AppState>,
) -> Result<String, OpError> {
    if roots.is_empty() || roots.iter().any(|root| root.trim().is_empty()) {
        return Err(OpError::new(
            ErrorCode::InvalidPath,
            Phase::Read,
            "Choose at least one library folder to scan.",
        ));
    }
    let (id, cancel, lease) = lock_jobs(&state).map_err(job_error)?.begin()?;
    let shared = state.jobs.clone();
    let spawned = std::thread::Builder::new()
        .name("video-scan".into())
        .spawn(move || {
            let _lease = lease;
            let mut last_update = Instant::now();
            let mut phase = String::new();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                vidlib_engine::scan(roots, mode, cancel, |progress| {
                    if progress.phase != phase
                        || last_update.elapsed() >= Duration::from_millis(100)
                    {
                        phase.clone_from(&progress.phase);
                        last_update = Instant::now();
                        if let Ok(mut jobs) = shared.lock() {
                            jobs.progress = progress;
                        }
                    }
                })
            }))
            .unwrap_or_else(|_| {
                Err("The scan stopped unexpectedly. Your files were not changed.".into())
            });
            if let Ok(mut jobs) = shared.lock() {
                jobs.finish(outcome);
            }
        });
    if let Err(error) = spawned {
        lock_jobs(&state)
            .map_err(job_error)?
            .finish(Err(error.to_string()));
        return Err(job_error(error.to_string()));
    }
    Ok(id)
}

#[tauri::command]
fn scan_status(state: State<'_, AppState>) -> Result<Status, String> {
    Ok(lock_jobs(&state)?.status())
}

#[tauri::command]
fn scan_result(id: String, state: State<'_, AppState>) -> Result<ScanResult, String> {
    Ok(lock_jobs(&state)?.result(&id)?.clone())
}

#[tauri::command]
fn cancel_scan(id: String, state: State<'_, AppState>) -> Result<(), String> {
    lock_jobs(&state)?.cancel(&id)
}

#[tauri::command]
async fn apply_actions(
    id: String,
    paths: Vec<String>,
    action: FileAction,
    destination: Option<String>,
    state: State<'_, AppState>,
) -> Result<ActionReport, OpError> {
    let (result, lease) = lock_jobs(&state).map_err(job_error)?.begin_action(&id)?;
    let shared = state.jobs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = jobs::run_action(&lease, &result, &paths, action, destination.as_deref());
        if let Ok(mut jobs) = shared.lock() {
            jobs.finish_action_result(&outcome, &lease);
        }
        outcome
    })
    .await
    .map_err(|error| job_error(format!("File operation failed: {error}")))?
}

#[tauri::command]
async fn export_report(id: String, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let json = {
        let jobs = lock_jobs(&state)?;
        serde_json::to_vec_pretty(jobs.result(&id)?).map_err(|error| error.to_string())?
    };
    let destination = rfd::AsyncFileDialog::new()
        .set_title("Export duplicate scan report")
        .set_file_name("vidlib-duplicates.json")
        .add_filter("JSON report", &["json"])
        .save_file()
        .await;
    if let Some(file) = destination {
        std::fs::write(file.path(), json)
            .map_err(|error| format!("Could not export report: {error}"))?;
        return Ok(Some(file.path().to_string_lossy().into_owned()));
    }
    Ok(None)
}

#[tauri::command]
fn reveal_file(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let allowed = lock_jobs(&state)?.result.as_ref().is_some_and(|result| {
        result
            .groups
            .iter()
            .any(|group| group.files.iter().any(|file| file.path == path))
    }) || preferences::load(&state.preferences)?
        .iter()
        .any(|root| root == &path);
    if !allowed {
        return Err("This path is not in your library or current results.".into());
    }
    let target = Path::new(&path);
    if !target.exists() {
        return Err("This file or drive is no longer available.".into());
    }
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("explorer.exe");
        if target.is_dir() {
            command.arg(target);
        } else {
            command.arg(format!("/select,{path}"));
        }
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = std::process::Command::new(if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        });
        command.arg(if target.is_dir() {
            target
        } else {
            target.parent().ok_or("File has no parent folder.")?
        });
        command
    };
    command
        .spawn()
        .map_err(|error| format!("Could not open the folder: {error}"))?;
    Ok(())
}

fn job_error(message: String) -> OpError {
    OpError::new(ErrorCode::Io, Phase::Read, message)
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let preferences = app.path().app_config_dir()?.join("libraries.json");
            let app_data = app.path().app_local_data_dir()?;
            let mut state = AppState::open(&app_data, preferences)?;
            state.metadata_provider = std::sync::Arc::new(metadata::DesktopMetadataProvider::from_resources(&app.path().resource_dir()?));
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.request_close() {
                    api.prevent_close();
                    rfd::MessageDialog::new().set_title("File operation in progress")
                        .set_description("Please wait for the current file operation or recovery to finish before closing VidLibOrganizer.")
                        .set_level(rfd::MessageLevel::Info).show();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![load_libraries, save_libraries, pick_folder, start_scan, scan_status, scan_result, cancel_scan, apply_actions, reveal_file, export_report, selection::select_sources, selection::select_destination, commands::rename::update_rename_inputs, commands::rename::start_inventory, commands::rename::inventory_page, commands::rename::start_rename_preview, commands::rename::job_status, commands::rename::cancel_job, commands::rename::rename_page, commands::rename::confirm_rename, commands::rename::apply_rename, commands::rename::operation_history, commands::rename::history_revision, commands::rename::recovery_preview, commands::rename::recovery_page, commands::rename::start_recovery, commands::rename::start_undo, commands::rename::rename_session, rename_preferences::save_rename_preset, rename_preferences::load_rename_preset, rename_preferences::list_rename_presets, rename_preferences::import_rename_recipe, rename_preferences::export_rename_recipe])
        .run(tauri::generate_context!())
        .expect("Could not start VidLibOrganizer");
}

pub mod metadata;
