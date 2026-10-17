//! Read-only reconciliation and complete namespace evidence for recovery.
use super::{error, RecoveryDisposition};
use crate::{
    events::{CompletedPhase, ContentDigest},
    journal::{PendingOperation, PendingStep, StepReceipt, StepState},
    platform::Durability,
    ErrorCode, Filesystem, OpError,
};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use vidlib_inventory::{decode_path, encode_path, ids::*, FileStamp, NativePathRecord, PhysicalId};
use vidlib_rename::{
    EntryPrestate, ExecutionAction, ExecutionStep, ExpectedIdentity, MetadataChange, TimestampField,
};
pub(super) fn stamp(op: &PendingOperation, entry: &EntryPrestate) -> Result<FileStamp, OpError> {
    match &entry.identity {
        ExpectedIdentity::Observed(s) if s.physical_id.is_some() => Ok(s.clone()),
        ExpectedIdentity::FromReceipt { step_id } => {
            let r = op
                .steps
                .iter()
                .chain(op.recovery_effects.iter().map(|e| &e.inverse))
                .find(|s| s.step.id == *step_id)
                .and_then(|s| s.receipts.last())
                .ok_or_else(|| error("source receipt is missing"))?;
            if r.after_path.as_ref() != Some(&encode_path(&entry.path)) {
                return Err(error("receipt native path differs from compiled source"));
            }
            r.after
                .clone()
                .ok_or_else(|| error("source receipt has no identity"))
        }
        _ => Err(error("source identity is unknown")),
    }
}
pub(super) fn absent(fs: &dyn Filesystem, path: &Path) -> bool {
    matches!(fs.inspect(path), Err(e) if e.code == ErrorCode::SourceMissing)
}
pub(super) fn paths(action: &ExecutionAction) -> (Option<&EntryPrestate>, Option<&Path>) {
    match action {
        ExecutionAction::RenameEntry { source, target, .. } => (Some(source), Some(target)),
        ExecutionAction::StageCopy { source, staging } => (Some(source), Some(staging)),
        ExecutionAction::PublishCopy {
            staging, target, ..
        } => (Some(&staging.entry), Some(target)),
        ExecutionAction::CreateDirectory { target } => (None, Some(target)),
        ExecutionAction::SetMetadata { entry, .. } => (Some(entry), Some(&entry.path)),
        ExecutionAction::RemoveSource { source, .. } => (Some(source), None),
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => (Some(&owned.entry), None),
    }
}
pub(super) fn phase(action: &ExecutionAction) -> CompletedPhase {
    match action {
        ExecutionAction::RenameEntry { .. } => CompletedPhase::Renamed,
        ExecutionAction::StageCopy { .. } => CompletedPhase::CopyStaged,
        ExecutionAction::PublishCopy { .. } => CompletedPhase::CopyPublished,
        ExecutionAction::CreateDirectory { .. } => CompletedPhase::DirectoryCreated,
        ExecutionAction::SetMetadata { .. } => CompletedPhase::MetadataSet,
        ExecutionAction::RemoveSource { .. } => CompletedPhase::SourceRemoved,
        ExecutionAction::RemoveOwnedCopy { .. } => CompletedPhase::OwnedCopyRemoved,
        ExecutionAction::RemoveOwnedEmptyDirectory { .. } => {
            CompletedPhase::OwnedEmptyDirectoryRemoved
        }
    }
}
pub(crate) fn rebase(path: &mut PathBuf, source: &Path, target: &Path) {
    if let Ok(suffix) = path.strip_prefix(source) {
        *path = if suffix.as_os_str().is_empty() {
            target.to_path_buf()
        } else {
            target.join(suffix)
        };
    }
}
pub(crate) fn parent_map(
    op: &PendingOperation,
    before: Option<StepId>,
) -> Result<Vec<(PathBuf, PhysicalId)>, OpError> {
    let mut parents = Vec::new();
    let values = op.inspection["expected_namespace"]["parents"]
        .as_array()
        .ok_or_else(|| error("original reviewed parent observations are unavailable"))?;
    for value in values {
        let native: NativePathRecord = serde_json::from_value(value["parent_path"].clone())
            .map_err(|_| error("invalid stored native parent"))?;
        let stamp: FileStamp = serde_json::from_value(value["stamp"].clone())
            .map_err(|_| error("invalid stored parent stamp"))?;
        parents.push((
            decode_path(&native).map_err(|_| error("unavailable native path platform"))?,
            stamp
                .physical_id
                .ok_or_else(|| error("unknown reviewed parent identity"))?,
        ));
    }
    for step in &op.steps {
        if Some(step.step.id) == before {
            break;
        }
        if !step.receipts.is_empty() {
            if let ExecutionAction::RenameEntry { source, target, .. } = &step.step.action {
                for (p, _) in &mut parents {
                    rebase(p, &source.path, target);
                }
            }
        }
    }
    if before.is_none() {
        for inverse in &op.recovery_effects {
            if !inverse.inverse.receipts.is_empty() {
                if let ExecutionAction::RenameEntry { source, target, .. } =
                    &inverse.inverse.step.action
                {
                    for (p, _) in &mut parents {
                        rebase(p, &source.path, target);
                    }
                }
            }
        }
    }
    Ok(parents)
}
pub(super) fn parents(
    op: &PendingOperation,
    step: &ExecutionStep,
    inverse: bool,
    fs: &dyn Filesystem,
) -> Result<(PhysicalId, PhysicalId), OpError> {
    let map = parent_map(op, if inverse { None } else { Some(step.id) })?;
    let (source, target) = paths(&step.action);
    let source_path = source
        .map(|s| s.path.as_path())
        .or(target)
        .ok_or_else(|| error("missing effect path"))?;
    let target_path = target.unwrap_or(source_path);
    let lookup = |p: &Path| -> Result<PhysicalId, OpError> {
        let p = p.parent().ok_or_else(|| error("missing effect parent"))?;
        let id = map
            .iter()
            .find(|(path, _)| path == p)
            .map(|(_, id)| id)
            .ok_or_else(|| error("parent lacks original reviewed authority"))?;
        if fs.inspect(p)?.stamp.physical_id.as_ref() != Some(id) {
            return Err(error("reviewed parent identity changed"));
        }
        Ok(id.clone())
    };
    Ok((lookup(source_path)?, lookup(target_path)?))
}
pub(super) fn digest_matches(
    fs: &dyn Filesystem,
    path: &Path,
    stamp: &FileStamp,
    digest: &ContentDigest,
) -> Result<bool, OpError> {
    if digest.algorithm != "md5" {
        return Err(error(
            "digest algorithm has no verified recovery implementation",
        ));
    }
    let mut file = fs.open_verified(path, stamp)?;
    let mut context = md5::Context::new();
    let mut bytes = [0u8; 65536];
    loop {
        let n = file
            .read(&mut bytes)
            .map_err(|_| error("cannot verify content"))?;
        if n == 0 {
            break;
        }
        context.consume(&bytes[..n]);
    }
    fs.verify(path, stamp)?;
    Ok(context.finalize().0.as_slice() == digest.bytes)
}
pub(super) fn receipt(op: &PendingOperation, id: StepId) -> Result<&StepReceipt, OpError> {
    op.steps
        .iter()
        .find(|s| s.step.id == id)
        .and_then(|s| s.receipts.last())
        .ok_or_else(|| error("required ownership receipt is missing"))
}
/// Resolve a single actual action gap; planned later steps are never guessed.
pub(super) fn probe(
    op: &PendingOperation,
    pending: &PendingStep,
    fs: &dyn Filesystem,
) -> Result<(RecoveryDisposition, Option<StepReceipt>), OpError> {
    let step = &pending.step;
    if matches!(pending.state, StepState::CompletionDurable) {
        return Ok((RecoveryDisposition::Completed, None));
    }
    if matches!(pending.state, StepState::Compensated) {
        return Ok((RecoveryDisposition::Compensated, None));
    }
    if pending.state == StepState::Planned {
        return Ok((RecoveryDisposition::Planned, None));
    }
    let inverse = op
        .recovery_effects
        .iter()
        .any(|e| e.inverse.step.id == step.id);
    parents(op, step, inverse, fs)?;
    let (source, target) = paths(&step.action);
    let before = source.map(|s| stamp(op, s)).transpose()?;
    let source_present = source
        .zip(before.as_ref())
        .is_some_and(|(s, e)| fs.verify(&s.path, e).is_ok());
    let source_absent = source.is_some_and(|s| absent(fs, &s.path));
    let target_absent = target.is_some_and(|t| absent(fs, t));
    let mut digest = None;
    let mut after = None;
    let disposition = match &step.action {
        ExecutionAction::RenameEntry { .. } | ExecutionAction::PublishCopy { .. } => {
            if let ExecutionAction::PublishCopy {
                staging,
                verified_by,
                ..
            } = &step.action
            {
                let created = receipt(op, staging.created_by)?;
                let verified = receipt(op, *verified_by)?;
                if created.after.as_ref().and_then(|s| s.physical_id.as_ref())
                    != before.as_ref().and_then(|s| s.physical_id.as_ref())
                {
                    return Err(error("copy ownership differs from creation receipt"));
                }
                digest = Some(
                    verified
                        .digest
                        .clone()
                        .ok_or_else(|| error("verified staging digest unavailable"))?,
                );
            }
            if source_present && target_absent {
                RecoveryDisposition::NotApplied
            } else if source_absent {
                let t = target.expect("rename target");
                let observed = fs.verify(t, before.as_ref().expect("rename source"))?;
                if let Some(d) = &digest {
                    if !digest_matches(fs, t, &observed.stamp, d)? {
                        return Err(error("published content changed"));
                    }
                }
                after = Some(observed.stamp);
                RecoveryDisposition::Applied
            } else {
                RecoveryDisposition::Unresolved
            }
        }
        ExecutionAction::CreateDirectory { .. } => {
            if target_absent {
                RecoveryDisposition::NotApplied
            } else {
                RecoveryDisposition::Unresolved
            }
        }
        ExecutionAction::StageCopy { .. } => {
            // No durable create-new identity exists in this intent/action gap.
            // An existing staging object cannot be claimed by its filename.
            if source_present && target_absent {
                RecoveryDisposition::NotApplied
            } else {
                RecoveryDisposition::Unresolved
            }
        }
        ExecutionAction::RemoveSource { published_by, .. } => {
            let published = receipt(op, *published_by)?;
            let path = decode_path(
                published
                    .after_path
                    .as_ref()
                    .ok_or_else(|| error("missing published path"))?,
            )
            .map_err(|_| error("invalid published path"))?;
            let post = published
                .after
                .as_ref()
                .ok_or_else(|| error("missing published identity"))?;
            let d = published
                .digest
                .as_ref()
                .ok_or_else(|| error("missing published digest"))?;
            if !digest_matches(fs, &path, post, d)? {
                return Err(error("published copy cannot protect source removal"));
            }
            if source_present {
                RecoveryDisposition::NotApplied
            } else if source_absent {
                RecoveryDisposition::Applied
            } else {
                RecoveryDisposition::Unresolved
            }
        }
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => {
            if receipt(op, owned.created_by)?
                .after
                .as_ref()
                .and_then(|s| s.physical_id.as_ref())
                != before.as_ref().and_then(|s| s.physical_id.as_ref())
            {
                return Err(error("cleanup lacks creation identity"));
            }
            if source_present {
                RecoveryDisposition::NotApplied
            } else if source_absent {
                RecoveryDisposition::Applied
            } else {
                RecoveryDisposition::Unresolved
            }
        }
        ExecutionAction::SetMetadata { entry, changes } => {
            let mut post = before.clone().expect("metadata source");
            for change in changes {
                match change {
                    MetadataChange::Timestamp { field, value_ns } => match field {
                        TimestampField::Created => post.created_ns = Some(*value_ns),
                        TimestampField::Modified => post.modified_ns = Some(*value_ns),
                        TimestampField::Accessed => post.accessed_ns = Some(*value_ns),
                    },
                    MetadataChange::WindowsAttributes { set, clear } => {
                        post.windows_attributes = Some(
                            (post
                                .windows_attributes
                                .ok_or_else(|| error("unknown original attributes"))?
                                | set)
                                & !clear,
                        );
                    }
                }
            }
            let observed = fs.inspect(&entry.path)?.stamp;
            if observed == post {
                after = Some(observed);
                RecoveryDisposition::Applied
            } else if before.as_ref() == Some(&observed) {
                RecoveryDisposition::NotApplied
            } else {
                RecoveryDisposition::Unresolved
            }
        }
    };
    let fact = (disposition == RecoveryDisposition::Applied).then(|| StepReceipt {
        step_id: step.id,
        phase: phase(&step.action),
        before_path: source.map(|s| encode_path(&s.path)),
        after_path: target.map(encode_path),
        before,
        after,
        digest,
        observed_ns: 0,
        durability: Durability::Unsupported,
    });
    Ok((disposition, fact))
}

