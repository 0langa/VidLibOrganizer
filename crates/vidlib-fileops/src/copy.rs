//! Bounded verified copy effects. The caller durably intends every effect;
//! creation ownership and streaming verification have separate receipts.
use crate::{
    events::{CompletedPhase, ContentDigest},
    journal::{PendingOperation, StepReceipt},
    platform::{Durability, MetadataUpdate, OwnedEntry},
    recovery::evidence::{parents, receipt, stamp},
    ErrorCode, Filesystem, OpError, Phase,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};
use vidlib_inventory::{encode_path, FileStamp, JobControl};
use vidlib_rename::{ExecutionAction, ExecutionStep};
const CHUNK: usize = 1024 * 1024;
fn error(message: &str) -> OpError {
    OpError::new(ErrorCode::RecoveryRequired, Phase::Verify, message)
}
fn io(error: std::io::Error) -> OpError {
    OpError::new(ErrorCode::Io, Phase::Write, error.to_string())
}
fn cancelled(control: &JobControl) -> Result<(), OpError> {
    if control.is_cancelled() {
        Err(error("copy cancelled with durable staging ownership"))
    } else {
        Ok(())
    }
}
pub(crate) fn sha256(
    fs: &dyn Filesystem,
    path: &Path,
    expected: &FileStamp,
    control: &JobControl,
) -> Result<ContentDigest, OpError> {
    let mut input = fs.open_verified(path, expected)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; CHUNK];
    loop {
        cancelled(control)?;
        let n = input.read(&mut buffer).map_err(io)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    fs.verify(path, expected)?;
    Ok(ContentDigest {
        algorithm: "sha256".into(),
        bytes: digest.finalize().to_vec(),
    })
}
pub(crate) fn effect(
    op: &PendingOperation,
    step: &ExecutionStep,
    fs: &mut dyn Filesystem,
    control: &JobControl,
) -> Result<StepReceipt, OpError> {
    let (source_parent, target_parent) = parents(
        op,
        step,
        op.recovery_effects
            .iter()
            .any(|e| e.inverse.step.id == step.id),
        fs,
    )?;
    let mut result = StepReceipt {
        step_id: step.id,
        phase: CompletedPhase::StagingCreated,
        before_path: None,
        after_path: None,
        before: None,
        after: None,
        digest: None,
        observed_ns: 0,
        durability: Durability::Unsupported,
    };
    match &step.action {
        ExecutionAction::CreateStaging { staging } => {
            let mut owned = fs.create_staging_reviewed(staging, &target_parent)?;
            result.durability = fs.refresh_staging(&mut owned)?;
            result.after_path = Some(encode_path(staging));
            result.after = Some(owned.ownership.stamp);
            // Close the writer before publication or cleanup (Windows sharing).
            drop(owned.file);
        }
        ExecutionAction::CreateDirectory { target } => {
            let owned = fs.create_directory_reviewed(target, &target_parent)?;
            result.phase = CompletedPhase::DirectoryCreated;
            result.after_path = Some(encode_path(target));
            result.after = Some(owned.stamp);
            result.durability = fs.flush_directory(
                target
                    .parent()
                    .ok_or_else(|| error("directory parent missing"))?,
            )?;
        }
        ExecutionAction::SetMetadata { entry, changes } => {
            return crate::metadata::effect(op, step, entry, changes, fs);
        }
        ExecutionAction::WriteStaging { source, staging } => {
            let expected = stamp(op, source)?;
            let created = receipt(op, staging.created_by)?;
            let expected_staging = fs.inspect(&staging.entry.path)?.stamp;
            if created.phase != CompletedPhase::StagingCreated
                || created.after.as_ref().and_then(|s| s.physical_id.as_ref())
                    != expected_staging.physical_id.as_ref()
            {
                return Err(error("staging has no durable create-new ownership"));
            }
            let mut input = fs.open_verified(&source.path, &expected)?;
            let metadata = input.metadata().map_err(io)?;
            let mut output = fs.open_owned_staging(
                &OwnedEntry {
                    path: staging.entry.path.clone(),
                    stamp: expected_staging,
                },
                &target_parent,
            )?;
            output.file.set_len(0).map_err(io)?;
            let mut digest = Sha256::new();
            let mut buffer = vec![0; CHUNK];
            let mut total = 0u64;
            loop {
                cancelled(control)?;
                let n = input.read(&mut buffer).map_err(io)?;
                if n == 0 {
                    break;
                }
                output.file.write_all(&buffer[..n]).map_err(io)?;
                digest.update(&buffer[..n]);
                total = total
                    .checked_add(n as u64)
                    .ok_or_else(|| error("copy length overflow"))?;
            }
            fs.verify(&source.path, &expected)?;
            if expected.size != Some(total) {
                return Err(error("source length changed during copy"));
            }
            if fs.refresh_staging(&mut output)? != Durability::Synced {
                return Err(error("staging data flush unconfirmed"));
            }
            let owned = output.ownership;
            drop(output.file);
            let digest = ContentDigest {
                algorithm: "sha256".into(),
                bytes: digest.finalize().to_vec(),
            };
            if sha256(fs, &owned.path, &owned.stamp, control)? != digest {
                return Err(error("independent destination digest differs"));
            }
            // Reopen after the independent read; restore intended metadata and
            // flush before closing every writer and authorizing publication.
            let mut output = fs.open_owned_staging(&owned, &target_parent)?;
            output
                .file
                .set_permissions(metadata.permissions())
                .map_err(io)?;
            result.durability = fs.preserve_staging_metadata(
                &mut output,
                &MetadataUpdate {
                    modified_ns: expected.modified_ns,
                    accessed_ns: expected.accessed_ns,
                    created_ns: if cfg!(windows) {
                        expected.created_ns
                    } else {
                        None
                    },
                    windows_attributes: expected.windows_attributes,
                    readonly: None,
                },
            )?;
            let post = output.ownership.stamp;
            drop(output.file);
            if post.size != expected.size
                || post.modified_ns != expected.modified_ns
                || post.accessed_ns != expected.accessed_ns
                || post.windows_attributes != expected.windows_attributes
            {
                return Err(error("intended copy metadata was not preserved"));
            }
            fs.verify(&source.path, &expected)?;
            result.phase = CompletedPhase::CopyStaged;
            result.before_path = Some(encode_path(&source.path));
            result.before = Some(expected);
            result.after_path = Some(encode_path(&staging.entry.path));
            result.after = Some(post);
            result.digest = Some(digest);
        }
        ExecutionAction::PublishCopy {
            staging,
            target,
            verified_by,
        } => {
            let expected = stamp(op, &staging.entry)?;
            let verified = receipt(op, *verified_by)?;
            let digest = verified
                .digest
                .as_ref()
                .ok_or_else(|| error("verified digest missing"))?;
            if digest.algorithm != "sha256"
                || sha256(fs, &staging.entry.path, &expected, control)? != *digest
            {
                return Err(error("staging content changed before publication"));
            }
            let outcome = fs.publish_staging_reviewed(
                &OwnedEntry {
                    path: staging.entry.path.clone(),
                    stamp: expected.clone(),
                },
                target,
                &source_parent,
                &target_parent,
            )?;
            result.phase = CompletedPhase::CopyPublished;
            result.before_path = Some(encode_path(&staging.entry.path));
            result.before = Some(expected);
            result.after_path = Some(encode_path(target));
            result.after = Some(outcome.observation.stamp);
            result.digest = Some(digest.clone());
            result.durability = outcome.durability;
        }
        ExecutionAction::RemoveSourceDirectory {
            source,
            destination_created_by,
            required_descendants,
        } => {
            verify_directory_destination(op, *destination_created_by, required_descendants, fs)?;
            let expected = stamp(op, source)?;
            if fs.verify(&source.path, &expected)?.kind != vidlib_inventory::EntryKind::Directory {
                return Err(error("source quarantine is not a directory"));
            }
            result.phase = CompletedPhase::SourceDirectoryRemoved;
            result.before_path = Some(encode_path(&source.path));
            result.before = Some(expected.clone());
            result.durability = fs.remove_quarantined(&crate::platform::OwnedRemoval {
                owned: OwnedEntry {
                    path: source.path.clone(),
                    stamp: expected,
                },
                parent: target_parent,
            })?;
        }
        ExecutionAction::RemoveSource {
            source,
            published_by,
        } => {
            let published = receipt(op, *published_by)?;
            let path = vidlib_inventory::decode_path(
                published
                    .after_path
                    .as_ref()
                    .ok_or_else(|| error("publication path missing"))?,
            )
            .map_err(|_| error("invalid publication path"))?;
            let post = published
                .after
                .as_ref()
                .ok_or_else(|| error("publication identity missing"))?;
            let digest = published
                .digest
                .as_ref()
                .ok_or_else(|| error("publication digest missing"))?;
            if digest.algorithm != "sha256" || sha256(fs, &path, post, control)? != *digest {
                return Err(error("publication changed; source removal refused"));
            }
            let expected = stamp(op, source)?;
            if sha256(fs, &source.path, &expected, control)? != *digest {
                return Err(error("source content changed after publication"));
            }
            let vidlib_rename::ExpectedIdentity::FromReceipt { step_id } = source.identity else {
                return Err(error("source removal requires durable quarantine"));
            };
            let moved = op
                .steps
                .iter()
                .find(|s| s.step.id == step_id)
                .ok_or_else(|| error("quarantine receipt missing"))?;
            if !matches!(&moved.step.action, ExecutionAction::RenameEntry { target, temporary: true, .. } if target == &source.path)
            {
                return Err(error("source removal lacks compiled quarantine"));
            }
            result.phase = CompletedPhase::SourceRemoved;
            result.before_path = Some(encode_path(&source.path));
            result.before = Some(expected.clone());
            result.durability = fs.remove_quarantined(&crate::platform::OwnedRemoval {
                owned: OwnedEntry {
                    path: source.path.clone(),
                    stamp: expected,
                },
                parent: target_parent,
            })?;
        }
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => {
            let mut expected = crate::recovery::evidence::material(op, &owned.entry, fs)?;
            let creator = receipt(op, owned.created_by)?;
            if expected.physical_id.is_none()
                || creator.after.as_ref().and_then(|s| s.physical_id.as_ref())
                    != expected.physical_id.as_ref()
            {
                return Err(error("cleanup ownership changed"));
            }
            if matches!(
                step.action,
                ExecutionAction::RemoveOwnedEmptyDirectory { .. }
            ) {
                let current = fs.inspect(&owned.entry.path)?;
                if current.kind != vidlib_inventory::EntryKind::Directory
                    || current.stamp.physical_id != expected.physical_id
                    || current.stamp.created_ns != expected.created_ns
                    || current.stamp.windows_attributes != expected.windows_attributes
                {
                    return Err(error("owned directory identity or metadata changed"));
                }
                expected = current.stamp;
            }
            fs.verify(&owned.entry.path, &expected)?;
            if let Some(digest) = op
                .steps
                .iter()
                .chain(op.recovery_effects.iter().map(|e| &e.inverse))
                .flat_map(|s| &s.receipts)
                .chain(&op.historical_receipts)
                .filter(|r| {
                    r.after.as_ref().and_then(|s| s.physical_id.as_ref())
                        == expected.physical_id.as_ref()
                })
                .filter_map(|r| r.digest.as_ref())
                .next_back()
            {
                if digest.algorithm != "sha256"
                    || sha256(fs, &owned.entry.path, &expected, control)? != *digest
                {
                    return Err(error("owned content changed; cleanup refused"));
                }
            }
            result.phase = if matches!(
                step.action,
                ExecutionAction::RemoveOwnedEmptyDirectory { .. }
            ) {
                CompletedPhase::OwnedEmptyDirectoryRemoved
            } else {
                CompletedPhase::OwnedCopyRemoved
            };
            result.before_path = Some(encode_path(&owned.entry.path));
            result.before = Some(expected.clone());
            let owned = OwnedEntry {
                path: owned.entry.path.clone(),
                stamp: expected,
            };
            result.durability = if crate::recovery::evidence::reserved(op, &owned.path) {
                fs.remove_quarantined(&crate::platform::OwnedRemoval {
                    owned,
                    parent: target_parent,
                })?
            } else {
                fs.remove_owned_reviewed(&owned, &target_parent)?
            };
        }
        _ => return Err(OpError::unsupported(Phase::Verify)),
    }
    result.observed_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i128;
    Ok(result)
}

