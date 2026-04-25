use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryFolder {
    pub id: Uuid,
    pub path: PathBuf,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoEntry {
    pub id: Uuid,
    pub path: PathBuf,
    pub file_name: String,
    pub extension: Option<String>,
    pub size_bytes: u64,
    pub modified_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub codec: Option<String>,
    pub exact_hash: Option<String>,
    pub chunk_hash: Option<String>,
    pub perceptual_hash: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanCheckpoint {
    pub library_id: Uuid,
    pub last_path: Option<PathBuf>,
    pub scanned_files: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub strategy: DuplicateStrategy,
    pub fingerprint: String,
    pub members: Vec<Uuid>,
    pub total_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DuplicateStrategy {
    ExactHash,
    ChunkHash,
    PerceptualHash,
    MetadataSimilarity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestructureAction {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub conflict: Option<String>,
    pub status: ActionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionStatus {
    Pending,
    Applied,
    Skipped,
    Reverted,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestructurePlan {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub actions: Vec<RestructureAction>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UndoManifest {
    pub plan_id: Uuid,
    pub generated_at: DateTime<Utc>,
    pub reverse_actions: Vec<RestructureAction>,
    pub audit_log: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: Option<String>,
    pub tags: Vec<String>,
    pub extension: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanSummary {
    pub root: PathBuf,
    pub discovered_files: u64,
    pub indexed_files: u64,
    pub skipped_files: u64,
    pub failed_files: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScanWarningKind {
    Walk,
    Metadata,
    Hash,
    Ml,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanWarningRecord {
    pub id: Uuid,
    pub library_id: Option<Uuid>,
    pub path: Option<PathBuf>,
    pub kind: ScanWarningKind,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditRecordKind {
    PlanApplied,
    PlanUndone,
    FileMoved,
    FileRestored,
    ScanStarted,
    ScanCompleted,
    ScanCancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditRecord {
    pub id: Uuid,
    pub plan_id: Option<Uuid>,
    pub kind: AuditRecordKind,
    pub source_path: Option<PathBuf>,
    pub destination_path: Option<PathBuf>,
    pub details: String,
    pub created_at: DateTime<Utc>,
}
