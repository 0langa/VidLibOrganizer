//! Durable local execution journal; no method performs media filesystem effects.
use crate::{events::*, platform::Durability, ErrorCode, OpError, Phase};
mod migrations;
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
#[derive(Clone, Debug)]
pub struct PendingStep {
    pub step: ExecutionStep,
    pub sequence: u64,
    pub state: StepState,
    pub receipts: Vec<StepReceipt>,
}
#[derive(Clone, Debug)]
pub struct PendingOperation {
    pub operation_id: OperationId,
    pub state: OperationState,
    pub steps: Vec<PendingStep>,
    /// Inspection-only snapshot. Recovery authority resides in compiled steps.
    pub inspection: serde_json::Value,
    pub groups: Vec<OperationGroup>,
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
    pub fn open(path: &Path) -> Result<Self, OpError> {
        if path.as_os_str().is_empty() || path == Path::new(":memory:") {
            return Err(invalid("journal requires a real local database file"));
        }
        let mut connection = Connection::open(path).map_err(sql_error)?;
        // Inspect before WAL or migrations: unknown evidence stays untouched.
        let fresh = migrations::check(&connection)?;
        configure(&connection, true)?;
        if fresh {
            migrations::migrate(&mut connection)?;
        }
        Ok(Self {
            connection,
            path: path.to_path_buf(),
            poisoned: false,
            thread: std::marker::PhantomData,
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
            tx.execute("INSERT INTO operations(id,state,inspection,groups_json) VALUES(?1,'Prepared',?2,?3)", params![op.to_string(), json(&inspection)?, json(&groups)?]).map_err(sql_error)?;
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
            let record = load_operation(tx, op)?;
            if !matches!(
                record.state,
                OperationState::Prepared | OperationState::Running
            ) {
                return Err(recovery("operation is not ready for an intent"));
            }
            let next = record
                .steps
                .iter()
                .find(|s| !finished(s.state))
                .ok_or_else(|| invalid("operation has no remaining steps"))?;
            if next.step != *step || next.state != StepState::Planned {
                return Err(invalid("step is not the next immutable planned effect"));
            }
            validate_resolved_inputs(step, &record)?;
            tx.execute(
                "UPDATE steps SET state='IntentDurable' WHERE operation_id=?1 AND id=?2",
                params![op.to_string(), step.id.to_string()],
            )
            .map_err(sql_error)?;
            tx.execute(
                "UPDATE operations SET state='Running' WHERE id=?1",
                [op.to_string()],
            )
            .map_err(sql_error)?;
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
            let record = load_operation(tx, op)?;
            if !matches!(
                record.state,
                OperationState::Running | OperationState::RecoveryRequired
            ) {
                return Err(invalid("operation cannot record a completion"));
            }
            let pending = record
                .steps
                .iter()
                .find(|s| s.step.id == receipt.step_id)
                .ok_or_else(|| invalid("unknown step"))?;
            item_id = Some(pending.step.entry_id);
            if !matches!(
                pending.state,
                StepState::IntentDurable | StepState::Unresolved
            ) {
                return Err(invalid("completion requires a durable unresolved intent"));
            }
            validate_receipt(&pending.step, receipt, &record)?;
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
            let approved_plan: PlanRef =
                from_json_value(record.inspection["approved_plan"].clone())?;
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
            let record = load_operation(tx, op)?;
            let unresolved = record
                .steps
                .iter()
                .any(|s| matches!(s.state, StepState::IntentDurable | StepState::Unresolved));
            let transition = matches!(
                (record.state, state),
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
                    && record.steps.iter().any(|s| !finished(s.state)))
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
            let record = load_operation(tx, op)?;
            if !record.steps.iter().any(|s| {
                s.step.id == step
                    && matches!(s.state, StepState::IntentDurable | StepState::Unresolved)
            }) {
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
    Ok(PendingOperation {
        operation_id: op,
        state: from_json(&format!("\"{state}\""))?,
        steps,
        inspection: from_json(&inspection)?,
        groups: from_json(&groups)?,
    })
}
fn finished(state: StepState) -> bool {
    matches!(state, StepState::CompletionDurable | StepState::Compensated)
}
fn state_name(state: OperationState) -> Result<String, OpError> {
    from_json_value(serde_json::to_value(state).map_err(|e| recovery(e.to_string()))?)
}
fn json(value: &impl Serialize) -> Result<String, OpError> {
    serde_json::to_string(value).map_err(|e| recovery(e.to_string()))
}
fn from_json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, OpError> {
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

fn earlier_receipt(record: &OperationRecord, id: StepId) -> Result<&StepReceipt, OpError> {
    record
        .steps
        .iter()
        .find(|s| s.step.id == id && finished(s.state))
        .and_then(|s| s.receipts.first())
        .ok_or_else(|| recovery("required durable receipt is missing; reconcile by identity"))
}
fn resolved_stamp<'a>(
    entry: &'a EntryPrestate,
    record: &'a OperationRecord,
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
fn validate_resolved_inputs(step: &ExecutionStep, record: &OperationRecord) -> Result<(), OpError> {
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
    record: &OperationRecord,
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
}