pub(crate) fn validate_plan(plan: &vidlib_rename::RenamePlan) -> Result<(), OpError> {
    if plan.inverse_of.is_some() {
        return Ok(());
    } // Journal::begin re-derives every historical inverse step.
    if !plan.execution.provenance.recipe.metadata_changes.is_empty() {
        return crate::metadata::validate_plan(plan);
    }
    if matches!(
        plan.execution.provenance.recipe.destination,
        vidlib_rename::Destination::CopyTo { .. } | vidlib_rename::Destination::MoveTo { .. }
    ) {
        return vidlib_rename::destination::validate_compiled(plan).map_err(|e| error(&e.message));
    }
    for group in &plan.execution.groups {
        for step in &group.steps {
            let row = plan
                .rows
                .iter()
                .find(|r| {
                    r.entry_id == step.entry_id && r.status == vidlib_rename::RowStatus::Changed
                })
                .ok_or_else(|| error("copy effect lacks selected row"))?;
            let source = plan
                .execution
                .provenance
                .inventory
                .entries
                .iter()
                .find(|e| {
                    e.id == step.entry_id
                        && e.absolute_path == row.source_path
                        && e.stamp == row.stamp
                })
                .ok_or_else(|| error("copy source differs from inventory"))?;
            let within_target = |path: &Path| {
                path.parent() == row.target_path.parent()
                    && plan
                        .execution
                        .expected_namespace
                        .policy
                        .allows_owned_staging(path)
            };
            match &step.action {
                ExecutionAction::CreateStaging { staging } if within_target(staging) => {}
                ExecutionAction::WriteStaging {
                    source: input,
                    staging,
                } if input.path == row.source_path
                    && input.identity
                        == vidlib_rename::ExpectedIdentity::Observed(source.stamp.clone())
                    && within_target(&staging.entry.path) => {}
                ExecutionAction::PublishCopy {
                    staging, target, ..
                } if *target == row.target_path && within_target(&staging.entry.path) => {}
                ExecutionAction::CreateDirectory { target }
                    if row.target_path.starts_with(target) => {}
                ExecutionAction::RenameEntry {
                    source: input,
                    target,
                    temporary: true,
                } if input.path == row.source_path
                    && input.identity
                        == vidlib_rename::ExpectedIdentity::Observed(source.stamp.clone())
                    && target.parent() == input.path.parent()
                    && plan
                        .execution
                        .expected_namespace
                        .policy
                        .allows_rename_reservation(target) => {}
                ExecutionAction::RemoveSource { source: input, .. }
                    if input.path.parent() == row.source_path.parent()
                        && plan
                            .execution
                            .expected_namespace
                            .policy
                            .allows_rename_reservation(&input.path) => {}
                _ => return Err(error("copy compiled effect differs from reviewed row")),
            }
            if !plan
                .grants
                .grants
                .iter()
                .any(|g| row.source_path.starts_with(&g.path))
                || !plan
                    .grants
                    .grants
                    .iter()
                    .any(|g| row.target_path.starts_with(&g.path))
            {
                return Err(error("copy path outside native grant"));
            }
        }
    }
    Ok(())
}

