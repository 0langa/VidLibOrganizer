//! Compiled-step recovery. Inspection never grants mutation authority; reviewed
//! application rechecks durable state and fresh native evidence under a lease.
pub(crate) mod directory;
use crate::{
    coordinator::{Lease, LeaseKind},
    events::CompletedPhase,
    journal::{Journal, OperationState, PendingOperation, PendingStep, StepReceipt, StepState},
    platform::{Durability, RenameRequest, RenameReservation},
    rename::OperationReport,
    ErrorCode, Filesystem, OpError, Phase,
};
use std::{collections::BTreeSet, path::PathBuf};
use vidlib_inventory::{
    decode_path, encode_path, ids::*, FileStamp, JobControl, NativePathRecord, PhysicalId,
};
use vidlib_rename::{EntryPrestate, ExecutionAction, ExecutionStep, ExpectedIdentity};

pub(crate) mod evidence;
use evidence::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryChoice {
    Resume,
    Restore,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryDisposition {
    Planned,
    NotApplied,
    Applied,
    Completed,
    Compensated,
    Unresolved,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryAction {
    pub step_id: StepId,
    pub group_id: GroupId,
    pub phase: CompletedPhase,
    /// Exact compiled native paths for inspection. Restore reverses the current
    /// group's rename effects in reverse order; it never reruns a recipe.
    pub before_path: Option<NativePathRecord>,
    pub after_path: Option<NativePathRecord>,
    pub disposition: RecoveryDisposition,
}
pub struct RecoveryPreview {
    pub actions: Vec<RecoveryAction>,
    pub temporary_names: Vec<PathBuf>,
    pub unresolved_groups: Vec<GroupId>,
    record: PendingOperation,
    nonce: StepId,
}
/// Process-local, single-use reviewed authority. Native paths come only from the
/// private journal snapshot; public display fields cannot alter the effects.
pub struct ReviewedRecovery(RecoveryPreview);
impl RecoveryPreview {
    pub fn review_nonce(&self) -> StepId {
        self.nonce
    }
    pub fn review(self, nonce: StepId) -> Result<ReviewedRecovery, OpError> {
        if nonce != self.nonce {
            return Err(error("recovery review nonce does not match"));
        }
        Ok(ReviewedRecovery(self))
    }
}
fn error(message: &str) -> OpError {
    OpError::new(ErrorCode::RecoveryRequired, Phase::Verify, message)
}
pub fn reconcile(
    op: &PendingOperation,
    fs: &mut dyn Filesystem,
) -> Result<RecoveryPreview, OpError> {
    let mut actions = Vec::new();
    let mut unresolved = BTreeSet::new();
    let mut temporary_names = Vec::new();
    for pending in op
        .steps
        .iter()
        .chain(op.recovery_effects.iter().map(|e| &e.inverse))
    {
        let disposition = probe(op, pending, fs)
            .map(|r| r.0)
            .unwrap_or(RecoveryDisposition::Unresolved);
        if disposition == RecoveryDisposition::Unresolved {
            unresolved.insert(pending.step.group_id);
        }
        let (before, after) = paths(&pending.step.action);
        actions.push(RecoveryAction {
            step_id: pending.step.id,
            group_id: pending.step.group_id,
            phase: phase(&pending.step.action),
            before_path: before.map(|entry| encode_path(&entry.path)),
            after_path: after.map(encode_path),
            disposition,
        });
        let temporary = match &pending.step.action {
            ExecutionAction::RenameEntry {
                target,
                temporary: true,
                ..
            } => Some(target),
            ExecutionAction::StageCopy { staging, .. }
            | ExecutionAction::CreateStaging { staging } => Some(staging),
            _ => None,
        };
        if let Some(path) = temporary {
            if fs.inspect(path).is_ok() && !temporary_names.contains(path) {
                temporary_names.push(path.clone());
            }
        }
    }
    Ok(RecoveryPreview {
        actions,
        temporary_names,
        unresolved_groups: unresolved.into_iter().collect(),
        record: op.clone(),
        nonce: StepId::new(),
    })
}
fn sync_receipt(fs: &dyn Filesystem, r: &mut StepReceipt) -> Result<(), OpError> {
    if let (Some(path), Some(expected)) = (&r.after_path, &r.after) {
        let path = decode_path(path).map_err(|_| error("invalid observed poststate path"))?;
        let observation = fs.verify(&path, expected)?;
        if matches!(
            r.phase,
            CompletedPhase::MetadataSet
                | CompletedPhase::CopyStaged
                | CompletedPhase::CopyPublished
        ) {
            if observation.kind == vidlib_inventory::EntryKind::Directory {
                if fs.flush_directory(&path)? != Durability::Synced {
                    return Err(error("directory metadata durability unconfirmed"));
                }
            } else {
                fs.open_verified(&path, expected)?
                    .sync_all()
                    .map_err(|_| error("file content or metadata durability unconfirmed"))?;
            }
            fs.verify(&path, expected)?;
        }
    }
    for path in [&r.before_path, &r.after_path].into_iter().flatten() {
        let path = decode_path(path).map_err(|_| error("invalid receipt path"))?;
        if fs.flush_directory(
            path.parent()
                .ok_or_else(|| error("missing receipt parent"))?,
        )? != Durability::Synced
        {
            return Err(error("recovery filesystem durability unconfirmed"));
        }
    }
    r.durability = Durability::Synced;
    r.observed_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i128;
    Ok(())
}
fn resolve_gap(
    op: &PendingOperation,
    pending: &PendingStep,
    journal: &mut Journal,
    fs: &mut dyn Filesystem,
) -> Result<(), OpError> {
    let (disposition, fact) = probe(op, pending, fs)?;
    match disposition {
        RecoveryDisposition::Applied => {
            let mut r = fact.expect("observed fact");
            sync_receipt(fs, &mut r)?;
            journal.complete(op.operation_id, &r)
        }
        RecoveryDisposition::NotApplied => {
            let (source, target) = paths(&pending.step.action);
            let before = source.map(|s| stamp(op, s)).transpose()?;
            let (sp, tp) = parents(
                op,
                &pending.step,
                op.recovery_effects
                    .iter()
                    .any(|e| e.inverse.step.id == pending.step.id),
                fs,
            )?;
            let evidence = serde_json::json!({"source":source.map(|s| encode_path(&s.path)),"before":before,"target":target.map(encode_path),"target_absent":target.is_some_and(|t| absent(fs,t)),"source_parent":sp,"target_parent":tp});
            journal.reset_non_application(op.operation_id, pending.step.id, &evidence)
        }
        RecoveryDisposition::Planned
        | RecoveryDisposition::Completed
        | RecoveryDisposition::Compensated => Ok(()),
        RecoveryDisposition::Unresolved => Err(error("effect ownership remains unresolved")),
    }
}
fn rename_effect(
    op: &PendingOperation,
    step: &ExecutionStep,
    expected: &FileStamp,
    ids: (&PhysicalId, &PhysicalId),
    fs: &mut dyn Filesystem,
) -> Result<StepReceipt, OpError> {
    let ExecutionAction::RenameEntry { source, target, .. } = &step.action else {
        return Err(error("this recovery effect requires its phase executor"));
    };
    let before = fs.verify(&source.path, expected)?.stamp;
    let rs = reserved(op, &source.path);
    let rt = reserved(op, target);
    let result = if rs || rt {
        fs.rename_reserved(&RenameReservation {
            operation: op.operation_id,
            step: step.id,
            source: source.path.clone(),
            target: target.clone(),
            expected: expected.clone(),
            source_parent: ids.0.clone(),
            target_parent: ids.1.clone(),
            reserved_source: rs,
            reserved_target: rt,
        })?
    } else {
        fs.rename_reviewed(&RenameRequest {
            source: &source.path,
            target,
            expected,
            source_parent: ids.0,
            target_parent: ids.1,
        })?
    };
    Ok(StepReceipt {
        step_id: step.id,
        phase: CompletedPhase::Renamed,
        before_path: Some(encode_path(&source.path)),
        after_path: Some(encode_path(target)),
        before: Some(before),
        after: Some(result.observation.stamp),
        digest: None,
        observed_ns: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as i128,
        durability: result.durability,
    })
}
pub fn apply_recovery(
    preview: ReviewedRecovery,
    choice: RecoveryChoice,
    lease: &Lease,
    journal: &mut Journal,
    fs: &mut dyn Filesystem,
    control: &JobControl,
) -> Result<OperationReport, OpError> {
    if lease.kind() != LeaseKind::Recovery || journal.database_path() != lease.database_path() {
        return Err(OpError::new(
            ErrorCode::PermissionDenied,
            Phase::Verify,
            "matching exclusive Recovery lease required",
        ));
    }
    if lease.status()?.interrupted_legacy_write {
        return Err(error(
            "legacy interrupted write has no per-item recovery evidence",
        ));
    }
    let reviewed = preview.0;
    let current = journal.operation(reviewed.record.operation_id)?;
    if current != reviewed.record {
        return Err(error("journal changed after recovery review"));
    }
    let fresh = reconcile(&current, fs)?;
    if fresh.actions != reviewed.actions || !fresh.unresolved_groups.is_empty() {
        return Err(error(
            "reviewed recovery observations changed or remain unresolved",
        ));
    }
    recover(current.operation_id, choice, lease, journal, fs, control)
}
fn recover(
    op_id: OperationId,
    choice: RecoveryChoice,
    lease: &Lease,
    journal: &mut Journal,
    fs: &mut dyn Filesystem,
    control: &JobControl,
) -> Result<OperationReport, OpError> {
    let result = (|| {
        let op = journal.operation(op_id)?;
        // Persist the reviewed Restore scope before gap resolution can turn an
        // interrupted group into a wholly receipted group, or reset its intent.
        let restore_group = if choice == RecoveryChoice::Restore {
            let group = interrupted_group(&op, journal)?;
            journal.remember_restore(op_id, group)?;
            group
        } else {
            if op.restore_context.is_some() {
                return Err(error(
                    "Restore was already selected; Resume is no longer authorized",
                ));
            }
            None
        };
        for pending in op
            .steps
            .iter()
            .chain(op.recovery_effects.iter().map(|e| &e.inverse))
        {
            for fact in pending
                .receipts
                .iter()
                .filter(|r| r.durability != Durability::Synced)
            {
                if journal.durability_verified(op_id, fact.step_id)? {
                    continue;
                }
                parents(
                    &op,
                    &pending.step,
                    op.recovery_effects
                        .iter()
                        .any(|e| e.inverse.step.id == pending.step.id),
                    fs,
                )?;
                let mut verified = fact.clone();
                sync_receipt(fs, &mut verified)?;
                journal.record_recovered_durability(op_id, &verified)?;
            }
        }
        for pending in &op.steps {
            if matches!(
                pending.state,
                StepState::IntentDurable | StepState::Unresolved
            ) {
                resolve_gap(&op, pending, journal, fs)?;
            }
        }
        match choice {
            RecoveryChoice::Resume => resume(op_id, journal, fs, control)?,
            RecoveryChoice::Restore => restore(op_id, journal, fs, control, restore_group)?,
        }
        Ok(())
    })();
    // Even recovery of an action/receipt gap must revoke older namespace views.
    lease.advance_namespace()?;
    result?;
    // The pending operation must continue gating writers if epoch durability
    // fails, even when Restore performed no new filesystem action.
    if journal.operation(op_id)?.state == OperationState::RecoveryRequired {
        journal.set_state(op_id, OperationState::Running)?;
    }
    journal.set_state(
        op_id,
        match choice {
            RecoveryChoice::Resume => OperationState::Completed,
            RecoveryChoice::Restore => OperationState::Cancelled,
        },
    )?;
    let op = journal.operation(op_id)?;
    Ok(OperationReport {
        operation_id: Some(op_id),
        state: op.state,
        completed_groups: op
            .groups
            .iter()
            .filter(|g| {
                op.steps
                    .iter()
                    .filter(|s| s.step.group_id == g.id)
                    .all(|s| s.state == StepState::CompletionDurable)
            })
            .map(|g| g.id)
            .collect(),
        receipts: op
            .steps
            .iter()
            .chain(op.recovery_effects.iter().map(|e| &e.inverse))
            .flat_map(|s| s.receipts.clone())
            .collect(),
        failure: None,
    })
}
fn resume(
    op_id: OperationId,
    journal: &mut Journal,
    fs: &mut dyn Filesystem,
    control: &JobControl,
) -> Result<(), OpError> {
    loop {
        let op = journal.operation(op_id)?;
        if !op.recovery_effects.is_empty() {
            return Err(error(
                "restoration has begun; resume is no longer authorized",
            ));
        }
        if control.is_cancelled() {
            return Err(error("recovery cancelled before next effect"));
        }
        let Some(pending) = op.steps.iter().find(|s| s.state == StepState::Planned) else {
            break;
        };
        if !matches!(pending.step.action, ExecutionAction::RenameEntry { .. }) {
            if op.state == OperationState::RecoveryRequired {
                journal.set_state(op_id, OperationState::Running)?;
            }
            let before = crate::metadata::prepare(&op, &pending.step, fs)?;
            if let Some(before) = before {
                journal.intent_metadata(op_id, &pending.step, &before)?;
            } else {
                journal.intent(op_id, &pending.step)?;
            }
            let receipt =
                crate::copy::effect(&journal.operation(op_id)?, &pending.step, fs, control)?;
            journal.complete(op_id, &receipt)?;
            if receipt.durability != Durability::Synced {
                return Err(error("recovered copy durability unconfirmed"));
            }
            continue;
        }
        let (source, target) = paths(&pending.step.action);
        let (sp, tp) = parents(&op, &pending.step, false, fs)?;
        let source =
            source.ok_or_else(|| error("creation execution belongs to its phase owner"))?;
        let expected = material(&op, source, fs)?;
        fs.verify(&source.path, &expected)?;
        if !target.is_some_and(|t| absent(fs, t)) {
            return Err(error("compiled resume target is occupied or unavailable"));
        }
        if !matches!(pending.step.action, ExecutionAction::RenameEntry { .. }) {
            return Err(error(
                "phase execution requires its implementation; evidence retained",
            ));
        }
        if op.state == OperationState::RecoveryRequired {
            journal.set_state(op_id, OperationState::Running)?;
        }
        journal.intent(op_id, &pending.step)?;
        let receipt = rename_effect(&op, &pending.step, &expected, (&sp, &tp), fs)?;
        journal.complete(op_id, &receipt)?;
        if receipt.durability != Durability::Synced {
            return Err(error("recovered effect durability unconfirmed"));
        }
    }
    Ok(())
}
fn restore(
    op_id: OperationId,
    journal: &mut Journal,
    fs: &mut dyn Filesystem,
    control: &JobControl,
    current_group: Option<GroupId>,
) -> Result<(), OpError> {
    // Only the interrupted group is compensated; completed earlier groups are facts.
    let Some(group) = current_group else {
        return Ok(());
    };
    loop {
        if control.is_cancelled() {
            return Err(error("restoration cancelled before next inverse"));
        }
        let op = journal.operation(op_id)?;
        let active_effects: Vec<_> =
            op.recovery_effects
                .iter()
                .filter(|e| {
                    e.inverse.step.group_id == group
                        && op.steps.iter().any(|s| {
                            s.step.id == e.original_step && s.state != StepState::Compensated
                        })
                })
                .collect();
        if op.recovery_effects.is_empty()
            && op.steps.iter().any(|s| {
                s.step.group_id == group
                    && matches!(
                        s.step.action,
                        ExecutionAction::RemoveSourceDirectory { .. }
                            | ExecutionAction::RemoveOwnedEmptyDirectory { .. }
                    )
            })
        {
            let effects = directory::compile_group(&op, group, fs)?;
            if !effects.is_empty() {
                journal.recovery_group(op_id, group, &effects)?;
                continue;
            }
        }
        if let Some(effect) = active_effects
            .iter()
            .find(|e| {
                !matches!(
                    e.inverse.state,
                    StepState::CompletionDurable | StepState::Compensated
                ) && e.depends_on.iter().all(|id| {
                    op.recovery_effects.iter().any(|p| {
                        p.inverse.step.id == *id
                            && matches!(
                                p.inverse.state,
                                StepState::CompletionDurable | StepState::Compensated
                            )
                    })
                })
            })
            .or(active_effects.first())
        {
            if !matches!(
                effect.inverse.state,
                StepState::CompletionDurable | StepState::Compensated
            ) {
                let (disposition, fact) = if effect.inverse.state == StepState::Planned {
                    (RecoveryDisposition::NotApplied, None)
                } else {
                    probe(&op, &effect.inverse, fs)?
                };
                match disposition {
                    RecoveryDisposition::Applied => {
                        let mut fact = fact.expect("inverse fact");
                        sync_receipt(fs, &mut fact)?;
                        journal.complete(op_id, &fact)?;
                    }
                    RecoveryDisposition::NotApplied => {
                        if effect.inverse.state != StepState::Planned {
                            resolve_gap(&op, &effect.inverse, journal, fs)?;
                        }
                        let before = crate::metadata::prepare(&op, &effect.inverse.step, fs)?;
                        journal.rearm_inverse(op_id, effect.inverse.step.id, before.as_ref())?;
                        let r = if matches!(
                            effect.inverse.step.action,
                            ExecutionAction::RenameEntry { .. }
                        ) {
                            let (s, _) = paths(&effect.inverse.step.action);
                            let expected = material(&op, s.expect("inverse source"), fs)?;
                            let (sp, tp) = parents(&op, &effect.inverse.step, true, fs)?;
                            rename_effect(&op, &effect.inverse.step, &expected, (&sp, &tp), fs)?
                        } else {
                            crate::copy::effect(
                                &journal.operation(op_id)?,
                                &effect.inverse.step,
                                fs,
                                control,
                            )?
                        };
                        journal.complete(op_id, &r)?;
                        if r.durability != Durability::Synced {
                            return Err(error("inverse durability unconfirmed"));
                        }
                    }
                    _ => return Err(error("inverse effect cannot be safely reconciled")),
                }
            }
            let fresh = journal.operation(op_id)?;
            if fresh
                .recovery_effects
                .iter()
                .filter(|e| e.original_step == effect.original_step)
                .all(|e| {
                    matches!(
                        e.inverse.state,
                        StepState::CompletionDurable | StepState::Compensated
                    )
                })
            {
                journal.finish_compensation(op_id, effect.original_step, effect.inverse.step.id)?;
            }
            continue;
        }
        let Some(original) = op
            .steps
            .iter()
            .rev()
            .find(|s| s.step.group_id == group && s.state == StepState::CompletionDurable)
        else {
            break;
        };
        if let Some(evidence) = crate::copy::consumed_inverse(&op, original, fs)? {
            journal.compensate_consumed(op_id, original.step.id, evidence)?;
            continue;
        }
        if !matches!(original.step.action, ExecutionAction::RenameEntry { .. }) {
            let chain = crate::copy::restore_chain(&op, original, fs)?;
            let effects = chain
                .into_iter()
                .map(|inverse| {
                    let (sp, tp) = parents(&op, &inverse, true, fs)?;
                    Ok((inverse, sp, tp))
                })
                .collect::<Result<Vec<_>, OpError>>()?;
            journal.recovery_chain(op_id, original.step.id, &effects)?;
            continue;
        }
        let ExecutionAction::RenameEntry { source, target, .. } = &original.step.action else {
            return Err(error("phase has no verified inverse executor"));
        };
        let prior = original
            .receipts
            .last()
            .and_then(|r| r.after.clone())
            .ok_or_else(|| error("missing compensation receipt"))?;
        let expected = verified_material(&op, target, &prior, fs)?;
        if !absent(fs, &source.path) {
            return Err(error("restoration destination is occupied or unavailable"));
        }
        let inverse = ExecutionStep {
            id: StepId::new(),
            entry_id: original.step.entry_id,
            group_id: group,
            action: ExecutionAction::RenameEntry {
                source: EntryPrestate {
                    path: target.clone(),
                    identity: ExpectedIdentity::Observed(expected.clone()),
                },
                target: source.path.clone(),
                temporary: reserved(&op, &source.path),
            },
        };
        let (sp, tp) = parents(&op, &inverse, true, fs)?;
        let mut chain = vec![(inverse.clone(), sp.clone(), tp.clone())];
        if fs.inspect(target)?.kind == vidlib_inventory::EntryKind::Directory {
            let changes = crate::metadata::directory_restore_changes(&op, original.step.entry_id)?;
            let mut identity = ExpectedIdentity::FromReceipt {
                step_id: inverse.id,
            };
            for change in changes {
                let id = StepId::new();
                chain.push((
                    ExecutionStep {
                        id,
                        entry_id: original.step.entry_id,
                        group_id: group,
                        action: ExecutionAction::SetMetadata {
                            entry: EntryPrestate {
                                path: source.path.clone(),
                                identity,
                            },
                            changes: vec![change],
                        },
                    },
                    tp.clone(),
                    tp.clone(),
                ));
                identity = ExpectedIdentity::FromReceipt { step_id: id };
            }
        }
        journal.recovery_chain(op_id, original.step.id, &chain)?;
    }
    Ok(())
}

/// Runtime compensation uses only the current executor's ordinary lease. This
/// does not admit arbitrary recovery requests through the ordinary plan API.
pub(crate) fn compensate_current(
    op_id: OperationId,
    group: GroupId,
    lease: &Lease,
    journal: &mut Journal,
    fs: &mut dyn Filesystem,
) -> Result<(), OpError> {
    if lease.kind() != LeaseKind::Write || lease.database_path() != journal.database_path() {
        return Err(error("current execution lease required"));
    }
    let op = journal.operation(op_id)?;
    journal.remember_restore(op_id, Some(group))?;
    for pending in op.steps.iter().filter(|s| s.step.group_id == group) {
        if matches!(
            pending.state,
            StepState::IntentDurable | StepState::Unresolved
        ) {
            resolve_gap(&op, pending, journal, fs)?;
        }
    }
    restore(op_id, journal, fs, &JobControl::default(), Some(group))
}

/// Transitional legacy writes have no item-level paths, identities or receipts.
/// This inspection record deliberately has no destructive resolution method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyInterruption {
    pub unresolved: bool,
    pub explanation: &'static str,
}
pub fn inspect_legacy_interruption(lease: &Lease) -> Result<Option<LegacyInterruption>, OpError> {
    Ok(lease.status()?.interrupted_legacy_write.then_some(LegacyInterruption{
        unresolved:true,
        explanation:"Interrupted legacy write has no item-level identity or completion evidence; preserve the marker and inspect read-only.",
    }))
}

// A durable prior group is never compensation authority merely by recency.
// Existing inverse links provide compatible v5 continuation evidence. Otherwise
// the first unfinished/unconfirmed group is the only candidate; Planned alone
// grants no inverse and safely cancels the remaining unexecuted work.
fn interrupted_group(op: &PendingOperation, journal: &Journal) -> Result<Option<GroupId>, OpError> {
    if let Some(context) = &op.restore_context {
        return Ok(context.group_id);
    }
    if let Some(effect) = op.recovery_effects.first() {
        let group = effect.inverse.step.group_id;
        if op
            .recovery_effects
            .iter()
            .any(|e| e.inverse.step.group_id != group)
        {
            return Err(error("inverse evidence spans multiple recovery groups"));
        }
        return Ok(Some(group));
    }
    for group in &op.groups {
        for step in op.steps.iter().filter(|s| s.step.group_id == group.id) {
            if !matches!(
                step.state,
                StepState::CompletionDurable | StepState::Compensated
            ) {
                return Ok(Some(group.id));
            }
            for fact in &step.receipts {
                if fact.durability != Durability::Synced
                    && !journal.durability_verified(op.operation_id, fact.step_id)?
                {
                    return Ok(Some(group.id));
                }
            }
        }
    }
    Ok(None)
}
