#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod jobs;
mod preferences;

use jobs::{Jobs, Status};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use tauri::{Manager, State};
use vidlib_engine::{ScanMode, ScanResult};
use vidlib_fileops::{ActionReport, FileAction};

struct AppState {
    jobs: Arc<Mutex<Jobs>>,
    preferences: PathBuf,
    preferences_lock: Mutex<()>,
}

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
) -> Result<String, String> {
    if roots.is_empty() || roots.iter().any(|root| root.trim().is_empty()) {
        return Err("Choose at least one library folder to scan.".into());
    }
    let (id, cancel) = lock_jobs(&state)?.begin()?;
    let shared = state.jobs.clone();
    let spawned = std::thread::Builder::new()
        .name("video-scan".into())
        .spawn(move || {
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
        lock_jobs(&state)?.finish(Err(error.to_string()));
        return Err(error.to_string());
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
) -> Result<ActionReport, String> {
    let result = lock_jobs(&state)?.begin_action(&id)?;
    let shared = state.jobs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            vidlib_fileops::apply_actions(&result, &paths, action, destination.as_deref())
        }))
        .unwrap_or_else(|_| {
            Err("The file operation stopped unexpectedly. Rescan before trying again.".into())
        });
        let completed = outcome
            .as_ref()
            .map(|report| report.completed.as_slice())
            .unwrap_or_default();
        if let Ok(mut jobs) = shared.lock() {
            jobs.finish_action(completed);
            match &outcome {
                Ok(report) => {
                    jobs.error = None;
                    if let Some(result) = &mut jobs.result {
                        result.issues.extend(report.issues.iter().cloned());
                    }
                }
                Err(error) => jobs.error = Some(error.clone()),
            }
        }
        outcome
    })
    .await
    .map_err(|error| format!("File operation failed: {error}"))?
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

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let preferences = app.path().app_config_dir()?.join("libraries.json");
            app.manage(AppState { jobs: Arc::new(Mutex::new(Jobs::default())), preferences, preferences_lock: Mutex::new(()) });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let acting = if let Ok(jobs) = state.jobs.lock() {
                    if !jobs.acting { jobs.cancel.store(true, std::sync::atomic::Ordering::Relaxed); }
                    jobs.acting
                } else { false };
                    if acting {
                        api.prevent_close();
                        rfd::MessageDialog::new().set_title("File operation in progress")
                            .set_description("Please wait for the current move or deletion to finish before closing VidLibOrganizer.")
                            .set_level(rfd::MessageLevel::Info).show();
                    }
            }
        })
        .invoke_handler(tauri::generate_handler![load_libraries, save_libraries, pick_folder, start_scan, scan_status, scan_result, cancel_scan, apply_actions, reveal_file, export_report])
        .run(tauri::generate_context!())
        .expect("Could not start VidLibOrganizer");
}
