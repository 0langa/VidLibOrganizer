//! Native filesystem primitives, independent of recipes, execution steps and journals.
use crate::{ErrorCode, OpError, Phase};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
use vidlib_inventory::{
    encode_path, DirectoryFacts, EntryKind, FileObservation, FileStamp, GrantSet, InventoryIssue,
    IssueCode, NamespaceEntry, NamespaceSnapshot, ObservationWindow, PhysicalId, ProtectedPolicy,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Durability {
    Synced,
    Unsupported,
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct MutationOutcome {
    pub observation: FileObservation,
    pub durability: Durability,
}
#[derive(Clone, Debug, Default)]
pub struct MetadataUpdate {
    pub modified_ns: Option<i128>,
    pub accessed_ns: Option<i128>,
    pub created_ns: Option<i128>,
    pub readonly: Option<bool>,
    pub windows_attributes: Option<u32>,
}
#[derive(Debug)]
pub struct OwnedEntry {
    pub path: PathBuf,
    pub stamp: FileStamp,
}
#[derive(Debug)]
pub struct StagingFile {
    pub ownership: OwnedEntry,
    pub file: File,
}

/// Exact rename input plus the reviewed parent identities. The executor rebases
/// paths after preceding directory moves while retaining those identities. Native
/// adapters must compare them with the held handles used for the rename syscall.
#[derive(Debug)]
pub struct RenameRequest<'a> {
    pub source: &'a Path,
    pub target: &'a Path,
    pub expected: &'a FileStamp,
    pub source_parent: &'a PhysicalId,
    pub target_parent: &'a PhysicalId,
}

/// Unforgeable outside this crate; minted only after the compiled intent commits.
/// Exact operation/step paths and identity, not a general staging exemption.
#[derive(Debug)]
pub struct RenameReservation {
    pub(crate) operation: vidlib_inventory::ids::OperationId,
    pub(crate) step: vidlib_inventory::ids::StepId,
    pub(crate) source: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) expected: FileStamp,
    pub(crate) source_parent: PhysicalId,
    pub(crate) target_parent: PhysicalId,
    pub(crate) reserved_source: bool,
    pub(crate) reserved_target: bool,
}
impl RenameReservation {
    pub fn operation_id(&self) -> vidlib_inventory::ids::OperationId {
        self.operation
    }
    pub fn step_id(&self) -> vidlib_inventory::ids::StepId {
        self.step
    }
    pub fn source(&self) -> &Path {
        &self.source
    }
    pub fn target(&self) -> &Path {
        &self.target
    }
}

/// Every implementation must reject uncertain capability, preserve a conflicting
/// destination, and verify fresh native identity before an operation.
/// Journal/coordinator authorization is the caller's responsibility.
pub trait Filesystem {
    /// No compatibility fallback: a reviewed execution must retain both parent
    /// identities all the way to the native handles used for mutation.
    fn rename_reviewed(
        &mut self,
        _request: &RenameRequest<'_>,
    ) -> Result<MutationOutcome, OpError> {
        Err(OpError::unsupported(Phase::Rename))
    }
    fn rename_reserved(
        &mut self,
        _authority: &RenameReservation,
    ) -> Result<MutationOutcome, OpError> {
        Err(OpError::unsupported(Phase::Rename))
    }

    fn capture_namespace(
        &self,
        grants: &GrantSet,
        candidates: &[PathBuf],
    ) -> Result<NamespaceSnapshot, OpError>;
    fn inspect(&self, path: &Path) -> Result<FileObservation, OpError>;
    fn verify(&self, path: &Path, expected: &FileStamp) -> Result<FileObservation, OpError>;
    fn rename_no_replace(
        &mut self,
        source: &Path,
        target: &Path,
        expected: &FileStamp,
    ) -> Result<MutationOutcome, OpError>;
    fn publish_staging(
        &mut self,
        owned: &OwnedEntry,
        target: &Path,
    ) -> Result<MutationOutcome, OpError>;
    fn create_directory(&mut self, path: &Path) -> Result<OwnedEntry, OpError>;
    fn create_staging(&mut self, path: &Path) -> Result<StagingFile, OpError>;
    fn refresh_staging(&self, staging: &mut StagingFile) -> Result<Durability, OpError>;
    fn open_verified(&self, path: &Path, expected: &FileStamp) -> Result<File, OpError>;
    fn set_metadata(
        &mut self,
        path: &Path,
        expected: &FileStamp,
        update: &MetadataUpdate,
    ) -> Result<MutationOutcome, OpError>;
    fn flush_directory(&self, path: &Path) -> Result<Durability, OpError>;
    fn remove_owned(&mut self, entry: &OwnedEntry) -> Result<Durability, OpError>;
}

