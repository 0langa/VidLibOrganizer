use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use vidlib_core::{
    AuditRecord, AuditRecordKind, LibraryFolder, ScanCheckpoint, ScanWarningKind,
    ScanWarningRecord, SearchQuery, VidLibError, VidLibResult, VideoEntry,
};

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open_default() -> VidLibResult<Self> {
        let root = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("VidLibOrganizer");
        fs::create_dir_all(&root)
            .map_err(|err| VidLibError::Database(format!("creating {}: {err}", root.display())))?;
        Self::open(root.join("vidlib.db"))
    }

    pub fn open(path: impl AsRef<Path>) -> VidLibResult<Self> {
        let conn = Connection::open(path.as_ref())?;
        let db = Self { conn };
        db.configure()?;
        db.migrate()?;
        Ok(db)
    }

    fn configure(&self) -> VidLibResult<()> {
        self.conn.busy_timeout(std::time::Duration::from_secs(5))?;
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            PRAGMA synchronous = NORMAL;
            PRAGMA temp_store = MEMORY;
            "#,
        )?;
        Ok(())
    }

    fn migrate(&self) -> VidLibResult<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS library_folders (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                recursive INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS video_entries (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                file_name TEXT NOT NULL,
                extension TEXT,
                size_bytes INTEGER NOT NULL,
                modified_at TEXT,
                duration_seconds REAL,
                width INTEGER,
                height INTEGER,
                codec TEXT,
                exact_hash TEXT,
                chunk_hash TEXT,
                perceptual_hash TEXT,
                tags_json TEXT NOT NULL DEFAULT '[]',
                indexed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE INDEX IF NOT EXISTS idx_video_entries_name ON video_entries(file_name);
            CREATE INDEX IF NOT EXISTS idx_video_entries_extension ON video_entries(extension);
            CREATE INDEX IF NOT EXISTS idx_video_entries_exact_hash ON video_entries(exact_hash);
            CREATE INDEX IF NOT EXISTS idx_video_entries_chunk_hash ON video_entries(chunk_hash);
            CREATE INDEX IF NOT EXISTS idx_video_entries_perceptual_hash ON video_entries(perceptual_hash);
            CREATE TABLE IF NOT EXISTS scan_checkpoints (
                library_id TEXT PRIMARY KEY,
                last_path TEXT,
                scanned_files INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS scan_warnings (
                id TEXT PRIMARY KEY,
                library_id TEXT,
                path TEXT,
                kind TEXT NOT NULL,
                message TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_scan_warnings_library_id ON scan_warnings(library_id);
            CREATE TABLE IF NOT EXISTS audit_records (
                id TEXT PRIMARY KEY,
                plan_id TEXT,
                kind TEXT NOT NULL,
                source_path TEXT,
                destination_path TEXT,
                details TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_records_plan_id ON audit_records(plan_id);
            "#,
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (1, ?1)",
            params![Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn add_library_folder(&self, folder: &LibraryFolder) -> VidLibResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO library_folders (id, path, recursive) VALUES (?1, ?2, ?3)",
            params![
                folder.id.to_string(),
                folder.path.to_string_lossy(),
                folder.recursive as i64
            ],
        )?;
        Ok(())
    }

    pub fn list_library_folders(&self) -> VidLibResult<Vec<LibraryFolder>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, recursive FROM library_folders ORDER BY path")?;
        let rows = stmt.query_map([], |row| {
            Ok(LibraryFolder {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                path: PathBuf::from(row.get::<_, String>(1)?),
                recursive: row.get::<_, i64>(2)? != 0,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(VidLibError::from)
    }

            pub fn upsert_video(&self, video: &VideoEntry) -> VidLibResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            r#"
            INSERT INTO video_entries (
                id, path, file_name, extension, size_bytes, modified_at, duration_seconds,
                width, height, codec, exact_hash, chunk_hash, perceptual_hash, tags_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(path) DO UPDATE SET
                file_name = excluded.file_name,
                extension = excluded.extension,
                size_bytes = excluded.size_bytes,
                modified_at = excluded.modified_at,
                duration_seconds = excluded.duration_seconds,
                width = excluded.width,
                height = excluded.height,
                codec = excluded.codec,
                exact_hash = excluded.exact_hash,
                chunk_hash = excluded.chunk_hash,
                perceptual_hash = excluded.perceptual_hash,
                tags_json = excluded.tags_json
            "#,
            params![
                video.id.to_string(),
                video.path.to_string_lossy(),
                video.file_name,
                video.extension,
                video.size_bytes,
                video.modified_at.map(|dt| dt.to_rfc3339()),
                video.duration_seconds,
                video.width,
                video.height,
                video.codec,
                video.exact_hash,
                video.chunk_hash,
                video.perceptual_hash,
                serde_json::to_string(&video.tags)?,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_videos(&mut self, videos: &[VideoEntry]) -> VidLibResult<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO video_entries (
                    id, path, file_name, extension, size_bytes, modified_at, duration_seconds,
                    width, height, codec, exact_hash, chunk_hash, perceptual_hash, tags_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                ON CONFLICT(path) DO UPDATE SET
                    file_name = excluded.file_name,
                    extension = excluded.extension,
                    size_bytes = excluded.size_bytes,
                    modified_at = excluded.modified_at,
                    duration_seconds = excluded.duration_seconds,
                    width = excluded.width,
                    height = excluded.height,
                    codec = excluded.codec,
                    exact_hash = excluded.exact_hash,
                    chunk_hash = excluded.chunk_hash,
                    perceptual_hash = excluded.perceptual_hash,
                    tags_json = excluded.tags_json,
                    indexed_at = CURRENT_TIMESTAMP
                "#,
            )?;
            for video in videos {
                stmt.execute(params![
                    video.id.to_string(),
                    video.path.to_string_lossy(),
                    video.file_name,
                    video.extension,
                    video.size_bytes,
                    video.modified_at.map(|dt| dt.to_rfc3339()),
                    video.duration_seconds,
                    video.width,
                    video.height,
                    video.codec,
                    video.exact_hash,
                    video.chunk_hash,
                    video.perceptual_hash,
                    serde_json::to_string(&video.tags)?,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn search(&self, query: &SearchQuery) -> VidLibResult<Vec<VideoEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, file_name, extension, size_bytes, modified_at, duration_seconds, width, height, codec, exact_hash, chunk_hash, perceptual_hash, tags_json FROM video_entries ORDER BY file_name",
        )?;
        let rows = stmt.query_map([], map_video_row)?;
        let mut items = Vec::new();
        for item in rows {
            let entry = item?;
            let text_ok = query
                .text
                .as_ref()
                .map(|text| {
                    let text = text.to_lowercase();
                    entry.file_name.to_lowercase().contains(&text)
                        || entry.path.to_string_lossy().to_lowercase().contains(&text)
                })
                .unwrap_or(true);
            let extension_ok = query
                .extension
                .as_ref()
                .map(|ext| entry.extension.as_deref() == Some(ext.as_str()))
                .unwrap_or(true);
            let tags_ok = query
                .tags
                .iter()
                .all(|tag| entry.tags.iter().any(|t| t == tag));
            if text_ok && extension_ok && tags_ok {
                items.push(entry);
            }
        }
        Ok(items)
    }

    pub fn all_videos(&self) -> VidLibResult<Vec<VideoEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, file_name, extension, size_bytes, modified_at, duration_seconds, width, height, codec, exact_hash, chunk_hash, perceptual_hash, tags_json FROM video_entries ORDER BY file_name",
        )?;
        let rows = stmt.query_map([], map_video_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(VidLibError::from)
    }

            pub fn save_checkpoint(&self, checkpoint: &ScanCheckpoint) -> VidLibResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO scan_checkpoints (library_id, last_path, scanned_files, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                checkpoint.library_id.to_string(),
                checkpoint.last_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                checkpoint.scanned_files,
                checkpoint.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn load_checkpoint(&self, library_id: Uuid) -> VidLibResult<Option<ScanCheckpoint>> {
        self.conn
            .query_row(
                "SELECT last_path, scanned_files, updated_at FROM scan_checkpoints WHERE library_id = ?1",
                params![library_id.to_string()],
                |row| {
                    let updated_at: String = row.get(2)?;
                    Ok(ScanCheckpoint {
                        library_id,
                        last_path: row.get::<_, Option<String>>(0)?.map(PathBuf::from),
                        scanned_files: row.get(1)?,
                        updated_at: DateTime::parse_from_rfc3339(&updated_at)
                            .unwrap()
                            .with_timezone(&Utc),
                    })
                },
            )
            .optional()
                .map_err(VidLibError::from)
    }

            pub fn find_library_by_path(&self, path: &Path) -> VidLibResult<Option<LibraryFolder>> {
        self.conn
            .query_row(
                "SELECT id, path, recursive FROM library_folders WHERE path = ?1",
                params![path.to_string_lossy().to_string()],
                |row| {
                    Ok(LibraryFolder {
                        id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                        path: PathBuf::from(row.get::<_, String>(1)?),
                        recursive: row.get::<_, i64>(2)? != 0,
                    })
                },
            )
            .optional()
                .map_err(VidLibError::from)
    }

            pub fn insert_scan_warnings(&mut self, warnings: &[ScanWarningRecord]) -> VidLibResult<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO scan_warnings (id, library_id, path, kind, message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for warning in warnings {
                stmt.execute(params![
                    warning.id.to_string(),
                    warning.library_id.map(|id| id.to_string()),
                    warning.path.as_ref().map(|path| path.to_string_lossy().to_string()),
                    scan_warning_kind_to_str(&warning.kind),
                    warning.message,
                    warning.created_at.to_rfc3339(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_scan_warnings(&self) -> VidLibResult<Vec<ScanWarningRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, library_id, path, kind, message, created_at FROM scan_warnings ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let created_at: String = row.get(5)?;
            Ok(ScanWarningRecord {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                library_id: row
                    .get::<_, Option<String>>(1)?
                    .and_then(|value| Uuid::parse_str(&value).ok()),
                path: row.get::<_, Option<String>>(2)?.map(PathBuf::from),
                kind: scan_warning_kind_from_str(&row.get::<_, String>(3)?),
                message: row.get(4)?,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(VidLibError::from)
    }

    pub fn insert_audit_records(&mut self, records: &[AuditRecord]) -> VidLibResult<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO audit_records (id, plan_id, kind, source_path, destination_path, details, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for record in records {
                stmt.execute(params![
                    record.id.to_string(),
                    record.plan_id.map(|id| id.to_string()),
                    audit_record_kind_to_str(&record.kind),
                    record.source_path.as_ref().map(|path| path.to_string_lossy().to_string()),
                    record.destination_path.as_ref().map(|path| path.to_string_lossy().to_string()),
                    record.details,
                    record.created_at.to_rfc3339(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_audit_records(&self) -> VidLibResult<Vec<AuditRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, plan_id, kind, source_path, destination_path, details, created_at FROM audit_records ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let created_at: String = row.get(6)?;
            Ok(AuditRecord {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                plan_id: row
                    .get::<_, Option<String>>(1)?
                    .and_then(|value| Uuid::parse_str(&value).ok()),
                kind: audit_record_kind_from_str(&row.get::<_, String>(2)?),
                source_path: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
                destination_path: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                details: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(VidLibError::from)
    }
}

fn scan_warning_kind_to_str(kind: &ScanWarningKind) -> &'static str {
    match kind {
        ScanWarningKind::Walk => "walk",
        ScanWarningKind::Metadata => "metadata",
        ScanWarningKind::Hash => "hash",
        ScanWarningKind::Ml => "ml",
        ScanWarningKind::Cancelled => "cancelled",
    }
}

fn scan_warning_kind_from_str(value: &str) -> ScanWarningKind {
    match value {
        "walk" => ScanWarningKind::Walk,
        "metadata" => ScanWarningKind::Metadata,
        "hash" => ScanWarningKind::Hash,
        "ml" => ScanWarningKind::Ml,
        "cancelled" => ScanWarningKind::Cancelled,
        _ => ScanWarningKind::Walk,
    }
}

fn audit_record_kind_to_str(kind: &AuditRecordKind) -> &'static str {
    match kind {
        AuditRecordKind::PlanApplied => "plan_applied",
        AuditRecordKind::PlanUndone => "plan_undone",
        AuditRecordKind::FileMoved => "file_moved",
        AuditRecordKind::FileRestored => "file_restored",
        AuditRecordKind::ScanStarted => "scan_started",
        AuditRecordKind::ScanCompleted => "scan_completed",
        AuditRecordKind::ScanCancelled => "scan_cancelled",
    }
}

fn audit_record_kind_from_str(value: &str) -> AuditRecordKind {
    match value {
        "plan_applied" => AuditRecordKind::PlanApplied,
        "plan_undone" => AuditRecordKind::PlanUndone,
        "file_moved" => AuditRecordKind::FileMoved,
        "file_restored" => AuditRecordKind::FileRestored,
        "scan_started" => AuditRecordKind::ScanStarted,
        "scan_completed" => AuditRecordKind::ScanCompleted,
        "scan_cancelled" => AuditRecordKind::ScanCancelled,
        _ => AuditRecordKind::ScanCompleted,
    }
}

fn map_video_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VideoEntry> {
    let modified_at = row.get::<_, Option<String>>(5)?.map(|dt| {
        DateTime::parse_from_rfc3339(&dt)
            .unwrap()
            .with_timezone(&Utc)
    });
    let tags: Vec<String> = serde_json::from_str(&row.get::<_, String>(13)?).unwrap_or_default();
    Ok(VideoEntry {
        id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
        path: PathBuf::from(row.get::<_, String>(1)?),
        file_name: row.get(2)?,
        extension: row.get(3)?,
        size_bytes: row.get(4)?,
        modified_at,
        duration_seconds: row.get(6)?,
        width: row.get(7)?,
        height: row.get(8)?,
        codec: row.get(9)?,
        exact_hash: row.get(10)?,
        chunk_hash: row.get(11)?,
        perceptual_hash: row.get(12)?,
        tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use vidlib_core::{
        AuditRecord, AuditRecordKind, LibraryFolder, ScanWarningKind, ScanWarningRecord,
        SearchQuery, VideoEntry,
    };

    #[test]
    fn persists_and_searches_video_entries() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("test.db")).unwrap();
        let folder = LibraryFolder {
            id: Uuid::new_v4(),
            path: dir.path().join("videos"),
            recursive: true,
        };
        db.add_library_folder(&folder).unwrap();
        let entry = VideoEntry {
            id: Uuid::new_v4(),
            path: folder.path.join("sample.mp4"),
            file_name: "sample.mp4".into(),
            extension: Some("mp4".into()),
            size_bytes: 42,
            modified_at: None,
            duration_seconds: Some(3.2),
            width: Some(1920),
            height: Some(1080),
            codec: Some("h264".into()),
            exact_hash: Some("abc".into()),
            chunk_hash: None,
            perceptual_hash: None,
            tags: vec!["holiday".into()],
        };
        db.upsert_video(&entry).unwrap();

        let results = db
            .search(&SearchQuery {
                text: Some("sample".into()),
                tags: vec!["holiday".into()],
                extension: Some("mp4".into()),
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file_name, "sample.mp4");
    }

    #[test]
    fn updates_existing_video_by_path() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("test.db")).unwrap();
        let path = dir.path().join("sample.mp4");
        let first = VideoEntry {
            id: Uuid::new_v4(),
            path: path.clone(),
            file_name: "sample.mp4".into(),
            extension: Some("mp4".into()),
            size_bytes: 10,
            modified_at: None,
            duration_seconds: None,
            width: None,
            height: None,
            codec: None,
            exact_hash: None,
            chunk_hash: Some("old".into()),
            perceptual_hash: None,
            tags: vec!["old".into()],
        };
        let second = VideoEntry {
            chunk_hash: Some("new".into()),
            tags: vec!["new".into()],
            ..first.clone()
        };
        db.upsert_video(&first).unwrap();
        db.upsert_video(&second).unwrap();
        let results = db.all_videos().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_hash.as_deref(), Some("new"));
        assert_eq!(results[0].tags, vec!["new"]);
    }

    #[test]
    fn persists_warnings_and_audit_records() {
        let dir = tempdir().unwrap();
        let mut db = Database::open(dir.path().join("test.db")).unwrap();
        let warning = ScanWarningRecord {
            id: Uuid::new_v4(),
            library_id: None,
            path: Some(dir.path().join("bad.mp4")),
            kind: ScanWarningKind::Metadata,
            message: "ffprobe failed".into(),
            created_at: Utc::now(),
        };
        let audit = AuditRecord {
            id: Uuid::new_v4(),
            plan_id: None,
            kind: AuditRecordKind::ScanCompleted,
            source_path: None,
            destination_path: None,
            details: "scan completed".into(),
            created_at: Utc::now(),
        };
        db.insert_scan_warnings(&[warning.clone()]).unwrap();
        db.insert_audit_records(&[audit.clone()]).unwrap();
        assert_eq!(db.list_scan_warnings().unwrap()[0].message, warning.message);
        assert_eq!(db.list_audit_records().unwrap()[0].details, audit.details);
    }
}
