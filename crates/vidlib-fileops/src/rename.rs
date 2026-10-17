//! Executes an immutable reviewed in-place graph on the owning blocking worker.
use crate::{
    coordinator::{Epochs, Lease, LeaseKind},
    events::CompletedPhase,
    journal::{Journal, OperationState, StepReceipt},
    platform::Durability,
    ErrorCode, Filesystem, OpError, Phase,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use vidlib_inventory::{encode_path, ids::*, FileStamp, JobControl, PhysicalId};
use vidlib_rename::{ExecutionAction, ExpectedIdentity, RenamePlan};

/// Backend-only report: completed groups and receipts remain factual on partial failure.
#[derive(Debug)]
pub struct OperationReport {
    pub operation_id: Option<OperationId>,
    pub state: OperationState,
    pub completed_groups: Vec<GroupId>,
    pub receipts: Vec<StepReceipt>,
    pub failure: Option<OpError>,
}
fn stale(message: &str) -> OpError {
    OpError::new(ErrorCode::StaleState, Phase::Verify, message)
}

pub fn execute(
    plan: &RenamePlan,
    lease: &Lease,
    journal: &mut Journal,
    fs: &mut dyn Filesystem,
    control: &JobControl,
) -> Result<OperationReport, OpError> {
    if lease.kind() != LeaseKind::Write || journal.database_path() != lease.database_path() {
        return Err(OpError::new(
            ErrorCode::PermissionDenied,
            Phase::Verify,
            "matching ordinary write lease and journal required",
        ));
    }
    if lease.status()?.requires_recovery() {
        return Err(OpError::new(
            ErrorCode::RecoveryRequired,
            Phase::Verify,
            "pending operation requires recovery",
        ));
    }
    let provenance = &plan.execution.provenance;
    lease.verify_epochs(Epochs {
        namespace: provenance.namespace_epoch,
        policy: provenance.policy_epoch,
    })?;
    if plan.version != provenance.approved_plan
        || plan.inventory_id != provenance.inventory.id
        || plan.generation != provenance.inventory.generation
        || plan.recipe_revision != provenance.recipe.revision
        || plan.grants != provenance.grants
        || plan.frozen_inputs != provenance.frozen_inputs
        || plan.counts.blocked != 0
        || plan.counts.needs_review != 0
    {
        return Err(stale("plan authority snapshots disagree or require review"));
    }
    if plan
        .execution
        .groups
        .iter()
        .flat_map(|g| &g.steps)
        .any(|s| !matches!(s.action, ExecutionAction::RenameEntry { .. }))
    {
        return Err(OpError::new(
            ErrorCode::Unsupported,
            Phase::Verify,
            "only compiled in-place renames are implemented",
        ));
    }
    validate_sources(plan)?;
    // Reuse original representatives: a file grant authorizes capturing its own
    // parent, without granting sibling sources or requesting an ungranted target.
    let expected = &plan.execution.expected_namespace;
    let mut candidates = Vec::new();
    for parent in &expected.parents {
        for grant in &parent.source_grants {
            let authority = plan
                .grants
                .grants
                .iter()
                .find(|g| g.id == *grant)
                .ok_or_else(|| stale("unknown parent grant"))?;
            let representative = provenance
                .inventory
                .entries
                .iter()
                .map(|e| &e.absolute_path)
                .chain(parent.entries.iter().map(|e| &e.absolute_path))
                .find(|p| {
                    p.parent() == Some(parent.parent_path.as_path())
                        && p.starts_with(&authority.path)
                })
                .cloned()
                .unwrap_or_else(|| parent.parent_path.join(".vidlib-capture"));
            if !representative.starts_with(&authority.path) {
                return Err(stale("missing native parent capture authority"));
            }
            candidates.push(representative);
        }
    }
    let fresh = fs.capture_namespace(&plan.grants, &candidates)?;
    vidlib_rename::validate::preflight(&plan.execution, &fresh)
        .map_err(|e| stale(&e.to_string()))?;
    let op = journal.begin(&plan.execution)?;
    let mut report = OperationReport {
        operation_id: Some(op),
        state: OperationState::Running,
        completed_groups: vec![],
        receipts: vec![],
        failure: None,
    };
    let mut receipts: BTreeMap<StepId, StepReceipt> = BTreeMap::new();
    let mut parents: Vec<(PathBuf, FileStamp)> = expected
        .parents
        .iter()
        .map(|p| (p.parent_path.clone(), p.stamp.clone()))
        .collect();
    let mut reservations: BTreeMap<PathBuf, (StepId, EntryId)> = BTreeMap::new();
    let mut attempted = false;
    let mut journal_failure = None;
    let mut current_group = None;
    let mut cancelled_partial = false;
    let result = (|| -> Result<(), OpError> {
        for group in &plan.execution.groups {
            if control.is_cancelled() {
                report.state = OperationState::Cancelled;
                break;
            }
            current_group = Some(group.id);
            for step in &group.steps {
                if control.is_cancelled() {
                    cancelled_partial = true;
                    report.state = OperationState::RecoveryRequired;
                    return Ok(());
                }
                let ExecutionAction::RenameEntry {
                    source,
                    target,
                    temporary,
                } = &step.action
                else {
                    unreachable!("validated action")
                };
                let effect = (|| -> Result<StepReceipt, OpError> {
                    let stamp = match &source.identity {
                        ExpectedIdentity::Observed(stamp) => stamp,
                        ExpectedIdentity::FromReceipt { step_id } => receipts
                            .get(step_id)
                            .and_then(|r| r.after.as_ref())
                            .ok_or_else(|| stale("missing durable source receipt"))?,
                    };
                    let reviewed_parent = |path: Option<&Path>| -> Result<PhysicalId, OpError> {
                        let path = path.ok_or_else(|| stale("missing parent path"))?;
                        let (_, expected) = parents
                            .iter()
                            .find(|(p, _)| p == path)
                            .ok_or_else(|| stale("missing original parent identity"))?;
                        let identity = expected
                            .physical_id
                            .as_ref()
                            .ok_or_else(|| stale("missing original parent identity"))?;
                        let observed = fs.inspect(path)?;
                        if observed.stamp.physical_id.as_ref() != Some(identity) {
                            return Err(stale("ancestor identity changed"));
                        }
                        Ok(identity.clone())
                    };
                    let source_parent = reviewed_parent(source.path.parent())?;
                    let target_parent = reviewed_parent(target.parent())?;
                    let before = fs.verify(&source.path, stamp)?;
                    let reserved_source =
                        reservations
                            .get(&source.path)
                            .is_some_and(|(creator, entry)| {
                                *entry == step.entry_id
                                    && source.identity
                                        == ExpectedIdentity::FromReceipt { step_id: *creator }
                            });
                    if expected.policy.excludes(&source.path) && !reserved_source {
                        return Err(stale("temporary source lacks durable reservation lineage"));
                    }
                    if *temporary && !expected.policy.allows_rename_reservation(target) {
                        return Err(stale("invalid compiled temporary reservation"));
                    }
                    journal.intent(op, step)?;
                    attempted = true;
                    let action = if *temporary || reserved_source {
                        fs.rename_reserved(&crate::platform::RenameReservation {
                            operation: op,
                            step: step.id,
                            source: source.path.clone(),
                            target: target.clone(),
                            expected: stamp.clone(),
                            source_parent,
                            target_parent,
                            reserved_source,
                            reserved_target: *temporary,
                        })
                    } else {
                        fs.rename_reviewed(&crate::platform::RenameRequest {
                            source: &source.path,
                            target,
                            expected: stamp,
                            source_parent: &source_parent,
                            target_parent: &target_parent,
                        })
                    };
                    let outcome = match action {
                        Ok(outcome) => outcome,
                        Err(error) => {
                            journal.mark_unresolved(op, step.id)?;
                            return Err(error);
                        }
                    };
                    let receipt = StepReceipt {
                        step_id: step.id,
                        phase: CompletedPhase::Renamed,
                        before_path: Some(encode_path(&source.path)),
                        after_path: Some(encode_path(target)),
                        before: Some(before.stamp),
                        after: Some(outcome.observation.stamp),
                        digest: None,
                        observed_ns: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos() as i128,
                        durability: outcome.durability,
                    };
                    journal.complete(op, &receipt)?;
                    Ok(receipt)
                })();
                match effect {
                    Ok(receipt) => {
                        for (path, _) in &mut parents {
                            if let Ok(relative) = path.strip_prefix(&source.path) {
                                *path = target.join(relative);
                            }
                        }
                        if *temporary {
                            reservations.insert(target.clone(), (step.id, step.entry_id));
                        }
                        reservations.remove(&source.path);
                        receipts.insert(step.id, receipt.clone());
                        report.receipts.push(receipt.clone());
                        if receipt.durability != Durability::Synced {
                            report.state = OperationState::RecoveryRequired;
                            let mut error = OpError::new(
                                ErrorCode::RecoveryRequired,
                                Phase::Flush,
                                "rename observed but filesystem durability unconfirmed",
                            );
                            error.operation_id = Some(op);
                            error.step_id = Some(step.id);
                            error.item_id = Some(step.entry_id);
                            report.failure = Some(error);
                            return Ok(());
                        }
                    }
                    Err(mut error) => {
                        if error.operation_id == Some(op) {
                            journal_failure = Some(error.clone());
                        }
                        error.operation_id = Some(op);
                        error.step_id = Some(step.id);
                        error.item_id = Some(step.entry_id);
                        report.state = if attempted {
                            OperationState::RecoveryRequired
                        } else {
                            OperationState::Failed
                        };
                        report.failure = Some(error);
                        return Ok(());
                    }
                }
            }
            report.completed_groups.push(group.id);
        }
        if report.state == OperationState::Running {
            report.state = OperationState::Completed;
        }
        Ok(())
    })();
    // Failed/cancelled current groups are compensated only through fresh compiled
    // ownership evidence. Journal/flush failures keep their original recovery gap.
    if journal_failure.is_none()
        && report.state == OperationState::RecoveryRequired
        && (cancelled_partial
            || report
                .failure
                .as_ref()
                .is_some_and(|e| e.phase != Phase::Flush))
    {
        if let Some(group) = current_group {
            match crate::recovery::compensate_current(
                op,
                group,
                lease,
                journal,
                fs,
                cancelled_partial,
            ) {
                Ok(()) => {
                    report.state = if cancelled_partial {
                        OperationState::Cancelled
                    } else {
                        OperationState::Failed
                    };
                    let record = journal.operation(op)?;
                    report.receipts = record
                        .steps
                        .iter()
                        .chain(record.recovery_effects.iter().map(|e| &e.inverse))
                        .flat_map(|s| s.receipts.clone())
                        .collect();
                }
                Err(error) if error.operation_id == Some(op) => journal_failure = Some(error),
                Err(_) => {} // uncertain ownership retains the recovery gate
            }
        }
    }
    // Advance while the pending journal still gates other processes. If either
    // finalization fails the operation remains pending, never falsely completed.
    if attempted {
        lease.advance_namespace().map_err(|mut e| {
            e.operation_id = Some(op);
            e
        })?;
    }
    result?;
    if let Some(error) = journal_failure {
        return Err(error);
    }
    if journal.operation(op)?.state != report.state {
        journal.set_state(op, report.state)?;
    }
    Ok(report)
}

// Backend plans are not imported authority. Still verify duplicated row/source
// ownership at this boundary before a forged/stale compiled effect can act.
fn validate_sources(plan: &RenamePlan) -> Result<(), OpError> {
    let inventory = &plan.execution.provenance.inventory;
    let mut locations: BTreeMap<_, _> = inventory
        .entries
        .iter()
        .map(|e| (e.id, e.absolute_path.clone()))
        .collect();
    for row in &plan.rows {
        let entry = inventory
            .entries
            .iter()
            .find(|e| e.id == row.entry_id)
            .ok_or_else(|| stale("row source missing from inventory"))?;
        if entry.absolute_path != row.source_path
            || !plan
                .grants
                .grants
                .iter()
                .any(|g| entry.absolute_path.starts_with(&g.path))
        {
            return Err(stale("row source is outside its native grant"));
        }
    }
    let mut last_steps: BTreeMap<EntryId, (StepId, PathBuf)> = BTreeMap::new();
    for step in plan.execution.groups.iter().flat_map(|g| &g.steps) {
        let row = plan
            .rows
            .iter()
            .find(|r| r.entry_id == step.entry_id && r.status == vidlib_rename::RowStatus::Changed)
            .ok_or_else(|| stale("compiled effect is not a changed selected row"))?;
        let ExecutionAction::RenameEntry {
            source,
            target,
            temporary,
        } = &step.action
        else {
            return Err(stale("unsupported effect"));
        };
        if locations.get(&step.entry_id) != Some(&source.path)
            || (!temporary && target != &row.target_path)
        {
            return Err(stale(
                "compiled paths differ from selected source or reviewed target",
            ));
        }
        match (&source.identity, last_steps.get(&step.entry_id)) {
            (ExpectedIdentity::Observed(stamp), None)
                if inventory
                    .entries
                    .iter()
                    .any(|e| e.id == step.entry_id && e.stamp == *stamp) => {}
            (ExpectedIdentity::FromReceipt { step_id }, Some((previous, path)))
                if step_id == previous && encode_path(path) == encode_path(&source.path) => {}
            _ => {
                return Err(stale(
                    "compiled identity differs from inventory or prior receipt",
                ))
            }
        }
        for location in locations.values_mut() {
            if let Ok(relative) = location.strip_prefix(&source.path) {
                *location = if relative.as_os_str().is_empty() {
                    target.clone()
                } else {
                    target.join(relative)
                };
            }
        }
        last_steps.insert(step.entry_id, (step.id, target.clone()));
    }
    for row in plan
        .rows
        .iter()
        .filter(|r| r.status == vidlib_rename::RowStatus::Changed)
    {
        if locations.get(&row.entry_id) != Some(&row.target_path) {
            return Err(stale("compiled graph does not reach reviewed target"));
        }
    }
    Ok(())
}