#[derive(Clone, Debug, Default)]
pub struct PlatformFs {
    pub policy: ProtectedPolicy,
    /// Supplied from the persisted operation store while its read lease is held.
    pub namespace_epoch: u64,
    pub policy_epoch: u64,
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as native;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as native;

fn epoch_ns(time: std::time::SystemTime) -> i128 {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    }
}
fn now_ns() -> i128 {
    epoch_ns(std::time::SystemTime::now())
}
fn io_error(error: std::io::Error, phase: Phase) -> OpError {
    use std::io::ErrorKind;
    let code = match error.kind() {
        ErrorKind::AlreadyExists => ErrorCode::DestinationExists,
        ErrorKind::NotFound => ErrorCode::SourceMissing,
        ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
        ErrorKind::Unsupported => ErrorCode::Unsupported,
        ErrorKind::CrossesDevices => ErrorCode::CrossDevice,
        ErrorKind::WouldBlock => ErrorCode::Busy,
        _ => ErrorCode::Io,
    };
    OpError::new(code, phase, error.to_string())
}
fn validate_path(path: &Path) -> Result<(), OpError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(OpError::new(
            ErrorCode::InvalidPath,
            Phase::Inspect,
            "absolute native path without parent traversal required",
        ));
    }
    for part in path.components() {
        if let std::path::Component::Normal(name) = part {
            encode_path(Path::new(name))
                .validate_relative()
                .map_err(|_| {
                    OpError::new(
                        ErrorCode::InvalidPath,
                        Phase::Inspect,
                        "invalid native component",
                    )
                })?;
        }
    }
    Ok(())
}
pub(crate) fn check_stamp(
    actual: &FileStamp,
    expected: &FileStamp,
    phase: Phase,
) -> Result<(), OpError> {
    // Access time can change as a consequence of our own verified content read.
    // Creation time, when supplied, must remain available and equal.
    if expected.physical_id.is_none()
        || actual.physical_id != expected.physical_id
        || actual.size != expected.size
        || actual.modified_ns != expected.modified_ns
        || expected
            .created_ns
            .is_some_and(|t| actual.created_ns != Some(t))
        || actual.windows_attributes != expected.windows_attributes
    {
        return Err(OpError::new(
            ErrorCode::StaleState,
            phase,
            "identity or freshness no longer matches the expected state",
        ));
    }
    Ok(())
}
#[cfg(any(target_os = "linux", windows))]
fn parent(path: &Path) -> Result<(File, &std::ffi::OsStr), OpError> {
    validate_path(path)?;
    let name = path.file_name().ok_or_else(|| {
        OpError::new(
            ErrorCode::InvalidPath,
            Phase::Inspect,
            "operation requires a named child",
        )
    })?;
    let directory =
        native::directory(path.parent().ok_or_else(|| {
            OpError::new(ErrorCode::InvalidPath, Phase::Inspect, "parent required")
        })?)?;
    Ok((directory, name))
}
#[cfg(any(target_os = "linux", windows))]
fn opened(path: &Path) -> Result<File, OpError> {
    let (parent, name) = parent(path)?;
    let file = native::open_child(&parent, name, false)?;
    let observation = native::observation(&file)?;
    match observation.kind {
        EntryKind::File | EntryKind::Directory => Ok(file),
        EntryKind::Symlink | EntryKind::ReparsePoint => Err(OpError::new(
            ErrorCode::ReparsePoint,
            Phase::Inspect,
            "link or reparse entry rejected",
        )),
        _ => Err(OpError::unsupported(Phase::Inspect)),
    }
}
impl PlatformFs {
    fn permit(&self, path: &Path) -> Result<(), OpError> {
        validate_path(path)?;
        if self.policy.excludes(path) {
            return Err(OpError::new(
                ErrorCode::ProtectedEntry,
                Phase::Verify,
                "protected entry cannot be changed",
            ));
        }
        Ok(())
    }
    fn permit_staging(&self, path: &Path) -> Result<(), OpError> {
        let parent = path.parent().ok_or_else(|| {
            OpError::new(
                ErrorCode::InvalidPath,
                Phase::Create,
                "staging parent required",
            )
        })?;
        self.permit(parent)?;
        validate_path(path)?;
        if !self.policy.allows_owned_staging(path) {
            return Err(OpError::new(
                ErrorCode::ProtectedEntry,
                Phase::Create,
                "owned staging cannot bypass cycle reservations or explicit protection",
            ));
        }
        Ok(())
    }
    #[cfg(any(target_os = "linux", windows))]
    fn move_entry(
        &self,
        source: &Path,
        target: &Path,
        expected: &FileStamp,
        reviewed_parents: Option<(&PhysicalId, &PhysicalId)>,
    ) -> Result<MutationOutcome, OpError> {
        let (source_parent, source_name) = parent(source)?;
        let (target_parent, target_name) = parent(target)?;
        if let Some((expected_source_parent, expected_target_parent)) = reviewed_parents {
            // These are the exact held descriptors passed to native::rename.
            // A fresh path binding alone cannot establish reviewed authority.
            for (held, expected) in [
                (&source_parent, expected_source_parent),
                (&target_parent, expected_target_parent),
            ] {
                if native::observation(held)?.stamp.physical_id.as_ref() != Some(expected) {
                    return Err(OpError::new(
                        ErrorCode::StaleState,
                        Phase::Rename,
                        "held parent identity differs from reviewed namespace",
                    ));
                }
            }
        }
        native::mutation_supported(&source_parent, Phase::Rename)?;
        native::mutation_supported(&target_parent, Phase::Rename)?;
        validate_destination_name(&target_parent, target_name)?;
        let source_file = native::open_mutable_child(&source_parent, source_name, Phase::Rename)?;
        let before = native::observation(&source_file)?;
        check_stamp(&before.stamp, expected, Phase::Rename)?;
        if !matches!(before.kind, EntryKind::File | EntryKind::Directory) {
            return Err(OpError::unsupported(Phase::Rename));
        }
        // Keep both parent descriptors and the source handle alive across the call.
        let immediate = native::child_observation(&source_parent, source_name)?;
        check_stamp(&immediate.stamp, expected, Phase::Rename)?;
        verify_parent_binding(
            source.parent().expect("validated parent"),
            &source_parent,
            Phase::Rename,
        )?;
        verify_parent_binding(
            target.parent().expect("validated parent"),
            &target_parent,
            Phase::Rename,
        )?;
        native::rename(
            &source_parent,
            source_name,
            &source_file,
            &target_parent,
            target_name,
        )?;
        let after = native::child_observation(&target_parent, target_name).map_err(|_| {
            OpError::new(
                ErrorCode::RecoveryRequired,
                Phase::Rename,
                "rename applied but destination cannot be observed",
            )
        })?;
        check_stamp(&after.stamp, expected, Phase::Rename).map_err(|_| {
            OpError::new(
                ErrorCode::RecoveryRequired,
                Phase::Rename,
                "rename applied with unexpected destination state; reconcile by identity",
            )
        })?;
        applied_parent_binding(
            source.parent().expect("validated parent"),
            &source_parent,
            Phase::Rename,
        )?;
        applied_parent_binding(
            target.parent().expect("validated parent"),
            &target_parent,
            Phase::Rename,
        )?;
        let durability =
            combine_flush(native::flush(&source_parent), native::flush(&target_parent));
        Ok(MutationOutcome {
            observation: after,
            durability,
        })
    }
}
fn combine_flush(a: Durability, b: Durability) -> Durability {
    match (a, b) {
        (Durability::Failed(e), _) | (_, Durability::Failed(e)) => Durability::Failed(e),
        (Durability::Unsupported, _) | (_, Durability::Unsupported) => Durability::Unsupported,
        _ => Durability::Synced,
    }
}
#[cfg(any(target_os = "linux", windows))]
impl Filesystem for PlatformFs {
    fn rename_reviewed(&mut self, request: &RenameRequest<'_>) -> Result<MutationOutcome, OpError> {
        self.permit(request.source)?;
        self.permit(request.target)?;
        self.move_entry(
            request.source,
            request.target,
            request.expected,
            Some((request.source_parent, request.target_parent)),
        )
    }
    fn rename_reserved(
        &mut self,
        authority: &RenameReservation,
    ) -> Result<MutationOutcome, OpError> {
        for (path, reserved) in [
            (&authority.source, authority.reserved_source),
            (&authority.target, authority.reserved_target),
        ] {
            if reserved {
                validate_path(path)?;
                if !self.policy.allows_rename_reservation(path) {
                    return Err(OpError::new(
                        ErrorCode::ProtectedEntry,
                        Phase::Verify,
                        "compiled reservation violates protected policy",
                    ));
                }
            } else {
                self.permit(path)?;
            }
        }
        self.move_entry(
            &authority.source,
            &authority.target,
            &authority.expected,
            Some((&authority.source_parent, &authority.target_parent)),
        )
    }
    fn capture_namespace(
        &self,
        grants: &GrantSet,
        candidates: &[PathBuf],
    ) -> Result<NamespaceSnapshot, OpError> {
        let started_ns = now_ns();
        let mut parents = Vec::new();
        for candidate in candidates {
            validate_path(candidate)?;
            let path = candidate.parent().ok_or_else(|| {
                OpError::new(
                    ErrorCode::InvalidPath,
                    Phase::Capture,
                    "candidate parent required",
                )
            })?;
            let source_grants: Vec<_> = grants
                .grants
                .iter()
                .filter(|grant| candidate.starts_with(&grant.path))
                .map(|g| g.id)
                .collect();
            if source_grants.is_empty() {
                return Err(OpError::new(
                    ErrorCode::OutsideGrant,
                    Phase::Capture,
                    "candidate is outside supplied native grants",
                ));
            }
            if let Some(existing) = parents
                .iter_mut()
                .find(|facts: &&mut DirectoryFacts| facts.parent_path == path)
            {
                for grant in source_grants {
                    if !existing.source_grants.contains(&grant) {
                        existing.source_grants.push(grant);
                    }
                }
                continue;
            }
            let started_ns = now_ns();
            let directory = native::directory(path)?;
            let before = native::observation(&directory)?;
            let mut facts = native::facts(&directory)?;
            facts.stamp = before.stamp.clone();
            facts.parent_path = path.to_owned();
            facts.source_grants = source_grants;
            let listing = native::listing(&directory)?;
            facts.children_complete = listing.errors.is_empty();
            for message in listing.errors {
                facts.issues.push(InventoryIssue {
                    code: IssueCode::ListingFailed,
                    relative_path: None,
                    message,
                });
            }
            for name in listing.names {
                let absolute_path = path.join(&name);
                let name = encode_path(Path::new(&name));
                let observed = native::child_observation(
                    &directory,
                    absolute_path.file_name().expect("listed name"),
                );
                let (kind, stamp) = match observed {
                    Ok(observation) => (observation.kind, observation.stamp),
                    Err(error) => {
                        facts.children_complete = false;
                        facts.issues.push(InventoryIssue {
                            code: IssueCode::StatFailed,
                            relative_path: Some(name.clone()),
                            message: error.message,
                        });
                        (EntryKind::Unknown, FileStamp::default())
                    }
                };
                facts.entries.push(NamespaceEntry {
                    name,
                    protected: self.policy.excludes(&absolute_path),
                    absolute_path,
                    kind,
                    stamp,
                });
            }
            let after = native::observation(&directory)?;
            if before.stamp.physical_id != after.stamp.physical_id
                || before.stamp.modified_ns != after.stamp.modified_ns
                || verify_parent_binding(path, &directory, Phase::Capture).is_err()
            {
                facts.children_complete = false;
                facts.issues.push(InventoryIssue {
                    code: IssueCode::DirectoryChanged,
                    relative_path: None,
                    message: "directory changed during namespace capture".into(),
                });
            }
            facts.window = ObservationWindow {
                started_ns,
                finished_ns: now_ns(),
            };
            parents.push(facts);
        }
        Ok(NamespaceSnapshot {
            parents,
            grants: grants.clone(),
            policy: self.policy.clone(),
            namespace_epoch: self.namespace_epoch,
            policy_epoch: self.policy_epoch,
            window: ObservationWindow {
                started_ns,
                finished_ns: now_ns(),
            },
        })
    }
    fn inspect(&self, path: &Path) -> Result<FileObservation, OpError> {
        native::observation(&opened(path)?)
    }
    fn verify(&self, path: &Path, expected: &FileStamp) -> Result<FileObservation, OpError> {
        let observation = self.inspect(path)?;
        check_stamp(&observation.stamp, expected, Phase::Verify)?;
        Ok(observation)
    }
    fn rename_no_replace(
        &mut self,
        source: &Path,
        target: &Path,
        expected: &FileStamp,
    ) -> Result<MutationOutcome, OpError> {
        self.permit(source)?;
        self.permit(target)?;
        self.move_entry(source, target, expected, None)
    }
    fn publish_staging(
        &mut self,
        owned: &OwnedEntry,
        target: &Path,
    ) -> Result<MutationOutcome, OpError> {
        self.permit_staging(&owned.path)?;
        self.permit(target)?;
        self.move_entry(&owned.path, target, &owned.stamp, None)
    }
    fn create_directory(&mut self, path: &Path) -> Result<OwnedEntry, OpError> {
        self.permit(path)?;
        let (parent, name) = parent(path)?;
        native::mutation_supported(&parent, Phase::Create)?;
        validate_destination_name(&parent, name)?;
        verify_parent_binding(
            path.parent().expect("validated parent"),
            &parent,
            Phase::Create,
        )?;
        let file = native::create(&parent, name, true)?;
        let stamp = native::observation(&file)
            .map_err(|_| {
                OpError::new(
                    ErrorCode::RecoveryRequired,
                    Phase::Create,
                    "created directory identity unavailable",
                )
            })?
            .stamp;
        applied_parent_binding(
            path.parent().expect("validated parent"),
            &parent,
            Phase::Create,
        )?;
        Ok(OwnedEntry {
            path: path.to_owned(),
            stamp,
        })
    }
    fn create_staging(&mut self, path: &Path) -> Result<StagingFile, OpError> {
        self.permit_staging(path)?;
        let (parent, name) = parent(path)?;
        native::mutation_supported(&parent, Phase::Create)?;
        validate_destination_name(&parent, name)?;
        verify_parent_binding(
            path.parent().expect("validated parent"),
            &parent,
            Phase::Create,
        )?;
        let file = native::create(&parent, name, false)?;
        let stamp = native::observation(&file)
            .map_err(|_| {
                OpError::new(
                    ErrorCode::RecoveryRequired,
                    Phase::Create,
                    "created staging identity unavailable",
                )
            })?
            .stamp;
        applied_parent_binding(
            path.parent().expect("validated parent"),
            &parent,
            Phase::Create,
        )?;
        Ok(StagingFile {
            ownership: OwnedEntry {
                path: path.to_owned(),
                stamp,
            },
            file,
        })
    }
    fn refresh_staging(&self, staging: &mut StagingFile) -> Result<Durability, OpError> {
        self.permit_staging(&staging.ownership.path)?;
        let (parent, name) = parent(&staging.ownership.path)?;
        native::mutation_supported(&parent, Phase::Write)?;
        let current = native::observation(&staging.file)?;
        if current.kind != EntryKind::File
            || current.stamp.physical_id.is_none()
            || current.stamp.physical_id != staging.ownership.stamp.physical_id
        {
            return Err(OpError::new(
                ErrorCode::StaleState,
                Phase::Write,
                "staging handle no longer matches its recorded ownership",
            ));
        }
        check_stamp(
            &native::child_observation(&parent, name)?.stamp,
            &current.stamp,
            Phase::Write,
        )?;
        verify_parent_binding(
            staging.ownership.path.parent().expect("validated parent"),
            &parent,
            Phase::Write,
        )?;
        let durability = combine_flush(native::flush(&staging.file), native::flush(&parent));
        staging.ownership.stamp = current.stamp;
        Ok(durability)
    }
    fn open_verified(&self, path: &Path, expected: &FileStamp) -> Result<File, OpError> {
        let file = opened(path)?;
        let observation = native::observation(&file)?;
        if observation.kind != EntryKind::File {
            return Err(OpError::unsupported(Phase::Read));
        }
        check_stamp(&observation.stamp, expected, Phase::Read)?;
        Ok(file)
    }
    fn set_metadata(
        &mut self,
        path: &Path,
        expected: &FileStamp,
        update: &MetadataUpdate,
    ) -> Result<MutationOutcome, OpError> {
        self.permit(path)?;
        let (parent, name) = parent(path)?;
        native::mutation_supported(&parent, Phase::Metadata)?;
        let file = native::open_mutable_child(&parent, name, Phase::Metadata)?;
        #[cfg(all(test, target_os = "linux"))]
        boundary_fixture::run(boundary_fixture::Boundary::MetadataBeforeRevalidation);
        check_stamp(
            &native::observation(&file)?.stamp,
            expected,
            Phase::Metadata,
        )?;
        verify_parent_binding(
            path.parent().expect("validated parent"),
            &parent,
            Phase::Metadata,
        )?;
        let immediate = native::child_observation(&parent, name).map_err(|_| {
            OpError::new(
                ErrorCode::StaleState,
                Phase::Metadata,
                "metadata target binding cannot be revalidated",
            )
        })?;
        check_stamp(&immediate.stamp, expected, Phase::Metadata)?;
        native::metadata(&file, update)?;
        applied_parent_binding(
            path.parent().expect("validated parent"),
            &parent,
            Phase::Metadata,
        )?;
        let observation = native::observation(&file).map_err(|_| {
            OpError::new(
                ErrorCode::RecoveryRequired,
                Phase::Metadata,
                "metadata applied but observation failed",
            )
        })?;
        native::child_observation(&parent, name)
            .and_then(|bound| check_stamp(&bound.stamp, &observation.stamp, Phase::Metadata))
            .map_err(|_| {
                OpError::new(
                    ErrorCode::RecoveryRequired,
                    Phase::Metadata,
                    "metadata applied but requested pathname no longer has the held object's state",
                )
            })?;
        Ok(MutationOutcome {
            observation,
            durability: native::flush(&file),
        })
    }
    fn flush_directory(&self, path: &Path) -> Result<Durability, OpError> {
        validate_path(path)?;
        Ok(native::flush(&native::directory(path)?))
    }
    fn remove_owned(&mut self, entry: &OwnedEntry) -> Result<Durability, OpError> {
        if self.permit(&entry.path).is_err() {
            self.permit_staging(&entry.path)?;
        }
        let (parent, name) = parent(&entry.path)?;
        native::mutation_supported(&parent, Phase::Remove)?;
        let file = native::open_mutable_child(&parent, name, Phase::Remove)?;
        let observation = native::observation(&file)?;
        check_stamp(&observation.stamp, &entry.stamp, Phase::Remove)?;
        let links_before = native::removal_link_count(&file)?;
        if links_before == 0 {
            return Err(OpError::new(
                ErrorCode::StaleState,
                Phase::Remove,
                "owned object was already unlinked",
            ));
        }
        check_stamp(
            &native::child_observation(&parent, name)?.stamp,
            &entry.stamp,
            Phase::Remove,
        )?;
        verify_parent_binding(
            entry.path.parent().expect("validated parent"),
            &parent,
            Phase::Remove,
        )?;
        native::remove(
            &parent,
            name,
            &file,
            observation.kind == EntryKind::Directory,
        )?;
        applied_parent_binding(
            entry.path.parent().expect("validated parent"),
            &parent,
            Phase::Remove,
        )?;
        let links_after = native::removal_link_count(&file).map_err(|_| {
            OpError::new(
                ErrorCode::RecoveryRequired,
                Phase::Remove,
                "unlink applied but held-object deletion evidence is unavailable",
            )
        })?;
        let intended_link_removed = if observation.kind == EntryKind::Directory {
            links_after == 0
        } else {
            links_before.checked_sub(1) == Some(links_after)
        };
        let name_absent = matches!(native::child_observation(&parent, name), Err(error) if error.code == ErrorCode::SourceMissing);
        if !intended_link_removed || !name_absent {
            return Err(OpError::new(ErrorCode::RecoveryRequired, Phase::Remove, "unlink applied but intended owned removal cannot be established from held-object and pathname evidence"));
        }
        Ok(native::flush(&parent))
    }
}