// Child renames legitimately change directory material stamps. Refresh only
// after checking the entire reviewed namespace transformed by durable effects,
// including both forward and inverse carries. Unknown children remain blockers.
pub(super) fn verified_material(
    op: &PendingOperation,
    path: &Path,
    expected: &FileStamp,
    fs: &dyn Filesystem,
) -> Result<FileStamp, OpError> {
    if let Ok(observed) = fs.verify(path, expected) {
        return Ok(observed.stamp);
    }
    let observed = fs.inspect(path)?;
    if observed.kind != vidlib_inventory::EntryKind::Directory
        || observed.stamp.physical_id != expected.physical_id
    {
        return Err(error("compensation material or identity changed"));
    }
    let (directories, entries, changed) = transformed_namespace(op)?;
    let identity = expected
        .physical_id
        .as_ref()
        .ok_or_else(|| error("directory identity missing"))?;
    if !changed.contains(identity)
        || !directories
            .iter()
            .any(|(p, id)| p == path && id == identity)
    {
        return Err(error(
            "directory stamp changed without proven child effects",
        ));
    }
    let mut grants = vidlib_inventory::GrantSet::default();
    for value in op.inspection["grants"]
        .as_array()
        .ok_or_else(|| error("missing native grants"))?
    {
        grants.grants.push(vidlib_inventory::SourceGrant {
            id: serde_json::from_value(value["id"].clone()).map_err(|_| error("invalid grant"))?,
            path: decode_path(
                &serde_json::from_value::<NativePathRecord>(value["path"].clone())
                    .map_err(|_| error("invalid grant path"))?,
            )
            .map_err(|_| error("unavailable grant path"))?,
            source: serde_json::from_value(value["source"].clone())
                .map_err(|_| error("invalid grant source"))?,
        });
    }
    let candidates = directories
        .iter()
        .map(|(p, _)| p.join(".vidlib-capture"))
        .collect::<Vec<_>>();
    let fresh = fs.capture_namespace(&grants, &candidates)?;
    for (directory, id) in &directories {
        let captured = fresh
            .parents
            .iter()
            .find(|p| p.parent_path == *directory)
            .ok_or_else(|| error("directory disappeared during verification"))?;
        if !captured.children_complete
            || !captured.issues.is_empty()
            || captured.stamp.physical_id.as_ref() != Some(id)
        {
            return Err(error("directory capture is incomplete or replaced"));
        }
        let expected_entries = entries
            .iter()
            .filter(|(p, _)| p.parent() == Some(directory.as_path()))
            .collect::<Vec<_>>();
        if captured.entries.len() != expected_entries.len() {
            return Err(error("unowned namespace entries block compensation"));
        }
        for (path, (stamp, kind)) in expected_entries {
            let actual = captured
                .entries
                .iter()
                .find(|e| e.absolute_path == *path)
                .ok_or_else(|| error("expected namespace entry missing"))?;
            if actual.kind != *kind || actual.stamp.physical_id != stamp.physical_id {
                return Err(error("namespace ownership changed"));
            }
            if *kind == vidlib_inventory::EntryKind::Directory
                && stamp
                    .physical_id
                    .as_ref()
                    .is_some_and(|id| changed.contains(id))
                && directories.iter().any(|(p, _)| p == path)
            {
                continue;
            }
            fs.verify(path, stamp)?;
        }
    }
    // Preserve the exact stamp observed with the verified child listing. A
    // later namespace mutation must not be blessed by taking a newer stamp.
    let captured = fresh
        .parents
        .iter()
        .find(|p| p.parent_path == path)
        .ok_or_else(|| error("verified directory capture missing"))?;
    Ok(fs.verify(path, &captured.stamp)?.stamp)
}

