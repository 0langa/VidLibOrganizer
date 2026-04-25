use chrono::Utc;
use uuid::Uuid;
use vidlib_core::{
    AuditRecord, AuditRecordKind, LibraryFolder, ProgressSnapshot, ScanCheckpoint, ScanSummary,
    VidLibResult, VideoEntry,
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
    let canonical_path = std::fs::canonicalize(&config.root_path).unwrap_or(config.root_path.clone());
    let library = db
        .find_library_by_path(&canonical_path)?
        .unwrap_or(LibraryFolder {
            id: Uuid::new_v4(),
            path: canonical_path.clone(),
            recursive: true,
        });
    db.add_library_folder(&library)?;

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
        |snapshot| on_progress(snapshot),
        cancellation,
    )?;

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

    Ok(ScanWorkflowOutcome {
        library,
        summary: scan_result.summary,
        indexed_videos: persisted,
    })
}