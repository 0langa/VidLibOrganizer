use crate::recipe::*;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};
use vidlib_inventory::{ids::*, FileStamp, GrantSet, Inventory};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenameErrorCode {
    UnknownRecipeVersion,
    InvalidRecipe,
    RecipeTooLarge,
    TooManyRules,
    UnknownEntry,
    UnselectedEntry,
    OutsideGrant,
    InvalidInventory,
    InvalidNumberingOrder,
    StaleInputs,
    InvalidFrozenInputs,
    UnsupportedRule,
    UnsupportedConfiguration,
    UnsupportedName,
    InvalidPlanRef,
    InvalidConfirmation,
    ArithmeticOverflow,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameError {
    pub code: RenameErrorCode,
    pub message: String,
}
impl RenameError {
    pub fn new(code: RenameErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl fmt::Display for RenameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}
impl std::error::Error for RenameError {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenInputs {
    pub inventory_id: InventoryId,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub generation: u64,
    #[serde(with = "vidlib_inventory::exact::i128_decimal")]
    pub now_ns: i128,
    pub timezone: FrozenTimezone,
    pub primary_order: Vec<EntryId>,
    pub values: Vec<FrozenValue>,
}
/// IANA zones must be confirmed by the input preparation adapter. No local zone
/// lookup, clock read, hashing, metadata probe or randomness occurs in evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FrozenTimezone {
    Utc,
    Iana { name: String },
    FixedOffset { seconds: i32 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenValue {
    pub entry_id: EntryId,
    pub rule_id: Option<String>,
    pub key: FieldKey,
    pub value: FrozenFieldValue,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FrozenFieldValue {
    Text(String),
    Integer(#[serde(with = "vidlib_inventory::exact::i128_decimal")] i128),
    Decimal {
        #[serde(with = "vidlib_inventory::exact::i128_decimal")]
        coefficient: i128,
        scale: u32,
    },
    Timestamp {
        #[serde(with = "vidlib_inventory::exact::i128_decimal")]
        ns: i128,
        timezone: Option<FrozenTimezone>,
    },
    Missing {
        decision: MissingDecision,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MissingDecision {
    Block,
    OmitOptional,
    Fallback(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRef {
    pub session_id: SessionId,
    pub plan_id: PlanId,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub revision: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowStatus {
    Changed,
    Unchanged,
    NeedsReview,
    Blocked,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleExplanation {
    pub rule_id: String,
    pub before: String,
    pub after: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanIssue {
    pub code: RenameErrorCode,
    pub entry_id: Option<EntryId>,
    pub rule_id: Option<String>,
    pub message: String,
}
/// Backend-only. Serialize `to_dto()` for UI; native paths never cross that boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRow {
    pub entry_id: EntryId,
    pub group_id: GroupId,
    pub primary_id: EntryId,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub old_display_name: String,
    pub new_display_name: String,
    pub old_display_path: String,
    pub new_display_path: String,
    pub stamp: FileStamp,
    pub status: RowStatus,
    pub explanations: Vec<RuleExplanation>,
    pub issues: Vec<PlanIssue>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRowDto {
    pub entry_id: EntryId,
    pub group_id: GroupId,
    pub primary_id: EntryId,
    pub old_display_name: String,
    pub new_display_name: String,
    pub old_display_path: String,
    pub new_display_path: String,
    pub size_bytes: Option<String>,
    pub created_ns: Option<String>,
    pub modified_ns: Option<String>,
    pub accessed_ns: Option<String>,
    pub status: RowStatus,
    pub explanations: Vec<RuleExplanation>,
    pub issues: Vec<PlanIssue>,
}
#[derive(Clone, Debug)]
pub struct EvaluatedBatch {
    pub inventory_id: InventoryId,
    pub generation: u64,
    pub recipe_revision: u64,
    pub rows: Vec<PlanRow>,
    pub frozen_inputs: FrozenInputs,
    pub grants: GrantSet,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluatedBatchDto {
    pub inventory_id: InventoryId,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub generation: u64,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub recipe_revision: u64,
    pub rows: Vec<PlanRowDto>,
}
impl EvaluatedBatch {
    pub fn to_dto(&self) -> EvaluatedBatchDto {
        EvaluatedBatchDto {
            inventory_id: self.inventory_id,
            generation: self.generation,
            recipe_revision: self.recipe_revision,
            rows: self.rows.iter().map(PlanRow::to_dto).collect(),
        }
    }
}

/// Exact backend execution data; Task 06 builds dependency groups/steps. Never
/// serializable to UI, and never imports a filesystem executor.
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    pub groups: Vec<ExecutionGroup>,
    pub provenance: InspectionProvenance,
}
#[derive(Clone, Debug)]
pub struct InspectionProvenance {
    pub approved_plan: PlanRef,
    pub inventory: Inventory,
    pub recipe: RecipeV1,
    pub frozen_inputs: FrozenInputs,
    pub grants: GrantSet,
    pub namespace_epoch: u64,
    pub policy_epoch: u64,
}
#[derive(Clone, Debug)]
pub struct ExecutionGroup {
    pub id: GroupId,
    pub depends_on: Vec<GroupId>,
    pub steps: Vec<ExecutionStep>,
}
#[derive(Clone, Debug)]
pub struct ExecutionStep {
    pub id: StepId,
    pub entry_id: EntryId,
    pub group_id: GroupId,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub expected_source: FileStamp,
    pub action: ExecutionAction,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionAction {
    RenameNoReplace,
    CreateDirectory,
    CopyNoReplace,
    VerifyCopy,
    RemoveVerifiedSource,
    SetMetadata { changes: Vec<MetadataChange> },
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCounts {
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub changed: u64,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub unchanged: u64,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub needs_review: u64,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub blocked: u64,
}
#[derive(Clone, Debug)]
pub struct RenamePlan {
    pub version: PlanRef,
    pub inventory_id: InventoryId,
    pub generation: u64,
    pub recipe_revision: u64,
    pub rows: Vec<PlanRow>,
    pub counts: PlanCounts,
    pub issues: Vec<PlanIssue>,
    pub execution: ExecutionPlan,
    pub frozen_inputs: FrozenInputs,
    pub grants: GrantSet,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenamePlanDto {
    pub version: PlanRef,
    pub inventory_id: InventoryId,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub generation: u64,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub recipe_revision: u64,
    pub rows: Vec<PlanRowDto>,
    pub counts: PlanCounts,
    pub issues: Vec<PlanIssue>,
}

impl PlanCounts {
    pub fn from_rows(rows: &[PlanRow]) -> Result<Self, RenameError> {
        let mut counts = Self::default();
        for row in rows {
            let count = match row.status {
                RowStatus::Changed => &mut counts.changed,
                RowStatus::Unchanged => &mut counts.unchanged,
                RowStatus::NeedsReview => &mut counts.needs_review,
                RowStatus::Blocked => &mut counts.blocked,
            };
            *count = count.checked_add(1).ok_or_else(|| {
                RenameError::new(RenameErrorCode::ArithmeticOverflow, "plan count overflow")
            })?;
        }
        Ok(counts)
    }
}
impl RenamePlan {
    pub fn to_dto(&self) -> RenamePlanDto {
        RenamePlanDto {
            version: self.version,
            inventory_id: self.inventory_id,
            generation: self.generation,
            recipe_revision: self.recipe_revision,
            rows: self.rows.iter().map(PlanRow::to_dto).collect(),
            counts: self.counts.clone(),
            issues: self.issues.clone(),
        }
    }
}

/// Session-owned immutable published plans. Adapters must invalidate on inventory,
/// recipe or selection changes and create a fresh session after process restart.
/// Supplied revisions increase monotonically even across invalidation; old refs
/// cannot be republished. Nonces come from a trusted backend adapter, never UI.
#[derive(Debug)]
pub struct PlanStore {
    session_id: SessionId,
    last_revision: Option<u64>,
    current: Option<RenamePlan>,
    confirmation: Option<(PlanRef, String)>,
    used_nonces: std::collections::BTreeSet<String>,
}
impl PlanStore {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            last_revision: None,
            current: None,
            confirmation: None,
            used_nonces: Default::default(),
        }
    }
    pub fn publish(&mut self, plan: RenamePlan) -> Result<(), RenameError> {
        let provenance = &plan.execution.provenance;
        if plan.version.session_id != self.session_id
            || self
                .last_revision
                .is_some_and(|revision| plan.version.revision <= revision)
            || provenance.approved_plan != plan.version
            || provenance.inventory.id != plan.inventory_id
            || provenance.inventory.generation != plan.generation
            || provenance.recipe.revision != plan.recipe_revision
            || provenance.frozen_inputs != plan.frozen_inputs
            || plan.frozen_inputs.inventory_id != plan.inventory_id
            || plan.frozen_inputs.generation != plan.generation
            || plan.counts != PlanCounts::from_rows(&plan.rows)?
        {
            return Err(RenameError::new(
                RenameErrorCode::InvalidPlanRef,
                "plan session, revision or provenance mismatch",
            ));
        }
        self.last_revision = Some(plan.version.revision);
        self.confirmation = None;
        self.current = Some(plan);
        Ok(())
    }
    pub fn current(&self, version: PlanRef) -> Result<&RenamePlan, RenameError> {
        self.current
            .as_ref()
            .filter(|plan| plan.version == version)
            .ok_or_else(|| {
                RenameError::new(
                    RenameErrorCode::InvalidPlanRef,
                    "plan is stale or belongs to another session",
                )
            })
    }
    pub fn confirm(&mut self, version: PlanRef, nonce: String) -> Result<(), RenameError> {
        self.current(version)?;
        if nonce.is_empty() || !self.used_nonces.insert(nonce.clone()) {
            return Err(RenameError::new(
                RenameErrorCode::InvalidConfirmation,
                "nonce is empty or already used",
            ));
        }
        self.confirmation = Some((version, nonce));
        Ok(())
    }
    pub fn consume_confirmation(
        &mut self,
        version: PlanRef,
        nonce: &str,
    ) -> Result<&RenamePlan, RenameError> {
        self.current(version)?;
        if !self
            .confirmation
            .as_ref()
            .is_some_and(|(confirmed, expected)| *confirmed == version && expected == nonce)
        {
            return Err(RenameError::new(
                RenameErrorCode::InvalidConfirmation,
                "confirmation is missing or invalid",
            ));
        }
        self.confirmation = None;
        self.current(version)
    }
    pub fn invalidate(&mut self) {
        self.current = None;
        self.confirmation = None;
    }
}

impl PlanRow {
    pub fn to_dto(&self) -> PlanRowDto {
        PlanRowDto {
            entry_id: self.entry_id,
            group_id: self.group_id,
            primary_id: self.primary_id,
            old_display_name: self.old_display_name.clone(),
            new_display_name: self.new_display_name.clone(),
            old_display_path: self.old_display_path.clone(),
            new_display_path: self.new_display_path.clone(),
            size_bytes: self.stamp.size.map(|v| v.to_string()),
            created_ns: self.stamp.created_ns.map(|v| v.to_string()),
            modified_ns: self.stamp.modified_ns.map(|v| v.to_string()),
            accessed_ns: self.stamp.accessed_ns.map(|v| v.to_string()),
            status: self.status,
            explanations: self.explanations.clone(),
            issues: self.issues.clone(),
        }
    }
}