#[cfg(any(target_os = "linux", windows))]
fn validate_destination_name(parent: &File, name: &std::ffi::OsStr) -> Result<(), OpError> {
    native::facts(parent)?
        .validate_name(&encode_path(Path::new(name)))
        .map_err(|violation| {
            OpError::new(
                if violation == vidlib_inventory::NameViolation::UnknownPolicy {
                    ErrorCode::Unsupported
                } else {
                    ErrorCode::InvalidPath
                },
                Phase::Verify,
                format!("invalid destination component: {violation:?}"),
            )
        })
}

#[cfg(not(any(target_os = "linux", windows)))]
impl Filesystem for PlatformFs {
    fn capture_namespace(&self, _: &GrantSet, _: &[PathBuf]) -> Result<NamespaceSnapshot, OpError> {
        Err(OpError::unsupported(Phase::Capture))
    }
    fn inspect(&self, _: &Path) -> Result<FileObservation, OpError> {
        Err(OpError::unsupported(Phase::Inspect))
    }
    fn verify(&self, _: &Path, _: &FileStamp) -> Result<FileObservation, OpError> {
        Err(OpError::unsupported(Phase::Verify))
    }
    fn rename_no_replace(
        &mut self,
        _: &Path,
        _: &Path,
        _: &FileStamp,
    ) -> Result<MutationOutcome, OpError> {
        Err(OpError::unsupported(Phase::Rename))
    }
    fn publish_staging(&mut self, _: &OwnedEntry, _: &Path) -> Result<MutationOutcome, OpError> {
        Err(OpError::unsupported(Phase::Rename))
    }
    fn create_directory(&mut self, _: &Path) -> Result<OwnedEntry, OpError> {
        Err(OpError::unsupported(Phase::Create))
    }
    fn create_staging(&mut self, _: &Path) -> Result<StagingFile, OpError> {
        Err(OpError::unsupported(Phase::Create))
    }
    fn refresh_staging(&self, _: &mut StagingFile) -> Result<Durability, OpError> {
        Err(OpError::unsupported(Phase::Write))
    }
    fn open_verified(&self, _: &Path, _: &FileStamp) -> Result<File, OpError> {
        Err(OpError::unsupported(Phase::Read))
    }
    fn set_metadata(
        &mut self,
        _: &Path,
        _: &FileStamp,
        _: &MetadataUpdate,
    ) -> Result<MutationOutcome, OpError> {
        Err(OpError::unsupported(Phase::Metadata))
    }
    fn flush_directory(&self, _: &Path) -> Result<Durability, OpError> {
        Err(OpError::unsupported(Phase::Flush))
    }
    fn remove_owned(&mut self, _: &OwnedEntry) -> Result<Durability, OpError> {
        Err(OpError::unsupported(Phase::Remove))
    }
}

