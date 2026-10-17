//! Exact, opt-in filesystem field effects and their receipt-derived inverses.
use crate::{
    events::CompletedPhase,
    journal::{PendingOperation, StepReceipt},
    platform::MetadataUpdate,
    ErrorCode, Filesystem, OpError, Phase,
};
use vidlib_inventory::{encode_path, FileStamp};
use vidlib_rename::*;
fn invalid(message: &str) -> OpError {
    OpError::new(ErrorCode::StaleState, Phase::Metadata, message)
}
pub(crate) fn poststate(
    before: &FileStamp,
    changes: &[MetadataChange],
) -> Result<FileStamp, OpError> {
    let mut after = before.clone();
    for change in changes {
        match change {
            MetadataChange::Timestamp { field, value_ns } => match field {
                TimestampField::Created => after.created_ns = Some(*value_ns),
                TimestampField::Modified => after.modified_ns = Some(*value_ns),
                TimestampField::Accessed => after.accessed_ns = Some(*value_ns),
            },
            MetadataChange::WindowsAttributes { set, clear } => {
                after.windows_attributes = Some(windows_attributes(
                    after
                        .windows_attributes
                        .ok_or_else(|| OpError::unsupported(Phase::Metadata))?,
                    *set,
                    *clear,
                ))
            }
            _ => return Err(invalid("unresolved metadata arithmetic in execution step")),
        }
    }
    Ok(after)
}
pub(crate) fn verify_fields(
    actual: &FileStamp,
    expected: &FileStamp,
    changes: &[MetadataChange],
) -> Result<(), OpError> {
    crate::platform::check_stamp(actual, expected, Phase::Metadata)?;
    if changes.iter().any(|c| {
        matches!(
            c,
            MetadataChange::Timestamp {
                field: TimestampField::Accessed,
                ..
            }
        )
    }) && actual.accessed_ns != expected.accessed_ns
    {
        return Err(invalid("explicit access timestamp poststate changed"));
    }
    Ok(())
}
pub(crate) fn effect(
    op: &PendingOperation,
    step: &ExecutionStep,
    entry: &EntryPrestate,
    changes: &[MetadataChange],
    fs: &mut dyn Filesystem,
) -> Result<StepReceipt, OpError> {
    let before = captured_prestate(op, step, entry)?;
    // Reprove dynamic directory authority after durable intent, immediately before effect.
    verify_directory_authority(op, entry, &before, fs)?;
    // A metadata action cannot silently adopt a changed explicitly addressed atime.
    let observed = fs.verify(&entry.path, &before)?.stamp;
    verify_fields(&observed, &before, changes)?;
    let desired = poststate(&before, changes)?;
    let mut update = MetadataUpdate {
        controlled_timestamps: controlled_fields(op, entry)?,
        ..Default::default()
    };
    crate::platform::check_timestamps(
        &observed,
        &before,
        &update.controlled_timestamps,
        Phase::Metadata,
    )?;
    for change in changes {
        match change {
            MetadataChange::Timestamp { field, value_ns } => match field {
                TimestampField::Created => update.created_ns = Some(*value_ns),
                TimestampField::Modified => update.modified_ns = Some(*value_ns),
                TimestampField::Accessed => update.accessed_ns = Some(*value_ns),
            },
            MetadataChange::WindowsAttributes { .. } => {
                update.windows_attributes = desired.windows_attributes
            }
            _ => return Err(invalid("metadata intent is not frozen")),
        }
    }
    let outcome = fs.set_metadata(&entry.path, &before, &update)?;
    verify_fields(&outcome.observation.stamp, &desired, changes).map_err(|_| {
        OpError::new(
            ErrorCode::Unsupported,
            Phase::Metadata,
            "filesystem did not retain the exact requested metadata; recovery required",
        )
    })?;
    Ok(StepReceipt {
        step_id: step.id,
        phase: CompletedPhase::MetadataSet,
        before_path: Some(encode_path(&entry.path)),
        after_path: Some(encode_path(&entry.path)),
        before: Some(before),
        after: Some(outcome.observation.stamp),
        digest: None,
        observed_ns: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as i128,
        durability: outcome.durability,
    })
}
pub(crate) fn validate_plan(plan: &RenamePlan) -> Result<(), OpError> {
    let p = &plan.execution.provenance;
    let mut rows = plan.rows.clone();
    if matches!(p.recipe.destination, Destination::SameDirectory) {
        for row in &mut rows {
            if let Some(ancestor) = plan
                .rows
                .iter()
                .filter(|a| {
                    a.entry_id != row.entry_id
                        && p.inventory.entries.iter().any(|e| {
                            e.id == a.entry_id && e.kind == vidlib_inventory::EntryKind::Directory
                        })
                        && row.target_path.starts_with(&a.target_path)
                })
                .max_by_key(|a| a.target_path.components().count())
            {
                row.target_path = ancestor.source_path.join(
                    row.target_path
                        .strip_prefix(&ancestor.target_path)
                        .map_err(|_| invalid("ancestor target differs"))?,
                );
            }
        }
    }
    let batch = EvaluatedBatch {
        inventory: p.inventory.clone(),
        recipe: p.recipe.clone(),
        inventory_id: plan.inventory_id,
        generation: plan.generation,
        recipe_revision: plan.recipe_revision,
        rows,
        frozen_inputs: p.frozen_inputs.clone(),
        grants: p.grants.clone(),
    };
    let canonical = vidlib_rename::plan(
        &p.inventory,
        &batch,
        &plan.execution.expected_namespace,
        plan.version,
    )
    .map_err(|e| invalid(&e.message))?;
    if canonical.execution.groups.len() != plan.execution.groups.len()
        || canonical
            .execution
            .groups
            .iter()
            .zip(&plan.execution.groups)
            .any(|(a, b)| a.id != b.id || a.depends_on != b.depends_on || a.steps != b.steps)
    {
        return Err(invalid(
            "metadata graph differs from immutable reviewed inputs",
        ));
    }
    Ok(())
}
/// Only fields that actually changed have inverses. Original facts stay immutable.
pub(crate) fn inverse_changes(
    fact: &StepReceipt,
    changes: &[MetadataChange],
) -> Result<Vec<MetadataChange>, OpError> {
    let before = fact
        .before
        .as_ref()
        .ok_or_else(|| invalid("metadata original state missing"))?;
    let after = fact
        .after
        .as_ref()
        .ok_or_else(|| invalid("metadata poststate missing"))?;
    let mut inverse = Vec::new();
    for change in changes {
        match change {
            MetadataChange::Timestamp { field, .. } => {
                let original = vidlib_rename::metadata::timestamp(before, *field);
                if original != vidlib_rename::metadata::timestamp(after, *field) {
                    inverse.push(MetadataChange::Timestamp {
                        field: *field,
                        value_ns: original
                            .ok_or_else(|| invalid("changed timestamp original unavailable"))?,
                    });
                }
            }
            MetadataChange::WindowsAttributes { set, clear } => {
                let before = before
                    .windows_attributes
                    .ok_or_else(|| invalid("original attributes missing"))?;
                let after = after
                    .windows_attributes
                    .ok_or_else(|| invalid("poststate attributes missing"))?;
                let changed = (before ^ after) & (set | clear);
                if changed != 0 {
                    inverse.push(MetadataChange::WindowsAttributes {
                        set: before & changed,
                        clear: after & changed,
                    });
                }
            }
            _ => return Err(invalid("historical metadata intent not resolved")),
        }
    }
    Ok(inverse)
}
/// Latest completed metadata fact for this exact native object and pathname.
pub(crate) fn latest_post<'a>(
    op: &'a PendingOperation,
    path: &std::path::Path,
    original: &'a FileStamp,
) -> &'a FileStamp {
    op.steps
        .iter()
        .chain(op.recovery_effects.iter().map(|e| &e.inverse))
        .filter(|s| !s.receipts.is_empty())
        .filter_map(|s| match &s.step.action {
            ExecutionAction::SetMetadata { entry, .. } if entry.path == path => s
                .receipts
                .last()
                .and_then(|r| r.after.as_ref())
                .filter(|after| after.physical_id == original.physical_id),
            _ => None,
        })
        .next_back()
        .unwrap_or(original)
}
pub(crate) fn restore_fields(
    op: &PendingOperation,
    path: &std::path::Path,
) -> Result<Vec<MetadataChange>, OpError> {
    let mut changes = Vec::new();
    for step in op
        .steps
        .iter()
        .rev()
        .filter(|s| s.state == crate::journal::StepState::CompletionDurable)
    {
        if let ExecutionAction::SetMetadata {
            entry,
            changes: requested,
        } = &step.step.action
        {
            if entry.path == path {
                changes.extend(inverse_changes(
                    step.receipts
                        .last()
                        .ok_or_else(|| invalid("metadata receipt missing"))?,
                    requested,
                )?);
            }
        }
    }
    Ok(changes)
}

