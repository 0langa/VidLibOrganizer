//! Private durable transitions used only after fresh recovery verification.
use super::*;
use vidlib_inventory::PhysicalId;

impl Journal {
    pub(crate) fn remember_restore(
        &mut self,
        op: OperationId,
        group: Option<GroupId>,
    ) -> Result<(), OpError> {
        self.write(Some(op), None, |tx| {
            if let Some(existing) = load_restore_context(tx, op)? {
                if existing.group_id != group {
                    return Err(invalid("Restore group authority cannot change"));
                }
                return Ok(());
            }
            if !matches!(
                operation_header(tx, op)?.0,
                OperationState::Prepared
                    | OperationState::Running
                    | OperationState::RecoveryRequired
            ) {
                return Err(invalid("terminal operation cannot start Restore"));
            }
            if let Some(group) = group {
                let belongs: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM steps WHERE operation_id=?1 AND group_id=?2)",
                        params![op.to_string(), group.to_string()],
                        |r| r.get(0),
                    )
                    .map_err(sql_error)?;
                if !belongs {
                    return Err(invalid("Restore group is not in this operation"));
                }
            }
            tx.execute(
                "INSERT INTO recovery_restore(operation_id,group_id) VALUES(?1,?2)",
                params![op.to_string(), group.map(|id| id.to_string())],
            )
            .map_err(sql_error)?;
            Ok(())
        })
    }

    pub(crate) fn durability_verified(
        &self,
        op: OperationId,
        step: StepId,
    ) -> Result<bool, OpError> {
        self.reader()?.query_row("SELECT EXISTS(SELECT 1 FROM recovery_audit WHERE operation_id=?1 AND step_id=?2 AND resolution='DurabilityVerified')",params![op.to_string(),step.to_string()],|r|r.get(0)).map_err(sql_error)
    }

    pub(crate) fn record_recovered_durability(
        &mut self,
        op: OperationId,
        verified: &StepReceipt,
    ) -> Result<(), OpError> {
        self.write(Some(op),Some(verified.step_id),|tx|{
            let payload:String=tx.query_row("SELECT payload FROM receipts WHERE operation_id=?1 AND step_id=?2",params![op.to_string(),verified.step_id.to_string()],|r|r.get(0)).map_err(sql_error)?;
            let original=from_json::<schema::StoredReceipt>(&payload)?.0;
            if original.after_path!=verified.after_path || original.after!=verified.after || verified.durability!=Durability::Synced {return Err(invalid("durability resolution differs from original effect"));}
            tx.execute("INSERT INTO recovery_audit(operation_id,step_id,resolution,evidence) VALUES(?1,?2,'DurabilityVerified',?3)",params![op.to_string(),verified.step_id.to_string(),json(&schema::StoredReceipt(verified.clone()))?]).map_err(sql_error)?;
            Ok(())
        })
    }

    pub(crate) fn rearm_inverse(&mut self, op: OperationId, step: StepId) -> Result<(), OpError> {
        self.write(Some(op),Some(step),|tx|{
            let linked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_effects WHERE operation_id=?1 AND inverse_step=?2)",params![op.to_string(),step.to_string()],|r|r.get(0)).map_err(sql_error)?;
            if !linked || load_current_step(tx,op,step)?.state!=StepState::Planned{return Err(invalid("inverse requires an audited non-application reset"));}
            tx.execute("UPDATE steps SET state='IntentDurable' WHERE operation_id=?1 AND id=?2",params![op.to_string(),step.to_string()]).map_err(sql_error)?;
            tx.execute("UPDATE operations SET state='RecoveryRequired' WHERE id=?1",[op.to_string()]).map_err(sql_error)?;
            Ok(())
        })
    }

    pub(crate) fn reset_non_application(
        &mut self,
        op: OperationId,
        step: StepId,
        evidence: &serde_json::Value,
    ) -> Result<(), OpError> {
        self.write(Some(op), Some(step), |tx| {
            let current = load_current_step(tx, op, step)?;
            if !matches!(current.state, StepState::IntentDurable | StepState::Unresolved) {
                return Err(invalid("only an unresolved durable intent can be reset"));
            }
            tx.execute("INSERT INTO recovery_audit(operation_id,step_id,resolution,evidence) VALUES(?1,?2,'NotApplied',?3)", params![op.to_string(),step.to_string(),json(evidence)?]).map_err(sql_error)?;
            tx.execute("UPDATE steps SET state='Planned' WHERE operation_id=?1 AND id=?2", params![op.to_string(),step.to_string()]).map_err(sql_error)?;
            tx.execute("UPDATE operations SET state='Running' WHERE id=?1", [op.to_string()]).map_err(sql_error)?;
            Ok(())
        })
    }
    pub(crate) fn recovery_intent(
        &mut self,
        op: OperationId,
        original: StepId,
        inverse: &ExecutionStep,
        parents: (&PhysicalId, &PhysicalId),
    ) -> Result<(), OpError> {
        self.write(Some(op), Some(inverse.id), |tx| {
            let current = load_current_step(tx, op, original)?;
            if current.state != StepState::CompletionDurable || current.step.group_id != inverse.group_id || current.step.entry_id != inverse.entry_id {
                return Err(invalid("inverse requires its original durable completion"));
            }
            if !matches!(operation_header(tx, op)?.0, OperationState::Running | OperationState::RecoveryRequired) { return Err(invalid("operation cannot compensate")); }
            let sequence: i64 = tx.query_row("SELECT coalesce(max(sequence),-1)+1 FROM steps WHERE operation_id=?1", [op.to_string()], |r| r.get(0)).map_err(sql_error)?;
            tx.execute("INSERT INTO steps(operation_id,id,group_id,sequence,state,compiled) VALUES(?1,?2,?3,?4,'IntentDurable',?5)", params![op.to_string(),inverse.id.to_string(),inverse.group_id.to_string(),sequence,json(&schema::StoredStep::from(inverse))?]).map_err(sql_error)?;
            tx.execute("INSERT INTO recovery_effects VALUES(?1,?2,?3,?4)",params![op.to_string(),original.to_string(),inverse.id.to_string(),json(&parents)?]).map_err(sql_error)?;
            tx.execute("UPDATE operations SET state='RecoveryRequired' WHERE id=?1", [op.to_string()]).map_err(sql_error)?;
            Ok(())
        })
    }
    pub(crate) fn finish_compensation(
        &mut self,
        op: OperationId,
        original: StepId,
        inverse: StepId,
    ) -> Result<(), OpError> {
        self.write(Some(op), Some(original), |tx| {
            let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_effects e JOIN steps s ON s.operation_id=e.operation_id AND s.id=e.inverse_step WHERE e.operation_id=?1 AND e.original_step=?2 AND e.inverse_step=?3 AND s.state='CompletionDurable')", params![op.to_string(), original.to_string(),inverse.to_string()], |r| r.get(0)).map_err(sql_error)?;
            if !valid { return Err(invalid("compensation requires a durable inverse receipt")); }
            tx.execute("UPDATE steps SET state='Compensated' WHERE operation_id=?1 AND id=?2", params![op.to_string(),original.to_string()]).map_err(sql_error)?;
            Ok(())
        })
    }
}
pub(super) fn load_effects(
    connection: &Connection,
    op: OperationId,
    steps: &mut Vec<PendingStep>,
) -> Result<Vec<RecoveryEffect>, OpError> {
    let mut stmt = connection.prepare("SELECT original_step,inverse_step,parents FROM recovery_effects WHERE operation_id=?1 ORDER BY rowid").map_err(sql_error)?;
    let rows = stmt
        .query_map([op.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    rows.into_iter()
        .map(|(original, inverse, parents)| {
            let id = parse_id::<StepId>(&inverse)?;
            let index = steps
                .iter()
                .position(|s| s.step.id == id)
                .ok_or_else(|| recovery("missing inverse effect"))?;
            let inverse = steps.remove(index);
            let (source_parent, target_parent) = from_json(&parents)?;
            Ok(RecoveryEffect {
                original_step: parse_id(&original)?,
                inverse,
                source_parent,
                target_parent,
            })
        })
        .collect()
}

pub(super) fn load_restore_context(
    connection: &Connection,
    op: OperationId,
) -> Result<Option<RestoreContext>, OpError> {
    use rusqlite::OptionalExtension;
    let group: Option<Option<String>> = connection
        .query_row(
            "SELECT group_id FROM recovery_restore WHERE operation_id=?1",
            [op.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    group
        .map(|group| {
            Ok(RestoreContext {
                group_id: group.as_deref().map(parse_id).transpose()?,
            })
        })
        .transpose()
}