/// A later inverse may already have consumed a staging object, or recreated a
/// removed source. Tie that non-effect to its durable inverse fact, never absence.
pub(crate) fn consumed_inverse(
    op: &PendingOperation,
    original: &crate::journal::PendingStep,
    fs: &dyn Filesystem,
) -> Result<Option<vidlib_inventory::ids::StepId>, OpError> {
    use crate::journal::StepState;
    let original_fact = receipt(op, original.step.id)?;
    for effect in &op.recovery_effects {
        if !matches!(
            effect.inverse.state,
            StepState::CompletionDurable | StepState::Compensated
        ) {
            continue;
        }
        let Some(fact) = effect.inverse.receipts.last() else {
            continue;
        };
        match &original.step.action {
            ExecutionAction::CreateStaging { .. } | ExecutionAction::WriteStaging { .. } => {
                let staging = match &original.step.action {
                    ExecutionAction::CreateStaging { staging } => staging,
                    ExecutionAction::WriteStaging { staging, .. } => &staging.entry.path,
                    _ => unreachable!("staging action"),
                };
                if fact.phase == CompletedPhase::OwnedCopyRemoved
                    && fact.before.as_ref().and_then(|s| s.physical_id.as_ref())
                        == original_fact
                            .after
                            .as_ref()
                            .and_then(|s| s.physical_id.as_ref())
                    && crate::recovery::evidence::absent(fs, staging)
                {
                    return Ok(Some(fact.step_id));
                }
            }
            ExecutionAction::SetMetadata { entry, .. } => {
                if matches!(
                    fact.phase,
                    CompletedPhase::OwnedEmptyDirectoryRemoved | CompletedPhase::OwnedCopyRemoved
                ) && fact.before.as_ref().and_then(|s| s.physical_id.as_ref())
                    == original_fact
                        .after
                        .as_ref()
                        .and_then(|s| s.physical_id.as_ref())
                    && crate::recovery::evidence::absent(fs, &entry.path)
                {
                    return Ok(Some(fact.step_id));
                }
            }
            ExecutionAction::RenameEntry {
                source,
                target,
                temporary: true,
            } => {
                let removed = op.steps.iter().find(|s| s.step.id == effect.original_step);
                if fact.phase == CompletedPhase::MetadataSet && fact.after_path.as_ref() == Some(&encode_path(&source.path)) && removed.is_some_and(|s| matches!(&s.step.action, ExecutionAction::RemoveSourceDirectory { source: removed, .. } if removed.path == *target)) && crate::recovery::evidence::absent(fs, target) { return Ok(Some(fact.step_id)); }
                if fact.phase==CompletedPhase::CopyPublished && fact.after_path.as_ref()==Some(&encode_path(&source.path)) && removed.is_some_and(|s| matches!(&s.step.action,ExecutionAction::RemoveSource{source:removed,..} if removed.path==*target) || matches!(&s.step.action, ExecutionAction::RemoveOwnedCopy { owned } if owned.entry.path == *target)) && crate::recovery::evidence::absent(fs,target) {
                    let stamp=fact.after.as_ref().ok_or_else(||error("restoration poststate missing"))?;
                    let digest=fact.digest.as_ref().ok_or_else(||error("restoration digest missing"))?;
                    let stamp = crate::metadata::latest_post(op, &source.path, stamp);
                    if sha256(fs,&source.path,stamp,&JobControl::default())?!=*digest {return Err(error("restored source changed"));}
                    return Ok(Some(fact.step_id));
                }
            }
            _ => {}
        }
    }
    Ok(None)
}
pub(crate) fn restore_chain(
    op: &PendingOperation,
    original: &crate::journal::PendingStep,
    fs: &dyn Filesystem,
) -> Result<Vec<ExecutionStep>, OpError> {
    use vidlib_inventory::ids::StepId;
    use vidlib_rename::{EntryPrestate, ExpectedIdentity, OwnedEntryPrestate};
    let fact = receipt(op, original.step.id)?;
    let observed = |path: &Path| -> Result<FileStamp, OpError> {
        let expected = fact
            .after
            .as_ref()
            .ok_or_else(|| error("inverse ownership missing"))?;
        let actual = fs.inspect(path)?;
        if actual.stamp.physical_id.is_none() || actual.stamp.physical_id != expected.physical_id {
            return Err(error("inverse object identity changed"));
        }
        Ok(actual.stamp)
    };
    let mk = |id, action| ExecutionStep {
        id,
        entry_id: original.step.entry_id,
        group_id: original.step.group_id,
        action,
    };
    let copy_back = |input: &Path,
                     expected: &FileStamp,
                     target: &Path|
     -> Result<Vec<ExecutionStep>, OpError> {
        let staging =
            target.with_file_name(format!(".vidlib-staging-restore-{}", original.step.id));
        if !crate::recovery::evidence::absent(fs, target)
            || !crate::recovery::evidence::absent(fs, &staging)
        {
            return Err(error("copy-back target occupied"));
        }
        let create = StepId::new();
        let write = StepId::new();
        Ok(vec![
            mk(
                create,
                ExecutionAction::CreateStaging {
                    staging: staging.clone(),
                },
            ),
            mk(
                write,
                ExecutionAction::WriteStaging {
                    source: EntryPrestate {
                        path: input.to_owned(),
                        identity: ExpectedIdentity::Observed(expected.clone()),
                    },
                    staging: Box::new(OwnedEntryPrestate {
                        entry: EntryPrestate {
                            path: staging.clone(),
                            identity: ExpectedIdentity::FromReceipt { step_id: create },
                        },
                        created_by: create,
                    }),
                },
            ),
            mk(
                StepId::new(),
                ExecutionAction::PublishCopy {
                    staging: OwnedEntryPrestate {
                        entry: EntryPrestate {
                            path: staging,
                            identity: ExpectedIdentity::FromReceipt { step_id: write },
                        },
                        created_by: create,
                    },
                    target: target.to_owned(),
                    verified_by: write,
                },
            ),
        ])
    };
    let cleanup = |path: &Path,
                   created_by,
                   expected: FileStamp,
                   directory: bool,
                   quarantine: bool|
     -> Result<Vec<ExecutionStep>, OpError> {
        let mut actions = Vec::new();
        let mut entry = EntryPrestate {
            path: path.to_owned(),
            identity: ExpectedIdentity::Observed(expected),
        };
        if quarantine {
            let target = path.with_file_name(format!(
                "{}cleanup-{}",
                vidlib_inventory::RENAME_TEMP_PREFIX,
                original.step.id
            ));
            if !crate::recovery::evidence::absent(fs, &target) {
                return Err(error("cleanup quarantine is occupied"));
            }
            let id = StepId::new();
            actions.push(mk(
                id,
                ExecutionAction::RenameEntry {
                    source: entry,
                    target: target.clone(),
                    temporary: true,
                },
            ));
            entry = EntryPrestate {
                path: target,
                identity: ExpectedIdentity::FromReceipt { step_id: id },
            };
        }
        let owned = OwnedEntryPrestate { entry, created_by };
        actions.push(mk(
            StepId::new(),
            if directory {
                ExecutionAction::RemoveOwnedEmptyDirectory { owned }
            } else {
                ExecutionAction::RemoveOwnedCopy { owned }
            },
        ));
        Ok(actions)
    };
    match &original.step.action {
        ExecutionAction::SetMetadata { entry, changes } => {
            let expected = fact
                .after
                .as_ref()
                .ok_or_else(|| error("metadata poststate missing"))?;
            let actual = fs.inspect(&entry.path)?.stamp;
            crate::metadata::verify_fields(&actual, expected, changes)?;
            let changes = crate::metadata::inverse_changes(fact, changes)?;
            Ok(vec![mk(
                StepId::new(),
                ExecutionAction::SetMetadata {
                    entry: EntryPrestate {
                        path: entry.path.clone(),
                        identity: ExpectedIdentity::Observed(expected.clone()),
                    },
                    changes,
                },
            )])
        }

        ExecutionAction::CreateStaging { staging } => {
            cleanup(staging, original.step.id, observed(staging)?, false, false)
        }
        ExecutionAction::WriteStaging { staging, .. } => {
            let expected = fact
                .after
                .as_ref()
                .ok_or_else(|| error("verified staging state missing"))?;
            let digest = fact
                .digest
                .as_ref()
                .ok_or_else(|| error("verified staging digest missing"))?;
            if sha256(fs, &staging.entry.path, expected, &JobControl::default())? != *digest {
                return Err(error("staging digest changed"));
            }
            cleanup(
                &staging.entry.path,
                staging.created_by,
                expected.clone(),
                false,
                false,
            )
        }
        ExecutionAction::PublishCopy {
            staging, target, ..
        } => {
            let expected = fact
                .after
                .as_ref()
                .ok_or_else(|| error("publication state missing"))?;
            let digest = fact
                .digest
                .as_ref()
                .ok_or_else(|| error("publication digest missing"))?;
            let expected = crate::metadata::latest_post(op, target, expected);
            if sha256(fs, target, expected, &JobControl::default())? != *digest {
                return Err(error("publication digest changed"));
            }
            cleanup(target, staging.created_by, expected.clone(), false, true)
        }
        ExecutionAction::CreateDirectory { target } => {
            let current = observed(target)?;
            cleanup(target, original.step.id, current, true, false)
        }
        ExecutionAction::RemoveSource {
            source,
            published_by,
        } => {
            let published = receipt(op, *published_by)?;
            let published_path = vidlib_inventory::decode_path(
                published
                    .after_path
                    .as_ref()
                    .ok_or_else(|| error("publication path missing"))?,
            )
            .map_err(|_| error("invalid publication path"))?;
            let expected = published
                .after
                .as_ref()
                .ok_or_else(|| error("publication identity missing"))?;
            let digest = published
                .digest
                .as_ref()
                .ok_or_else(|| error("publication digest missing"))?;
            let expected = crate::metadata::latest_post(op, &published_path, expected);
            if sha256(fs, &published_path, expected, &JobControl::default())? != *digest {
                return Err(error("copy-back digest changed"));
            }
            let ExpectedIdentity::FromReceipt { step_id } = source.identity else {
                return Err(error("removed source quarantine missing"));
            };
            let prior = op
                .steps
                .iter()
                .find(|s| s.step.id == step_id)
                .ok_or_else(|| error("quarantine source missing"))?;
            let ExecutionAction::RenameEntry {
                source: original_source,
                ..
            } = &prior.step.action
            else {
                return Err(error("quarantine lineage missing"));
            };
            let mut chain = copy_back(&published_path, expected, &original_source.path)?;
            let mut prior = chain
                .last()
                .ok_or_else(|| error("copy-back publication missing"))?
                .id;
            for change in crate::metadata::restore_fields(op, &published_path)? {
                let id = StepId::new();
                chain.push(mk(
                    id,
                    ExecutionAction::SetMetadata {
                        entry: EntryPrestate {
                            path: original_source.path.clone(),
                            identity: ExpectedIdentity::FromReceipt { step_id: prior },
                        },
                        changes: vec![change],
                    },
                ));
                prior = id;
            }
            Ok(chain)
        }
        ExecutionAction::RemoveOwnedCopy { owned } => {
            if op.inverse_of.is_none() {
                return Err(error("deleted copy lacks historical inverse authority"));
            }
            let deleted = fact
                .before
                .as_ref()
                .ok_or_else(|| error("deleted copy identity missing"))?;
            let digest = op
                .historical_receipts
                .iter()
                .find(|r| {
                    r.phase == CompletedPhase::CopyPublished
                        && r.after.as_ref().and_then(|s| s.physical_id.as_ref())
                            == deleted.physical_id.as_ref()
                })
                .and_then(|r| r.digest.as_ref())
                .ok_or_else(|| error("deleted copy historical digest missing"))?;
            let ExpectedIdentity::FromReceipt { step_id } = owned.entry.identity else {
                return Err(error("deleted copy quarantine lineage missing"));
            };
            let previous = op
                .steps
                .iter()
                .find(|s| s.step.id == step_id)
                .ok_or_else(|| error("deleted copy quarantine missing"))?;
            let ExecutionAction::RenameEntry {
                source: target,
                target: quarantine,
                ..
            } = &previous.step.action
            else {
                return Err(error("deleted copy quarantine invalid"));
            };
            if quarantine != &owned.entry.path {
                return Err(error("deleted copy quarantine path differs"));
            }
            let candidates = op
                .steps
                .iter()
                .flat_map(|s| &s.receipts)
                .filter(|r| {
                    r.phase == CompletedPhase::CopyPublished && r.digest.as_ref() == Some(digest)
                })
                .filter_map(|r| r.after_path.as_ref().zip(r.after.as_ref()))
                .chain(
                    op.historical_receipts
                        .iter()
                        .filter(|r| {
                            r.phase == CompletedPhase::CopyStaged
                                && r.digest.as_ref() == Some(digest)
                        })
                        .filter_map(|r| r.before_path.as_ref().zip(r.before.as_ref())),
                );
            for (path, expected) in candidates {
                let path = vidlib_inventory::decode_path(path)
                    .map_err(|_| error("copy-back source path unavailable"))?;
                if sha256(fs, &path, expected, &JobControl::default())
                    .is_ok_and(|actual| actual == *digest)
                {
                    return copy_back(&path, expected, &target.path);
                }
            }
            Err(error(
                "deleted inverse copy has no unchanged full-digest source",
            ))
        }
        _ => Err(error("phase has no verified copy inverse")),
    }
}

