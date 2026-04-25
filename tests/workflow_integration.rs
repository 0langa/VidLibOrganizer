use std::fs;
use tempfile::tempdir;
use uuid::Uuid;
use vidlib_core::{AuditRecordKind, LibraryFolder, SearchQuery, VideoEntry};
use vidlib_db::Database;
use vidlib_duplicates::group_duplicates;
use vidlib_fileops::{apply_plan, plan_by_extension, undo_from_manifest};

fn sample_video(path: std::path::PathBuf, hash: &str) -> VideoEntry {
    VideoEntry {
        id: Uuid::new_v4(),
        file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
        extension: Some("mp4".into()),
        path,
        size_bytes: 12,
        modified_at: None,
        duration_seconds: Some(1.0),
        width: Some(640),
        height: Some(480),
        codec: Some("h264".into()),
        exact_hash: Some(hash.into()),
        chunk_hash: Some(hash.into()),
        perceptual_hash: Some(hash.repeat(8)),
        tags: vec!["sample".into()],
    }
}

#[test]
fn scan_persist_duplicate_search_plan_apply_and_undo_flow() {
    let dir = tempdir().unwrap();
    let library_path = dir.path().join("library");
    let organized_path = dir.path().join("organized");
    fs::create_dir_all(&library_path).unwrap();
    let file_a = library_path.join("a.mp4");
    let file_b = library_path.join("b.mp4");
    fs::write(&file_a, b"same-content").unwrap();
    fs::write(&file_b, b"same-content").unwrap();

    let mut db = Database::open(dir.path().join("vidlib.db")).unwrap();
    let folder = LibraryFolder {
        id: Uuid::new_v4(),
        path: library_path.clone(),
        recursive: true,
    };
    db.add_library_folder(&folder).unwrap();

    let entries = vec![sample_video(file_a.clone(), "dup"), sample_video(file_b.clone(), "dup")];
    db.upsert_videos(&entries).unwrap();

    let duplicates = group_duplicates(&db.all_videos().unwrap());
    assert!(!duplicates.is_empty());

    let results = db
        .search(&SearchQuery {
            text: Some("a".into()),
            tags: vec!["sample".into()],
            extension: Some("mp4".into()),
        })
        .unwrap();
    assert_eq!(results.len(), 1);

    let mut plan = plan_by_extension(&db.all_videos().unwrap(), &organized_path);
    plan.dry_run = false;
    let manifest = apply_plan(&plan, &dir.path().join("undo.json"), true).unwrap();
    assert!(organized_path.join("mp4").join("a.mp4").exists());
    undo_from_manifest(&manifest).unwrap();
    assert!(file_a.exists());

    db.insert_audit_records(&[vidlib_core::AuditRecord {
        id: Uuid::new_v4(),
        plan_id: Some(plan.id),
        kind: AuditRecordKind::PlanApplied,
        source_path: None,
        destination_path: None,
        details: "integration audit".into(),
        created_at: chrono::Utc::now(),
    }])
    .unwrap();
    assert!(!db.list_audit_records().unwrap().is_empty());
}