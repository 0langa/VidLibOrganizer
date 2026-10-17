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
            let blocked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_effects current JOIN recovery_effects prior ON prior.operation_id=current.operation_id AND prior.original_step=current.original_step AND prior.ordinal<current.ordinal JOIN steps s ON s.operation_id=prior.operation_id AND s.id=prior.inverse_step WHERE current.operation_id=?1 AND current.inverse_step=?2 AND s.state NOT IN ('CompletionDurable','Compensated'))",params![op.to_string(),step.to_string()],|r|r.get(0)).map_err(sql_error)?;
            if blocked {return Err(invalid("earlier inverse effects are incomplete"));}
            let blocked: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_dependencies d JOIN steps s ON s.operation_id=d.operation_id AND s.id=d.prerequisite_step WHERE d.operation_id=?1 AND d.inverse_step=?2 AND s.state NOT IN ('CompletionDurable','Compensated'))", params![op.to_string(),step.to_string()], |r| r.get(0)).map_err(sql_error)?;
            if blocked { return Err(invalid("inverse dependency is incomplete")); }
            let current=load_current_step(tx,op,step)?;
            validate_resolved_inputs(&current.step,&load_prerequisites(tx,op,&current.step)?)?;
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
        self.recovery_chain(
            op,
            original,
            &[(inverse.clone(), parents.0.clone(), parents.1.clone())],
        )
    }
    pub(crate) fn recovery_chain(
        &mut self,
        op: OperationId,
        original: StepId,
        effects: &[(ExecutionStep, PhysicalId, PhysicalId)],
    ) -> Result<(), OpError> {
        self.write(Some(op), effects.first().map(|e| e.0.id), |tx| {
            let current = load_current_step(tx, op, original)?;
            if current.state != StepState::CompletionDurable || effects.is_empty() || effects.iter().any(|(s,_,_)| s.group_id != current.step.group_id || s.entry_id != current.step.entry_id) { return Err(invalid("inverse chain requires its original durable completion and group")); }
            if !matches!(operation_header(tx, op)?.0, OperationState::Running | OperationState::RecoveryRequired) { return Err(invalid("operation cannot compensate")); }
            let sequence: i64 = tx.query_row("SELECT coalesce(max(sequence),-1)+1 FROM steps WHERE operation_id=?1", [op.to_string()], |r| r.get(0)).map_err(sql_error)?;
            for (ordinal,(inverse,sp,tp)) in effects.iter().enumerate() {
                tx.execute("INSERT INTO steps(operation_id,id,group_id,sequence,state,compiled) VALUES(?1,?2,?3,?4,'Planned',?5)", params![op.to_string(),inverse.id.to_string(),inverse.group_id.to_string(),sequence+ordinal as i64,json(&schema::StoredStep::from(inverse))?]).map_err(sql_error)?;
                tx.execute("INSERT INTO recovery_effects VALUES(?1,?2,?3,?4,?5)",params![op.to_string(),original.to_string(),inverse.id.to_string(),json(&(sp,tp))?,ordinal as i64]).map_err(sql_error)?;
            }
            tx.execute("UPDATE operations SET state='RecoveryRequired' WHERE id=?1", [op.to_string()]).map_err(sql_error)?;
            Ok(())
        })
    }
    pub(crate) fn recovery_group(
        &mut self,
        op: OperationId,
        group: GroupId,
        effects: &[crate::recovery::directory::CompiledInverse],
    ) -> Result<(), OpError> {
        self.write(Some(op), None, |tx| {
            if load_restore_context(tx, op)?.and_then(|c| c.group_id) != Some(group) { return Err(invalid("inverse group is not the pinned Restore scope")); }
            let existing: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_effects e JOIN steps s ON s.operation_id=e.operation_id AND s.id=e.inverse_step WHERE e.operation_id=?1 AND s.group_id=?2)", params![op.to_string(),group.to_string()], |r| r.get(0)).map_err(sql_error)?;
            if existing { return Err(invalid("inverse group already has compiled effects")); }
            let sequence: i64 = tx.query_row("SELECT coalesce(max(sequence),-1)+1 FROM steps WHERE operation_id=?1", [op.to_string()], |r| r.get(0)).map_err(sql_error)?;
            let mut seen = std::collections::BTreeMap::new();
            let mut ordinals = std::collections::BTreeMap::<StepId, i64>::new();
            for (index, effect) in effects.iter().enumerate() {
                let original = load_current_step(tx,op,effect.original)?;
                if original.state != StepState::CompletionDurable || original.step.group_id != group || effect.step.group_id != group || effect.step.entry_id != original.step.entry_id || seen.contains_key(&effect.step.id) { return Err(invalid("inverse lineage differs from completed original group")); }
                for dependency in &effect.depends_on { if !seen.contains_key(dependency) { return Err(invalid("inverse graph is not closed and acyclic")); } }
                for parent in [&effect.source_parent, &effect.target_parent] {
                    if let ParentAuthority::Created { created_by, path } = parent {
                        let Some(previous): Option<&&ExecutionStep> = seen.get(created_by) else { return Err(invalid("future parent has no earlier inverse effect")); };
                        if !effect.depends_on.contains(created_by) || !matches!(&previous.action, ExecutionAction::CreateDirectory { target } if vidlib_inventory::encode_path(target) == *path) { return Err(invalid("future parent path, kind or dependency differs")); }
                    }
                }
                let ordinal = ordinals.entry(effect.original).or_default();
                tx.execute("INSERT INTO steps(operation_id,id,group_id,sequence,state,compiled) VALUES(?1,?2,?3,?4,'Planned',?5)", params![op.to_string(),effect.step.id.to_string(),group.to_string(),sequence+index as i64,json(&schema::StoredStep::from(&effect.step))?]).map_err(sql_error)?;
                tx.execute("INSERT INTO recovery_effects VALUES(?1,?2,?3,?4,?5)",params![op.to_string(),effect.original.to_string(),effect.step.id.to_string(),json(&(&effect.source_parent,&effect.target_parent))?,*ordinal]).map_err(sql_error)?;
                *ordinal += 1;
                for dep in &effect.depends_on { tx.execute("INSERT INTO recovery_dependencies VALUES(?1,?2,?3)", params![op.to_string(),effect.step.id.to_string(),dep.to_string()]).map_err(sql_error)?; }
                seen.insert(effect.step.id, &effect.step);
            }
            let canonical = effects.iter().map(|effect| canonical_effect(effect.original, &effect.step, &effect.source_parent, &effect.target_parent, &effect.depends_on)).collect::<Result<Vec<_>, _>>()?;
            tx.execute("INSERT INTO recovery_groups VALUES(?1,?2,?3)", params![op.to_string(),group.to_string(),json(&canonical)?]).map_err(sql_error)?;
            tx.execute("UPDATE operations SET state='RecoveryRequired' WHERE id=?1", [op.to_string()]).map_err(sql_error)?;
            Ok(())
        })
    }
    pub(crate) fn compensate_consumed(
        &mut self,
        op: OperationId,
        original: StepId,
        evidence: StepId,
    ) -> Result<(), OpError> {
        self.write(Some(op), Some(original), |tx| {
            let current=load_current_step(tx,op,original)?;
            let fact=load_current_step(tx,op,evidence)?;
            let linked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_effects WHERE operation_id=?1 AND inverse_step=?2)",params![op.to_string(),evidence.to_string()],|r|r.get(0)).map_err(sql_error)?;
            if current.state!=StepState::CompletionDurable || !linked || current.step.group_id!=fact.step.group_id || !matches!(fact.state,StepState::CompletionDurable|StepState::Compensated) {return Err(invalid("consumed phase requires durable same-group inverse evidence"));}
            tx.execute("INSERT INTO recovery_audit(operation_id,step_id,resolution,evidence) VALUES(?1,?2,'CompensatedBy',?3)",params![op.to_string(),original.to_string(),json(&evidence)?]).map_err(sql_error)?;
            tx.execute("UPDATE steps SET state='Compensated' WHERE operation_id=?1 AND id=?2",params![op.to_string(),original.to_string()]).map_err(sql_error)?;
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
            let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_effects e JOIN steps s ON s.operation_id=e.operation_id AND s.id=e.inverse_step WHERE e.operation_id=?1 AND e.original_step=?2 AND e.inverse_step=?3 AND s.state IN ('CompletionDurable','Compensated') AND NOT EXISTS(SELECT 1 FROM recovery_effects rest JOIN steps incomplete ON incomplete.operation_id=rest.operation_id AND incomplete.id=rest.inverse_step WHERE rest.operation_id=e.operation_id AND rest.original_step=e.original_step AND incomplete.state NOT IN ('CompletionDurable','Compensated')))", params![op.to_string(), original.to_string(),inverse.to_string()], |r| r.get(0)).map_err(sql_error)?;
            if !valid { return Err(invalid("compensation requires a durable inverse receipt")); }
            tx.execute("UPDATE steps SET state='Compensated' WHERE operation_id=?1 AND id=?2", params![op.to_string(),original.to_string()]).map_err(sql_error)?;
            Ok(())
        })
    }
}
// The full immutable compiled set accompanies individual mutable phase states.
// Compare exact native paths, original lineage, parent bindings and all required
// actions on reopen so missing rows cannot shorten an original's inverse plan.
fn canonical_effect(
    original: StepId,
    step: &ExecutionStep,
    source_parent: &ParentAuthority,
    target_parent: &ParentAuthority,
    depends_on: &[StepId],
) -> Result<serde_json::Value, OpError> {
    serde_json::to_value((
        original,
        schema::StoredStep::from(step),
        source_parent,
        target_parent,
        depends_on
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
    ))
    .map_err(|e| recovery(e.to_string()))
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
    let effects: Vec<RecoveryEffect> = rows.into_iter()
        .map(|(original, inverse, parents)| {
            let id = parse_id::<StepId>(&inverse)?;
            let index = steps
                .iter()
                .position(|s| s.step.id == id)
                .ok_or_else(|| recovery("missing inverse effect"))?;
            let inverse = steps.remove(index);
            let (source_parent, target_parent) = from_json(&parents)?;
            let depends_on = connection.prepare("SELECT prerequisite_step FROM recovery_dependencies WHERE operation_id=?1 AND inverse_step=?2 ORDER BY prerequisite_step").map_err(sql_error)?
                .query_map(params![op.to_string(),id.to_string()], |r| r.get::<_,String>(0)).map_err(sql_error)?.collect::<Result<Vec<_>, _>>().map_err(sql_error)?
                .iter().map(|id| parse_id(id)).collect::<Result<Vec<_>, _>>()?;
            Ok(RecoveryEffect {
                original_step: parse_id(&original)?,
                inverse,
                source_parent,
                target_parent,
                depends_on,
            })
        })
        .collect::<Result<_, OpError>>()?;
    let mut seen = std::collections::BTreeMap::<StepId, &RecoveryEffect>::new();
    let mut previous = std::collections::BTreeMap::<GroupId, StepId>::new();
    for effect in &effects {
        let original = steps
            .iter()
            .find(|s| s.step.id == effect.original_step)
            .ok_or_else(|| recovery("inverse original missing"))?;
        let group = effect.inverse.step.group_id;
        if original.step.group_id != group
            || original.step.entry_id != effect.inverse.step.entry_id
            || original.receipts.is_empty()
        {
            return Err(recovery("inverse original lineage differs"));
        }
        for dep in &effect.depends_on {
            if !seen.get(dep).is_some_and(|prior| {
                prior.inverse.step.group_id == group
                    && prior.inverse.sequence < effect.inverse.sequence
            }) {
                return Err(recovery("inverse dependency graph is cyclic or incomplete"));
            }
        }
        let compiled: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM recovery_groups WHERE operation_id=?1 AND group_id=?2)", params![op.to_string(),group.to_string()], |r| r.get(0)).map_err(sql_error)?;
        if !compiled
            && (matches!(effect.source_parent, ParentAuthority::Created { .. })
                || matches!(effect.target_parent, ParentAuthority::Created { .. })
                || crate::recovery::evidence::paths(&effect.inverse.step.action)
                    .0
                    .is_some_and(|entry| {
                        matches!(
                            entry.identity,
                            ExpectedIdentity::DirectoryAfterEffects { .. }
                        )
                    }))
        {
            return Err(recovery("inverse group manifest is missing"));
        }
        let mut expected: std::collections::BTreeSet<_> =
            previous.get(&group).copied().into_iter().collect();
        let (source, target) = crate::recovery::evidence::paths(&effect.inverse.step.action);
        let source = source
            .map(|e| e.path.as_path())
            .or(target)
            .ok_or_else(|| recovery("inverse path missing"))?;
        for (authority, child) in [
            (&effect.source_parent, source),
            (&effect.target_parent, target.unwrap_or(source)),
        ] {
            if let ParentAuthority::Created { created_by, path } = authority {
                let creator = seen
                    .get(created_by)
                    .ok_or_else(|| recovery("future parent creator missing"))?;
                if creator.inverse.step.group_id != group
                    || !effect.depends_on.contains(created_by)
                    || child.parent().map(vidlib_inventory::encode_path).as_ref() != Some(path)
                    || !matches!(&creator.inverse.step.action, ExecutionAction::CreateDirectory { target } if vidlib_inventory::encode_path(target) == *path)
                {
                    return Err(recovery("future parent lineage, path or kind differs"));
                }
                expected.insert(*created_by);
            }
        }
        if compiled
            && (load_restore_context(connection, op)?.and_then(|c| c.group_id) != Some(group)
                || expected != effect.depends_on.iter().copied().collect()
                || effect.depends_on.len() != expected.len())
        {
            return Err(recovery("canonical inverse group dependencies differ"));
        }
        previous.insert(group, effect.inverse.step.id);
        seen.insert(effect.inverse.step.id, effect);
    }
    let manifests = connection
        .prepare("SELECT group_id,canonical_effects FROM recovery_groups WHERE operation_id=?1")
        .map_err(sql_error)?
        .query_map([op.to_string()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    for (group, manifest) in manifests {
        let group = parse_id::<GroupId>(&group)?;
        let retained: Vec<serde_json::Value> = from_json(&manifest)?;
        let actual = effects
            .iter()
            .filter(|e| e.inverse.step.group_id == group)
            .map(|e| {
                canonical_effect(
                    e.original_step,
                    &e.inverse.step,
                    &e.source_parent,
                    &e.target_parent,
                    &e.depends_on,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if retained != actual {
            return Err(recovery("canonical inverse effect set differs"));
        }
    }
    Ok(effects)
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
