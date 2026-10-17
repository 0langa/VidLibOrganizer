//! Read-only shared observations. No Rename or fileops dependencies belong here.
use crate::{ids::*, NativePathRecord};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    pub physical_id: Option<PhysicalId>,
    #[serde(with = "crate::exact::option_u64_decimal")]
    pub size: Option<u64>,
    #[serde(with = "crate::exact::option_i128_decimal")]
    pub created_ns: Option<i128>,
    #[serde(with = "crate::exact::option_i128_decimal")]
    pub modified_ns: Option<i128>,
    #[serde(with = "crate::exact::option_i128_decimal")]
    pub accessed_ns: Option<i128>,
    pub windows_attributes: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalId {
    pub platform: crate::PathPlatform,
    pub volume_identity: Vec<u8>,
    pub file_identity: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    ReparsePoint,
    Other,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueCode {
    StatFailed,
    ListingFailed,
    Cancelled,
    UnsupportedName,
    InvalidSource,
    ProtectedEntry,
    EvidenceMismatch,
    DirectoryChanged,
    IdentityUnavailable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryIssue {
    pub code: IssueCode,
    /// Exact root-relative path when an entry is known; display text is never a key.
    pub relative_path: Option<NativePathRecord>,
    pub message: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct EntrySnapshot {
    pub id: EntryId,
    pub root_id: RootId,
    pub relative_path: NativePathRecord,
    #[serde(skip)]
    pub absolute_path: PathBuf,
    pub display_name: String,
    pub display_path: String,
    pub kind: EntryKind,
    pub stamp: FileStamp,
    #[serde(with = "crate::exact::i128_decimal")]
    pub observed_ns: i128,
    pub issues: Vec<InventoryIssue>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceGrant {
    pub id: GrantId,
    /// Backend native selection or validated manual input, never reconstructed UI text.
    #[serde(skip)]
    pub path: PathBuf,
    pub source: GrantSource,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum GrantSource {
    NativeSelection,
    ValidatedManual,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct GrantSet {
    pub grants: Vec<SourceGrant>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RootEvidenceState {
    #[default]
    Absent,
    Matches,
    Mismatch,
    Unavailable,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RootEvidence {
    pub state: RootEvidenceState,
    pub expected_identity: Option<PhysicalId>,
    pub observed_identity: Option<PhysicalId>,
    pub collection_id: Option<CollectionId>,
    pub marker_root_id: Option<RootId>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Support {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FilesystemCapabilities {
    pub read_only: Option<bool>,
    pub physical_identity: Support,
    pub hard_links: Support,
    pub atomic_rename: Support,
    pub durable_flush: Support,
    pub local_filesystem: Option<bool>,
}
#[derive(Clone, Debug, Serialize)]
pub struct RootBinding {
    pub id: RootId,
    pub source_grant: SourceGrant,
    #[serde(skip)]
    pub path: PathBuf,
    pub evidence: RootEvidence,
    pub capabilities: FilesystemCapabilities,
}
impl RootBinding {
    /// Called only after backend selection/validation. No filesystem changes or
    /// marker reads/writes occur here. Parent traversal is rejected, not collapsed
    /// across a potentially linked component.
    pub fn new(path: PathBuf) -> Result<Self, crate::InventoryError> {
        if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(crate::InventoryError::InvalidRequest(
                "source must be absolute without parent traversal".into(),
            ));
        }
        let path: PathBuf = path.components().collect();
        Ok(Self {
            id: RootId::new(),
            source_grant: SourceGrant {
                id: GrantId::new(),
                path: path.clone(),
                source: GrantSource::ValidatedManual,
            },
            path,
            evidence: RootEvidence::default(),
            capabilities: FilesystemCapabilities::default(),
        })
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct InventoryRequest {
    pub roots: Vec<RootBinding>,
    #[serde(with = "crate::exact::u64_decimal")]
    pub generation: u64,
    pub policy: ProtectedPolicy,
}
impl InventoryRequest {
    pub fn new(roots: Vec<RootBinding>) -> Self {
        Self {
            roots,
            generation: 0,
            policy: ProtectedPolicy::default(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationWindow {
    #[serde(with = "crate::exact::i128_decimal")]
    pub started_ns: i128,
    #[serde(with = "crate::exact::i128_decimal")]
    pub finished_ns: i128,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverageOutcome {
    Complete,
    Failed,
    Cancelled,
    Excluded,
}
#[derive(Clone, Debug, Serialize)]
pub struct ScopeCoverage {
    pub root_id: RootId,
    #[serde(skip)]
    pub directory: PathBuf,
    /// None denotes the selected directory itself.
    pub relative_directory: Option<NativePathRecord>,
    pub listed_names: Vec<NativePathRecord>,
    pub children_complete: bool,
    pub subtree_complete: bool,
    pub outcome: CoverageOutcome,
    pub filter_fingerprint: String,
    pub window: ObservationWindow,
    pub issues: Vec<InventoryIssue>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Inventory {
    pub id: InventoryId,
    #[serde(with = "crate::exact::u64_decimal")]
    pub generation: u64,
    pub roots: Vec<RootBinding>,
    pub entries: Vec<EntrySnapshot>,
    pub coverage: Vec<ScopeCoverage>,
    pub window: ObservationWindow,
    pub issues: Vec<InventoryIssue>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaseSensitivity {
    Sensitive,
    Insensitive,
    #[default]
    Unknown,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NameNormalization {
    Preserved,
    Nfc,
    Nfd,
    #[default]
    Unknown,
}
/// Exact names/observations captured per parent by a read-only platform adapter.
#[derive(Clone, Debug, Serialize)]
pub struct NamespaceEntry {
    pub name: NativePathRecord,
    #[serde(skip)]
    pub absolute_path: PathBuf,
    pub kind: EntryKind,
    pub stamp: FileStamp,
    pub protected: bool,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct DirectoryFacts {
    #[serde(skip)]
    pub parent_path: PathBuf,
    pub source_grants: Vec<GrantId>,
    pub entries: Vec<NamespaceEntry>,
    pub case_sensitivity: CaseSensitivity,
    pub normalization: NameNormalization,
    #[serde(with = "crate::exact::option_u64_decimal")]
    pub max_component_units: Option<u64>,
    #[serde(with = "crate::exact::option_u64_decimal")]
    pub max_path_units: Option<u64>,
    pub forbidden_characters: Vec<String>,
    pub trailing_dots_spaces_allowed: Option<bool>,
    pub capabilities: FilesystemCapabilities,
    pub children_complete: bool,
    pub window: ObservationWindow,
    pub issues: Vec<InventoryIssue>,
}
pub const BLUEPRINT_MARKER_NAME: &str = ".vidlib-blueprint.json";
pub const STAGING_NAME_PREFIX: &str = ".vidlib-staging";

#[derive(Clone, Debug, Serialize)]
pub struct ProtectedPolicy {
    #[serde(skip)]
    pub protected_roots: Vec<PathBuf>,
    pub marker_name: String,
    pub staging_prefixes: Vec<String>,
}
impl Default for ProtectedPolicy {
    fn default() -> Self {
        Self {
            protected_roots: Vec::new(),
            marker_name: BLUEPRINT_MARKER_NAME.into(),
            staging_prefixes: vec![STAGING_NAME_PREFIX.into()],
        }
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct NamespaceSnapshot {
    pub parents: Vec<DirectoryFacts>,
    pub grants: GrantSet,
    pub policy: ProtectedPolicy,
    #[serde(with = "crate::exact::u64_decimal")]
    pub namespace_epoch: u64,
    #[serde(with = "crate::exact::u64_decimal")]
    pub policy_epoch: u64,
    pub window: ObservationWindow,
}

impl ProtectedPolicy {
    /// Canonical exclusion rule shared by inventory and later operation adapters.
    /// Matching uses native units; display strings never participate in lookup.
    /// Case behavior is unknown here, so reserved ASCII variants are protected on
    /// Unix too; Windows uses native ordinal Unicode comparison for root aliases.
    pub fn excludes(&self, path: &Path) -> bool {
        if self
            .protected_roots
            .iter()
            .any(|root| crate::path::conservative_path_is_within(path, root))
        {
            return true;
        }
        path.components().any(|component| {
            let Component::Normal(name) = component else {
                return false;
            };
            if crate::path::conservative_name_matches(name, self.marker_name.as_ref(), false) {
                return true;
            }
            self.staging_prefixes
                .iter()
                .any(|prefix| crate::path::conservative_name_matches(name, prefix.as_ref(), true))
        })
    }
}
