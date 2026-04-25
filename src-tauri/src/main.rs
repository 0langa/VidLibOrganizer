#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use parking_lot::Mutex;
use std::path::PathBuf;
use tauri::{Emitter, State};
use uuid::Uuid;
use vidlib_core::{
    format_user_error, JobRecord, LibraryFolder, ProgressSnapshot, SearchQuery, VidLibError,
};
use vidlib_db::Database;
use vidlib_duplicates::group_duplicates;
use vidlib_fileops::plan_by_extension;
use vidlib_metadata::ffprobe_available;
use vidlib_scanner::CancellationToken;
use vidlib_workflows::{run_scan_workflow, ScanWorkflowConfig};

struct AppState {
    db: Mutex<Database>,
}

#[derive(Serialize)]
struct DashboardData {
    libraries: usize,
    videos: usize,
    duplicate_groups: usize,
    ffprobe_available: bool,
}

#[derive(Serialize)]
struct ScanResponse {
    job_id: String,
    indexed_videos: usize,
}

#[derive(Serialize)]
struct JobsResponse {
    jobs: Vec<JobRecord>,
}

fn user_error(error: VidLibError) -> String {
    format_user_error(&error)
}

#[tauri::command]
fn dashboard(state: State<'_, AppState>) -> Result<DashboardData, String> {
    let db = state.db.lock();
    let libraries = db.list_library_folders().map_err(user_error)?;
    let videos = db.all_videos().map_err(user_error)?;
    let duplicate_groups = group_duplicates(&videos);
    Ok(DashboardData {
        libraries: libraries.len(),
        videos: videos.len(),
        duplicate_groups: duplicate_groups.len(),
        ffprobe_available: ffprobe_available(),
    })
}

#[tauri::command]
fn add_library(path: String, recursive: bool, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock();
    let folder = LibraryFolder {
        id: Uuid::new_v4(),
        path: PathBuf::from(path),
        recursive,
    };
    db.add_library_folder(&folder).map_err(user_error)
}

#[tauri::command]
fn run_scan(path: String, state: State<'_, AppState>, window: tauri::Window) -> Result<ScanResponse, String> {
    let token = CancellationToken::new();
    let mut db = state.db.lock();
    let outcome = run_scan_workflow(
        &mut db,
        ScanWorkflowConfig {
            root_path: PathBuf::from(&path),
            compute_exact_hash: false,
            skip_extensions: Vec::new(),
            onnx_model: None,
        },
        |progress: ProgressSnapshot| {
            let _ = window.emit("scan-progress", &progress);
        },
        Some(&token),
    )
    .map_err(user_error)?;
    Ok(ScanResponse {
        job_id: outcome.job.id.to_string(),
        indexed_videos: outcome.indexed_videos.len(),
    })
}

#[tauri::command]
fn cancel_scan(job_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let _ = (job_id, state);
    Ok(())
}

#[tauri::command]
fn list_jobs(state: State<'_, AppState>) -> Result<JobsResponse, String> {
    let db = state.db.lock();
    let jobs = db.list_jobs().map_err(user_error)?;
    Ok(JobsResponse { jobs })
}

#[tauri::command]
fn search(text: Option<String>, state: State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock();
    let results = db
        .search(&SearchQuery {
            text,
            tags: Vec::new(),
            extension: None,
        })
        .map_err(user_error)?;
    serde_json::to_string(&results).map_err(|e| e.to_string())
}

#[tauri::command]
fn generate_plan(destination_root: String, state: State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock();
    let plan = plan_by_extension(
        &db.all_videos().map_err(user_error)?,
        PathBuf::from(destination_root).as_path(),
    );
    serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())
}

fn main() {
    let db = Database::open_default().expect("open db");
    tauri::Builder::default()
        .manage(AppState {
            db: Mutex::new(db),
        })
        .invoke_handler(tauri::generate_handler![
            dashboard,
            add_library,
            run_scan,
            cancel_scan,
            list_jobs,
            search,
            generate_plan
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