fn pending<'a>(
    op: &'a PendingOperation,
    step: &ExecutionStep,
) -> Option<&'a crate::journal::PendingStep> {
    op.steps
        .iter()
        .chain(op.recovery_effects.iter().map(|e| &e.inverse))
        .find(|s| s.step.id == step.id)
}
pub(crate) fn captured_prestate(
    op: &PendingOperation,
    step: &ExecutionStep,
    entry: &EntryPrestate,
) -> Result<FileStamp, OpError> {
    if let Some(stamp) = pending(op, step).and_then(|p| p.metadata_prestate.clone()) {
        return Ok(stamp);
    }
    // Historical static facts retain their exact meaning. Dynamic directory gaps
    // without an intent-time observation cannot invent their original fields.
    if matches!(
        entry.identity,
        ExpectedIdentity::DirectoryAfterEffects { .. }
    ) {
        return Err(invalid(
            "legacy dynamic metadata intent has no provable factual prestate",
        ));
    }
    crate::recovery::evidence::stamp(op, entry)
}
pub(crate) fn prepare(
    op: &PendingOperation,
    step: &ExecutionStep,
    fs: &dyn Filesystem,
) -> Result<Option<FileStamp>, OpError> {
    let ExecutionAction::SetMetadata { entry, changes } = &step.action else {
        return Ok(None);
    };
    let material = crate::recovery::evidence::material(op, entry, fs)?;
    verify_directory_authority(op, entry, &material, fs)?;
    let actual = fs.verify(&entry.path, &material)?.stamp;
    crate::platform::check_timestamps(
        &actual,
        &material,
        &controlled_fields(op, entry)?,
        Phase::Metadata,
    )?;
    if let Some(before) = pending(op, step).and_then(|p| p.metadata_prestate.as_ref()) {
        verify_fields(&actual, before, changes)?;
        return Ok(Some(before.clone()));
    }
    verify_fields(&actual, &material, changes)?;
    Ok(Some(actual))
}

