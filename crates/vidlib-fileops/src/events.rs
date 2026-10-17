//! Data-only facts for downstream consumers. These records never authorize effects.
use serde::{Deserialize, Serialize};
use vidlib_inventory::{ids::*, FileStamp, NativePathRecord};
use vidlib_rename::PlanRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletedPhase {
    Renamed,
    DirectoryCreated,
    CopyStaged,
    CopyPublished,
    SourceRemoved,
    MetadataSet,
    OwnedCopyRemoved,
    OwnedEmptyDirectoryRemoved,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    Rename,
    Directory,
    Copy,
    Removal,
    Metadata,
    Compensation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventOrigin {
    App,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDigest {
    pub algorithm: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationEventV1 {
    pub schema_version: u32,
    pub event_id: EventId,
    pub operation_id: OperationId,
    pub group_id: GroupId,
    pub step_id: StepId,
    pub kind: EventKind,
    pub phase: CompletedPhase,
    pub before_path: Option<NativePathRecord>,
    pub after_path: Option<NativePathRecord>,
    pub before: Option<FileStamp>,
    pub after: Option<FileStamp>,
    pub digest: Option<ContentDigest>,
    #[serde(with = "vidlib_inventory::exact::i128_decimal")]
    pub observed_ns: i128,
    pub origin: EventOrigin,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub journal_sequence: u64,
    pub approved_plan: PlanRef,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventPage {
    pub items: Vec<OperationEventV1>,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub next_cursor: u64,
}
