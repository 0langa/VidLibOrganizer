//! Compile a whole Restore group before effects. Parents are recreated first;
//! original directory metadata is finalized after all descendant inverses.
use super::{error, evidence::*};
use crate::{
    journal::{ParentAuthority, PendingOperation, StepState},
    Filesystem, OpError,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use vidlib_inventory::{encode_path, ids::*, FileStamp};
use vidlib_rename::{EntryPrestate, ExecutionAction, ExecutionStep, ExpectedIdentity};

pub(crate) struct CompiledInverse {
    pub original: StepId,
    pub step: ExecutionStep,
    pub source_parent: ParentAuthority,
    pub target_parent: ParentAuthority,
    pub depends_on: Vec<StepId>,
}
fn touches_child(step: &ExecutionStep, directory: &Path) -> bool {
    let (source, target) = paths(&step.action);
    source
        .map(|e| e.path.as_path())
        .into_iter()
        .chain(target)
        .any(|p| p != directory && p.starts_with(directory))
}
fn child_effects(
    op: &PendingOperation,
    compiled: &[(StepId, ExecutionStep)],
    directory: &Path,
) -> Vec<StepId> {
    op.steps
        .iter()
        .filter(|s| !s.receipts.is_empty())
        .map(|s| &s.step)
        .chain(compiled.iter().map(|(_, s)| s))
        .filter(|s| touches_child(s, directory))
        .map(|s| s.id)
        .collect()
}

pub(crate) fn compile_group(
    op: &PendingOperation,
    group: GroupId,
    fs: &dyn Filesystem,
) -> Result<Vec<CompiledInverse>, OpError> {
    let done: Vec<_> = op
        .steps
        .iter()
        .filter(|s| s.step.group_id == group && s.state == StepState::CompletionDurable)
        .collect();
    let mut compiled: Vec<(StepId, ExecutionStep)> = Vec::new();
    let mut consumed = BTreeSet::new();
    let mut finalized: Vec<(PathBuf, FileStamp, Option<StepId>, StepId, EntryId)> = Vec::new();
    let mk = |original: &crate::journal::PendingStep, action| ExecutionStep {
        id: StepId::new(),
        entry_id: original.step.entry_id,
        group_id: group,
        action,
    };
    // Undo may itself have deleted owned destination directories. Recreate
    // those parents before restoring their deleted children from verified copies.
    let mut owned_directories: Vec<_> = op
        .steps
        .iter()
        .filter_map(|s| match &s.step.action {
            ExecutionAction::RemoveOwnedEmptyDirectory { owned } if s.step.group_id == group => {
                Some((s, owned))
            }
            _ => None,
        })
        .collect();
    owned_directories.sort_by_key(|(_, owned)| owned.entry.path.components().count());
    for (removed, owned) in owned_directories {
        let path = &owned.entry.path;
        let owner = (removed.state == StepState::CompletionDurable)
            .then_some(removed)
            .or_else(|| {
                done.iter()
                    .rev()
                    .find(|s| touches_child(&s.step, path))
                    .copied()
            });
        let Some(owner) = owner else {
            continue;
        };
        let original = stamp(op, &owned.entry)?;
        let created_by = if removed.state == StepState::CompletionDurable {
            if !absent(fs, path) {
                return Err(error("deleted inverse directory recreated externally"));
            }
            let step = mk(
                owner,
                ExecutionAction::CreateDirectory {
                    target: path.clone(),
                },
            );
            let id = step.id;
            compiled.push((owner.step.id, step));
            consumed.insert(removed.step.id);
            Some(id)
        } else {
            None
        };
        finalized.push((
            path.clone(),
            original,
            created_by,
            owner.step.id,
            owner.step.entry_id,
        ));
    }
    // This loop includes planned ancestor quarantines: completed child effects
    // can have changed their original material even before the ancestor moves.
    let mut directories: Vec<_> = op
        .steps
        .iter()
        .filter_map(|s| match &s.step.action {
            ExecutionAction::RenameEntry {
                source:
                    EntryPrestate {
                        path,
                        identity:
                            ExpectedIdentity::DirectoryAfterEffects {
                                original,
                                created_by: None,
                                ..
                            },
                    },
                target,
                ..
            } if s.step.group_id == group => Some((s, path, original, target)),
            _ => None,
        })
        .collect();
    directories.sort_by_key(|(_, path, _, _)| path.components().count());
    for (quarantine, path, original, temporary) in directories {
        let removed = done.iter().find(|s| matches!(&s.step.action, ExecutionAction::RemoveSourceDirectory { source, .. } if source.path == *temporary));
        let owner = removed
            .copied()
            .or_else(|| (quarantine.state == StepState::CompletionDurable).then_some(quarantine))
            .or_else(|| {
                done.iter()
                    .rev()
                    .find(|s| touches_child(&s.step, path))
                    .copied()
            });
        let Some(owner) = owner else {
            continue;
        };
        let created_by = if removed.is_some() {
            if !absent(fs, path) {
                return Err(error("restoration directory recreated externally"));
            }
            let step = mk(
                owner,
                ExecutionAction::CreateDirectory {
                    target: path.clone(),
                },
            );
            let id = step.id;
            compiled.push((owner.step.id, step));
            consumed.insert(quarantine.step.id);
            Some(id)
        } else if quarantine.state == StepState::CompletionDurable {
            if !absent(fs, path) {
                return Err(error("restoration directory target occupied"));
            }
            let expected = receipt(op, quarantine.step.id)?
                .after
                .clone()
                .ok_or_else(|| error("directory quarantine identity missing"))?;
            fs.verify(temporary, &expected)?;
            compiled.push((
                owner.step.id,
                mk(
                    owner,
                    ExecutionAction::RenameEntry {
                        source: EntryPrestate {
                            path: temporary.clone(),
                            identity: ExpectedIdentity::Observed(expected),
                        },
                        target: path.clone(),
                        temporary: false,
                    },
                ),
            ));
            consumed.insert(quarantine.step.id);
            None
        } else {
            None
        };
        finalized.push((
            path.clone(),
            original.clone(),
            created_by,
            owner.step.id,
            owner.step.entry_id,
        ));
    }
    for original in done.iter().rev() {
        if consumed.contains(&original.step.id) {
            continue;
        }
        match &original.step.action {
            ExecutionAction::RemoveSourceDirectory { .. } => continue,
            ExecutionAction::SetMetadata { .. } => continue, // consumed by owned directory cleanup
            ExecutionAction::CreateStaging { .. } if done.iter().any(|s| matches!(&s.step.action, ExecutionAction::WriteStaging { staging, .. } if staging.created_by == original.step.id)) => continue,
            ExecutionAction::CreateStaging { .. } | ExecutionAction::WriteStaging { .. }
                if done.iter().any(|s| matches!(&s.step.action, ExecutionAction::PublishCopy { staging, verified_by, .. } if staging.created_by == original.step.id || *verified_by == original.step.id)) => continue,
            ExecutionAction::RenameEntry { .. } if done.iter().any(|s| matches!(&s.step.action, ExecutionAction::RemoveSource { source: EntryPrestate { identity: ExpectedIdentity::FromReceipt { step_id }, .. }, .. } if *step_id == original.step.id) || matches!(&s.step.action, ExecutionAction::RemoveOwnedCopy { owned } if matches!(owned.entry.identity, ExpectedIdentity::FromReceipt { step_id } if step_id == original.step.id))) => continue,
            ExecutionAction::RenameEntry { source, target, .. } => {
                let expected = receipt(op, original.step.id)?.after.clone().ok_or_else(|| error("inverse rename poststate missing"))?;
                fs.verify(target, &expected)?;
                if !absent(fs, &source.path) { return Err(error("inverse rename target occupied")); }
                compiled.push((original.step.id, mk(original, ExecutionAction::RenameEntry { source: EntryPrestate { path: target.clone(), identity: ExpectedIdentity::Observed(expected) }, target: source.path.clone(), temporary: reserved(op, &source.path) })));
            }
            _ => {
                let chain = crate::copy::restore_chain(op, original, fs)?;
                for mut step in chain {
                    if let ExecutionAction::RemoveOwnedEmptyDirectory { owned } = &mut step.action {
                        let expected = receipt(op, owned.created_by)?.after.clone().ok_or_else(|| error("directory ownership stamp missing"))?;
                        owned.entry.identity = ExpectedIdentity::DirectoryAfterEffects { original: expected, created_by: Some(owned.created_by), effects: child_effects(op, &compiled, &owned.entry.path) };
                    }
                    compiled.push((original.step.id, step));
                }
            }
        }
    }
    finalized.sort_by_key(|(path, ..)| std::cmp::Reverse(path.components().count()));
    for (path, original, created_by, owner, entry_id) in finalized {
        let effects = child_effects(op, &compiled, &path);
        compiled.push((
            owner,
            ExecutionStep {
                id: StepId::new(),
                entry_id,
                group_id: group,
                action: ExecutionAction::SetMetadata {
                    entry: EntryPrestate {
                        path,
                        identity: ExpectedIdentity::DirectoryAfterEffects {
                            original: original.clone(),
                            created_by,
                            effects,
                        },
                    },
                    changes: vidlib_rename::destination::directory_metadata_changes(&original),
                },
            },
        ));
    }
    // Compile typed future parent bindings; never synthesize a native identity.
    let map = parent_map(op, None)?;
    let mut future = BTreeMap::new();
    let mut result = Vec::new();
    let mut previous = None;
    for (original, step) in compiled {
        let (source, target) = paths(&step.action);
        let source = source
            .map(|s| s.path.as_path())
            .or(target)
            .ok_or_else(|| error("inverse path missing"))?;
        let target = target.unwrap_or(source);
        let parent = |p: &Path| -> Result<ParentAuthority, OpError> {
            let p = p.parent().ok_or_else(|| error("inverse parent missing"))?;
            if let Some(id) = future.get(p) {
                return Ok(ParentAuthority::Created {
                    created_by: *id,
                    path: encode_path(p),
                });
            }
            let (_, id) = map
                .iter()
                .rev()
                .find(|(path, _)| path == p)
                .ok_or_else(|| error("inverse parent lacks reviewed authority"))?;
            if fs.inspect(p)?.stamp.physical_id.as_ref() != Some(id) {
                return Err(error("inverse parent replaced"));
            }
            Ok(ParentAuthority::Observed(id.clone()))
        };
        let source_parent = parent(source)?;
        let target_parent = parent(target)?;
        if let ExecutionAction::CreateDirectory { target } = &step.action {
            future.insert(target.clone(), step.id);
        }
        let mut depends_on: Vec<_> = previous.into_iter().collect();
        for parent in [&source_parent, &target_parent] {
            if let ParentAuthority::Created { created_by, .. } = parent {
                if !depends_on.contains(created_by) {
                    depends_on.push(*created_by);
                }
            }
        }
        previous = Some(step.id);
        result.push(CompiledInverse {
            original,
            step,
            source_parent,
            target_parent,
            depends_on,
        });
    }
    Ok(result)
}
