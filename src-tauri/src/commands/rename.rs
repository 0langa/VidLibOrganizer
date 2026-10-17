//! Owned desktop boundary. Native inventory, plans, grants and recovery stay here.
#[path = "destination_dto.rs"]
mod destination_dto;
#[path = "metadata_dto.rs"]
mod metadata_dto;
use crate::app_state::AppState;
pub use metadata_dto::MetadataChangeSetDto;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, MutexGuard},
};
use vidlib_fileops::{
    coordinator::{Lease, LeaseKind},
    journal::{HistoryQuery, HistoryQueryCursor, HistoryQueryPage, HistorySort, Journal},
    recovery::{self, RecoveryAction, RecoveryPreview},
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
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
            status: None,
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
    #[serde(default)]
    pub mapping: Option<MappingInputDto>,
    #[serde(default)]
    pub metadata_changes: Option<MetadataChangeSetDto>,
    #[serde(default)]
    pub destination: Option<DestinationDto>,
    #[serde(default)]
    pub recipe_json: Option<String>,
    #[serde(default)]
    pub rebind_selection: bool,
    #[serde(with = "exact::u64_decimal")]
    pub revision: u64,
    pub selected_ids: Vec<EntryId>,
    #[serde(default)]
    pub rule: NamingRule,
    #[serde(default)]
    pub fixed_stem: String,
    #[serde(default)]
    pub selection_filter: Option<crate::selection::SelectionFilterDto>,
    #[serde(default)]
    pub numbering_scope: NumberingScope,
    #[serde(default)]
    pub row_overrides: Vec<RowOverrideDto>,
    #[serde(default)]
    pub companions: Option<CompanionPolicyDto>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MappingInputDto {
    pub format: MappingInputFormat,
    pub text: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MappingInputFormat {
    Csv,
    Tab,
    Names,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportPlanRequest {
    pub plan_ref: UiPlanRef,
    pub mode: ExportPlanMode,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportPlanMode {
    SpreadsheetSafe,
    ExactMapping,
}
pub fn export_plan_owned(state: &AppState, request: ExportPlanRequest) -> Result<Value> {
    let lease = state.coordinator.try_acquire(LeaseKind::Read)?;
    let r = lock(state)?;
    let plan = plans(state)?.snapshot(request.plan_ref.into(), &lease)?;
    drop(r);
    let bytes = export_report(
        &plan,
        match request.mode {
            ExportPlanMode::SpreadsheetSafe => ExportMode::SpreadsheetSafe,
            ExportPlanMode::ExactMapping => ExportMode::ExactMapping,
        },
    )?;
    let text = String::from_utf8(bytes).map_err(|_| stale("Mapping export is not UTF-8"))?;
    Ok(json!({"text":text,"rowCount":plan.rows.len().to_string()}))
}
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum DestinationDto {
    InPlace {},
    CopyTo {
        #[serde(rename = "grantId")]
        grant_id: GrantId,
        layout: DestinationLayoutDto,
    },
    MoveTo {
        #[serde(rename = "grantId")]
        grant_id: GrantId,
        layout: DestinationLayoutDto,
    },
}
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum DestinationLayoutDto {
    PreserveHierarchy {},
    FolderTemplate {
        parts: Vec<destination_dto::TemplatePartDto>,
    },
}
impl From<DestinationLayoutDto> for DestinationLayout {
    fn from(value: DestinationLayoutDto) -> Self {
        match value {
            DestinationLayoutDto::PreserveHierarchy {} => Self::PreserveHierarchy,
            DestinationLayoutDto::FolderTemplate { parts } => Self::FolderTemplate {
                parts: parts.into_iter().map(Into::into).collect(),
            },
        }
    }
}
impl From<DestinationDto> for Destination {
    fn from(value: DestinationDto) -> Self {
        match value {
            DestinationDto::InPlace {} => Self::SameDirectory,
            DestinationDto::CopyTo { grant_id, layout } => Self::CopyTo {
                grant_id,
                layout: layout.into(),
            },
            DestinationDto::MoveTo { grant_id, layout } => Self::MoveTo {
                grant_id,
                layout: layout.into(),
            },
        }
    }
}
/// The command envelope uses camelCase; portable recipe documents keep their
/// versioned core representation. Only inventory IDs cross this boundary.
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum CompanionPolicyDto {
    None,
    Explicit {
        associations: Vec<CompanionAssociationDto>,
    },
    MatchExact {
        associations: Vec<CompanionAssociationDto>,
    },
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompanionAssociationDto {
    pub primary_id: EntryId,
    pub companion_ids: Vec<EntryId>,
}
impl From<CompanionPolicyDto> for CompanionPolicy {
    fn from(value: CompanionPolicyDto) -> Self {
        let convert = |bindings: Vec<CompanionAssociationDto>| {
            bindings
                .into_iter()
                .map(|b| CompanionAssociation {
                    primary: b.primary_id,
                    companions: b.companion_ids,
                })
                .collect()
        };
        match value {
            CompanionPolicyDto::None => Self::None,
            CompanionPolicyDto::Explicit { associations } => Self::Explicit {
                associations: convert(associations),
            },
            CompanionPolicyDto::MatchExact { associations } => Self::MatchExact {
                associations: convert(associations),
            },
        }
    }
}
/// Desktop literal edit; core mapping/recipe data keeps its portable names.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RowOverrideDto {
    pub entry_id: EntryId,
    pub new_name: String,
}
impl From<RowOverrideDto> for RowOverride {
    fn from(value: RowOverrideDto) -> Self {
        Self {
            entry_id: value.entry_id,
            new_name: value.new_name,
        }
    }
}
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NamingRule {
    #[default]
    Keep,
    FixedStem,
    Template {
        parts: Vec<TemplatePart>,
    },
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryRequest {
    pub query: PageQuery,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationDto {
    pub operation_id: OperationId,
    pub state: String,
    pub inverse_of: Option<OperationId>,
    #[serde(with = "exact::u64_decimal")]
    pub completed_steps: u64,
    pub undo_available: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryResponse {
    pub items: Vec<OperationDto>,
    pub next_cursor: Option<String>,
    #[serde(with = "exact::u64_decimal")]
    pub revision: u64,
    pub pending_operations: Vec<OperationId>,
    #[serde(with = "exact::u64_decimal")]
    pub pending_count: u64,
    pub legacy_unresolved: bool,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputBinding {
    revision: u64,
    selection_generation: u64,
}
impl InputBinding {
    fn capture(state: &RenameState) -> Self {
        Self {
            revision: state.revision,
            selection_generation: state.selection_generation,
        }
    }
}
struct RecoveryReview {
    preview: RecoveryPreview,
    choices: Vec<Choice>,
    binding: InputBinding,
    actions: Arc<Vec<RecoveryAction>>,
    metadata: Arc<BTreeMap<StepId, Value>>,
}
#[derive(Default)]
pub struct RenameState {
    pub(crate) roots: Vec<RootBinding>,
    pub(crate) destination_grant: Option<SourceGrant>,
    pub(crate) revision: u64,
    pub(crate) selection_generation: u64,
    pub(crate) inventory: Option<Arc<Inventory>>,
    frozen_inputs: Arc<Mutex<FrozenInputCache>>,
    jobs: BTreeMap<JobId, crate::jobs::RenameJob>,
    plan_revision: u64,
    reviews: BTreeMap<StepId, RecoveryReview>,
    policy: ProtectedPolicy,
    library_roots: Vec<std::path::PathBuf>,
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
fn refresh_library_policy(state: &AppState) -> Result<()> {
    let mut roots = crate::selection::library_paths(state)?;
    roots.sort();
    roots.dedup();
    let mut r = lock(state)?;
    if roots != r.library_roots {
        r.guard_library_roots(roots);
        plans(state)?.invalidate();
        r.reviews.clear();
    }
    Ok(())
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
    if r.jobs
        .values()
        .any(|j| j.status.state == "running" && !j.control.is_cancelled())
    {
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
    refresh_library_policy(state)?;
    let mut r = lock(state)?;
    set_revision(state, &mut r, request.revision)?;
    plans(state)?.invalidate();
    r.inventory = None;
    r.frozen_inputs = Arc::default();
    let mut input = InventoryRequest::new(r.roots.clone());
    input.generation = request.revision;
    input.policy = r.effective_policy();
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
fn natural_cmp(mut a: &str, mut b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    while !a.is_empty() && !b.is_empty() {
        let ac = a.chars().next().unwrap();
        let bc = b.chars().next().unwrap();
        if ac.is_ascii_digit() && bc.is_ascii_digit() {
            let ae = a.find(|c: char| !c.is_ascii_digit()).unwrap_or(a.len());
            let be = b.find(|c: char| !c.is_ascii_digit()).unwrap_or(b.len());
            let an = a[..ae].trim_start_matches('0');
            let bn = b[..be].trim_start_matches('0');
            let order = an.len().cmp(&bn.len()).then_with(|| an.cmp(bn));
            if order != Ordering::Equal {
                return order;
            }
            a = &a[ae..];
            b = &b[be..];
        } else {
            let order = ac.cmp(&bc);
            if order != Ordering::Equal {
                return order;
            }
            a = &a[ac.len_utf8()..];
            b = &b[bc.len_utf8()..];
        }
    }
    a.len().cmp(&b.len())
}
fn page_items<T>(
    mut rows: Vec<&T>,
    q: &PageQuery,
    revision: u64,
    key: impl Fn(&T, &str) -> (String, String),
    project: impl Fn(&T) -> Value,
) -> Result<Page> {
    if q.expected_revision != revision {
        return Err(stale("Page revision expired"));
    }
    if !["name", "natural", "path", "size", "status"].contains(&q.sort.as_str())
        || q.filter.len() > 4096
    {
        return Err(stale("Unsupported page query"));
    }
    if q.status
        .as_ref()
        .is_some_and(|s| !["changed", "unchanged", "needsReview", "blocked"].contains(&s.as_str()))
    {
        return Err(stale("Unsupported status filter"));
    }
    let filter = q.filter.to_lowercase();
    rows.retain(|v| {
        key(v, "search").0.to_lowercase().contains(&filter)
            && q.status
                .as_ref()
                .is_none_or(|status| key(v, "status").0 == *status)
    });
    if q.sort == "natural" {
        rows.sort_by(|a, b| {
            let a = key(a, "name");
            let b = key(b, "name");
            natural_cmp(&a.0, &b.0).then_with(|| a.1.cmp(&b.1))
        });
    } else {
        rows.sort_by_cached_key(|v| key(v, &q.sort));
    }
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
        |e, sort| {
            (
                match sort {
                    "path" => e.display_path.clone(),
                    "search" => format!("{} {}", e.display_name, e.display_path),
                    "size" => format!("{:020}", e.stamp.size.unwrap_or(0)),
                    _ => e.display_name.clone(),
                },
                e.id.to_string(),
            )
        },
        |e| json!({"itemId":e.id,"displayName":e.display_name,"displayPath":e.display_path,"kind":camel_tag(&format!("{:?}",e.kind)),"sizeBytes":e.stamp.size.map(|n|n.to_string())}),
    )
}
pub fn preview_owned(state: &AppState, request: RenameRequestDto) -> Result<JobId> {
    if request
        .mapping
        .as_ref()
        .is_some_and(|m| m.text.len() as u64 > MAPPING_BYTE_LIMIT)
    {
        return Err(
            RenameError::new(RenameErrorCode::ImportLimit, "mapping exceeds 64 MiB").into(),
        );
    }
    if request.selected_ids.len() > 1_000_000
        || request.fixed_stem.len() > 16_384
        || request.row_overrides.len() > MAPPING_ROW_LIMIT
        || request
            .row_overrides
            .iter()
            .any(|r| r.new_name.len() > 16_384)
    {
        return Err(stale("Preview inputs exceed limits"));
    }
    if request
        .recipe_json
        .as_ref()
        .is_some_and(|json| json.len() > presets::RECIPE_BYTE_LIMIT)
    {
        return Err(RenameError::new(RenameErrorCode::ImportLimit, "recipe exceeds 1 MiB").into());
    }
    if request.recipe_json.is_some() && request.selection_filter.is_some() {
        return Err(stale(
            "Configure the selection filter inside the recipe document",
        ));
    }
    let lease = state.coordinator.try_acquire(LeaseKind::Read)?;
    refresh_library_policy(state)?;
    let mut r = lock(state)?;
    set_revision(state, &mut r, request.revision)?;
    plans(state)?.invalidate();
    let inventory = r
        .inventory
        .clone()
        .ok_or_else(|| stale("Scan sources first"))?;
    let policy = r.effective_policy();
    let destination_grant = r.destination_grant.clone();
    let frozen_inputs = r.frozen_inputs.clone();
    let metadata_provider = state.metadata_provider.clone();
    let selection_generation = r.selection_generation;
    let version = next_plan(state, &mut r)?;
    let (id, control) = reserve(&mut r)?;
    let shared = state.rename.clone();
    let registry = state.plans.clone();
    drop(r);
    spawn(state, id, move || {
        let fs = fs_for(&lease, &policy)?;
        // Parse/validate (including bounded regex compilation) on the worker.
        let imported = request
            .recipe_json
            .as_deref()
            .map(|json| presets::PresetStore::import(json.as_bytes()))
            .transpose()?;
        let mut recipe = if let Some(mut recipe) = imported {
            if request.rebind_selection {
                recipe.selection = request.selected_ids.clone();
                recipe.numbering_order = request.selected_ids.clone();
            } else if recipe.selection != request.selected_ids {
                return Err(stale("Saved selection differs. Explicitly bind the recipe to the current checked items."));
            }
            recipe.revision = request.revision;
            recipe
        } else {
            RecipeV1 {
                schema_version: 1,
                revision: request.revision,
                selection: request.selected_ids.clone(),
                selection_filter: None,
                numbering_order: request.selected_ids.clone(),
                numbering_scope: request.numbering_scope,
                rules: vec![Rule {
                    id: "name".into(),
                    enabled: true,
                    segment: SegmentSelector::Stem,
                    parameters: match request.rule {
                        NamingRule::Keep => RuleParameters::Keep,
                        NamingRule::Template { parts } => RuleParameters::Template { parts },
                        NamingRule::FixedStem => RuleParameters::FixedStem {
                            value: request.fixed_stem,
                        },
                    },
                }],
                companions: CompanionPolicy::None,
                destination: Destination::SameDirectory,
                collision_policy: CollisionPolicy::Block,
                metadata_changes: vec![],
            }
        };
        if let Some(changes) = request.metadata_changes {
            recipe.metadata_changes =
                vidlib_rename::metadata::MetadataChangeSet::from(changes).changes();
        }
        if let Some(destination) = request.destination {
            recipe.destination = destination.into();
        }
        if let Destination::CopyTo { grant_id, .. } | Destination::MoveTo { grant_id, .. } =
            recipe.destination
        {
            if !destination_grant.as_ref().is_some_and(|g| g.id == grant_id) {
                return Err(stale("Choose the current destination folder with the native picker, then preview again."));
            }
        }
        if let Some(companions) = request.companions {
            recipe.companions = companions.into();
        }
        let known = inventory
            .entries
            .iter()
            .map(|e| e.id)
            .collect::<BTreeSet<_>>();
        if recipe.selection.iter().any(|id| !known.contains(id)) {
            return Err(RenameError::new(
                RenameErrorCode::UnknownEntry,
                "selected ID is absent from inventory",
            )
            .into());
        }
        if let Some(filter) = request
            .selection_filter
            .map(Into::into)
            .or_else(|| recipe.selection_filter.clone())
        {
            let allowed = select_controlled(&inventory, &filter, &control)?
                .into_iter()
                .collect::<BTreeSet<_>>();
            recipe.selection.retain(|id| allowed.contains(id));
            recipe.numbering_order.retain(|id| allowed.contains(id));
            recipe.selection_filter = Some(filter);
        }
        prepare_associations(&inventory, &mut recipe)?;
        let requirements = InputRequirements::from_recipe(&recipe, FrozenTimezone::Utc)?;
        let inputs = frozen_inputs
            .lock()
            .map_err(|_| stale("Frozen input state unavailable"))?
            .prepare_with_metadata(
                &requirements,
                &inventory,
                &control,
                metadata_provider.as_ref(),
            )?;
        let mappings = if let Some(mapping) = request.mapping {
            let format = match mapping.format {
                MappingInputFormat::Csv => MappingFormat::Csv,
                MappingInputFormat::Tab => MappingFormat::Delimited { delimiter: b'\t' },
                MappingInputFormat::Names => MappingFormat::NameList {
                    order: recipe.selection.clone(),
                },
            };
            let grants = GrantSet {
                grants: inventory
                    .roots
                    .iter()
                    .map(|r| r.source_grant.clone())
                    .collect(),
            };
            import_mappings(mapping.text.as_bytes(), format, &inventory, &grants)?
        } else {
            MappingSet {
                inventory_id: inventory.id,
                generation: inventory.generation,
                rows: vec![],
            }
        };
        let overrides = request
            .row_overrides
            .into_iter()
            .map(Into::into)
            .collect::<Vec<RowOverride>>();
        let evaluated = evaluate_with_mappings_controlled(
            &inventory, &recipe, &inputs, &mappings, &overrides, &control,
        )?;
        let mut grants = evaluated.grants.clone();
        if matches!(
            recipe.destination,
            Destination::CopyTo { .. } | Destination::MoveTo { .. }
        ) {
            grants.grants.push(
                destination_grant
                    .ok_or_else(|| stale("Current native destination grant missing"))?,
            );
        }
        let evaluated = resolve_destinations(evaluated, &recipe.destination, &grants)?;
        let namespace = fs.capture_namespace(
            &evaluated.grants,
            &destination::capture_candidates(&evaluated),
        )?;
        let plan = vidlib_rename::plan(&inventory, &evaluated, &namespace, version)?;
        let result = json!({"planRef":UiPlanRef::from(version),"counts":counts(&plan.counts),"summary":summary(&plan),"inputRevision":request.revision.to_string()});
        if control.is_cancelled() {
            return Err(stale("Preview cancelled"));
        }
        let r = shared
            .lock()
            .map_err(|_| stale("Rename state unavailable"))?;
        if r.revision != request.revision || r.selection_generation != selection_generation {
            return Err(stale("Preview superseded"));
        }
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
fn summary(plan: &RenamePlan) -> Value {
    let entries = plan
        .execution
        .provenance
        .inventory
        .entries
        .iter()
        .map(|e| (e.id, e))
        .collect::<BTreeMap<_, _>>();
    let files = plan
        .rows
        .iter()
        .filter(|r| {
            entries
                .get(&r.entry_id)
                .is_some_and(|e| e.kind == EntryKind::File)
        })
        .count();
    let folders = plan
        .rows
        .iter()
        .filter(|r| {
            entries
                .get(&r.entry_id)
                .is_some_and(|e| e.kind == EntryKind::Directory)
        })
        .count();
    let steps = plan
        .execution
        .groups
        .iter()
        .flat_map(|g| &g.steps)
        .collect::<Vec<_>>();
    let destinations = plan
        .rows
        .iter()
        .map(|r| {
            r.target_path
                .parent()
                .unwrap_or(&r.target_path)
                .to_string_lossy()
                .into_owned()
        })
        .collect::<BTreeSet<_>>();
    let mut value = counts(&plan.counts);
    value["selected"] = json!(plan.rows.len().to_string());
    value["files"] = json!(files.to_string());
    value["folders"] = json!(folders.to_string());
    let moved_directories = plan
        .rows
        .iter()
        .filter(|r| {
            r.source_path != r.target_path
                && entries
                    .get(&r.entry_id)
                    .is_some_and(|e| e.kind == EntryKind::Directory)
        })
        .map(|r| r.source_path.as_path())
        .collect::<BTreeSet<_>>();
    let selected = plan
        .rows
        .iter()
        .map(|r| r.entry_id)
        .collect::<BTreeSet<_>>();
    let carried = entries
        .values()
        .filter(|e| {
            e.absolute_path
                .ancestors()
                .skip(1)
                .any(|path| moved_directories.contains(path))
        })
        .collect::<Vec<_>>();
    value["carriedDescendants"] = json!(carried.len().to_string());
    value["unselectedDescendants"] = json!(carried
        .iter()
        .filter(|e| !selected.contains(&e.id))
        .count()
        .to_string());
    value["companions"] = json!(plan
        .rows
        .iter()
        .filter(|r| r.entry_id != r.primary_id)
        .count()
        .to_string());
    value["metadataActions"] = json!(steps
        .iter()
        .filter(|s| matches!(s.action, ExecutionAction::SetMetadata { .. }))
        .count()
        .to_string());
    value["createdCopyRemovals"] = json!(steps
        .iter()
        .filter(|s| matches!(s.action, ExecutionAction::RemoveOwnedCopy { .. }))
        .count()
        .to_string());
    value["ownedDirectoryRemovals"] = json!(steps
        .iter()
        .filter(|s| matches!(s.action, ExecutionAction::RemoveOwnedEmptyDirectory { .. }))
        .count()
        .to_string());
    value["destinationCount"] = json!(destinations.len().to_string());
    value["destinations"] = json!(destinations.into_iter().take(200).collect::<Vec<_>>());
    value["destinationMode"] = json!(match plan.execution.provenance.recipe.destination {
        Destination::SameDirectory => "inPlace",
        Destination::CopyTo { .. } => "copyTo",
        Destination::MoveTo { .. } => "moveTo",
        Destination::Granted { .. } => "legacy",
    });
    value["isUndo"] = json!(plan.inverse_of.is_some());
    value
}
// Review projections are data only. They never grant paths or update journal evidence.
fn stamp_dto(stamp: &FileStamp) -> Value {
    json!({"createdNs":stamp.created_ns.map(|v|v.to_string()),"modifiedNs":stamp.modified_ns.map(|v|v.to_string()),"accessedNs":stamp.accessed_ns.map(|v|v.to_string()),"windowsAttributes":stamp.windows_attributes})
}
fn metadata_change_dto(change: &MetadataChange) -> Value {
    match change {
        MetadataChange::Timestamp { field, value_ns } => {
            json!({"type":"timestamp","field":field,"valueNs":value_ns.to_string()})
        }
        MetadataChange::TimestampOffset { field, offset_ns } => {
            json!({"type":"timestampOffset","field":field,"offsetNs":offset_ns.to_string()})
        }
        MetadataChange::TimestampIncrement {
            field,
            start_ns,
            step_ns,
        } => {
            json!({"type":"timestampIncrement","field":field,"startNs":start_ns.to_string(),"stepNs":step_ns.to_string()})
        }
        MetadataChange::WindowsAttributes { set, clear } => {
            json!({"type":"windowsAttributes","setBits":set,"clearBits":clear})
        }
    }
}
fn reviewed_row_dto(plan: &RenamePlan, row: &PlanRow) -> Value {
    let mut value = row_dto(row);
    let entry = plan
        .execution
        .provenance
        .inventory
        .entries
        .iter()
        .find(|e| e.id == row.entry_id);
    value["kind"] = json!(entry.map(|e| camel_tag(&format!("{:?}", e.kind))));
    value["metadata"] = json!(plan.execution.groups.iter().flat_map(|g|&g.steps).filter(|s|s.entry_id==row.entry_id).filter_map(|s|if let ExecutionAction::SetMetadata{changes,..}=&s.action {
        Some(json!({"requested":changes.iter().map(metadata_change_dto).collect::<Vec<_>>(),"original":stamp_dto(&row.stamp)}))
    }else{None}).collect::<Vec<_>>());
    value
}
fn row_dto(row: &PlanRow) -> Value {
    let warnings = row
        .explanations
        .iter()
        .filter(|e| e.rule_id == "companion_warning")
        .map(|e| &e.after)
        .collect::<Vec<_>>();
    let explanations = row
        .explanations
        .iter()
        .map(|e| json!({"ruleId": e.rule_id, "before": e.before, "after": e.after}))
        .collect::<Vec<_>>();
    json!({
        "itemId": row.entry_id,
        "association": { "primaryId": row.primary_id, "groupId": row.group_id, "isCompanion": row.entry_id != row.primary_id },
        "companionWarnings": warnings,
        "explanations": explanations,
        "oldName": row.old_display_name,
        "newName": row.new_display_name,
        "oldDisplay": row.old_display_path,
        "newDisplay": row.new_display_path,
        "sizeBytes": row.stamp.size.map(|n| n.to_string()),
        "modifiedNs": row.stamp.modified_ns.map(|n| n.to_string()),
        "status": camel_tag(&format!("{:?}", row.status)),
        "carriedDescendants": row.carried_descendants.to_string(),
        "issues": row.issues.iter().map(|i| &i.message).collect::<Vec<_>>()
    })
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
                match status {
                    "status" => camel_tag(&format!("{:?}", row.status)),
                    "path" => row.old_display_path.clone(),
                    "search" => format!(
                        "{} {} {} {}",
                        row.old_display_name,
                        row.new_display_name,
                        row.old_display_path,
                        row.new_display_path
                    ),
                    "size" => format!("{:020}", row.stamp.size.unwrap_or(0)),
                    _ => row.old_display_name.clone(),
                },
                row.entry_id.to_string(),
            )
        },
        |row| reviewed_row_dto(&plan, row),
    )
}
pub fn confirm_owned(state: &AppState, request: PlanRequest) -> Result<Confirmation> {
    let lease = state.coordinator.try_acquire(LeaseKind::Read)?;
    refresh_library_policy(state)?;
    let plan = {
        let _r = lock(state)?;
        plans(state)?.snapshot(request.plan_ref.into(), &lease)?
    };
    if plan.counts.changed == 0 || plan.counts.blocked != 0 || plan.counts.needs_review != 0 {
        return Err(stale("Preview has no executable changes or needs review"));
    }
    let summary = summary(&plan);
    let nonce = StepId::new().to_string();
    // Revalidate the reference after projection: an edit may have revoked it.
    let _r = lock(state)?;
    plans(state)?.confirm(request.plan_ref.into(), nonce.clone(), &lease)?;
    Ok(Confirmation { nonce, summary })
}
fn report_dto(report: vidlib_fileops::OperationReport) -> Value {
    json!({"operationId":report.operation_id,"state":camel_tag(&format!("{:?}",report.state)),"completedGroups":report.completed_groups.len().to_string(),"completedSteps":report.receipts.len().to_string(),"failure":report.failure.map(CommandError::from)})
}
pub fn apply_owned(state: &AppState, request: ApplyRequest) -> Result<JobId> {
    let lease = state.coordinator.try_acquire(LeaseKind::Write)?;
    refresh_library_policy(state)?;
    let mut r = lock(state)?;
    let policy = r.effective_policy();
    let plan = plans(state)?.consume_owned(request.plan_ref.into(), &request.nonce, &lease)?;
    let (id, control) = reserve(&mut r)?;
    plans(state)?.invalidate();
    r.inventory = None;
    r.frozen_inputs = Arc::default();
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
fn history_response(
    page: HistoryQueryPage,
    pending: Vec<OperationId>,
    pending_count: u64,
    legacy_unresolved: bool,
) -> Result<HistoryResponse> {
    Ok(HistoryResponse {
        items: page
            .items
            .into_iter()
            .map(|i| OperationDto {
                operation_id: i.operation_id,
                state: camel_tag(&format!("{:?}", i.state)),
                inverse_of: i.inverse_of,
                completed_steps: i.completed_steps,
                undo_available: i.undo_available,
            })
            .collect(),
        next_cursor: page
            .next_cursor
            .map(|c| serde_json::to_string(&c))
            .transpose()
            .map_err(|e| stale(e.to_string()))?,
        revision: page.revision,
        pending_operations: pending,
        pending_count,
        legacy_unresolved,
    })
}
pub fn history_owned(state: &AppState, request: HistoryRequest) -> Result<Value> {
    let lease = state.coordinator.try_acquire(LeaseKind::Recovery)?;
    let status = lease.status()?;
    let query = request.query;
    let sort = match query.sort.as_str() {
        "name" | "operationId" => HistorySort::OperationId,
        "state" => HistorySort::State,
        "oldest" => HistorySort::Oldest,
        "newest" => HistorySort::Newest,
        _ => return Err(stale("Unsupported history sort")),
    };
    if query.cursor.as_ref().is_some_and(|s| s.len() > 32768) {
        return Err(stale("History cursor exceeds limit"));
    }
    let cursor: Option<HistoryQueryCursor> = query
        .cursor
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|_| stale("Invalid history cursor"))?;
    let journal = Journal::open(&state.operations)?;
    let page = journal.history_query(HistoryQuery {
        cursor,
        limit: query.limit,
        sort,
        filter: query.filter,
        expected_revision: query.expected_revision,
    })?;
    let pending = journal.pending()?;
    let pending_count = pending.len() as u64;
    let response = history_response(
        page,
        pending
            .into_iter()
            .take(200)
            .map(|p| p.operation_id)
            .collect(),
        pending_count,
        status.interrupted_legacy_write,
    )?;
    serde_json::to_value(response).map_err(|e| stale(e.to_string()))
}
pub fn history_revision_owned(state: &AppState, _request: EmptyRequest) -> Result<Value> {
    Ok(json!({"revision":Journal::open(&state.operations)?.history_revision()?.to_string()}))
}
/// Owned, nonserializable capture phase. The matching exclusive lease stays
/// with this value through native reconciliation and publication.
pub struct RecoveryInspection {
    lease: Lease,
    record: vidlib_fileops::journal::OperationRecord,
    policy: ProtectedPolicy,
    binding: InputBinding,
}
pub fn prepare_recovery_owned(
    state: &AppState,
    request: OperationRequest,
) -> Result<RecoveryInspection> {
    let lease = state.coordinator.try_acquire(LeaseKind::Recovery)?;
    refresh_library_policy(state)?;
    if lease.status()?.interrupted_legacy_write {
        return Err(OpError::new(
            ErrorCode::RecoveryRequired,
            Phase::Verify,
            "Legacy interruption remains unresolved; no per-item evidence exists.",
        )
        .into());
    }
    let r = lock(state)?;
    let binding = InputBinding::capture(&r);
    let policy = r.effective_policy();
    drop(r);
    let journal = Journal::open(&state.operations)?;
    let record = journal.operation(request.operation_id)?;
    if !journal
        .pending()?
        .iter()
        .any(|p| p.operation_id == request.operation_id)
    {
        return Err(stale("Operation does not require recovery"));
    }
    Ok(RecoveryInspection {
        lease,
        record,
        policy,
        binding,
    })
}
fn action_dto(action: &RecoveryAction) -> Value {
    json!({"stepId":action.step_id,"disposition":camel_tag(&format!("{:?}",action.disposition)),"beforeDisplay":action.before_path.as_ref().and_then(|p|decode_path(p).ok()).map(|p|p.to_string_lossy().into_owned()),"afterDisplay":action.after_path.as_ref().and_then(|p|decode_path(p).ok()).map(|p|p.to_string_lossy().into_owned())})
}
fn action_page(actions: &[RecoveryAction], query: &PageQuery, revision: u64) -> Result<Page> {
    page_items(
        actions.iter().collect(),
        query,
        revision,
        |action, status| {
            let name = if status == "status" {
                camel_tag(&format!("{:?}", action.disposition))
            } else {
                action
                    .before_path
                    .as_ref()
                    .and_then(|p| decode_path(p).ok())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            (name, action.step_id.to_string())
        },
        action_dto,
    )
}
fn recovery_metadata(
    record: &vidlib_fileops::journal::PendingOperation,
) -> BTreeMap<StepId, Value> {
    let originals = record.inspection["inventory"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                serde_json::from_value::<EntryId>(entry["id"].clone()).ok()?,
                serde_json::from_value::<FileStamp>(entry["stamp"].clone()).ok()?,
            ))
        })
        .collect::<BTreeMap<_, _>>();
    record.steps.iter().chain(record.recovery_effects.iter().map(|effect|&effect.inverse)).filter_map(|pending| {
        let ExecutionAction::SetMetadata{changes,..} = &pending.step.action else {return None};
        let after = pending.receipts.iter().rev().find(|receipt|receipt.phase==vidlib_fileops::events::CompletedPhase::MetadataSet).and_then(|receipt|receipt.after.as_ref());
        Some((pending.step.id,json!({"original":originals.get(&pending.step.entry_id).map(stamp_dto),"factualBefore":pending.metadata_prestate.as_ref().map(stamp_dto),"observedAfter":after.map(stamp_dto),"requested":changes.iter().map(metadata_change_dto).collect::<Vec<_>>()})))
    }).collect()
}
fn decorate_recovery_page(mut page: Page, metadata: &BTreeMap<StepId, Value>) -> Page {
    for row in &mut page.items {
        if let Some(value) = serde_json::from_value::<StepId>(row["stepId"].clone())
            .ok()
            .and_then(|id| metadata.get(&id))
        {
            row["metadata"] = value.clone();
        }
    }
    page
}
pub fn publish_recovery_owned(state: &AppState, inspection: RecoveryInspection) -> Result<Value> {
    let RecoveryInspection {
        lease,
        record,
        policy,
        binding,
    } = inspection;
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
    let actions = Arc::new(preview.actions.clone());
    let metadata = Arc::new(recovery_metadata(&record));
    let page = decorate_recovery_page(
        action_page(
            &actions,
            &PageQuery::new(binding.revision),
            binding.revision,
        )?,
        &metadata,
    );
    let result = json!({"recoveryRef":reference,"nonce":nonce,"operationId":record.operation_id,"revision":binding.revision.to_string(),"choices":choices,"continuingRestore":record.restore_context.is_some(),"unresolvedGroups":preview.unresolved_groups.len().to_string(),"page":page,"actionCount":actions.len().to_string()});
    let mut r = lock(state)?;
    if InputBinding::capture(&r) != binding {
        return Err(stale(
            "Recovery review was superseded by newer inputs or native selection",
        ));
    }
    r.reviews.clear();
    r.reviews.insert(
        reference,
        RecoveryReview {
            preview,
            choices,
            binding,
            actions,
            metadata,
        },
    );
    Ok(result)
}
pub fn recovery_preview_owned(state: &AppState, request: OperationRequest) -> Result<Value> {
    publish_recovery_owned(state, prepare_recovery_owned(state, request)?)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryPageRequest {
    pub recovery_ref: StepId,
    pub query: PageQuery,
}
pub fn recovery_page_owned(state: &AppState, request: RecoveryPageRequest) -> Result<Page> {
    let r = lock(state)?;
    let review = r
        .reviews
        .get(&request.recovery_ref)
        .ok_or_else(|| stale("Recovery review expired"))?;
    if review.binding != InputBinding::capture(&r) {
        return Err(stale("Recovery review expired"));
    }
    let actions = review.actions.clone();
    let metadata = review.metadata.clone();
    let revision = review.binding.revision;
    drop(r);
    Ok(decorate_recovery_page(
        action_page(&actions, &request.query, revision)?,
        &metadata,
    ))
}
pub fn recovery_owned(state: &AppState, request: RecoveryRequest) -> Result<JobId> {
    let lease = state.coordinator.try_acquire(LeaseKind::Recovery)?;
    refresh_library_policy(state)?;
    let mut r = lock(state)?;
    let review = r
        .reviews
        .remove(&request.recovery_ref)
        .ok_or_else(|| stale("Recovery review expired"))?;
    if review.binding != InputBinding::capture(&r) {
        return Err(stale("Recovery review expired"));
    }
    if !review.choices.iter().any(|c| {
        matches!(
            (c, request.choice),
            (Choice::Resume, Choice::Resume) | (Choice::Restore, Choice::Restore)
        )
    }) {
        return Err(stale("Recovery choice unavailable"));
    }
    let policy = r.effective_policy();
    let reviewed = review.preview.review(request.nonce)?;
    let (id, control) = reserve(&mut r)?;
    plans(state)?.invalidate();
    r.inventory = None;
    r.frozen_inputs = Arc::default();
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
    refresh_library_policy(state)?;
    let mut r = lock(state)?;
    set_revision(state, &mut r, request.revision)?;
    plans(state)?.invalidate();
    let policy = r.effective_policy();
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
        let result = json!({"planRef":UiPlanRef::from(version),"counts":counts(&plan.counts),"summary":summary(&plan),"inputRevision":request.revision.to_string()});
        if control.is_cancelled() {
            return Err(stale("Undo preview cancelled"));
        }
        let r = shared
            .lock()
            .map_err(|_| stale("Rename state unavailable"))?;
        if r.revision != request.revision || r.selection_generation != selection_generation {
            return Err(stale("Undo preview superseded"));
        }
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
command!(
    export_rename_plan,
    ExportPlanRequest,
    Value,
    export_plan_owned
);
command!(confirm_rename, PlanRequest, Confirmation, confirm_owned);
command!(apply_rename, ApplyRequest, JobId, apply_owned);
command!(operation_history, HistoryRequest, Value, history_owned);
command!(
    history_revision,
    EmptyRequest,
    Value,
    history_revision_owned
);
command!(
    recovery_preview,
    OperationRequest,
    Value,
    recovery_preview_owned
);
command!(
    recovery_page,
    RecoveryPageRequest,
    Page,
    recovery_page_owned
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
    pub(crate) fn replace_destination(&mut self, grant: SourceGrant) -> Result<()> {
        self.selection_generation = self
            .selection_generation
            .checked_add(1)
            .ok_or_else(|| stale("Selection generation overflow"))?;
        self.destination_grant = Some(grant);
        self.reviews.clear();
        Ok(())
    }
    fn effective_policy(&self) -> ProtectedPolicy {
        let mut policy = self.policy.clone();
        for root in &self.library_roots {
            if !policy.guarded_roots.contains(root) {
                policy.guarded_roots.push(root.clone());
            }
        }
        policy
    }
    pub(crate) fn guard_library_roots(&mut self, mut roots: Vec<std::path::PathBuf>) {
        roots.sort();
        roots.dedup();
        // Library configuration owns only this layer. Removal must preserve
        // independently protected roots and marker policy in the base layer.
        self.library_roots = roots;
    }
    pub(crate) fn replace_sources(&mut self, roots: Vec<RootBinding>) -> Result<()> {
        self.selection_generation = self
            .selection_generation
            .checked_add(1)
            .ok_or_else(|| stale("Selection generation overflow"))?;
        // Native source replacement runs under a coordinator read lease; the
        // legacy backend seam also supports retained recovery inspection.
        // Cancel superseded readers; their generation checks
        // prevent publication while a new read job starts.
        for job in self.jobs.values() {
            if job.status.state == "running" {
                job.control.cancel();
            }
        }
        self.roots = roots;
        self.inventory = None;
        self.frozen_inputs = Arc::default();
        self.reviews.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn await_job(state: &AppState, id: JobId) -> Value {
        for _ in 0..1000 {
            let status = status_owned(state, JobRequest { job_id: id }).unwrap();
            if status.state != "running" {
                assert_eq!(status.state, "completed", "{status:?}");
                return status.result.unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("job timeout")
    }
    #[test]
    fn desktop_mapping_and_review_query_wire_are_supported() {
        let id = EntryId::new();
        let mapping = serde_json::from_value::<RenameRequestDto>(
            json!({"revision":"1","selectedIds":[id],"mapping":{"format":"names","text":"new.mkv"}}),
        );
        assert!(
            mapping.is_ok(),
            "bounded mapping input must reach the native importer: {:?}",
            mapping.err()
        );
        let query = serde_json::from_value::<PageQuery>(
            json!({"cursor":null,"limit":200,"sort":"natural","filter":"movie","status":"changed","expectedRevision":"1"}),
        );
        assert!(
            query.is_ok(),
            "status filter must be a typed independent display query"
        );
    }
    #[test]
    fn desktop_cpu_preview_reports_between_items_and_can_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir(&media).unwrap();
        for i in 0..1200 {
            std::fs::write(
                media.join(format!("movie-{}-{i:04}.mkv", "1234567890".repeat(20))),
                b"a",
            )
            .unwrap();
        }
        let state =
            AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
        crate::selection::install_selection(&state, vec![media]).unwrap();
        await_job(
            &state,
            inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
        );
        let ids = lock(&state)
            .unwrap()
            .inventory
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .map(|e| e.id)
            .collect::<Vec<_>>();
        let rules=(0..100).map(|i|json!({"id":format!("r{i}"),"enabled":true,"segment":{"type":"stem"},"parameters":{"type":"regex","rule":{"pattern":"[0-9]","replacement":"0","scope":"all","case_sensitive":true}}})).collect::<Vec<_>>();
        let recipe = json!({"schema_version":1,"revision":"2","selection":ids,"numbering_order":ids,"rules":rules,"companions":{"type":"none"},"destination":{"type":"same_directory"},"collision_policy":{"type":"block"},"metadata_changes":[]});
        let request = serde_json::from_value(
            json!({"revision":"2","selectedIds":ids,"recipeJson":recipe.to_string()}),
        )
        .unwrap();
        let job = preview_owned(&state, request).unwrap();
        let mut observed = false;
        for _ in 0..5000 {
            let status = status_owned(&state, JobRequest { job_id: job }).unwrap();
            if status.progress.as_ref().is_some_and(|p| {
                p["phase"] == "evaluating"
                    && p["done"]
                        .as_str()
                        .and_then(|n| n.parse::<u64>().ok())
                        .is_some_and(|done| done > 0 && done < 1200)
            }) {
                observed = true;
                cancel_owned(&state, JobRequest { job_id: job }).unwrap();
                break;
            }
            if status.state != "running" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            observed,
            "actual CPU preview must expose between-item evaluation progress before completion"
        );
        for _ in 0..1000 {
            let status = status_owned(&state, JobRequest { job_id: job }).unwrap();
            if status.state != "running" {
                assert_eq!(status.state, "cancelled");
                assert!(status.result.is_none());
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("CPU cancellation did not finish between items");
    }
    #[test]
    fn desktop_cpu_selection_reports_between_items_and_can_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir(&media).unwrap();
        for i in 0..1200 {
            std::fs::write(
                media.join(format!("movie-{}-{i:04}.mkv", "1234567890".repeat(20))),
                b"a",
            )
            .unwrap();
        }
        let state =
            AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
        crate::selection::install_selection(&state, vec![media]).unwrap();
        await_job(
            &state,
            inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
        );
        let ids = lock(&state)
            .unwrap()
            .inventory
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .map(|e| e.id)
            .collect::<Vec<_>>();
        let rules=(0..100).map(|i|json!({"id":format!("r{i}"),"enabled":true,"segment":{"type":"stem"},"parameters":{"type":"regex","rule":{"pattern":"[0-9]","replacement":"0","scope":"all","case_sensitive":true}}})).collect::<Vec<_>>();
        let mut recipe = json!({"schema_version":1,"revision":"2","selection":ids,"numbering_order":ids,"rules":rules,"companions":{"type":"none"},"destination":{"type":"same_directory"},"collision_policy":{"type":"block"},"metadata_changes":[]});
        recipe["selection_filter"] = json!({"exclude":(0..100).map(|_|json!({"field":"name","syntax":"regex","value":"^[a-z0-9-]+x$","case_sensitive":true})).collect::<Vec<_>>()});
        let request = serde_json::from_value(
            json!({"revision":"2","selectedIds":ids,"recipeJson":recipe.to_string()}),
        )
        .unwrap();
        let job = preview_owned(&state, request).unwrap();
        let mut observed = false;
        for _ in 0..5000 {
            let status = status_owned(&state, JobRequest { job_id: job }).unwrap();
            if status.progress.as_ref().is_some_and(|p| {
                p["phase"] == "selecting"
                    && p["done"]
                        .as_str()
                        .and_then(|n| n.parse::<u64>().ok())
                        .is_some_and(|done| done > 0 && done < 1200)
            }) {
                observed = true;
                cancel_owned(&state, JobRequest { job_id: job }).unwrap();
                break;
            }
            if status.state != "running" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            observed,
            "actual CPU preview must expose between-item selection progress before completion"
        );
        for _ in 0..1000 {
            let status = status_owned(&state, JobRequest { job_id: job }).unwrap();
            if status.state != "running" {
                assert_eq!(status.state, "cancelled");
                assert!(status.result.is_none());
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("CPU cancellation did not finish between items");
    }
    #[test]
    fn desktop_summary_includes_hidden_rows_and_native_inventory_kind() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir(&media).unwrap();
        for i in 0..205 {
            std::fs::write(media.join(format!("movie{i}.mkv")), b"x").unwrap();
        }
        let state =
            AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
        crate::selection::install_selection(&state, vec![media]).unwrap();
        await_job(
            &state,
            inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
        );
        let page = inventory_page_owned(
            &state,
            InventoryPageRequest {
                query: PageQuery::new(1),
            },
        )
        .unwrap();
        assert_eq!(page.items[0]["kind"], "file");
        let ids = lock(&state)
            .unwrap()
            .inventory
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .map(|e| e.id)
            .collect::<Vec<_>>();
        let request =
            serde_json::from_value(json!({"revision":"2","selectedIds":ids,"rule":"keep"}))
                .unwrap();
        let result = await_job(&state, preview_owned(&state, request).unwrap());
        assert_eq!(result["summary"]["selected"], "205");
        assert_eq!(result["summary"]["files"], "205");
        assert_eq!(result["summary"]["folders"], "0");
    }
    #[test]
    fn desktop_filter_override_and_scope_changes_revoke_previous_plan() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir(&media).unwrap();
        std::fs::write(media.join("a.mkv"), b"a").unwrap();
        std::fs::write(media.join("b.txt"), b"b").unwrap();
        let state =
            AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
        crate::selection::install_selection(&state, vec![media]).unwrap();
        await_job(
            &state,
            inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
        );
        let ids = lock(&state)
            .unwrap()
            .inventory
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .map(|e| e.id)
            .collect::<Vec<_>>();
        let movie = lock(&state)
            .unwrap()
            .inventory
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|e| e.display_name == "a.mkv")
            .unwrap()
            .id;
        let parsed = serde_json::from_value::<RenameRequestDto>(
            json!({"revision":"2","selectedIds":ids,"rule":"keep","fixedStem":"","selectionFilter":{"extensions":["mkv"],"maxDepth":0,"include":[{"field":"name","syntax":"wildcard","value":"A.*","caseSensitive":false}],"size":{"min":"1","max":"1"}},"numberingScope":"files","rowOverrides":[{"entryId":movie,"newName":"edited.mkv"}]}),
        );
        assert!(
            parsed.is_ok(),
            "selection and row override DTO must be supported: {:?}",
            parsed.err()
        );
        let preview = await_job(&state, preview_owned(&state, parsed.unwrap()).unwrap());
        let reference: UiPlanRef = serde_json::from_value(preview["planRef"].clone()).unwrap();
        let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
        let first = plans(&state)
            .unwrap()
            .snapshot(reference.into(), &lease)
            .unwrap();
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.rows[0].new_display_name, "edited.mkv");
        drop(lease);
        update_inputs_owned(&state, InputRevision { revision: 3 }).unwrap();
        let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
        assert!(plans(&state)
            .unwrap()
            .snapshot(reference.into(), &lease)
            .is_err());
    }
    #[test]
    fn repeated_desktop_preview_retains_frozen_clock_until_inventory_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir(&media).unwrap();
        std::fs::write(media.join("movie.mkv"), b"abc").unwrap();
        let state =
            AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
        crate::selection::install_selection(&state, vec![media]).unwrap();
        await_job(
            &state,
            inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
        );
        let preview = |revision| {
            let selected_ids = lock(&state)
                .unwrap()
                .inventory
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .filter(|e| e.kind == EntryKind::File)
                .map(|e| e.id)
                .collect();
            let value = await_job(
                &state,
                preview_owned(
                    &state,
                    RenameRequestDto {
                        mapping: None,
                        metadata_changes: None,
                        destination: None,
                        recipe_json: None,
                        rebind_selection: false,
                        revision,
                        selected_ids,
                        rule: NamingRule::Keep,
                        fixed_stem: String::new(),
                        selection_filter: None,
                        numbering_scope: Default::default(),
                        row_overrides: vec![],
                        companions: None,
                    },
                )
                .unwrap(),
            );
            let reference: UiPlanRef = serde_json::from_value(value["planRef"].clone()).unwrap();
            let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
            let inputs = plans(&state)
                .unwrap()
                .snapshot(reference.into(), &lease)
                .unwrap()
                .frozen_inputs
                .clone();
            inputs
        };
        let first = preview(2);
        let second = preview(3);
        assert_eq!(
            first.now_ns, second.now_ns,
            "same inventory must retain clock across previews"
        );
        await_job(
            &state,
            inventory_owned(&state, InventoryRequestDto { revision: 4 }).unwrap(),
        );
        let refreshed = preview(5);
        assert_ne!(first.inventory_id, refreshed.inventory_id);
        assert_ne!(first.now_ns, refreshed.now_ns);
    }
    #[test]
    fn desktop_typed_template_freezes_hash_and_generated_values() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir(&media).unwrap();
        std::fs::write(media.join("movie.mkv"), b"abc").unwrap();
        let state =
            AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
        crate::selection::install_selection(&state, vec![media]).unwrap();
        await_job(
            &state,
            inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
        );
        let ids = lock(&state)
            .unwrap()
            .inventory
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .map(|e| e.id)
            .collect::<Vec<_>>();
        let mut previous = None;
        for revision in [2, 3] {
            let parsed = serde_json::from_value::<RenameRequestDto>(
                json!({"revision":revision.to_string(),"selectedIds":ids,"fixedStem":"","rule":{"template":{"parts":[
                    {"type":"field","value":{"key":{"type":"uuid"},"format":{"type":"text"},"fallback":null}},
                    {"type":"literal","value":"_"},
                    {"type":"field","value":{"key":{"type":"hash","algorithm":"md5"},"format":{"type":"text"},"fallback":null}}
                ]}}}),
            );
            assert!(
                parsed.is_ok(),
                "typed template must reach the backend preview: {:?}",
                parsed.err()
            );
            let value = await_job(&state, preview_owned(&state, parsed.unwrap()).unwrap());
            let reference: UiPlanRef = serde_json::from_value(value["planRef"].clone()).unwrap();
            let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
            let plan = plans(&state)
                .unwrap()
                .snapshot(reference.into(), &lease)
                .unwrap();
            assert_eq!(plan.rows[0].status, RowStatus::Changed);
            let name = plan.rows[0].new_display_name.clone();
            assert!(name.ends_with("_900150983cd24fb0d6963f7d28e17f72.mkv"));
            if let Some(previous) = previous {
                assert_eq!(name, previous);
            }
            previous = Some(name);
        }
    }
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
        let id: EntryId = serde_json::from_value(golden["row"]["itemId"].clone()).unwrap();
        let row = PlanRow {
            carried_descendants: 0,
            entry_id: id,
            group_id: GroupId::from_uuid(*id.as_uuid()),
            primary_id: id,
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
    // Emulate a late publisher restoring an entry after the normal clear. The
    // consumer must independently validate both parts of the stored binding.
    #[test]
    fn recovery_consumption_rechecks_revision_and_native_generation() {
        fn done(state: &AppState, id: JobId) -> Value {
            for _ in 0..1000 {
                let status = status_owned(state, JobRequest { job_id: id }).unwrap();
                if status.state != "running" {
                    assert_eq!(status.state, "completed", "{status:?}");
                    return status.result.unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            panic!("job timeout")
        }
        for native in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let media = dir.path().join("media");
            std::fs::create_dir(&media).unwrap();
            std::fs::write(media.join("original.mp4"), b"media").unwrap();
            let state =
                AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
            crate::selection::install_selection(&state, vec![media.clone()]).unwrap();
            done(
                &state,
                inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
            );
            let page = inventory_page_owned(
                &state,
                InventoryPageRequest {
                    query: PageQuery::new(1),
                },
            )
            .unwrap();
            let item = serde_json::from_value(page.items[0]["itemId"].clone()).unwrap();
            let preview = done(
                &state,
                preview_owned(
                    &state,
                    RenameRequestDto {
                        mapping: None,
                        metadata_changes: None,
                        destination: None,
                        recipe_json: None,
                        rebind_selection: false,
                        revision: 2,
                        selected_ids: vec![item],
                        rule: NamingRule::FixedStem,
                        fixed_stem: "renamed".into(),
                        selection_filter: None,
                        numbering_scope: Default::default(),
                        row_overrides: vec![],
                        companions: None,
                    },
                )
                .unwrap(),
            );
            let reference: UiPlanRef = serde_json::from_value(preview["planRef"].clone()).unwrap();
            let plan = {
                let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
                plans(&state)
                    .unwrap()
                    .snapshot(reference.into(), &lease)
                    .unwrap()
            };
            let operation = {
                let _lease = state.coordinator.try_acquire(LeaseKind::Write).unwrap();
                Journal::open(&state.operations)
                    .unwrap()
                    .begin(&plan.execution)
                    .unwrap()
            };
            let review = recovery_preview_owned(
                &state,
                OperationRequest {
                    operation_id: operation,
                },
            )
            .unwrap();
            let reference = serde_json::from_value(review["recoveryRef"].clone()).unwrap();
            let retained = lock(&state).unwrap().reviews.remove(&reference).unwrap();
            if native {
                let replacement = dir.path().join("replacement");
                std::fs::create_dir(&replacement).unwrap();
                crate::selection::install_selection(&state, vec![replacement]).unwrap();
            } else {
                update_inputs_owned(&state, InputRevision { revision: 3 }).unwrap();
            }
            lock(&state).unwrap().reviews.insert(reference, retained);
            assert_eq!(
                recovery_owned(
                    &state,
                    RecoveryRequest {
                        recovery_ref: reference,
                        choice: Choice::Resume,
                        nonce: serde_json::from_value(review["nonce"].clone()).unwrap()
                    }
                )
                .unwrap_err()
                .code,
                "staleState"
            );
            assert!(media.join("original.mp4").exists());
            assert!(!media.join("renamed.mp4").exists());
        }
    }
}