fn windows_attributes(before: u32, set: u32, clear: u32) -> u32 {
    let changed = (before | set) & !clear;
    if changed & !0x80 == 0 {
        0x80
    } else {
        changed & !0x80
    }
}

/// Ruling17: directory restoration targets are frozen original values; the
/// intent-time and receipt before-state remain distinct immutable facts.
pub(crate) fn original_directory_targets(
    op: &PendingOperation,
    entry: vidlib_inventory::ids::EntryId,
    changes: &mut [MetadataChange],
) -> Result<(), OpError> {
    let original = op.inspection["inventory"]["entries"]
        .as_array()
        .and_then(|entries| {
            entries.iter().find(|v| {
                serde_json::from_value::<vidlib_inventory::ids::EntryId>(v["id"].clone()).ok()
                    == Some(entry)
            })
        })
        .ok_or_else(|| invalid("original directory metadata missing"))?;
    let stamp: FileStamp = serde_json::from_value(original["stamp"].clone())
        .map_err(|_| invalid("invalid original directory metadata"))?;
    for change in changes {
        match change {
            MetadataChange::Timestamp { field, value_ns } => {
                *value_ns = vidlib_rename::metadata::timestamp(&stamp, *field)
                    .ok_or_else(|| invalid("original directory timestamp unavailable"))?
            }
            MetadataChange::WindowsAttributes { set, clear } => {
                let mask = *set | *clear;
                let bits = stamp
                    .windows_attributes
                    .ok_or_else(|| invalid("original directory attributes unavailable"))?;
                *set = bits & mask;
                *clear = !bits & mask;
            }
            _ => return Err(invalid("unresolved original directory intent")),
        }
    }
    Ok(())
}
pub(crate) fn directory_restore_changes(
    op: &PendingOperation,
    entry: vidlib_inventory::ids::EntryId,
) -> Result<Vec<MetadataChange>, OpError> {
    let mut out = Vec::new();
    for step in op
        .steps
        .iter()
        .rev()
        .filter(|s| s.step.entry_id == entry && !s.receipts.is_empty())
    {
        if let ExecutionAction::SetMetadata { changes, .. } = &step.step.action {
            out.extend(inverse_changes(
                step.receipts.last().expect("receipt"),
                changes,
            )?);
        }
    }
    original_directory_targets(op, entry, &mut out)?;
    Ok(out)
}

