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
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceGrant {
    pub id: GrantId,
    /// Backend native selection or validated manual input, never reconstructed UI text.
    #[serde(skip)]
    pub path: PathBuf,
    pub source: GrantSource,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrantSource {
    NativeSelection,
    /// Explicit native entry selection, including the directory itself.
    /// Unlike older subtree grants, permits the exact selected root entry to
    /// be renamed to a sibling; never grants its parent or sibling contents.
    NativeSelectedEntry,
    ValidatedManual,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
/// Native syntax policy observed for one parent, independent of execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamePolicy {
    Posix,
    Windows,
    #[default]
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameViolation {
    UnknownPolicy,
    InvalidComponent,
    TooLong,
    ForbiddenCharacter,
    ReservedName,
    TrailingDotOrSpace,
}

/// Exact names/observations captured per parent by a read-only platform adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NamespaceEntry {
    pub name: NativePathRecord,
    #[serde(skip)]
    pub absolute_path: PathBuf,
    pub kind: EntryKind,
    pub stamp: FileStamp,
    pub protected: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DirectoryFacts {
    /// Identity of the observed directory; never inferred from its children.
    pub stamp: FileStamp,
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
    pub name_policy: NamePolicy,
    pub reserved_names: Vec<String>,
    pub trailing_dots_spaces_allowed: Option<bool>,
    pub capabilities: FilesystemCapabilities,
    pub children_complete: bool,
    pub window: ObservationWindow,
    pub issues: Vec<InventoryIssue>,
}
pub const BLUEPRINT_MARKER_NAME: &str = ".vidlib-blueprint.json";
pub const STAGING_NAME_PREFIX: &str = ".vidlib-staging";
/// Cycle reservations live under the canonical excluded operation namespace.
pub const RENAME_TEMP_PREFIX: &str = ".vidlib-staging-rename-";

#[derive(Clone, Debug, Serialize)]
pub struct ProtectedPolicy {
    /// Guard these exact directory roots and moves of their ancestors, while
    /// allowing ordinary descendants to be renamed. Distinct from exclusions.
    #[serde(skip)]
    pub guarded_roots: Vec<PathBuf>,
    #[serde(skip)]
    pub protected_roots: Vec<PathBuf>,
    pub marker_name: String,
    pub staging_prefixes: Vec<String>,
}
impl Default for ProtectedPolicy {
    fn default() -> Self {
        Self {
            protected_roots: Vec::new(),
            guarded_roots: Vec::new(),
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
    pub fn guards_move(&self, path: &Path) -> bool {
        self.excludes(path) || self.guards_root_relocation(path)
    }

    /// Exact configured/protected root or carrying ancestor, without treating
    /// owned staging names or ordinary descendants as root relocations.
    pub fn guards_root_relocation(&self, path: &Path) -> bool {
        self.guarded_roots
            .iter()
            .chain(&self.protected_roots)
            .any(|root| crate::path::conservative_path_is_within(root, path))
    }

    /// Narrow path-policy exception for an already compiled operation reservation.
    /// This is not mutation authority: the executor must additionally require the
    /// exact owned step, fresh identity and an absent target under its write lease.
    pub fn allows_rename_reservation(&self, path: &Path) -> bool {
        self.allows_operation_path(path, RENAME_TEMP_PREFIX)
    }

    /// Path-policy eligibility only. Create-new/identity ownership remains the
    /// native caller's responsibility. Copy staging cannot claim cycle paths.
    pub fn allows_owned_staging(&self, path: &Path) -> bool {
        self.allows_operation_path(path, STAGING_NAME_PREFIX)
            && path.file_name().is_some_and(|name| {
                !crate::path::conservative_name_matches(name, RENAME_TEMP_PREFIX.as_ref(), true)
            })
    }

    fn allows_operation_path(&self, path: &Path, prefix: &str) -> bool {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return false;
        };
        crate::path::conservative_name_matches(name, prefix.as_ref(), true)
            && !self.excludes(parent)
            && !crate::path::conservative_name_matches(name, self.marker_name.as_ref(), false)
            && !self
                .guarded_roots
                .iter()
                .chain(&self.protected_roots)
                .any(|root| crate::path::conservative_path_is_within(root, path))
            && !self
                .staging_prefixes
                .iter()
                .filter(|prefix| prefix.as_str() != STAGING_NAME_PREFIX)
                .any(|prefix| crate::path::conservative_name_matches(name, prefix.as_ref(), true))
    }

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
            crate::path::conservative_name_matches(name, STAGING_NAME_PREFIX.as_ref(), true)
                || self.staging_prefixes.iter().any(|prefix| {
                    crate::path::conservative_name_matches(name, prefix.as_ref(), true)
                })
        })
    }
}