pub(crate) fn verify_historical_source(
    op: &PendingOperation,
    step: &ExecutionStep,
    fs: &dyn Filesystem,
    control: &JobControl,
) -> Result<(), OpError> {
    if op.inverse_of.is_none() {
        return Ok(());
    }
    let (source, _) = crate::recovery::evidence::paths(&step.action);
    let Some(source) = source else {
        return Ok(());
    };
    let expected = stamp(op, source)?;
    for fact in op.historical_receipts.iter().filter(|r| {
        r.phase == CompletedPhase::CopyPublished
            && r.after.as_ref().and_then(|s| s.physical_id.as_ref())
                == expected.physical_id.as_ref()
    }) {
        let digest = fact
            .digest
            .as_ref()
            .ok_or_else(|| error("historical full digest missing"))?;
        if digest.algorithm != "sha256" || sha256(fs, &source.path, &expected, control)? != *digest
        {
            return Err(error("historical copied content changed"));
        }
    }
    Ok(())
}

pub(crate) fn verify_directory_destination(
    op: &PendingOperation,
    created_by: vidlib_inventory::ids::StepId,
    descendants: &[vidlib_inventory::ids::StepId],
    fs: &dyn Filesystem,
) -> Result<(), OpError> {
    let created = receipt(op, created_by)?;
    if created.phase != CompletedPhase::DirectoryCreated {
        return Err(error("destination directory ownership missing"));
    }
    let path = vidlib_inventory::decode_path(
        created
            .after_path
            .as_ref()
            .ok_or_else(|| error("destination path missing"))?,
    )
    .map_err(|_| error("destination path unavailable"))?;
    let stamp = created
        .after
        .clone()
        .ok_or_else(|| error("destination identity missing"))?;
    for id in descendants {
        receipt(op, *id)?;
    }
    let entry = vidlib_rename::EntryPrestate {
        path,
        identity: vidlib_rename::ExpectedIdentity::DirectoryAfterEffects {
            original: stamp,
            created_by: Some(created_by),
            effects: descendants.to_vec(),
        },
    };
    crate::recovery::evidence::material(op, &entry, fs)?;
    Ok(())
}
