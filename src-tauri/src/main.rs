#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{Emitter, State};
use uuid::Uuid;
use vidlib_core::{
    format_user_error, AuditRecord, AuditRecordKind, LibraryFolder, ProgressSnapshot,
    SearchQuery, VidLibError,
};
use vidlib_db::Database;
use vidlib_duplicates::group_duplicates;
use vidlib_fileops::plan_by_extension;
use vidlib_metadata::{ffprobe_available, FfprobeMetadataProvider};
use vidlib_ml::{LocalHeuristicEngine, MlInferenceEngine};
use vidlib_scanner::{scan_path, CancellationToken, ScanOptions};

struct AppState {
    db: Mutex<Database>,
    jobs: Mutex<HashMap<String, bool>>,
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
    let provider = FfprobeMetadataProvider;
    let job_id = Uuid::new_v4().to_string();
    state.jobs.lock().insert(job_id.clone(), false);
    let token = CancellationToken::new();
    let scan_result = scan_path(
        PathBuf::from(&path).as_path(),
        &ScanOptions::default(),
        &provider,
        |progress: ProgressSnapshot| {
            let _ = window.emit("scan-progress", &progress);
        },
        Some(&token),
    )
    .map_err(user_error)?;

    let mut db = state.db.lock();
    for mut video in scan_result.videos {
        let tags = LocalHeuristicEngine
            .describe_video(&video.path)
            .map_err(user_error)?;
        video.tags = tags.into_iter().map(|tag| tag.label).collect();
        db.upsert_video(&video).map_err(user_error)?;
    }
    db.insert_scan_warnings(&scan_result.warnings)
        .map_err(user_error)?;
    db.insert_audit_records(&[AuditRecord {
        id: Uuid::new_v4(),
        plan_id: None,
        kind: AuditRecordKind::ScanCompleted,
        source_path: Some(PathBuf::from(&path)),
        destination_path: None,
        details: format!("scan job {} completed", job_id),
        created_at: chrono::Utc::now(),
    }])
    .map_err(user_error)?;
    Ok(ScanResponse {
        job_id,
        indexed_videos: db.all_videos().map_err(user_error)?.len(),
    })
}

#[tauri::command]
fn cancel_scan(job_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.jobs.lock().insert(job_id, true);
    Ok(())
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
            jobs: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            dashboard,
            add_library,
            run_scan,
            cancel_scan,
            search,
            generate_plan
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