// Receipt-derived directory fields retain the exact predecessor identity. Their
// additional namespace proof names every durable fact without replacing that
// expected prestate with a fresh observation.
fn verify_directory_authority(
    op: &PendingOperation,
    entry: &EntryPrestate,
    expected: &FileStamp,
    fs: &dyn Filesystem,
) -> Result<(), OpError> {
    if matches!(
        entry.identity,
        ExpectedIdentity::DirectoryAfterEffects { .. }
    ) || fs.inspect(&entry.path)?.kind == vidlib_inventory::EntryKind::Directory
    {
        crate::recovery::evidence::material(op, &directory_proof(op, entry, expected), fs)?;
    }
    Ok(())
}
pub(crate) fn directory_proof(
    op: &PendingOperation,
    entry: &EntryPrestate,
    expected: &FileStamp,
) -> EntryPrestate {
    if matches!(
        entry.identity,
        ExpectedIdentity::DirectoryAfterEffects { .. }
    ) {
        return entry.clone();
    }
    EntryPrestate {
        path: entry.path.clone(),
        identity: ExpectedIdentity::DirectoryAfterEffects {
            original: expected.clone(),
            created_by: None,
            effects: op
                .steps
                .iter()
                .chain(op.recovery_effects.iter().map(|e| &e.inverse))
                .filter(|s| !s.receipts.is_empty())
                .map(|s| s.step.id)
                .collect(),
        },
    }
}

/// Field requirements follow exact durable receipt lineage. Named metadata
/// effects update values through their receipts; path effects do not drop them.
pub(crate) fn controlled_fields(
    op: &PendingOperation,
    entry: &EntryPrestate,
) -> Result<Vec<TimestampField>, OpError> {
    let mut fields = Vec::new();
    let mut current = &entry.identity;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        match current {
            ExpectedIdentity::MetadataChecked {
                fields: checked, ..
            } => {
                for field in checked {
                    if !fields.contains(field) {
                        fields.push(*field);
                    }
                }
                break;
            }
            ExpectedIdentity::FromReceipt { step_id } => {
                if !seen.insert(*step_id) {
                    return Err(invalid("cyclic metadata authority"));
                }
                let prior = op
                    .steps
                    .iter()
                    .chain(op.recovery_effects.iter().map(|e| &e.inverse))
                    .find(|s| s.step.id == *step_id)
                    .ok_or_else(|| invalid("metadata authority receipt missing"))?;
                if let ExecutionAction::SetMetadata { changes, .. } = &prior.step.action {
                    for change in changes {
                        if let MetadataChange::Timestamp { field, .. } = change {
                            if !fields.contains(field) {
                                fields.push(*field);
                            }
                        }
                    }
                }
                let Some(source) = crate::recovery::evidence::paths(&prior.step.action).0 else {
                    break;
                };
                current = &source.identity;
            }
            _ => break,
        }
    }
    Ok(fields)
}
pub(crate) fn checked_identity(stamp: FileStamp, fields: Vec<TimestampField>) -> ExpectedIdentity {
    if fields.is_empty() {
        ExpectedIdentity::Observed(stamp)
    } else {
        ExpectedIdentity::MetadataChecked { stamp, fields }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opted_in_flags_preserve_unselected_bits_and_normal_attribute_semantics() {
        let mut before = FileStamp {
            physical_id: None,
            size: None,
            created_ns: None,
            modified_ns: None,
            accessed_ns: None,
            windows_attributes: Some(0x400),
        };
        let set = poststate(
            &before,
            &[MetadataChange::WindowsAttributes {
                set: 1 | 2 | 4 | 32,
                clear: 0,
            }],
        )
        .unwrap();
        assert_eq!(set.windows_attributes, Some(0x427));
        let clear = poststate(
            &set,
            &[MetadataChange::WindowsAttributes {
                set: 0,
                clear: 1 | 2 | 4 | 32,
            }],
        )
        .unwrap();
        assert_eq!(clear.windows_attributes, Some(0x400));
        before.windows_attributes = Some(0x80);
        let hidden = poststate(
            &before,
            &[MetadataChange::WindowsAttributes { set: 2, clear: 0 }],
        )
        .unwrap();
        assert_eq!(hidden.windows_attributes, Some(2));
        assert_eq!(
            poststate(
                &hidden,
                &[MetadataChange::WindowsAttributes { set: 0, clear: 2 }]
            )
            .unwrap()
            .windows_attributes,
            Some(0x80)
        );
    }
}