#[cfg(any(target_os = "linux", windows))]
fn verify_parent_binding(path: &Path, held: &File, phase: Phase) -> Result<(), OpError> {
    let current = native::directory(path).and_then(|file| native::observation(&file));
    let identity = native::observation(held)?.stamp.physical_id;
    if identity.is_none()
        || !current.is_ok_and(|observation| observation.stamp.physical_id == identity)
    {
        return Err(OpError::new(
            ErrorCode::StaleState,
            phase,
            "parent path binding changed",
        ));
    }
    Ok(())
}
#[cfg(any(target_os = "linux", windows))]
fn applied_parent_binding(path: &Path, held: &File, phase: Phase) -> Result<(), OpError> {
    verify_parent_binding(path, held, phase).map_err(|_| {
        OpError::new(
            ErrorCode::RecoveryRequired,
            phase,
            "operation applied but parent path binding changed; reconcile by identity",
        )
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn replaced_parent_binding_is_stale_even_while_old_handle_remains_valid() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        let held = native::directory(&parent).unwrap();
        std::fs::rename(&parent, dir.path().join("old-parent")).unwrap();
        std::fs::create_dir(&parent).unwrap();
        assert_eq!(
            verify_parent_binding(&parent, &held, Phase::Verify)
                .unwrap_err()
                .code,
            ErrorCode::StaleState
        );
    }
}

// Private deterministic fixture seam: omitted from every non-test build, with
// thread-local one-shot callbacks so concurrent tests cannot affect one another.
#[cfg(all(test, target_os = "linux"))]
mod boundary_fixture {
    use std::cell::RefCell;
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) enum Boundary {
        MetadataBeforeRevalidation,
        MetadataBeforeEffect,
        MetadataAfterEffect,
        RemoveBeforeEffect,
        RemoveAfterEffect,
    }
    type Hook = (Boundary, Box<dyn FnOnce()>);
    thread_local! { static HOOK: RefCell<Option<Hook>> = RefCell::new(None); }
    pub(super) struct Guard;
    pub(super) fn install(boundary: Boundary, action: impl FnOnce() + 'static) -> Guard {
        HOOK.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some((boundary, Box::new(action)));
        });
        Guard
    }
    pub(super) fn run(boundary: Boundary) {
        let hook = HOOK.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_some_and(|(at, _)| *at == boundary) {
                slot.take()
            } else {
                None
            }
        });
        if let Some((_, action)) = hook {
            action();
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            HOOK.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod boundary_regressions {
    use super::boundary_fixture::{install, Boundary};
    use super::*;
    use std::fs;
    const UPDATED: i128 = 1_700_000_000_123_456_789;

    #[test]
    fn metadata_rebinding_before_final_validation_is_stale_without_effect() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let moved = dir.path().join("moved");
        fs::write(&source, b"owned").unwrap();
        let mut adapter = PlatformFs::default();
        let expected = adapter.inspect(&source).unwrap().stamp;
        let at_source = source.clone();
        let at_moved = moved.clone();
        let _guard = install(Boundary::MetadataBeforeRevalidation, move || {
            fs::rename(&at_source, at_moved).unwrap();
            fs::write(at_source, b"replacement").unwrap();
        });
        let result = adapter.set_metadata(
            &source,
            &expected,
            &MetadataUpdate {
                modified_ns: Some(UPDATED),
                ..Default::default()
            },
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::StaleState);
        assert_eq!(
            adapter.inspect(&moved).unwrap().stamp.modified_ns,
            expected.modified_ns
        );
        assert_eq!(fs::read(source).unwrap(), b"replacement");
    }
    #[test]
    fn metadata_rebinding_at_native_effect_requires_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let moved = dir.path().join("moved");
        fs::write(&source, b"owned").unwrap();
        let mut adapter = PlatformFs::default();
        let expected = adapter.inspect(&source).unwrap().stamp;
        let at_source = source.clone();
        let at_moved = moved.clone();
        let _guard = install(Boundary::MetadataBeforeEffect, move || {
            fs::rename(&at_source, at_moved).unwrap();
            fs::write(at_source, b"replacement").unwrap();
        });
        let result = adapter.set_metadata(
            &source,
            &expected,
            &MetadataUpdate {
                modified_ns: Some(UPDATED),
                ..Default::default()
            },
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::RecoveryRequired);
        assert_eq!(
            adapter.inspect(&moved).unwrap().stamp.modified_ns,
            Some(UPDATED)
        );
        assert_ne!(
            adapter.inspect(&source).unwrap().stamp.modified_ns,
            Some(UPDATED)
        );
        assert_eq!(fs::read(source).unwrap(), b"replacement");
    }
    #[test]
    fn metadata_path_missing_after_native_effect_requires_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        fs::write(&source, b"owned").unwrap();
        let mut adapter = PlatformFs::default();
        let expected = adapter.inspect(&source).unwrap().stamp;
        let at_source = source.clone();
        let _guard = install(Boundary::MetadataAfterEffect, move || {
            fs::remove_file(at_source).unwrap()
        });
        assert_eq!(
            adapter
                .set_metadata(
                    &source,
                    &expected,
                    &MetadataUpdate {
                        modified_ns: Some(UPDATED),
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .code,
            ErrorCode::RecoveryRequired
        );
        assert!(!source.exists());
    }
    #[test]
    fn owned_removal_rebinding_at_unlink_requires_recovery_for_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let mut adapter = PlatformFs::default();
        for directory in [false, true] {
            let source = dir.path().join(if directory {
                "folder"
            } else {
                ".vidlib-staging-owned"
            });
            let moved = dir.path().join(if directory {
                "moved-folder"
            } else {
                "moved-file"
            });
            let owned = if directory {
                adapter.create_directory(&source).unwrap()
            } else {
                adapter.create_staging(&source).unwrap().ownership
            };
            let at_source = source.clone();
            let at_moved = moved.clone();
            let _guard = install(Boundary::RemoveBeforeEffect, move || {
                fs::rename(&at_source, at_moved).unwrap();
                if directory {
                    fs::create_dir(at_source).unwrap();
                } else {
                    fs::write(at_source, b"replacement").unwrap();
                }
            });
            assert_eq!(
                adapter.remove_owned(&owned).unwrap_err().code,
                ErrorCode::RecoveryRequired
            );
            assert_eq!(
                adapter.inspect(&moved).unwrap().stamp.physical_id,
                owned.stamp.physical_id
            );
            assert!(!source.exists()); // Linux removed the replacement, not the held owned object.
        }
    }
    #[test]
    fn owned_removal_recreated_name_after_unlink_requires_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join(".vidlib-staging-owned");
        let mut adapter = PlatformFs::default();
        let owned = adapter.create_staging(&source).unwrap().ownership;
        let at_source = source.clone();
        let _guard = install(Boundary::RemoveAfterEffect, move || {
            fs::write(at_source, b"new binding").unwrap()
        });
        assert_eq!(
            adapter.remove_owned(&owned).unwrap_err().code,
            ErrorCode::RecoveryRequired
        );
        assert_eq!(fs::read(source).unwrap(), b"new binding");
    }
    #[test]
    fn owned_removal_of_one_hard_link_preserves_other_alias() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join(".vidlib-staging-owned");
        let alias = dir.path().join("alias");
        let mut adapter = PlatformFs::default();
        let mut staging = adapter.create_staging(&source).unwrap();
        use std::io::Write;
        staging.file.write_all(b"owned").unwrap();
        adapter.refresh_staging(&mut staging).unwrap();
        fs::hard_link(&source, &alias).unwrap();
        assert_eq!(
            adapter.remove_owned(&staging.ownership).unwrap(),
            Durability::Synced
        );
        assert!(!source.exists());
        assert_eq!(fs::read(alias).unwrap(), b"owned");
    }
}