impl DirectoryFacts {
    /// Pure native-unit validation shared by the planner and mutation adapter.
    pub fn validate_name(&self, name: &NativePathRecord) -> Result<(), NameViolation> {
        use crate::PathPlatform;
        use NameViolation::*;
        let units: Vec<u16> = match (self.name_policy, name.platform) {
            (NamePolicy::Unknown, _) => return Err(UnknownPolicy),
            (NamePolicy::Posix, PathPlatform::UnixBytes) => {
                name.bytes.iter().map(|b| u16::from(*b)).collect()
            }
            (NamePolicy::Windows, PathPlatform::WindowsUtf16Le)
                if name.bytes.len().is_multiple_of(2) =>
            {
                name.bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes(*b))
                    .collect()
            }
            _ => return Err(InvalidComponent),
        };
        if units.is_empty() || units == [46] || units == [46, 46] {
            return Err(InvalidComponent);
        }
        if self
            .max_component_units
            .is_some_and(|limit| units.len() as u64 > limit)
        {
            return Err(TooLong);
        }
        if units.iter().any(|u| {
            *u == 0
                || *u == 47
                || (self.name_policy == NamePolicy::Windows
                    && (*u < 32 || [34, 42, 58, 60, 62, 63, 92, 124].contains(u)))
        }) {
            return Err(ForbiddenCharacter);
        }
        if self.trailing_dots_spaces_allowed == Some(false)
            && units.last().is_some_and(|u| *u == 32 || *u == 46)
        {
            return Err(TrailingDotOrSpace);
        }
        if self.name_policy == NamePolicy::Windows {
            let stem = units.split(|u| *u == 46).next().unwrap_or_default();
            let end = stem.iter().rposition(|u| *u != 32).map_or(0, |i| i + 1);
            let stem = &stem[..end];
            let fold = |unit: u16| {
                if (97..=122).contains(&unit) {
                    unit - 32
                } else {
                    unit
                }
            };
            if self.reserved_names.iter().any(|reserved| {
                stem.iter()
                    .copied()
                    .map(fold)
                    .eq(reserved.encode_utf16().map(fold))
            }) {
                return Err(ReservedName);
            }
        }
        for forbidden in &self.forbidden_characters {
            let forbidden: Vec<_> = if self.name_policy == NamePolicy::Posix {
                forbidden.bytes().map(u16::from).collect()
            } else {
                forbidden.encode_utf16().collect()
            };
            if !forbidden.is_empty()
                && units
                    .windows(forbidden.len())
                    .any(|window| window == forbidden)
            {
                return Err(ForbiddenCharacter);
            }
        }
        Ok(())
    }
}

impl DirectoryFacts {
    /// Compare native names under this parent's observed policy. Unknown Unicode
    /// folding/normalization is an uncertainty, never permission to overwrite.
    pub fn names_equal(
        &self,
        left: &std::ffi::OsStr,
        right: &std::ffi::OsStr,
    ) -> Result<bool, NameViolation> {
        if left == right {
            return Ok(true);
        }
        if self.normalization != NameNormalization::Preserved
            || self.case_sensitivity == CaseSensitivity::Unknown
        {
            return Err(NameViolation::UnknownPolicy);
        }
        if self.case_sensitivity == CaseSensitivity::Sensitive {
            return Ok(false);
        }
        #[cfg(windows)]
        {
            Ok(crate::path::conservative_name_matches(left, right, false))
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            if !left.as_bytes().is_ascii() || !right.as_bytes().is_ascii() {
                return Err(NameViolation::UnknownPolicy);
            }
            Ok(left.as_bytes().eq_ignore_ascii_case(right.as_bytes()))
        }
    }

    /// Path length is measured in the same native units as component limits.
    pub fn validate_path(&self, path: &Path) -> Result<(), NameViolation> {
        let name = path.file_name().ok_or(NameViolation::InvalidComponent)?;
        self.validate_name(&crate::encode_path(Path::new(name)))?;
        if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(NameViolation::InvalidComponent);
        }
        #[cfg(windows)]
        if path.components().any(|c| matches!(c, Component::Prefix(p) if !matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::UNC(_, _) | std::path::Prefix::VerbatimDisk(_) | std::path::Prefix::VerbatimUNC(_, _)))) {
            return Err(NameViolation::InvalidComponent);
        }
        if self.name_policy == NamePolicy::Windows {
            for component in path.components() {
                if let Component::Normal(name) = component {
                    self.validate_name(&crate::encode_path(Path::new(name)))?;
                }
            }
        }
        let record = crate::encode_path(path);
        let units = record.bytes.len()
            / if record.platform == crate::PathPlatform::WindowsUtf16Le {
                2
            } else {
                1
            };
        if self
            .max_path_units
            .is_some_and(|limit| units as u64 > limit)
        {
            return Err(NameViolation::TooLong);
        }
        Ok(())
    }
}
