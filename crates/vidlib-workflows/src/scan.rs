use chrono::Utc;
use std::path::PathBuf;
use uuid::Uuid;
use vidlib_core::{
    AuditRecord, AuditRecordKind, JobFailure, JobKind, JobProgress, JobRecord, JobState,
    LibraryFolder, ProgressSnapshot, ScanCheckpoint, ScanSummary, VidLibResult, VideoEntry,
};
use vidlib_db::Database;
use vidlib_metadata::FfprobeMetadataProvider;
use vidlib_ml::{LocalHeuristicEngine, MlInferenceEngine, OnnxInferenceEngine};
use vidlib_scanner::{scan_path, CancellationToken, ScanOptions};

pub struct ScanWorkflowConfig {
    pub root_path: std::path::PathBuf,
    pub compute_exact_hash: bool,
    pub skip_extensions: Vec<String>,
    pub onnx_model: Option<std::path::PathBuf>,
}

pub struct ScanWorkflowOutcome {
    pub job: JobRecord,
    pub library: LibraryFolder,
    pub summary: ScanSummary,
    pub indexed_videos: Vec<VideoEntry>,
}

pub fn run_scan_workflow(
    db: &mut Database,
    config: ScanWorkflowConfig,
    mut on_progress: impl FnMut(ProgressSnapshot),
    cancellation: Option<&CancellationToken>,
) -> VidLibResult<ScanWorkflowOutcome> {
    let now = Utc::now();
    let job_id = Uuid::new_v4();
    let canonical_path = std::fs::canonicalize(&config.root_path).unwrap_or(config.root_path.clone());
    let library = db
        .find_library_by_path(&canonical_path)?
        .unwrap_or(LibraryFolder {
            id: Uuid::new_v4(),
            path: canonical_path.clone(),
            recursive: true,
        });
    db.add_library_folder(&library)?;

    let mut job = JobRecord {
        id: job_id,
        kind: JobKind::ScanLibrary,
        state: JobState::Running,
        library_id: Some(library.id),
        root_path: Some(canonical_path.clone()),
        created_at: now,
        started_at: Some(now),
        finished_at: None,
        progress: None,
        failure: None,
    };
    db.upsert_job(&job)?;

    let provider = FfprobeMetadataProvider;
    let onnx_engine = config
        .onnx_model
        .as_ref()
        .map(|path| OnnxInferenceEngine::from_model_path(path))
        .transpose()?;

    db.insert_audit_records(&[AuditRecord {
        id: Uuid::new_v4(),
        plan_id: None,
        kind: AuditRecordKind::ScanStarted,
        source_path: Some(canonical_path.clone()),
        destination_path: None,
        details: "scan started".into(),
        created_at: Utc::now(),
    }])?;

    let mut latest_progress: Option<JobProgress> = None;

    let scan_result = scan_path(
        &canonical_path,
        &ScanOptions {
            recursive: true,
            compute_exact_hash: config.compute_exact_hash,
            compute_chunk_hash: true,
            ignore_hidden: false,
            skip_extensions: config.skip_extensions,
        },
        &provider,
        |snapshot| {
            let progress = JobProgress {
                processed_files: snapshot.processed_files,
                discovered_files: snapshot.discovered_files,
                bytes_processed: snapshot.bytes_processed,
                percent: snapshot.percent,
                current_path: snapshot.current_path.as_ref().map(PathBuf::from),
                message: snapshot.message.clone(),
            };
            latest_progress = Some(progress.clone());
            job.progress = Some(progress);
            let _ = db.upsert_job(&job);
            on_progress(snapshot)
        },
        cancellation,
    );

    let scan_result = match scan_result {
        Ok(result) => result,
        Err(err) => {
            let error_message = err.to_string();
            let cancelled = error_message.to_lowercase().contains("cancel");
            job.state = if cancelled {
                JobState::Cancelled
            } else {
                JobState::Failed
            };
            job.finished_at = Some(Utc::now());
            job.progress = latest_progress;
            job.failure = Some(JobFailure {
                message: error_message.clone(),
                occurred_at: Utc::now(),
            });
            if cancelled {
                db.insert_audit_records(&[AuditRecord {
                    id: Uuid::new_v4(),
                    plan_id: None,
                    kind: AuditRecordKind::ScanCancelled,
                    source_path: Some(library.path.clone()),
                    destination_path: None,
                    details: error_message,
                    created_at: Utc::now(),
                }])?;
            }
            db.upsert_job(&job)?;
            return Err(err);
        }
    };

    let mut persisted = Vec::with_capacity(scan_result.videos.len());
    for mut video in scan_result.videos {
        let ml_tags = if let Some(engine) = &onnx_engine {
            engine.describe_video(&video.path)?
        } else {
            LocalHeuristicEngine.describe_video(&video.path)?
        };
        video.tags = ml_tags.into_iter().map(|tag| tag.label).collect();
        persisted.push(video);
    }

    db.upsert_videos(&persisted)?;
    db.insert_scan_warnings(&scan_result.warnings)?;
    db.save_checkpoint(&ScanCheckpoint {
        library_id: library.id,
        last_path: Some(canonical_path),
        scanned_files: persisted.len() as u64,
        updated_at: Utc::now(),
    })?;
    db.insert_audit_records(&[AuditRecord {
        id: Uuid::new_v4(),
        plan_id: None,
        kind: AuditRecordKind::ScanCompleted,
        source_path: Some(library.path.clone()),
        destination_path: None,
        details: format!("scan completed with {} indexed files", persisted.len()),
        created_at: Utc::now(),
    }])?;

    job.state = JobState::Completed;
    job.finished_at = Some(Utc::now());
    job.progress = Some(JobProgress {
        processed_files: scan_result.summary.indexed_files,
        discovered_files: scan_result.summary.discovered_files,
        bytes_processed: persisted.iter().map(|video| video.size_bytes).sum(),
        percent: 100.0,
        current_path: None,
        message: format!("scan completed with {} indexed files", persisted.len()),
    });
    db.upsert_job(&job)?;

    Ok(ScanWorkflowOutcome {
        job,
        library,
        summary: scan_result.summary,
        indexed_videos: persisted,
    })
}