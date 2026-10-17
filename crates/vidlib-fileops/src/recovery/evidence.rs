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
use vidlib_rename::{EntryPrestate, ExecutionAction, ExecutionStep, ExpectedIdentity};
pub(crate) fn stamp(op: &PendingOperation, entry: &EntryPrestate) -> Result<FileStamp, OpError> {
    match &entry.identity {
        ExpectedIdentity::Observed(s) | ExpectedIdentity::MetadataChecked { stamp: s, .. }
            if s.physical_id.is_some() =>
        {
            Ok(s.clone())
        }
        ExpectedIdentity::DirectoryAfterEffects {
            original,
            created_by,
            effects,
        } => {
            for id in effects {
                receipt(op, *id)?;
            }
            if let Some(id) = created_by {
                let fact = receipt(op, *id)?;
                if fact.phase != CompletedPhase::DirectoryCreated
                    || fact.after_path.as_ref() != Some(&encode_path(&entry.path))
                {
                    return Err(error("directory creator path or kind differs"));
                }
                fact.after
                    .clone()
                    .ok_or_else(|| error("directory creator identity missing"))
            } else {
                Ok(original.clone())
            }
        }
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
pub(crate) fn material(
    op: &PendingOperation,
    entry: &EntryPrestate,
    fs: &dyn Filesystem,
) -> Result<FileStamp, OpError> {
    let expected = prestate_with_effects(op, entry)?;
    let fields = crate::metadata::controlled_fields(op, entry)?;
    if !fields.is_empty() {
        let actual = fs.inspect(&entry.path)?;
        // Preflight pinned the original controlled poststate. Only this operation's
        // durable child path effects can advance directory mtime; historical
        // forward children never authorize a refreshed inverse prestate.
        let child_path_effect = op
            .steps
            .iter()
            .chain(op.recovery_effects.iter().map(|e| &e.inverse))
            .filter(|s| {
                !s.receipts.is_empty()
                    && !matches!(s.step.action, ExecutionAction::SetMetadata { .. })
            })
            .flat_map(|s| &s.receipts)
            .any(|fact| {
                [&fact.before_path, &fact.after_path]
                    .into_iter()
                    .flatten()
                    .filter_map(|p| decode_path(p).ok())
                    .any(|p| p != entry.path && p.starts_with(&entry.path))
            });
        if actual.kind == vidlib_inventory::EntryKind::Directory && child_path_effect {
            let retained: Vec<_> = fields
                .iter()
                .copied()
                .filter(|f| *f != vidlib_rename::TimestampField::Modified)
                .collect();
            crate::platform::check_timestamps(
                &actual.stamp,
                &expected,
                &retained,
                crate::Phase::Verify,
            )?;
            let proof = crate::metadata::directory_proof(op, entry, &expected);
            return material(op, &proof, fs);
        }
        crate::platform::check_timestamps(&actual.stamp, &expected, &fields, crate::Phase::Verify)?;
    }
    if matches!(
        entry.identity,
        ExpectedIdentity::DirectoryAfterEffects { .. }
    ) {
        verified_material_inner(op, &entry.path, &expected, fs, true, None, None)
    } else {
        Ok(expected)
    }
}
// Durable lineage and grant authority do not depend on the pre-action pathname
// still existing. Receipt-gap probing must establish these before the outcome.
fn prestate_with_effects(
    op: &PendingOperation,
    entry: &EntryPrestate,
) -> Result<FileStamp, OpError> {
    let expected = stamp(op, entry)?;
    if let ExpectedIdentity::DirectoryAfterEffects { effects, .. } = &entry.identity {
        let authorized = op.inspection["grants"]
            .as_array()
            .ok_or_else(|| error("missing native grants"))?
            .iter()
            .map(|value| {
                serde_json::from_value::<NativePathRecord>(value["path"].clone())
                    .map_err(|_| error("invalid grant path"))
                    .and_then(|path| {
                        decode_path(&path).map_err(|_| error("unavailable grant path"))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|grant| entry.path.starts_with(grant));
        if !authorized {
            return Err(error("directory material is outside native grants"));
        }
        for pending in op
            .steps
            .iter()
            .chain(op.recovery_effects.iter().map(|e| &e.inverse))
        {
            for fact in &pending.receipts {
                let child = [&fact.before_path, &fact.after_path]
                    .into_iter()
                    .flatten()
                    .map(decode_path)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| error("invalid child receipt path"))?
                    .iter()
                    .any(|p| p != &entry.path && p.starts_with(&entry.path));
                if child && !effects.contains(&fact.step_id) {
                    return Err(error("unnamed directory child effect"));
                }
            }
        }
    }
    Ok(expected)
}
pub(crate) fn absent(fs: &dyn Filesystem, path: &Path) -> bool {
    matches!(fs.inspect(path), Err(e) if e.code == ErrorCode::SourceMissing)
}
pub(crate) fn paths(action: &ExecutionAction) -> (Option<&EntryPrestate>, Option<&Path>) {
    match action {
        ExecutionAction::RenameEntry { source, target, .. } => (Some(source), Some(target)),
        ExecutionAction::StageCopy { source, staging } => (Some(source), Some(staging)),
        ExecutionAction::CreateStaging { staging } => (None, Some(staging)),
        ExecutionAction::WriteStaging { source, staging } => {
            (Some(source), Some(&staging.entry.path))
        }
        ExecutionAction::PublishCopy {
            staging, target, ..
        } => (Some(&staging.entry), Some(target)),
        ExecutionAction::CreateDirectory { target } => (None, Some(target)),
        ExecutionAction::SetMetadata { entry, .. } => (Some(entry), Some(&entry.path)),
        ExecutionAction::RemoveSource { source, .. }
        | ExecutionAction::RemoveSourceDirectory { source, .. } => (Some(source), None),
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => (Some(&owned.entry), None),
    }
}
pub(crate) fn phase(action: &ExecutionAction) -> CompletedPhase {
    match action {
        ExecutionAction::RenameEntry { .. } => CompletedPhase::Renamed,
        ExecutionAction::CreateStaging { .. } => CompletedPhase::StagingCreated,
        ExecutionAction::StageCopy { .. } | ExecutionAction::WriteStaging { .. } => {
            CompletedPhase::CopyStaged
        }
        ExecutionAction::PublishCopy { .. } => CompletedPhase::CopyPublished,
        ExecutionAction::CreateDirectory { .. } => CompletedPhase::DirectoryCreated,
        ExecutionAction::SetMetadata { .. } => CompletedPhase::MetadataSet,
        ExecutionAction::RemoveSource { .. } => CompletedPhase::SourceRemoved,
        ExecutionAction::RemoveSourceDirectory { .. } => CompletedPhase::SourceDirectoryRemoved,
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
            if let ExecutionAction::CreateDirectory { target } = &step.step.action {
                let id = step
                    .receipts
                    .last()
                    .and_then(|r| r.after.as_ref())
                    .and_then(|s| s.physical_id.clone())
                    .ok_or_else(|| error("created directory identity missing"))?;
                parents.push((target.clone(), id));
            }
            if let ExecutionAction::RemoveSourceDirectory { source, .. } = &step.step.action {
                parents.retain(|(p, _)| !p.starts_with(&source.path));
            }
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
                if let ExecutionAction::CreateDirectory { target } = &inverse.inverse.step.action {
                    let fact = receipt(op, inverse.inverse.step.id)?;
                    let id = fact
                        .after
                        .as_ref()
                        .and_then(|s| s.physical_id.clone())
                        .ok_or_else(|| error("inverse directory identity missing"))?;
                    parents.retain(|(p, _)| p != target);
                    parents.push((target.clone(), id));
                }
                if let ExecutionAction::RemoveOwnedEmptyDirectory { owned } =
                    &inverse.inverse.step.action
                {
                    parents.retain(|(p, _)| !p.starts_with(&owned.entry.path));
                }
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
pub(crate) fn parents(
    op: &PendingOperation,
    step: &ExecutionStep,
    inverse: bool,
    fs: &dyn Filesystem,
) -> Result<(PhysicalId, PhysicalId), OpError> {
    if let Some(effect) = op
        .recovery_effects
        .iter()
        .find(|e| e.inverse.step.id == step.id)
    {
        let (source, target) = paths(&step.action);
        let source_path = source
            .map(|s| s.path.as_path())
            .or(target)
            .ok_or_else(|| error("missing inverse path"))?;
        let target_path = target.unwrap_or(source_path);
        let resolve = |authority: &crate::journal::ParentAuthority,
                       path: &Path|
         -> Result<PhysicalId, OpError> {
            let parent = path
                .parent()
                .ok_or_else(|| error("inverse parent missing"))?;
            let id = match authority {
                crate::journal::ParentAuthority::Observed(id) => id.clone(),
                crate::journal::ParentAuthority::Created { created_by, path } => {
                    let creator = op
                        .recovery_effects
                        .iter()
                        .find(|e| e.inverse.step.id == *created_by)
                        .ok_or_else(|| error("inverse parent creator lineage missing"))?;
                    if creator.inverse.step.group_id != step.group_id
                        || !effect.depends_on.contains(created_by)
                        || decode_path(path).ok().as_deref() != Some(parent)
                        || !matches!(&creator.inverse.step.action, ExecutionAction::CreateDirectory { target } if target == parent)
                    {
                        return Err(error("inverse parent path, kind or dependency differs"));
                    }
                    let fact = receipt(op, *created_by)?;
                    if fact.phase != CompletedPhase::DirectoryCreated
                        || fact.after_path.as_ref() != Some(path)
                    {
                        return Err(error("inverse parent lacks durable creation receipt"));
                    }
                    fact.after
                        .as_ref()
                        .and_then(|s| s.physical_id.clone())
                        .ok_or_else(|| error("inverse parent identity missing"))?
                }
            };
            if fs.inspect(parent)?.stamp.physical_id.as_ref() != Some(&id) {
                return Err(error("inverse parent identity changed"));
            }
            Ok(id)
        };
        return Ok((
            resolve(&effect.source_parent, source_path)?,
            resolve(&effect.target_parent, target_path)?,
        ));
    }
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
pub(crate) fn digest_matches(
    fs: &dyn Filesystem,
    path: &Path,
    stamp: &FileStamp,
    digest: &ContentDigest,
) -> Result<bool, OpError> {
    if digest.algorithm == "sha256" {
        return Ok(
            crate::copy::sha256(fs, path, stamp, &vidlib_inventory::JobControl::default())?
                == *digest,
        );
    }
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
pub(crate) fn receipt(op: &PendingOperation, id: StepId) -> Result<&StepReceipt, OpError> {
    op.steps
        .iter()
        .chain(op.recovery_effects.iter().map(|e| &e.inverse))
        .find(|s| s.step.id == id)
        .and_then(|s| s.receipts.last())
        .or_else(|| op.historical_receipts.iter().find(|r| r.step_id == id))
        .ok_or_else(|| error("required ownership receipt is missing"))
}
/// Resolve a single actual action gap; planned later steps are never guessed.
pub(crate) fn probe(
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
    let source_absent = source.is_some_and(|s| absent(fs, &s.path));
    let mut before = source
        .map(|s| {
            if let ExecutionAction::SetMetadata { changes, .. } = &step.action {
                let before = crate::metadata::captured_prestate(op, step, s)?;
                if fs.inspect(&s.path)?.kind == vidlib_inventory::EntryKind::Directory {
                    let proof = crate::metadata::directory_proof(op, s, &before);
                    prestate_with_effects(op, &proof)?;
                    // Only this intent's exact requested fields may differ at its
                    // receipt gap. Keep all child and unrelated attributes factual.
                    let post = crate::metadata::poststate(&before, changes).ok();
                    verified_material_inner(op, &s.path, &before, fs, true, None, post.as_ref())?;
                }
                Ok(before)
            } else if source_absent {
                prestate_with_effects(op, s)
            } else {
                material(op, s, fs)
            }
        })
        .transpose()?;
    if !matches!(step.action, ExecutionAction::SetMetadata { .. }) {
        if let (Some(entry), Some(expected)) = (source, before.as_ref()) {
            let fields = crate::metadata::controlled_fields(op, entry)?;
            let surviving = if !source_absent {
                Some(entry.path.as_path())
            } else if matches!(step.action, ExecutionAction::RenameEntry { .. }) {
                target
            } else {
                None
            };
            if let Some(path) = surviving {
                if !fields.is_empty() {
                    crate::platform::check_timestamps(
                        &fs.inspect(path)?.stamp,
                        expected,
                        &fields,
                        crate::Phase::Verify,
                    )?;
                }
            }
        }
    }
    if matches!(
        step.action,
        ExecutionAction::RemoveOwnedEmptyDirectory { .. }
    ) {
        if let (Some(source), Some(expected)) = (source, before.as_ref()) {
            if let Ok(current) = fs.inspect(&source.path) {
                if current.kind != vidlib_inventory::EntryKind::Directory
                    || current.stamp.physical_id != expected.physical_id
                    || current.stamp.created_ns != expected.created_ns
                    || current.stamp.windows_attributes != expected.windows_attributes
                {
                    return Err(error("owned directory replaced"));
                }
                before = Some(current.stamp);
            }
        }
    }
    let source_present = source
        .zip(before.as_ref())
        .is_some_and(|(s, e)| fs.verify(&s.path, e).is_ok());
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
                let expected = before.as_ref().expect("rename source");
                if let Some(source) = source.filter(|s| {
                    matches!(s.identity, ExpectedIdentity::DirectoryAfterEffects { .. })
                }) {
                    // Prove the exact carried namespace at its surviving location;
                    // this is a read-only projection, never a fabricated receipt.
                    before = Some(verified_material_inner(
                        op,
                        t,
                        expected,
                        fs,
                        true,
                        Some(&source.path),
                        None,
                    )?);
                }
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
        ExecutionAction::CreateDirectory { .. } | ExecutionAction::CreateStaging { .. } => {
            if target_absent {
                RecoveryDisposition::NotApplied
            } else {
                RecoveryDisposition::Unresolved
            }
        }
        ExecutionAction::WriteStaging { staging, .. } => {
            let created = receipt(op, staging.created_by)?;
            let current = fs.inspect(&staging.entry.path)?;
            if source_present
                && current.kind == vidlib_inventory::EntryKind::File
                && created.phase == CompletedPhase::StagingCreated
                && current.stamp.physical_id.is_some()
                && current.stamp.physical_id
                    == created.after.as_ref().and_then(|s| s.physical_id.clone())
            {
                // The write/verification was not durably completed. Rewriting
                // this recorded object is safe; never synthesize a digest.
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
        ExecutionAction::RemoveSourceDirectory {
            destination_created_by,
            required_descendants,
            ..
        } => {
            crate::copy::verify_directory_destination(
                op,
                *destination_created_by,
                required_descendants,
                fs,
            )?;
            if source_present {
                RecoveryDisposition::NotApplied
            } else if source_absent {
                RecoveryDisposition::Applied
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
                if matches!(
                    step.action,
                    ExecutionAction::RemoveOwnedEmptyDirectory { .. }
                ) && matches!(
                    owned.entry.identity,
                    ExpectedIdentity::DirectoryAfterEffects { .. }
                ) {
                    let expected = before.as_ref().expect("owned directory prestate");
                    let (directories, entries, _) = transformed_namespace(op)?;
                    let creator = receipt(op, owned.created_by)?;
                    if creator.phase != CompletedPhase::DirectoryCreated
                        || creator.after_path.as_ref() != Some(&encode_path(&owned.entry.path))
                        || !directories.iter().any(|(p, id)| {
                            p == &owned.entry.path && Some(id) == expected.physical_id.as_ref()
                        })
                        || entries
                            .keys()
                            .any(|p| p != &owned.entry.path && p.starts_with(&owned.entry.path))
                    {
                        return Err(error(
                            "absent owned directory lacks empty descendant evidence",
                        ));
                    }
                }
                RecoveryDisposition::Applied
            } else {
                RecoveryDisposition::Unresolved
            }
        }
        ExecutionAction::SetMetadata { entry, changes } => {
            let observed = fs.inspect(&entry.path)?.stamp;
            if crate::metadata::verify_fields(
                &observed,
                before.as_ref().expect("metadata source"),
                changes,
            )
            .is_ok()
            {
                RecoveryDisposition::NotApplied
            } else if crate::metadata::verify_fields(
                &observed,
                &crate::metadata::poststate(before.as_ref().expect("metadata source"), changes)?,
                changes,
            )
            .is_ok()
            {
                after = Some(observed);
                RecoveryDisposition::Applied
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
pub(crate) fn verified_material(
    op: &PendingOperation,
    path: &Path,
    expected: &FileStamp,
    fs: &dyn Filesystem,
) -> Result<FileStamp, OpError> {
    verified_material_inner(op, path, expected, fs, false, None, None)
}
fn verified_material_inner(
    op: &PendingOperation,
    path: &Path,
    expected: &FileStamp,
    fs: &dyn Filesystem,
    force: bool,
    relocated_from: Option<&Path>,
    metadata_post: Option<&FileStamp>,
) -> Result<FileStamp, OpError> {
    if !force {
        if let Ok(observed) = fs.verify(path, expected) {
            return Ok(observed.stamp);
        }
    }
    let observed = fs.inspect(path)?;
    if observed.kind != vidlib_inventory::EntryKind::Directory
        || observed.stamp.physical_id != expected.physical_id
    {
        return Err(error("compensation material or identity changed"));
    }
    let (mut directories, mut entries, changed) = transformed_namespace(op)?;
    if let Some(source) = relocated_from {
        if !directories
            .iter()
            .any(|(p, id)| p == source && Some(id) == expected.physical_id.as_ref())
            || entries
                .keys()
                .any(|p| p.starts_with(path) && !p.starts_with(source))
        {
            return Err(error("directory rename lacks unique pre-action namespace"));
        }
        for (directory, _) in &mut directories {
            rebase(directory, source, path);
        }
        entries = entries
            .into_iter()
            .map(|(mut p, value)| {
                rebase(&mut p, source, path);
                (p, value)
            })
            .collect();
    }
    let factual = entries
        .get(path)
        .map(|(stamp, _)| stamp)
        .unwrap_or(expected);
    let unchanged_attributes = |stamp: &FileStamp| {
        (stamp.created_ns == factual.created_ns
            && stamp.windows_attributes == factual.windows_attributes)
            || metadata_post.is_some_and(|post| {
                stamp.created_ns == post.created_ns
                    && stamp.windows_attributes == post.windows_attributes
            })
    };
    if !unchanged_attributes(&observed.stamp) {
        return Err(error(
            "directory attributes changed without an authorized effect",
        ));
    }
    let identity = expected
        .physical_id
        .as_ref()
        .ok_or_else(|| error("directory identity missing"))?;
    if (!force && !changed.contains(identity))
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
    if let Some(source) = relocated_from {
        for grant in &mut grants.grants {
            rebase(&mut grant.path, source, path);
        }
    }
    let directories = directories
        .into_iter()
        .filter(|(p, _)| !force || p.starts_with(path))
        .collect::<Vec<_>>();
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
        if entries.get(directory).is_some_and(|(stamp, _)| {
            if directory == path {
                !unchanged_attributes(&captured.stamp)
            } else {
                captured.stamp.created_ns != stamp.created_ns
                    || captured.stamp.windows_attributes != stamp.windows_attributes
            }
        }) {
            return Err(error("captured descendant directory attributes changed"));
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
                && (actual.stamp.created_ns != stamp.created_ns
                    || actual.stamp.windows_attributes != stamp.windows_attributes)
            {
                return Err(error("descendant directory attributes changed"));
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
    if !unchanged_attributes(&captured.stamp) {
        return Err(error("directory attributes changed during verification"));
    }
    Ok(fs.verify(path, &captured.stamp)?.stamp)
}

pub(crate) fn reserved(op: &PendingOperation, path: &Path) -> bool {
    op.steps.iter().chain(op.recovery_effects.iter().map(|e| &e.inverse)).any(|s| matches!(&s.step.action,ExecutionAction::RenameEntry{target,temporary:true,..} if target==path))
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
        let fact = pending.receipts.last().expect("factual effect");
        let before = fact
            .before_path
            .as_ref()
            .map(decode_path)
            .transpose()
            .map_err(|_| error("invalid effect path"))?;
        let after = fact
            .after_path
            .as_ref()
            .map(decode_path)
            .transpose()
            .map_err(|_| error("invalid effect path"))?;
        for (directory, id) in &directories {
            if before.as_ref().and_then(|p| p.parent()) == Some(directory.as_path())
                || after.as_ref().and_then(|p| p.parent()) == Some(directory.as_path())
            {
                changed.push(id.clone());
            }
        }
        match &pending.step.action {
            ExecutionAction::RenameEntry { .. } | ExecutionAction::PublishCopy { .. } => {
                let source = before
                    .as_ref()
                    .ok_or_else(|| error("move source missing"))?;
                let target = after.as_ref().ok_or_else(|| error("move target missing"))?;
                entries = entries
                    .into_iter()
                    .map(|(mut p, v)| {
                        rebase(&mut p, source, target);
                        (p, v)
                    })
                    .collect();
                for (p, _) in &mut directories {
                    rebase(p, source, target);
                }
                if let Some((stamp, _)) = entries.get_mut(target) {
                    *stamp = fact
                        .after
                        .clone()
                        .ok_or_else(|| error("move stamp missing"))?;
                }
            }
            ExecutionAction::CreateDirectory { .. }
            | ExecutionAction::CreateStaging { .. }
            | ExecutionAction::StageCopy { .. }
            | ExecutionAction::WriteStaging { .. }
            | ExecutionAction::SetMetadata { .. } => {
                let path = after.ok_or_else(|| error("created path missing"))?;
                let stamp = fact
                    .after
                    .clone()
                    .ok_or_else(|| error("created stamp missing"))?;
                let kind = if matches!(pending.step.action, ExecutionAction::CreateDirectory { .. })
                {
                    vidlib_inventory::EntryKind::Directory
                } else {
                    entries
                        .get(&path)
                        .map_or(vidlib_inventory::EntryKind::File, |(_, k)| *k)
                };
                if kind == vidlib_inventory::EntryKind::Directory
                    && !directories.iter().any(|(p, _)| *p == path)
                {
                    directories.push((
                        path.clone(),
                        stamp
                            .physical_id
                            .clone()
                            .ok_or_else(|| error("directory identity missing"))?,
                    ));
                }
                entries.insert(path, (stamp, kind));
            }
            ExecutionAction::RemoveSource { .. }
            | ExecutionAction::RemoveSourceDirectory { .. }
            | ExecutionAction::RemoveOwnedCopy { .. }
            | ExecutionAction::RemoveOwnedEmptyDirectory { .. } => {
                let path = before.ok_or_else(|| error("removed path missing"))?;
                entries.remove(&path);
                directories.retain(|(p, _)| *p != path);
            }
        }
    }
    Ok((directories, entries, changed))
}
