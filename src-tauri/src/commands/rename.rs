//! Owned desktop boundary. Native inventory, plans, grants and recovery stay here.
use crate::app_state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, MutexGuard},
};
use vidlib_fileops::{
    coordinator::{Lease, LeaseKind},
    journal::{HistoryCursor, Journal},
    recovery::{self, RecoveryPreview},
    ErrorCode, Filesystem, OpError, Phase, PlatformFs,
};
use vidlib_inventory::{ids::*, *};
use vidlib_rename::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    pub phase: String,
    pub item_id: Option<EntryId>,
    pub operation_id: Option<OperationId>,
    pub retryable: bool,
}
impl From<OpError> for CommandError {
    fn from(e: OpError) -> Self {
        Self {
            code: camel_tag(&format!("{:?}", e.code)),
            message: e.message,
            phase: camel_tag(&format!("{:?}", e.phase)),
            item_id: e.item_id,
            operation_id: e.operation_id,
            retryable: e.retryable,
        }
    }
}
impl From<RenameError> for CommandError {
    fn from(e: RenameError) -> Self {
        CommandError {
            code: camel_tag(&format!("{:?}", e.code)),
            message: e.message,
            phase: "verify".into(),
            item_id: None,
            operation_id: None,
            retryable: false,
        }
    }
}
fn camel_tag(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn stale(s: impl Into<String>) -> CommandError {
    OpError::new(ErrorCode::StaleState, Phase::Verify, s).into()
}
fn busy() -> CommandError {
    OpError::new(
        ErrorCode::Busy,
        Phase::Read,
        "Another Rename job is running.",
    )
    .into()
}
type Result<T> = std::result::Result<T, CommandError>;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPlanRef {
    pub session_id: SessionId,
    pub plan_id: PlanId,
    #[serde(with = "exact::u64_decimal")]
    pub revision: u64,
}
impl From<PlanRef> for UiPlanRef {
    fn from(p: PlanRef) -> Self {
        Self {
            session_id: p.session_id,
            plan_id: p.plan_id,
            revision: p.revision,
        }
    }
}
impl From<UiPlanRef> for PlanRef {
    fn from(p: UiPlanRef) -> Self {
        Self {
            session_id: p.session_id,
            plan_id: p.plan_id,
            revision: p.revision,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageQuery {
    pub cursor: Option<String>,
    pub limit: usize,
    pub sort: String,
    pub filter: String,
    #[serde(with = "exact::u64_decimal")]
    pub expected_revision: u64,
}
impl PageQuery {
    pub fn new(revision: u64) -> Self {
        Self {
            cursor: None,
            limit: 200,
            sort: "name".into(),
            filter: String::new(),
            expected_revision: revision,
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Value>,
    pub next_cursor: Option<String>,
    #[serde(with = "exact::u64_decimal")]
    pub revision: u64,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputRevision {
    #[serde(with = "exact::u64_decimal")]
    pub revision: u64,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryRequestDto {
    #[serde(with = "exact::u64_decimal")]
    pub revision: u64,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryPageRequest {
    pub query: PageQuery,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameRequestDto {
    #[serde(with = "exact::u64_decimal")]
    pub revision: u64,
    pub selected_ids: Vec<EntryId>,
    pub rule: NamingRule,
    pub fixed_stem: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NamingRule {
    Keep,
    FixedStem,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanRequest {
    pub plan_ref: UiPlanRef,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageRequest {
    pub plan_ref: UiPlanRef,
    pub query: PageQuery,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyRequest {
    pub plan_ref: UiPlanRef,
    pub nonce: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobRequest {
    pub job_id: JobId,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryRequest {
    pub cursor: Option<HistoryCursor>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationRequest {
    pub operation_id: OperationId,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryRequest {
    pub recovery_ref: StepId,
    pub choice: Choice,
    pub nonce: StepId,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Choice {
    Resume,
    Restore,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Confirmation {
    pub nonce: String,
    pub summary: Value,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatusDto {
    pub state: String,
    pub result: Option<Value>,
    pub error: Option<CommandError>,
    pub progress: Option<Value>,
}
#[derive(Default)]
pub struct RenameState {
    pub(crate) roots: Vec<RootBinding>,
    pub(crate) revision: u64,
    pub(crate) selection_generation: u64,
    pub(crate) inventory: Option<Arc<Inventory>>,
    jobs: BTreeMap<JobId, crate::jobs::RenameJob>,
    plan_revision: u64,
    reviews: BTreeMap<StepId, (RecoveryPreview, Vec<Choice>)>,
    policy: ProtectedPolicy,
}
fn lock(state: &AppState) -> Result<MutexGuard<'_, RenameState>> {
    state
        .rename
        .lock()
        .map_err(|_| stale("Rename state is unavailable. Restart the app."))
}
fn plans(state: &AppState) -> Result<MutexGuard<'_, crate::app_state::PlanRegistry>> {
    state
        .plans
        .lock()
        .map_err(|_| stale("Plan state is unavailable. Restart the app."))
}
fn next_plan(state: &AppState, r: &mut RenameState) -> Result<PlanRef> {
    r.plan_revision = r
        .plan_revision
        .checked_add(1)
        .ok_or_else(|| stale("Revision overflow"))?;
    Ok(PlanRef {
        session_id: plans(state)?.session_id(),
        plan_id: PlanId::new(),
        revision: r.plan_revision,
    })
}
fn set_revision(state: &AppState, r: &mut RenameState, revision: u64) -> Result<()> {
    if revision < r.revision {
        return Err(stale("Inputs changed. Preview again."));
    }
    if revision > r.revision {
        r.revision = revision;
        plans(state)?.invalidate();
        r.reviews.clear();
    }
    Ok(())
}
pub fn update_inputs_owned(state: &AppState, request: InputRevision) -> Result<()> {
    let mut r = lock(state)?;
    set_revision(state, &mut r, request.revision)
}
fn fs_for(lease: &Lease, policy: &ProtectedPolicy) -> Result<PlatformFs> {
    let e = lease.epochs()?;
    Ok(PlatformFs {
        namespace_epoch: e.namespace,
        policy_epoch: e.policy,
        policy: policy.clone(),
    })
}
fn reserve(r: &mut RenameState) -> Result<(JobId, JobControl)> {
    if r.jobs.values().any(|j| j.status.state == "running") {
        return Err(busy());
    }
    if r.jobs.len() >= 64 {
        if let Some(id) = r.jobs.keys().next().copied() {
            r.jobs.remove(&id);
        }
    }
    let id = JobId::new();
    let job = crate::jobs::RenameJob::new();
    let control = job.control.clone();
    r.jobs.insert(id, job);
    Ok((id, control))
}
fn spawn<F>(state: &AppState, id: JobId, work: F) -> Result<JobId>
where
    F: FnOnce() -> Result<Value> + Send + 'static,
{
    let shared = state.rename.clone();
    let finish = move |outcome: Result<Value>| {
        if let Ok(mut r) = shared.lock() {
            if let Some(job) = r.jobs.get_mut(&id) {
                job.finish(outcome);
            }
        }
    };
    match std::thread::Builder::new()
        .name("rename-worker".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| {
                    Err(stale(
                        "Worker stopped. Check recovery history before retrying.",
                    ))
                });
            finish(outcome);
        }) {
        Ok(_) => Ok(id),
        Err(e) => {
            let mut r = lock(state)?;
            r.jobs.remove(&id);
            Err(stale(e.to_string()))
        }
    }
}
pub fn status_owned(state: &AppState, request: JobRequest) -> Result<JobStatusDto> {
    lock(state)?
        .jobs
        .get_mut(&request.job_id)
        .map(|j| j.status())
        .ok_or_else(|| stale("Unknown job"))
}
pub fn cancel_owned(state: &AppState, request: JobRequest) -> Result<()> {
    lock(state)?
        .jobs
        .get(&request.job_id)
        .ok_or_else(|| stale("Unknown job"))?
        .control
        .cancel();
    Ok(())
}
pub fn inventory_owned(state: &AppState, request: InventoryRequestDto) -> Result<JobId> {
    let lease = state.coordinator.try_acquire(LeaseKind::Read)?;
    let mut r = lock(state)?;
    set_revision(state, &mut r, request.revision)?;
    plans(state)?.invalidate();
    r.inventory = None;
    let mut input = InventoryRequest::new(r.roots.clone());
    input.generation = request.revision;
    input.policy = r.policy.clone();
    let (id, control) = reserve(&mut r)?;
    let shared = state.rename.clone();
    let selection_generation = r.selection_generation;
    drop(r);
    spawn(state, id, move || {
        let _lease = lease;
        let inventory =
            vidlib_inventory::scan(&input, &control).map_err(|e| stale(e.to_string()))?;
        if control.is_cancelled() {
            return Err(stale("Inventory cancelled"));
        }
        let mut r = shared
            .lock()
            .map_err(|_| stale("Rename state unavailable"))?;
        if r.revision != request.revision || r.selection_generation != selection_generation {
            return Err(stale("Inventory superseded"));
        }
        let result = json!({"inventoryId":inventory.id,"revision":request.revision.to_string(),"itemCount":inventory.entries.len().to_string()});
        r.inventory = Some(Arc::new(inventory));
        Ok(result)
    })
}
fn page_items<T>(
    mut rows: Vec<&T>,
    q: &PageQuery,
    revision: u64,
    key: impl Fn(&T, bool) -> (String, EntryId),
    project: impl Fn(&T) -> Value,
) -> Result<Page> {
    if q.expected_revision != revision {
        return Err(stale("Page revision expired"));
    }
    if !["name", "status"].contains(&q.sort.as_str()) || q.filter.len() > 4096 {
        return Err(stale("Unsupported page query"));
    }
    let filter = q.filter.to_lowercase();
    rows.retain(|v| key(v, false).0.to_lowercase().contains(&filter));
    rows.sort_by_cached_key(|v| key(v, q.sort == "status"));
    let offset = q
        .cursor
        .as_deref()
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| stale("Invalid page cursor"))?;
    if offset > rows.len() {
        return Err(stale("Invalid page cursor"));
    }
    let end = offset.saturating_add(q.limit.clamp(1, 200)).min(rows.len());
    Ok(Page {
        items: rows[offset..end].iter().map(|row| project(row)).collect(),
        next_cursor: (end < rows.len()).then(|| end.to_string()),
        revision,
    })
}
pub fn inventory_page_owned(state: &AppState, request: InventoryPageRequest) -> Result<Page> {
    let r = lock(state)?;
    let revision = r.revision;
    let inventory = r
        .inventory
        .clone()
        .ok_or_else(|| stale("Scan sources first"))?;
    drop(r);
    let rows = inventory
        .entries
        .iter()
        .filter(|e| matches!(e.kind, EntryKind::File | EntryKind::Directory))
        .collect();
    page_items(
        rows,
        &request.query,
        revision,
        |e, _| (e.display_name.clone(), e.id),
        |e| json!({"itemId":e.id,"displayName":e.display_name,"displayPath":e.display_path,"kind":format!("{:?}",e.kind),"sizeBytes":e.stamp.size.map(|n|n.to_string())}),
    )
}
pub fn preview_owned(state: &AppState, request: RenameRequestDto) -> Result<JobId> {
    if request.selected_ids.len() > 1_000_000 || request.fixed_stem.len() > 16_384 {
        return Err(stale("Preview inputs exceed limits"));
    }
    let lease = state.coordinator.try_acquire(LeaseKind::Read)?;
    let mut r = lock(state)?;
    set_revision(state, &mut r, request.revision)?;
    plans(state)?.invalidate();
    let inventory = r
        .inventory
        .clone()
        .ok_or_else(|| stale("Scan sources first"))?;
    let policy = r.policy.clone();
    let selection_generation = r.selection_generation;
    let version = next_plan(state, &mut r)?;
    let (id, control) = reserve(&mut r)?;
    let shared = state.rename.clone();
    let registry = state.plans.clone();
    drop(r);
    spawn(state, id, move || {
        let fs = fs_for(&lease, &policy)?;
        let recipe = RecipeV1 {
            schema_version: 1,
            revision: request.revision,
            selection: request.selected_ids.clone(),
            numbering_order: request.selected_ids.clone(),
            rules: vec![Rule {
                id: "name".into(),
                enabled: true,
                segment: SegmentSelector::Stem,
                parameters: match request.rule {
                    NamingRule::Keep => RuleParameters::Keep,
                    NamingRule::FixedStem => RuleParameters::FixedStem {
                        value: request.fixed_stem,
                    },
                },
            }],
            companions: CompanionPolicy::None,
            destination: Destination::SameDirectory,
            collision_policy: CollisionPolicy::Block,
            metadata_changes: vec![],
        };
        let inputs = FrozenInputs {
            inventory_id: inventory.id,
            generation: inventory.generation,
            now_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as i128,
            timezone: FrozenTimezone::Utc,
            primary_order: request.selected_ids,
            values: vec![],
        };
        let evaluated = evaluate(&inventory, &recipe, &inputs)?;
        // Capture originals, including unselected carried descendants. A selected
        // file itself authorizes parent occupancy without granting sibling sources.
        let selected: BTreeSet<_> = evaluated.rows.iter().map(|r| r.entry_id).collect();
        let selected_directories: BTreeSet<_> = inventory
            .entries
            .iter()
            .filter(|e| selected.contains(&e.id) && e.kind == EntryKind::Directory)
            .map(|e| e.absolute_path.clone())
            .collect();
        let carried =
            |path: &std::path::Path| path.ancestors().any(|p| selected_directories.contains(p));
        let mut candidates: Vec<_> = inventory
            .entries
            .iter()
            .filter(|e| selected.contains(&e.id) || carried(&e.absolute_path))
            .map(|e| e.absolute_path.clone())
            .chain(
                inventory
                    .coverage
                    .iter()
                    .filter(|c| carried(&c.directory))
                    .map(|c| c.directory.join(".vidlib-capture")),
            )
            .collect();
        candidates.sort();
        candidates.dedup();
        let namespace = fs.capture_namespace(&evaluated.grants, &candidates)?;
        let plan = vidlib_rename::plan(&inventory, &evaluated, &namespace, version)?;
        if control.is_cancelled() {
            return Err(stale("Preview cancelled"));
        }
        let r = shared
            .lock()
            .map_err(|_| stale("Rename state unavailable"))?;
        if r.revision != request.revision || r.selection_generation != selection_generation {
            return Err(stale("Preview superseded"));
        }
        let result = json!({"planRef":UiPlanRef::from(version),"counts":counts(&plan.counts),"inputRevision":request.revision.to_string()});
        registry
            .lock()
            .map_err(|_| stale("Plan state unavailable"))?
            .publish(plan, &lease)?;
        Ok(result)
    })
}
fn counts(c: &PlanCounts) -> Value {
    json!({"changed":c.changed.to_string(),"unchanged":c.unchanged.to_string(),"blocked":c.blocked.to_string(),"needsReview":c.needs_review.to_string()})
}
fn row_dto(row: &PlanRow) -> Value {
    json!({"itemId":row.entry_id,"oldName":row.old_display_name,"newName":row.new_display_name,"oldDisplay":row.old_display_path,"newDisplay":row.new_display_path,"sizeBytes":row.stamp.size.map(|n|n.to_string()),"modifiedNs":row.stamp.modified_ns.map(|n|n.to_string()),"status":camel_tag(&format!("{:?}",row.status)),"carriedDescendants":row.carried_descendants.to_string(),"issues":row.issues.iter().map(|i|&i.message).collect::<Vec<_>>()})
}
pub fn page_owned(state: &AppState, request: PageRequest) -> Result<Page> {
    let lease = state.coordinator.try_acquire(LeaseKind::Read)?;
    let r = lock(state)?;
    let revision = r.revision;
    let plan = plans(state)?.snapshot(request.plan_ref.into(), &lease)?;
    drop(r);
    drop(lease);
    page_items(
        plan.rows.iter().collect(),
        &request.query,
        revision,
        |row, status| {
            (
                if status {
                    camel_tag(&format!("{:?}", row.status))
                } else {
                    row.old_display_name.clone()
                },
                row.entry_id,
            )
        },
        row_dto,
    )
}
pub fn confirm_owned(state: &AppState, request: PlanRequest) -> Result<Confirmation> {
    let lease = state.coordinator.try_acquire(LeaseKind::Read)?;
    let _r = lock(state)?;
    let mut registry = plans(state)?;
    let plan = registry.current(request.plan_ref.into(), &lease)?;
    if plan.counts.changed == 0 || plan.counts.blocked != 0 || plan.counts.needs_review != 0 {
        return Err(stale("Preview has no executable changes or needs review"));
    }
    let summary = counts(&plan.counts);
    let nonce = StepId::new().to_string();
    registry.confirm(request.plan_ref.into(), nonce.clone(), &lease)?;
    Ok(Confirmation { nonce, summary })
}
fn report_dto(report: vidlib_fileops::OperationReport) -> Value {
    json!({"operationId":report.operation_id,"state":camel_tag(&format!("{:?}",report.state)),"completedGroups":report.completed_groups.len().to_string(),"completedSteps":report.receipts.len().to_string(),"failure":report.failure.map(CommandError::from)})
}
pub fn apply_owned(state: &AppState, request: ApplyRequest) -> Result<JobId> {
    let lease = state.coordinator.try_acquire(LeaseKind::Write)?;
    let mut r = lock(state)?;
    let policy = r.policy.clone();
    let plan = plans(state)?.consume_owned(request.plan_ref.into(), &request.nonce, &lease)?;
    let (id, control) = reserve(&mut r)?;
    plans(state)?.invalidate();
    r.inventory = None;
    let database = state.operations.clone();
    drop(r);
    spawn(state, id, move || {
        let mut journal = Journal::open(&database)?;
        let mut fs = fs_for(&lease, &policy)?;
        Ok(report_dto(vidlib_fileops::execute(
            &plan,
            &lease,
            &mut journal,
            &mut fs,
            &control,
        )?))
    })
}
pub fn history_owned(state: &AppState, request: HistoryRequest) -> Result<Value> {
    let lease = state.coordinator.try_acquire(LeaseKind::Recovery)?;
    let status = lease.status()?;
    let journal = Journal::open(&state.operations)?;
    let page = journal.history(request.cursor)?;
    let pending = journal.pending()?;
    Ok(
        json!({"items":page.items.iter().map(|i|json!({"operationId":i.operation_id,"state":camel_tag(&format!("{:?}",i.state)),"inverseOf":i.inverse_of,"completedSteps":i.completed_steps.to_string(),"undoAvailable":i.undo_available})).collect::<Vec<_>>(),"nextCursor":page.next_cursor,"pendingOperations":pending.iter().take(200).map(|p|p.operation_id).collect::<Vec<_>>(),"pendingCount":pending.len().to_string(),"legacyUnresolved":status.interrupted_legacy_write}),
    )
}
pub fn recovery_preview_owned(state: &AppState, request: OperationRequest) -> Result<Value> {
    let lease = state.coordinator.try_acquire(LeaseKind::Recovery)?;
    if lease.status()?.interrupted_legacy_write {
        return Err(OpError::new(
            ErrorCode::RecoveryRequired,
            Phase::Verify,
            "Legacy interruption remains unresolved; no per-item evidence exists.",
        )
        .into());
    }
    let journal = Journal::open(&state.operations)?;
    let record = journal.operation(request.operation_id)?;
    if !journal
        .pending()?
        .iter()
        .any(|p| p.operation_id == request.operation_id)
    {
        return Err(stale("Operation does not require recovery"));
    }
    let policy = lock(state)?.policy.clone();
    let mut fs = fs_for(&lease, &policy)?;
    let preview = recovery::reconcile(&record, &mut fs)?;
    let nonce = preview.review_nonce();
    let reference = StepId::new();
    let choices = if !preview.unresolved_groups.is_empty() {
        vec![]
    } else if record.restore_context.is_some() {
        vec![Choice::Restore]
    } else {
        vec![Choice::Resume, Choice::Restore]
    };
    let result = json!({"recoveryRef":reference,"nonce":nonce,"operationId":request.operation_id,"choices":choices,"continuingRestore":record.restore_context.is_some(),"unresolvedGroups":preview.unresolved_groups.len().to_string(),"actions":preview.actions.iter().take(200).map(|a|json!({"stepId":a.step_id,"disposition":camel_tag(&format!("{:?}",a.disposition)),"beforeDisplay":a.before_path.as_ref().and_then(|p|decode_path(p).ok()).map(|p|p.to_string_lossy().into_owned()),"afterDisplay":a.after_path.as_ref().and_then(|p|decode_path(p).ok()).map(|p|p.to_string_lossy().into_owned())})).collect::<Vec<_>>(),"actionCount":preview.actions.len().to_string()});
    let mut r = lock(state)?;
    r.reviews.clear();
    r.reviews.insert(reference, (preview, choices));
    Ok(result)
}
pub fn recovery_owned(state: &AppState, request: RecoveryRequest) -> Result<JobId> {
    let lease = state.coordinator.try_acquire(LeaseKind::Recovery)?;
    let mut r = lock(state)?;
    let (preview, choices) = r
        .reviews
        .remove(&request.recovery_ref)
        .ok_or_else(|| stale("Recovery review expired"))?;
    if !choices.iter().any(|c| {
        matches!(
            (c, request.choice),
            (Choice::Resume, Choice::Resume) | (Choice::Restore, Choice::Restore)
        )
    }) {
        return Err(stale("Recovery choice unavailable"));
    }
    let policy = r.policy.clone();
    let reviewed = preview.review(request.nonce)?;
    let (id, control) = reserve(&mut r)?;
    plans(state)?.invalidate();
    r.inventory = None;
    let database = state.operations.clone();
    drop(r);
    spawn(state, id, move || {
        let mut journal = Journal::open(&database)?;
        let mut fs = fs_for(&lease, &policy)?;
        Ok(report_dto(recovery::apply_recovery(
            reviewed,
            match request.choice {
                Choice::Resume => recovery::RecoveryChoice::Resume,
                Choice::Restore => recovery::RecoveryChoice::Restore,
            },
            &lease,
            &mut journal,
            &mut fs,
            &control,
        )?))
    })
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UndoRequest {
    pub operation_id: OperationId,
    #[serde(with = "exact::u64_decimal")]
    pub revision: u64,
}
pub fn undo_owned(state: &AppState, request: UndoRequest) -> Result<JobId> {
    let lease = state.coordinator.try_acquire(LeaseKind::Read)?;
    let mut r = lock(state)?;
    set_revision(state, &mut r, request.revision)?;
    plans(state)?.invalidate();
    let policy = r.policy.clone();
    let selection_generation = r.selection_generation;
    let version = next_plan(state, &mut r)?;
    let (id, control) = reserve(&mut r)?;
    let shared = state.rename.clone();
    let registry = state.plans.clone();
    let database = state.operations.clone();
    drop(r);
    spawn(state, id, move || {
        let journal = Journal::open(&database)?;
        let completed = journal.operation(request.operation_id)?;
        let context = vidlib_fileops::undo::capture_context(&completed)?;
        let fs = fs_for(&lease, &policy)?;
        let namespace = fs.capture_namespace(&context.grants, &context.candidates)?;
        let plan = vidlib_fileops::undo::plan_undo(&completed, &namespace, version)?;
        if control.is_cancelled() {
            return Err(stale("Undo preview cancelled"));
        }
        let r = shared
            .lock()
            .map_err(|_| stale("Rename state unavailable"))?;
        if r.revision != request.revision || r.selection_generation != selection_generation {
            return Err(stale("Undo preview superseded"));
        }
        let result = json!({"planRef":UiPlanRef::from(version),"counts":counts(&plan.counts),"inputRevision":request.revision.to_string()});
        registry
            .lock()
            .map_err(|_| stale("Plan state unavailable"))?
            .publish(plan, &lease)?;
        Ok(result)
    })
}
pub(crate) fn cancel_all(state: &AppState) {
    if let Ok(r) = state.rename.lock() {
        for job in r.jobs.values() {
            job.control.cancel();
        }
    }
}
// Every new command accepts exactly one owned request. Worker closures own only
// Arc-managed handles and values; no mutex guard or SQLite borrow crosses await.
macro_rules! command {
    ($name:ident,$request:ty,$response:ty,$handler:ident) => {
        #[tauri::command]
        pub async fn $name(
            request: $request,
            state: tauri::State<'_, AppState>,
        ) -> Result<$response> {
            let owned = state.inner().clone();
            tauri::async_runtime::spawn_blocking(move || $handler(&owned, request))
                .await
                .map_err(|e| stale(e.to_string()))?
        }
    };
}
command!(update_rename_inputs, InputRevision, (), update_inputs_owned);
command!(start_inventory, InventoryRequestDto, JobId, inventory_owned);
command!(
    inventory_page,
    InventoryPageRequest,
    Page,
    inventory_page_owned
);
command!(start_rename_preview, RenameRequestDto, JobId, preview_owned);
command!(job_status, JobRequest, JobStatusDto, status_owned);
command!(cancel_job, JobRequest, (), cancel_owned);
command!(rename_page, PageRequest, Page, page_owned);
command!(confirm_rename, PlanRequest, Confirmation, confirm_owned);
command!(apply_rename, ApplyRequest, JobId, apply_owned);
command!(operation_history, HistoryRequest, Value, history_owned);
command!(
    recovery_preview,
    OperationRequest,
    Value,
    recovery_preview_owned
);
command!(start_recovery, RecoveryRequest, JobId, recovery_owned);
command!(start_undo, UndoRequest, JobId, undo_owned);

impl RenameState {
    pub fn new(policy: ProtectedPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyRequest {}
pub fn session_owned(state: &AppState, _request: EmptyRequest) -> Result<Value> {
    let r = lock(state)?;
    Ok(json!({"revision":r.revision.to_string(),"sessionId":plans(state)?.session_id()}))
}
command!(rename_session, EmptyRequest, Value, session_owned);
impl RenameState {
    pub(crate) fn replace_sources(&mut self, roots: Vec<RootBinding>) -> Result<()> {
        self.selection_generation = self
            .selection_generation
            .checked_add(1)
            .ok_or_else(|| stale("Selection generation overflow"))?;
        self.roots = roots;
        self.inventory = None;
        self.reviews.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_signal_does_not_hide_factual_operation_report() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(dir.path(), dir.path().join("libraries.json")).unwrap();
        let (id, control) = reserve(&mut lock(&state).unwrap()).unwrap();
        control.cancel();
        spawn(&state,id,||Ok(json!({"operationId":"00000000-0000-4000-8000-000000000006","state":"Cancelled","completedGroups":"1","completedSteps":"1","failure":null}))).unwrap();
        for _ in 0..500 {
            let status = status_owned(&state, JobRequest { job_id: id }).unwrap();
            if status.state != "running" {
                assert_eq!(
                    status.state, "completed",
                    "cancellation must retain the factual operation result"
                );
                assert_eq!(status.result.unwrap()["completedGroups"], "1");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("job timeout");
    }
    #[test]
    fn shared_browser_golden_projects_exact_rows_and_errors() {
        let golden: Value = serde_json::from_str(include_str!(
            "../../../tests/ui/fixtures/command-contracts.json"
        ))
        .unwrap();
        let row = PlanRow {
            carried_descendants: 0,
            entry_id: serde_json::from_value(golden["row"]["itemId"].clone()).unwrap(),
            group_id: GroupId::new(),
            primary_id: EntryId::new(),
            source_path: "/backend/only/original.mp4".into(),
            target_path: "/backend/only/renamed.mp4".into(),
            old_display_name: "original.mp4".into(),
            new_display_name: "renamed.mp4".into(),
            old_display_path: "Library / original.mp4".into(),
            new_display_path: "Library / renamed.mp4".into(),
            stamp: FileStamp {
                size: Some(9_007_199_254_740_993),
                modified_ns: Some(-9_007_199_254_740_993),
                ..Default::default()
            },
            status: RowStatus::Changed,
            explanations: vec![],
            issues: vec![],
        };
        assert_eq!(row_dto(&row), golden["row"]);
        assert_eq!(
            serde_json::to_value(stale("Preview expired. Preview again.")).unwrap(),
            golden["error"]
        );
    }
}
