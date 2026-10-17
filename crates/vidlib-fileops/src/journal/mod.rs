//! Durable local execution journal; no method performs media filesystem effects.
use crate::{events::*, platform::Durability, ErrorCode, OpError, Phase};
mod migrations;
mod recovery;
mod schema;
use rusqlite::{params, Connection, OpenFlags, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;
use vidlib_inventory::{ids::*, FileStamp, NativePathRecord};
use vidlib_rename::{EntryPrestate, ExecutionAction, ExpectedIdentity, PlanRef};
use vidlib_rename::{ExecutionPlan, ExecutionStep};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    Prepared,
    Running,
    Completed,
    Cancelled,
    Failed,
    RecoveryRequired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Applied exists only in executor memory. Without a committed receipt, a
/// restart retains IntentDurable/Unresolved and must reconcile the action gap.
pub enum StepState {
    Planned,
    IntentDurable,
    CompletionDurable,
    Compensated,
    Unresolved,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepReceipt {
    pub step_id: StepId,
    pub phase: CompletedPhase,
    pub before_path: Option<NativePathRecord>,
    pub after_path: Option<NativePathRecord>,
    pub before: Option<FileStamp>,
    pub after: Option<FileStamp>,
    pub digest: Option<ContentDigest>,
    pub observed_ns: i128,
    pub durability: Durability,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingStep {
    pub step: ExecutionStep,
    pub sequence: u64,
    pub state: StepState,
    pub receipts: Vec<StepReceipt>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingOperation {
    pub operation_id: OperationId,
    pub state: OperationState,
    pub steps: Vec<PendingStep>,
    /// Inspection-only snapshot. Recovery authority resides in compiled steps.
    pub inspection: serde_json::Value,
    pub groups: Vec<OperationGroup>,
    pub recovery_effects: Vec<RecoveryEffect>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryEffect {
    pub original_step: StepId,
    pub inverse: PendingStep,
    pub source_parent: vidlib_inventory::PhysicalId,
    pub target_parent: vidlib_inventory::PhysicalId,
}
pub type OperationRecord = PendingOperation;
pub use schema::StoredGroup as OperationGroup;
/// Construct and use on its owning blocking thread. SQLite readers are separate
/// connections; this handle is intentionally !Send and !Sync.
pub struct Journal {
    connection: Connection,
    path: std::path::PathBuf,
    poisoned: bool,
    thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Journal {
    pub(crate) fn database_path(&self) -> &Path {
        &self.path
    }
    pub fn open(path: &Path) -> Result<Self, OpError> {
        if path.as_os_str().is_empty() || path == Path::new(":memory:") {
            return Err(invalid("journal requires a real local database file"));
        }
        let mut connection = Connection::open(path).map_err(sql_error)?;
        // Inspect before WAL or migrations: unknown evidence stays untouched.
        let version = migrations::check(&connection)?;
        configure(&connection, true)?;
        if version < schema::VERSION {
            migrations::migrate(&mut connection)?;
        }
        Ok(Self {
            connection,
            path: path.to_path_buf(),
            poisoned: false,
            thread: std::marker::PhantomData,
        })
    }
    // Coordinator opens this separate read-only connection on the calling thread.
    pub(crate) fn coordination_status(
        path: &Path,
    ) -> Result<crate::coordinator::CoordinationStatus, OpError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(sql_error)?;
        if migrations::check(&connection)? != schema::VERSION {
            return Err(OpError::new(
                ErrorCode::Busy,
                Phase::Read,
                "operation store upgrade is pending",
            ));
        }
        configure(&connection, false)?;
        coordination_state(&connection)
    }
    pub(crate) fn change_coordination(
        &mut self,
        namespace: bool,
        policy: bool,
        interrupted: Option<bool>,
    ) -> Result<crate::coordinator::Epochs, OpError> {
        self.write(None, None, |tx| {
            let mut epochs = coordination_state(tx)?.epochs;
            if namespace { epochs.namespace = epochs.namespace.checked_add(1).ok_or_else(|| recovery("namespace epoch exhausted"))?; }
            if policy { epochs.policy = epochs.policy.checked_add(1).ok_or_else(|| recovery("policy epoch exhausted"))?; }
            tx.execute("UPDATE coordination SET namespace_epoch=?1,policy_epoch=?2,interrupted_write=COALESCE(?3,interrupted_write) WHERE singleton=1", params![epochs.namespace.to_string(), epochs.policy.to_string(), interrupted]).map_err(sql_error)?;
            Ok(epochs)
        })
    }
    pub fn begin(&mut self, plan: &ExecutionPlan) -> Result<OperationId, OpError> {
        schema::validate_plan(plan)?;
        let inspection = schema::inspection(plan)?;
        let groups: Vec<_> = plan
            .groups
            .iter()
            .map(|g| OperationGroup {
                id: g.id,
                depends_on: g.depends_on.clone(),
            })
            .collect();
        let op = OperationId::new();
        self.write(Some(op), None, |tx| {
            tx.execute("INSERT INTO operations(id,state,inspection,groups_json,approved_plan,inspection_version) VALUES(?1,'Prepared',?2,?3,?4,2)", params![op.to_string(), json(&inspection)?, json(&groups)?, json(&plan.provenance.approved_plan)?]).map_err(sql_error)?;
            tx.execute("INSERT INTO history_visibility(operation_id) VALUES(?1)", [op.to_string()]).map_err(sql_error)?;
            for (sequence, step) in plan.groups.iter().flat_map(|g| &g.steps).enumerate() {
                let sequence = i64::try_from(sequence).map_err(|_| invalid("too many steps"))?;
                tx.execute("INSERT INTO steps(operation_id,id,group_id,sequence,state,compiled) VALUES(?1,?2,?3,?4,'Planned',?5)", params![op.to_string(), step.id.to_string(), step.group_id.to_string(), sequence, json(&schema::StoredStep::from(step))?]).map_err(sql_error)?;
            }
            Ok(op)
        })
    }
    pub fn intent(&mut self, op: OperationId, step: &ExecutionStep) -> Result<(), OpError> {
        self.write(Some(op), Some(step.id), |tx| {
            let (state, _) = operation_header(tx, op)?;
            if !matches!(state, OperationState::Prepared | OperationState::Running) {
                return Err(recovery("operation is not ready for an intent"));
            }
            let next = load_next_step(tx, op)?;
            if next.step != *step || next.state != StepState::Planned {
                return Err(invalid("step is not the next immutable planned effect"));
            }
            let prerequisites = load_prerequisites(tx, op, step)?;
            validate_resolved_inputs(step, &prerequisites)?;
            tx.execute(
                "UPDATE steps SET state='IntentDurable' WHERE operation_id=?1 AND id=?2",
                params![op.to_string(), step.id.to_string()],
            )
            .map_err(sql_error)?;
            // The operation row owns the large inspection snapshot. Avoid
            // rebuilding it for an unchanged Running state on every effect.
            if state == OperationState::Prepared {
                tx.execute(
                    "UPDATE operations SET state='Running' WHERE id=?1",
                    [op.to_string()],
                )
                .map_err(sql_error)?;
            }
            Ok(())
        })
        .map_err(|mut e| {
            e.item_id = Some(step.entry_id);
            e
        })
    }
    pub fn complete(&mut self, op: OperationId, receipt: &StepReceipt) -> Result<(), OpError> {
        let mut item_id = None;
        let result = self.write(Some(op), Some(receipt.step_id), |tx| {
            let (state, approved_plan) = operation_header(tx, op)?;
            if !matches!(
                state,
                OperationState::Running | OperationState::RecoveryRequired
            ) {
                return Err(invalid("operation cannot record a completion"));
            }
            let pending = load_current_step(tx, op, receipt.step_id)?;
            item_id = Some(pending.step.entry_id);
            if !matches!(
                pending.state,
                StepState::IntentDurable | StepState::Unresolved
            ) {
                return Err(invalid("completion requires a durable unresolved intent"));
            }
            let prerequisites = load_prerequisites(tx, op, &pending.step)?;
            validate_receipt(&pending.step, receipt, &prerequisites)?;
            tx.execute(
                "INSERT INTO receipts(operation_id,step_id,payload) VALUES(?1,?2,?3)",
                params![
                    op.to_string(),
                    receipt.step_id.to_string(),
                    json(&schema::StoredReceipt(receipt.clone()))?
                ],
            )
            .map_err(sql_error)?;
            let event_id = EventId::new();
            tx.execute(
                "INSERT INTO operation_events(id,operation_id,step_id,payload) VALUES(?1,?2,?3,'')",
                params![
                    event_id.to_string(),
                    op.to_string(),
                    receipt.step_id.to_string()
                ],
            )
            .map_err(sql_error)?;
            let sequence = u64::try_from(tx.last_insert_rowid())
                .map_err(|_| recovery("event sequence overflow"))?;
            let event = OperationEventV1 {
                schema_version: 1,
                event_id,
                operation_id: op,
                group_id: pending.step.group_id,
                step_id: receipt.step_id,
                kind: event_kind(receipt.phase),
                phase: receipt.phase,
                before_path: receipt.before_path.clone(),
                after_path: receipt.after_path.clone(),
                before: receipt.before.clone(),
                after: receipt.after.clone(),
                digest: receipt.digest.clone(),
                observed_ns: receipt.observed_ns,
                origin: EventOrigin::App,
                journal_sequence: sequence,
                approved_plan,
            };
            tx.execute(
                "UPDATE operation_events SET payload=?1 WHERE id=?2",
                params![json(&event)?, event_id.to_string()],
            )
            .map_err(sql_error)?;
            let state = if matches!(
                receipt.phase,
                CompletedPhase::OwnedCopyRemoved | CompletedPhase::OwnedEmptyDirectoryRemoved
            ) {
                "Compensated"
            } else {
                "CompletionDurable"
            };
            tx.execute(
                "UPDATE steps SET state=?1 WHERE operation_id=?2 AND id=?3",
                params![state, op.to_string(), receipt.step_id.to_string()],
            )
            .map_err(sql_error)?;
            if receipt.durability != Durability::Synced {
                tx.execute(
                    "UPDATE operations SET state='RecoveryRequired' WHERE id=?1",
                    [op.to_string()],
                )
                .map_err(sql_error)?;
            }
            Ok(())
        });
        result.map_err(|mut e| {
            e.item_id = item_id;
            e
        })
    }
    pub fn pending(&self) -> Result<Vec<PendingOperation>, OpError> {
        let mut reader = self.reader()?;
        let tx = reader.transaction().map_err(sql_error)?;
        let ids = {
            let mut stmt = tx.prepare("SELECT id FROM operations WHERE state IN ('Prepared','Running','RecoveryRequired') ORDER BY rowid").map_err(sql_error)?;
            let ids = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?;
            ids
        };
        let records = ids
            .into_iter()
            .map(|id| load_operation(&tx, parse_id(&id)?))
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().map_err(sql_error)?;
        Ok(records)
    }
    pub fn operation(&self, op: OperationId) -> Result<OperationRecord, OpError> {
        let mut reader = self.reader()?;
        let tx = reader.transaction().map_err(sql_error)?;
        let result = load_operation(&tx, op)?;
        tx.commit().map_err(sql_error)?;
        Ok(result)
    }
    /// Unacknowledged facts in durable sequence order. Cursor 0 starts the outbox;
    /// acknowledgments suppress delivery without removing receipts/audit events.
    pub fn events_after(&self, cursor: u64, limit: usize) -> Result<EventPage, OpError> {
        if limit == 0 {
            return Err(invalid("event page limit must be positive"));
        }
        let cursor_sql = i64::try_from(cursor).map_err(|_| invalid("event cursor out of range"))?;
        let reader = self.reader()?;
        let mut stmt = reader.prepare("SELECT payload FROM operation_events e WHERE sequence>?1 AND NOT EXISTS(SELECT 1 FROM outbox_ack a WHERE a.event_id=e.id) ORDER BY sequence LIMIT ?2").map_err(sql_error)?;
        let values = stmt
            .query_map(params![cursor_sql, limit.min(200) as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        let items: Vec<OperationEventV1> = values
            .iter()
            .map(|v| from_json(v))
            .collect::<Result<_, _>>()?;
        let next_cursor = items.last().map_or(cursor, |e| e.journal_sequence);
        Ok(EventPage { items, next_cursor })
    }
    pub fn ack_events(&mut self, ids: &[EventId]) -> Result<(), OpError> {
        self.write(None, None, |tx| {
            for id in ids {
                let exists: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM operation_events WHERE id=?1)",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .map_err(sql_error)?;
                if !exists {
                    return Err(invalid("cannot acknowledge unknown event"));
                }
                tx.execute(
                    "INSERT OR IGNORE INTO outbox_ack(event_id) VALUES(?1)",
                    [id.to_string()],
                )
                .map_err(sql_error)?;
            }
            Ok(())
        })
    }
    pub fn set_state(&mut self, op: OperationId, state: OperationState) -> Result<(), OpError> {
        self.write(Some(op), None, |tx| {
            let (current_state, _) = operation_header(tx, op)?;
            let (unfinished, unresolved): (bool, bool) = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM steps WHERE operation_id=?1 AND state IN ('Planned','IntentDurable','Unresolved')), EXISTS(SELECT 1 FROM steps WHERE operation_id=?1 AND state IN ('Planned','IntentDurable','Unresolved') AND state!='Planned')",
                [op.to_string()], |r| Ok((r.get(0)?,r.get(1)?))
            ).map_err(sql_error)?;
            let transition = matches!(
                (current_state, state),
                (
                    OperationState::Prepared,
                    OperationState::Cancelled
                        | OperationState::Failed
                        | OperationState::RecoveryRequired
                ) | (
                    OperationState::Running,
                    OperationState::Completed
                        | OperationState::Cancelled
                        | OperationState::Failed
                        | OperationState::RecoveryRequired
                ) | (OperationState::RecoveryRequired, OperationState::Running)
            );
            if !transition
                || (unresolved && state != OperationState::RecoveryRequired)
                || (state == OperationState::Completed
                    && unfinished)
            {
                return Err(invalid("invalid operation state transition"));
            }
            tx.execute(
                "UPDATE operations SET state=?1 WHERE id=?2",
                params![state_name(state)?, op.to_string()],
            )
            .map_err(sql_error)?;
            Ok(())
        })
    }
    /// Persist an inconclusive reconciliation without granting permission to retry.
    pub fn mark_unresolved(&mut self, op: OperationId, step: StepId) -> Result<(), OpError> {
        self.write(Some(op), Some(step), |tx| {
            let current = load_current_step(tx, op, step)?;
            if !matches!(
                current.state,
                StepState::IntentDurable | StepState::Unresolved
            ) {
                return Err(invalid("only a durable action gap can become unresolved"));
            }
            tx.execute(
                "UPDATE steps SET state='Unresolved' WHERE operation_id=?1 AND id=?2",
                params![op.to_string(), step.to_string()],
            )
            .map_err(sql_error)?;
            tx.execute(
                "UPDATE operations SET state='RecoveryRequired' WHERE id=?1",
                [op.to_string()],
            )
            .map_err(sql_error)?;
            Ok(())
        })
    }
    fn reader(&self) -> Result<Connection, OpError> {
        let connection = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(sql_error)?;
        configure(&connection, false)?;
        Ok(connection)
    }
    fn write<T>(
        &mut self,
        op: Option<OperationId>,
        step: Option<StepId>,
        work: impl FnOnce(&Transaction<'_>) -> Result<T, OpError>,
    ) -> Result<T, OpError> {
        if self.poisoned {
            return Err(context(recovery("journal write failed; close this handle and reconcile pending operation before continuing"), op, step));
        }
        let result: Result<T, OpError> = (|| {
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(sql_error)?;
            let result = work(&tx)?;
            commit(tx)?;
            Ok(result)
        })();
        if let Err(error) = &result {
            if matches!(error.code, ErrorCode::Busy | ErrorCode::RecoveryRequired) {
                self.poisoned = true;
            }
        }
        result.map_err(|e| context(e, op, step))
    }
}
fn coordination_state(
    connection: &Connection,
) -> Result<crate::coordinator::CoordinationStatus, OpError> {
    let (namespace, policy, interrupted, pending): (String, String, bool, bool) = connection.query_row(
        "SELECT namespace_epoch,policy_epoch,interrupted_write,EXISTS(SELECT 1 FROM operations WHERE state IN ('Prepared','Running','RecoveryRequired')) FROM coordination WHERE singleton=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(sql_error)?;
    let parse = |value: String| -> Result<u64, OpError> {
        let epoch = value
            .parse::<u64>()
            .map_err(|_| recovery("invalid persisted epoch"))?;
        if epoch.to_string() != value {
            return Err(recovery("noncanonical persisted epoch"));
        }
        Ok(epoch)
    };
    Ok(crate::coordinator::CoordinationStatus {
        epochs: crate::coordinator::Epochs {
            namespace: parse(namespace)?,
            policy: parse(policy)?,
        },
        pending_journal: pending,
        interrupted_legacy_write: interrupted,
    })
}
fn configure(connection: &Connection, writer: bool) -> Result<(), OpError> {
    connection
        .busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(sql_error)?;
    if writer {
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(sql_error)?;
    }
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(sql_error)?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(sql_error)?;
    let mode: String = connection
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .map_err(sql_error)?;
    let sync: i64 = connection
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .map_err(sql_error)?;
    let fk: i64 = connection
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .map_err(sql_error)?;
    let busy: i64 = connection
        .pragma_query_value(None, "busy_timeout", |r| r.get(0))
        .map_err(sql_error)?;
    if mode != "wal" || sync != 2 || fk != 1 || busy != 5000 {
        return Err(recovery("required journal durability PRAGMAs unavailable"));
    }
    Ok(())
}
fn commit(tx: Transaction<'_>) -> Result<(), OpError> {
    #[cfg(test)]
    if FAIL_COMMIT.with(|flag| flag.replace(false)) {
        return Err(recovery("fixture commit failure"));
    }
    tx.commit().map_err(sql_error)
}
#[cfg(test)]
thread_local! { static FAIL_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// Bounded effect context. Large inspection snapshots are accessed only by
/// load_operation for explicit history/recovery reads.
struct CurrentStep {
    step: ExecutionStep,
    state: StepState,
}
type Prerequisites = std::collections::BTreeMap<StepId, StepReceipt>;
fn operation_header(
    connection: &Connection,
    op: OperationId,
) -> Result<(OperationState, PlanRef), OpError> {
    // Read the compact covering index rather than traversing overflow pages in
    // the operation row to reach a column after its large inspection payload.
    let (state, plan): (String, String) = connection
        .query_row(
            "SELECT state,approved_plan FROM operations INDEXED BY operation_effect_header WHERE id=?1",
            [op.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| missing_or_sql(e, "unknown operation"))?;
    Ok((from_json(&format!("\"{state}\""))?, from_json(&plan)?))
}
fn current_step(row: (String, String)) -> Result<CurrentStep, OpError> {
    Ok(CurrentStep {
        state: from_json(&format!("\"{}\"", row.0))?,
        step: from_json::<schema::StoredStep>(&row.1)?.into(),
    })
}
fn load_next_step(connection: &Connection, op: OperationId) -> Result<CurrentStep, OpError> {
    let row = connection.query_row("SELECT state,compiled FROM steps WHERE operation_id=?1 AND state IN ('Planned','IntentDurable','Unresolved') ORDER BY sequence LIMIT 1", [op.to_string()], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| missing_or_sql(e,"operation has no remaining steps"))?;
    current_step(row)
}
fn load_current_step(
    connection: &Connection,
    op: OperationId,
    step: StepId,
) -> Result<CurrentStep, OpError> {
    let row = connection
        .query_row(
            "SELECT state,compiled FROM steps WHERE operation_id=?1 AND id=?2",
            params![op.to_string(), step.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| missing_or_sql(e, "unknown step"))?;
    current_step(row)
}
fn missing_or_sql(error: rusqlite::Error, message: &str) -> OpError {
    if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
        invalid(message)
    } else {
        sql_error(error)
    }
}
fn load_prerequisites(
    connection: &Connection,
    op: OperationId,
    step: &ExecutionStep,
) -> Result<Prerequisites, OpError> {
    let mut ids = std::collections::BTreeSet::new();
    if let Some(EntryPrestate {
        identity: ExpectedIdentity::FromReceipt { step_id },
        ..
    }) = schema::source(&step.action)
    {
        ids.insert(*step_id);
    }
    if let Some(owned) = schema::owned(&step.action) {
        ids.insert(owned.created_by);
    }
    match &step.action {
        ExecutionAction::PublishCopy { verified_by, .. } => {
            ids.insert(*verified_by);
        }
        ExecutionAction::RemoveSource { published_by, .. } => {
            ids.insert(*published_by);
        }
        _ => {}
    }
    let mut prerequisites = Prerequisites::new();
    for id in ids {
        let payload: String = connection.query_row("SELECT r.payload FROM receipts r JOIN steps s ON s.operation_id=r.operation_id AND s.id=r.step_id WHERE r.operation_id=?1 AND r.step_id=?2 AND s.state IN ('CompletionDurable','Compensated')", params![op.to_string(),id.to_string()], |r| r.get(0)).map_err(|e| if matches!(e,rusqlite::Error::QueryReturnedNoRows) { recovery("required durable receipt is missing; reconcile by identity") } else { sql_error(e) })?;
        prerequisites.insert(id, from_json::<schema::StoredReceipt>(&payload)?.0);
    }
    Ok(prerequisites)
}
fn load_operation(connection: &Connection, op: OperationId) -> Result<OperationRecord, OpError> {
    let (state, inspection, groups): (String, String, String) = connection
        .query_row(
            "SELECT state,inspection,groups_json FROM operations WHERE id=?1",
            [op.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| {
            if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                invalid("unknown operation")
            } else {
                sql_error(e)
            }
        })?;
    let mut statement = connection
        .prepare(
            "SELECT sequence,state,compiled FROM steps WHERE operation_id=?1 ORDER BY sequence",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map([op.to_string()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    let mut steps = Vec::new();
    for (sequence, state, compiled) in rows {
        let step: ExecutionStep = from_json::<schema::StoredStep>(&compiled)?.into();
        let mut stmt = connection
            .prepare("SELECT payload FROM receipts WHERE operation_id=?1 AND step_id=?2")
            .map_err(sql_error)?;
        let receipts = stmt
            .query_map(params![op.to_string(), step.id.to_string()], |r| {
                r.get::<_, String>(0)
            })
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        let receipts = receipts
            .iter()
            .map(|r| from_json::<schema::StoredReceipt>(r).map(|s| s.0))
            .collect::<Result<_, _>>()?;
        steps.push(PendingStep {
            step,
            sequence: u64::try_from(sequence).map_err(|_| recovery("invalid step sequence"))?,
            state: from_json(&format!("\"{state}\""))?,
            receipts,
        });
    }
    let recovery_effects = recovery::load_effects(connection, op, &mut steps)?;
    Ok(PendingOperation {
        recovery_effects,
        operation_id: op,
        state: from_json(&format!("\"{state}\""))?,
        steps,
        inspection: from_json(&inspection)?,
        groups: from_json(&groups)?,
    })
}
fn state_name(state: OperationState) -> Result<String, OpError> {
    from_json_value(serde_json::to_value(state).map_err(|e| recovery(e.to_string()))?)
}
fn json(value: &impl Serialize) -> Result<String, OpError> {
    serde_json::to_string(value).map_err(|e| recovery(e.to_string()))
}
#[cfg(test)]
thread_local! { static DECODED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
fn from_json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, OpError> {
    #[cfg(test)]
    DECODED_BYTES.with(|count| count.set(count.get() + s.len()));
    serde_json::from_str(s).map_err(|e| recovery(format!("invalid journal record: {e}")))
}
fn from_json_value<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, OpError> {
    serde_json::from_value(v).map_err(|e| recovery(format!("invalid journal record: {e}")))
}
fn parse_id<T: std::str::FromStr>(s: &str) -> Result<T, OpError> {
    s.parse().map_err(|_| recovery("invalid journal ID"))
}
fn invalid(message: impl Into<String>) -> OpError {
    OpError::new(ErrorCode::StaleState, Phase::Verify, message)
}
fn recovery(message: impl Into<String>) -> OpError {
    OpError::new(ErrorCode::RecoveryRequired, Phase::Write, message)
}
fn context(mut error: OpError, op: Option<OperationId>, step: Option<StepId>) -> OpError {
    error.operation_id = op;
    error.step_id = step;
    error
}
fn sql_error(error: rusqlite::Error) -> OpError {
    let code = match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            ErrorCode::Busy
        }
        _ => ErrorCode::RecoveryRequired,
    };
    OpError::new(code, Phase::Write, error.to_string())
}
fn event_kind(phase: CompletedPhase) -> EventKind {
    match phase {
        CompletedPhase::Renamed => EventKind::Rename,
        CompletedPhase::DirectoryCreated => EventKind::Directory,
        CompletedPhase::CopyStaged | CompletedPhase::CopyPublished => EventKind::Copy,
        CompletedPhase::SourceRemoved => EventKind::Removal,
        CompletedPhase::MetadataSet => EventKind::Metadata,
        CompletedPhase::OwnedCopyRemoved | CompletedPhase::OwnedEmptyDirectoryRemoved => {
            EventKind::Compensation
        }
    }
}

fn earlier_receipt(record: &Prerequisites, id: StepId) -> Result<&StepReceipt, OpError> {
    record
        .get(&id)
        .ok_or_else(|| recovery("required durable receipt is missing; reconcile by identity"))
}
fn resolved_stamp<'a>(
    entry: &'a EntryPrestate,
    record: &'a Prerequisites,
) -> Result<&'a FileStamp, OpError> {
    let stamp = match &entry.identity {
        ExpectedIdentity::Observed(stamp) => stamp,
        ExpectedIdentity::FromReceipt { step_id } => {
            let fact = earlier_receipt(record, *step_id)?;
            if fact.after_path.as_ref() != Some(&vidlib_inventory::encode_path(&entry.path)) {
                return Err(recovery(
                    "receipt poststate path does not match compiled input",
                ));
            }
            fact.after
                .as_ref()
                .ok_or_else(|| recovery("receipt has no observed poststate"))?
        }
    };
    if stamp.physical_id.is_none() {
        return Err(recovery("physical identity is unknown"));
    }
    Ok(stamp)
}
fn validate_resolved_inputs(step: &ExecutionStep, record: &Prerequisites) -> Result<(), OpError> {
    if let Some(entry) = schema::source(&step.action) {
        resolved_stamp(entry, record)?;
    }
    if let Some(owned) = schema::owned(&step.action) {
        let created = earlier_receipt(record, owned.created_by)?;
        let identity = &resolved_stamp(&owned.entry, record)?.physical_id;
        if created.after.as_ref().and_then(|s| s.physical_id.as_ref()) != identity.as_ref() {
            return Err(recovery(
                "owned entry does not match its creation receipt identity",
            ));
        }
    }
    match &step.action {
        ExecutionAction::PublishCopy { verified_by, .. } => {
            let verified = earlier_receipt(record, *verified_by)?;
            if verified.phase != CompletedPhase::CopyStaged
                || !valid_digest(verified.digest.as_ref())
            {
                return Err(recovery("staging has no verified content receipt"));
            }
        }
        ExecutionAction::RemoveSource { published_by, .. } => {
            let published = earlier_receipt(record, *published_by)?;
            if published.phase != CompletedPhase::CopyPublished
                || !valid_digest(published.digest.as_ref())
            {
                return Err(recovery("source removal has no verified published copy"));
            }
        }
        _ => {}
    }
    Ok(())
}
fn valid_digest(digest: Option<&ContentDigest>) -> bool {
    digest.is_some_and(|d| !d.algorithm.is_empty() && !d.bytes.is_empty())
}
fn validate_receipt(
    step: &ExecutionStep,
    receipt: &StepReceipt,
    record: &Prerequisites,
) -> Result<(), OpError> {
    validate_resolved_inputs(step, record)?;
    if schema::phase(&step.action) != receipt.phase {
        return Err(invalid("receipt phase differs from compiled effect"));
    }
    let before = schema::source(&step.action);
    let after = schema::output(&step.action);
    if before.map(|e| vidlib_inventory::encode_path(&e.path)) != receipt.before_path
        || after.map(vidlib_inventory::encode_path) != receipt.after_path
        || before.is_some() != receipt.before.is_some()
        || after.is_some() != receipt.after.is_some()
    {
        return Err(invalid(
            "receipt paths/poststate do not describe the compiled effect",
        ));
    }
    if let (Some(entry), Some(observed)) = (before, &receipt.before) {
        let expected = resolved_stamp(entry, record)?;
        if observed.physical_id != expected.physical_id
            || (expected.size.is_some() && observed.size != expected.size)
            || (expected.modified_ns.is_some() && observed.modified_ns != expected.modified_ns)
        {
            return Err(invalid(
                "receipt prestate differs from expected identity/state",
            ));
        }
    }
    if let Some(observed) = &receipt.after {
        if observed.physical_id.is_none() {
            return Err(invalid("receipt lacks native poststate identity"));
        }
        if matches!(
            step.action,
            ExecutionAction::RenameEntry { .. }
                | ExecutionAction::PublishCopy { .. }
                | ExecutionAction::SetMetadata { .. }
        ) && receipt.before.as_ref().and_then(|s| s.physical_id.as_ref())
            != observed.physical_id.as_ref()
        {
            return Err(invalid(
                "receipt changed identity across an identity-preserving action",
            ));
        }
    }
    if matches!(
        receipt.phase,
        CompletedPhase::CopyStaged | CompletedPhase::CopyPublished
    ) && !valid_digest(receipt.digest.as_ref())
    {
        return Err(invalid("copy receipt requires a verified digest"));
    }
    if let ExecutionAction::PublishCopy { verified_by, .. } = step.action {
        if earlier_receipt(record, verified_by)?.digest != receipt.digest {
            return Err(invalid("published digest differs from verified staging"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // A missing connection-local setting on either writer reopen or fresh reader
    // would weaken the journal's durability/integrity guarantees.
    #[test]
    fn effective_pragmas_are_verified_on_reopened_writer_and_separate_reader() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.sqlite3");
        drop(Journal::open(&path).unwrap());
        let journal = Journal::open(&path).unwrap();
        for connection in [&journal.connection, &journal.reader().unwrap()] {
            assert_eq!(
                connection
                    .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
                    .unwrap(),
                "wal"
            );
            assert_eq!(
                connection
                    .pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0))
                    .unwrap(),
                2
            );
            assert_eq!(
                connection
                    .pragma_query_value(None, "foreign_keys", |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                connection
                    .pragma_query_value(None, "busy_timeout", |r| r.get::<_, i64>(0))
                    .unwrap(),
                5000
            );
        }
    }
    // A v2 upgrade must retain all prior pending/effect evidence and index contracts.
    #[test]
    fn v2_epoch_migration_preserves_pending_operations_and_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(schema::SQL).unwrap();
        connection.execute_batch(schema::V2_SQL).unwrap();
        for (version, sql) in [(1, schema::SQL), (2, schema::V2_SQL)] {
            connection
                .execute(
                    "INSERT INTO schema_migrations VALUES(?1,?2)",
                    params![version, format!("{:x}", md5::compute(sql))],
                )
                .unwrap();
        }
        connection.pragma_update(None, "user_version", 2).unwrap();
        connection.execute_batch("INSERT INTO operations VALUES('op','Running','inspection evidence','[]','approved reference');
            INSERT INTO steps VALUES('op','step','group',0,'IntentDurable','compiled evidence');
            INSERT INTO receipts VALUES('op','step','receipt evidence');
            INSERT INTO operation_events(id,operation_id,step_id,payload) VALUES('event','op','step','event evidence');").unwrap();
        drop(connection);
        let journal = Journal::open(&path).unwrap();
        assert_eq!(
            Journal::coordination_status(&path).unwrap(),
            crate::coordinator::CoordinationStatus {
                epochs: crate::coordinator::Epochs::default(),
                pending_journal: true,
                interrupted_legacy_write: false
            }
        );
        let db = &journal.connection;
        assert_eq!(
            db.query_row(
                "SELECT state || ':' || inspection FROM operations",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "Running:inspection evidence"
        );
        assert_eq!(
            db.query_row("SELECT state || ':' || compiled FROM steps", [], |r| r
                .get::<_, String>(
                0
            ))
            .unwrap(),
            "IntentDurable:compiled evidence"
        );
        assert_eq!(
            db.query_row("SELECT payload FROM receipts", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "receipt evidence"
        );
        assert_eq!(
            db.query_row("SELECT payload FROM operation_events", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "event evidence"
        );
        assert_eq!(db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('operation_effect_header','unfinished_steps','pending_operations')", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
    }

    #[test]
    fn failed_v2_epoch_migration_rolls_back_without_losing_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(schema::SQL).unwrap();
        connection.execute_batch(schema::V2_SQL).unwrap();
        for (version, sql) in [(1, schema::SQL), (2, schema::V2_SQL)] {
            connection
                .execute(
                    "INSERT INTO schema_migrations VALUES(?1,?2)",
                    params![version, format!("{:x}", md5::compute(sql))],
                )
                .unwrap();
        }
        connection.pragma_update(None, "user_version", 2).unwrap();
        connection.execute_batch("CREATE TABLE coordination(conflicting_evidence TEXT); INSERT INTO coordination VALUES('retain me');").unwrap();
        assert!(Journal::open(&path).is_err());
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM schema_migrations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT conflicting_evidence FROM coordination", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "retain me"
        );
    }

    // Private failpoint rollback must poison the writer; it cannot be selected
    // through a release build, public API or IPC.
    #[test]
    fn private_commit_failpoint_rolls_back_and_poison_stops_next_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.sqlite3");
        let mut journal = Journal::open(&path).unwrap();
        FAIL_COMMIT.with(|flag| flag.set(true));
        let result = journal.write(None, None, |tx| {
            tx.execute("INSERT INTO operations(id,state,inspection,groups_json) VALUES('fixture','Prepared','{}','[]')", []).map_err(sql_error)?;
            Ok(())
        });
        assert_eq!(result.unwrap_err().code, ErrorCode::RecoveryRequired);
        assert_eq!(
            journal
                .connection
                .query_row("SELECT count(*) FROM operations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            journal.write(None, None, |_| Ok(())).unwrap_err().code,
            ErrorCode::RecoveryRequired
        );
    }
    // Real persisted v1 evidence, including a large inspection-only snapshot.
    // It exercises migrations as well as the public intent/completion boundary.
    fn measured_execution(count: usize, padding: usize) -> (u64, usize, u64) {
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(schema::SQL).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations VALUES(1,?1)",
                [format!("{:x}", md5::compute(schema::SQL))],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        let op = OperationId::new();
        let group = GroupId::new();
        let approved = PlanRef {
            session_id: SessionId::new(),
            plan_id: PlanId::new(),
            revision: u64::MAX,
        };
        let inspection =
            serde_json::json!({"approved_plan":approved, "inventory_notes":"x".repeat(padding)});
        connection.execute("INSERT INTO operations(id,state,inspection,groups_json) VALUES(?1,'Prepared',?2,'[]')", params![op.to_string(),json(&inspection).unwrap()]).unwrap();
        let mut steps = Vec::new();
        for sequence in 0..count {
            let step = ExecutionStep {
                id: StepId::new(),
                entry_id: EntryId::new(),
                group_id: group,
                action: ExecutionAction::CreateDirectory {
                    target: dir.path().join(sequence.to_string()),
                },
            };
            connection.execute("INSERT INTO steps(operation_id,id,group_id,sequence,state,compiled) VALUES(?1,?2,?3,?4,'Planned',?5)", params![op.to_string(),step.id.to_string(),group.to_string(),sequence as i64,json(&schema::StoredStep::from(&step)).unwrap()]).unwrap();
            steps.push(step);
        }
        drop(connection);
        let mut journal = Journal::open(&path).unwrap();
        let work = Arc::new(AtomicU64::new(0));
        let counter = work.clone();
        journal
            .connection
            .progress_handler(
                1,
                Some(move || {
                    counter.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        let operation_writes = Arc::new(AtomicU64::new(0));
        let updates = operation_writes.clone();
        journal
            .connection
            .update_hook(Some(
                move |action: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                    if action == rusqlite::hooks::Action::SQLITE_UPDATE && table == "operations" {
                        updates.fetch_add(1, Ordering::Relaxed);
                    }
                },
            ))
            .unwrap();
        DECODED_BYTES.with(|bytes| bytes.set(0));
        for step in &steps {
            journal.intent(op, step).unwrap();
            let receipt = StepReceipt {
                step_id: step.id,
                phase: CompletedPhase::DirectoryCreated,
                before_path: None,
                after_path: schema::output(&step.action).map(vidlib_inventory::encode_path),
                before: None,
                after: Some(FileStamp {
                    physical_id: Some(vidlib_inventory::PhysicalId {
                        platform: if cfg!(windows) {
                            vidlib_inventory::PathPlatform::WindowsUtf16Le
                        } else {
                            vidlib_inventory::PathPlatform::UnixBytes
                        },
                        volume_identity: vec![1],
                        file_identity: step.id.to_string().into_bytes(),
                    }),
                    ..Default::default()
                }),
                digest: None,
                observed_ns: 42,
                durability: Durability::Synced,
            };
            journal.complete(op, &receipt).unwrap();
        }
        let work = work.load(Ordering::Relaxed);
        let bytes = DECODED_BYTES.with(|count| count.get());
        assert_eq!(journal.events_after(0, 200).unwrap().items.len(), count);
        assert_eq!(journal.operation(op).unwrap().inspection, inspection);
        (work, bytes, operation_writes.load(Ordering::Relaxed))
    }
    // Counting SQLite VM instructions catches N receipt SELECTs and unindexed
    // scans per effect without relying on disk or wall-clock speed.
    #[test]
    fn effect_work_growth_is_bounded_as_batch_size_rises() {
        let small = measured_execution(16, 0);
        let large = measured_execution(64, 0);
        eprintln!("16 steps: {small:?}; 64 steps: {large:?}");
        assert!(large.0 <= small.0 * 4 + 1024, "SQLite work grew faster than the 4x batch with index allowance: small={small:?}, large={large:?}");
        assert!(
            large.1 <= small.1 * 4 + 4096,
            "decoded JSON grew faster than batch size: small={small:?}, large={large:?}"
        );
    }
    // Inspection size must not affect per-effect JSON decoding. History still
    // reconstructs it explicitly after measurement.
    #[test]
    fn effect_reads_do_not_decode_inspection_snapshots() {
        let small = measured_execution(16, 0);
        let large = measured_execution(16, 262144);
        eprintln!("empty inspection: {small:?}; padded inspection: {large:?}");
        assert!(
            large.1 <= small.1 + 1024,
            "effect path decoded inspection-only payload: small={small:?}, large={large:?}"
        );
    }

    // An already-Running operation's large inspection row must not be rebuilt
    // by a redundant state UPDATE for every subsequent effect.
    #[test]
    fn effect_writes_operation_header_only_on_state_change() {
        let counts = measured_execution(16, 262144);
        assert_eq!(
            counts.2, 1,
            "only Prepared -> Running should update the operation row"
        );
    }
    // A malformed legacy reference must roll back both schema and extracted
    // provenance, retaining the original evidence for diagnosis/recovery.
    #[test]
    fn failed_v1_header_migration_retains_original_schema_and_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(schema::SQL).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations VALUES(1,?1)",
                [format!("{:x}", md5::compute(schema::SQL))],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection.execute("INSERT INTO operations(id,state,inspection,groups_json) VALUES('legacy','Prepared',?1,'[]')",[r#"{"approved_plan":{},"inventory_notes":"retain original evidence"}"#]).unwrap();
        drop(connection);
        assert!(Journal::open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM schema_migrations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(connection.query_row("SELECT count(*) FROM pragma_table_info('operations') WHERE name='approved_plan'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(
            connection
                .query_row(
                    "SELECT inspection FROM operations WHERE id='legacy'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            r#"{"approved_plan":{},"inventory_notes":"retain original evidence"}"#
        );
    }
}