pub(super) fn reserved(op: &PendingOperation, path: &Path) -> bool {
    op.steps.iter().any(|s| matches!(&s.step.action,ExecutionAction::RenameEntry{target,temporary:true,..} if target==path))
}

/// Replay factual original and compensation effects, never just effective step
/// labels: a compensated original has both a forward and an inverse receipt.
pub(crate) type NamespaceEvidence = (
    Vec<(PathBuf, PhysicalId)>,
    std::collections::BTreeMap<PathBuf, (FileStamp, vidlib_inventory::EntryKind)>,
    Vec<PhysicalId>,
);
pub(crate) fn transformed_namespace(op: &PendingOperation) -> Result<NamespaceEvidence, OpError> {
    let values = op.inspection["expected_namespace"]["parents"]
        .as_array()
        .ok_or_else(|| error("directory namespace evidence unavailable"))?;
    let native_path = |value: &serde_json::Value| -> Result<PathBuf, OpError> {
        let record: NativePathRecord = serde_json::from_value(value.clone())
            .map_err(|_| error("invalid native namespace path"))?;
        decode_path(&record).map_err(|_| error("native namespace path unavailable"))
    };
    let mut directories = parent_map(op, op.steps.first().map(|s| s.step.id))?;
    let mut entries = std::collections::BTreeMap::new();
    for parent in values {
        if parent["children_complete"] != true {
            return Err(error("directory namespace was incomplete"));
        }
        for entry in parent["entries"]
            .as_array()
            .ok_or_else(|| error("missing directory entries"))?
        {
            let stamp: FileStamp = serde_json::from_value(entry["stamp"].clone())
                .map_err(|_| error("invalid namespace stamp"))?;
            let kind: vidlib_inventory::EntryKind = serde_json::from_value(entry["kind"].clone())
                .map_err(|_| error("invalid namespace kind"))?;
            entries.insert(native_path(&entry["absolute_path"])?, (stamp, kind));
        }
    }
    let mut changed = Vec::new();
    for pending in op
        .steps
        .iter()
        .chain(op.recovery_effects.iter().map(|e| &e.inverse))
        .filter(|s| !s.receipts.is_empty())
    {
        let ExecutionAction::RenameEntry { source, target, .. } = &pending.step.action else {
            return Err(error("directory refresh lacks phase namespace evidence"));
        };
        for (directory, id) in &directories {
            if Some(directory.as_path()) == source.path.parent()
                || Some(directory.as_path()) == target.parent()
            {
                changed.push(id.clone());
            }
        }
        entries = entries
            .into_iter()
            .map(|(mut path, value)| {
                rebase(&mut path, &source.path, target);
                (path, value)
            })
            .collect();
        for (directory, _) in &mut directories {
            rebase(directory, &source.path, target);
        }
    }
    Ok((directories, entries, changed))
}
