//! Typed, read-only metadata boundary. Producers live in desktop adapters.
use crate::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use vidlib_inventory::JobControl;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "property",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PropertyKey {
    Media(MediaProperty),
    Recorded,
    Windows(WindowsProperty),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "zone",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TimezoneEvidence {
    Unknown,
    NotApplicable,
    Known(FrozenTimezone),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataIssueCode {
    Timeout,
    Cancelled,
    OutputLimit,
    InvalidData,
    Unavailable,
    Io,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataIssue {
    pub code: MetadataIssueCode,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MetadataState {
    Value(FrozenFieldValue),
    Missing,
    Unsupported,
    Error(MetadataIssue),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataValue {
    pub value: MetadataState,
    pub source_key: String,
    pub provider: String,
    pub version: String,
    pub timezone: TimezoneEvidence,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataRecord {
    pub values: BTreeMap<PropertyKey, MetadataValue>,
}
pub trait MetadataProvider: Send + Sync {
    fn version(&self) -> &str;
    fn probe(
        &self,
        path: &Path,
        fields: &[PropertyKey],
        control: &JobControl,
    ) -> Result<MetadataRecord, MetadataIssue>;
}
impl FrozenFieldValue {
    pub fn resolved(&self) -> Self {
        match self {
            Self::Metadata(value) => match &value.value {
                MetadataState::Value(value) if !matches!(value, Self::Metadata(_)) => value.clone(),
                _ => Self::Missing {
                    decision: MissingDecision::Block,
                },
            },
            value => value.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataCacheKey {
    pub physical_id: Option<vidlib_inventory::PhysicalId>,
    pub path: std::path::PathBuf,
    pub size: Option<u64>,
    pub modified_ns: Option<i128>,
    pub provider_version: String,
    pub fields: Vec<PropertyKey>,
}
#[derive(Clone)]
pub(crate) struct CacheEntry {
    key: MetadataCacheKey,
    record: MetadataRecord,
}
impl PropertyKey {
    pub fn from_field(key: &FieldKey) -> Option<Self> {
        match key {
            FieldKey::WindowsProperty { property } => Some(Self::Windows(*property)),
            FieldKey::Media { property } => Some(Self::Media(*property)),
            FieldKey::Date {
                source: DateSource::Recorded,
            } => Some(Self::Recorded),
            _ => None,
        }
    }
}
impl FrozenInputCache {
    /// Produces requested metadata under native inventory verification, publishing
    /// only after all preparation succeeds. Generated UUID/random/clock data survive.
    pub fn prepare_with_metadata(
        &mut self,
        request: &InputRequirements,
        inventory: &vidlib_inventory::Inventory,
        control: &JobControl,
        provider: &dyn MetadataProvider,
    ) -> Result<FrozenInputs, RenameError> {
        let mut next = self.clone();
        let mut output = next.prepare(request, inventory, control)?;
        let mut fields = BTreeMap::<
            vidlib_inventory::ids::EntryId,
            std::collections::BTreeSet<PropertyKey>,
        >::new();
        for field in &request.fields {
            if let Some(key) = PropertyKey::from_field(&field.key) {
                fields.entry(field.entry_id).or_default().insert(key);
            }
        }
        for (id, fields) in fields {
            let entry = inventory
                .entries
                .iter()
                .find(|e| e.id == id)
                .expect("prepare validates IDs");
            if entry.kind != vidlib_inventory::EntryKind::File {
                continue;
            }
            let key = MetadataCacheKey {
                physical_id: entry.stamp.physical_id.clone(),
                path: entry.absolute_path.clone(),
                size: entry.stamp.size,
                modified_ns: entry.stamp.modified_ns,
                provider_version: provider.version().into(),
                fields: fields.into_iter().collect(),
            };
            let old = next
                .metadata_cache
                .iter()
                .find(|cached| cached.key == key)
                .map(|cached| cached.record.clone());
            let record = vidlib_inventory::with_verified_helper_path(inventory, id, |safe_path| {
                if control.is_cancelled() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "metadata cancelled",
                    ));
                }
                Ok(old.unwrap_or_else(|| {
                    provider
                        .probe(safe_path, &key.fields, control)
                        .unwrap_or_else(|error| MetadataRecord {
                            values: key
                                .fields
                                .iter()
                                .map(|field| {
                                    (
                                        field.clone(),
                                        MetadataValue {
                                            value: MetadataState::Error(error.clone()),
                                            source_key: format!("{field:?}"),
                                            provider: "metadata".into(),
                                            version: provider.version().into(),
                                            timezone: TimezoneEvidence::Unknown,
                                        },
                                    )
                                })
                                .collect(),
                        })
                }))
            })
            .map_err(|e| {
                RenameError::new(
                    if control.is_cancelled() {
                        RenameErrorCode::Cancelled
                    } else if e.kind() == std::io::ErrorKind::Unsupported {
                        RenameErrorCode::InvalidFrozenInputs
                    } else {
                        RenameErrorCode::StaleInputs
                    },
                    e.to_string(),
                )
            })?;
            if control.is_cancelled() {
                return Err(RenameError::new(
                    RenameErrorCode::Cancelled,
                    "metadata cancelled",
                ));
            }
            // Failed probes remain retryable; never freeze transient error placeholders.
            if !record
                .values
                .values()
                .any(|v| matches!(v.value, MetadataState::Error(_)))
            {
                next.metadata_cache
                    .retain(|cached| cached.key.path != key.path);
                next.metadata_cache.push(CacheEntry {
                    key: key.clone(),
                    record: record.clone(),
                });
            }
            for value in output.values.iter_mut().filter(|v| v.entry_id == id) {
                let Some(field) = PropertyKey::from_field(&value.key) else {
                    continue;
                };
                // Explicit user fallback/optional choices retain authority over automatic inputs.
                if matches!(
                    value.value,
                    FrozenFieldValue::Missing {
                        decision: MissingDecision::Fallback(_) | MissingDecision::OmitOptional
                    }
                ) {
                    continue;
                }
                let metadata = record.values.get(&field).cloned().unwrap_or(MetadataValue {
                    value: MetadataState::Missing,
                    source_key: format!("{field:?}"),
                    provider: "metadata".into(),
                    version: provider.version().into(),
                    timezone: TimezoneEvidence::Unknown,
                });
                value.value = FrozenFieldValue::Metadata(Box::new(metadata));
                let retained = next.retained.as_mut().expect("prepared retention");
                retained.values.retain(|old| {
                    !(old.scope == value.scope
                        && old.entry_id == value.entry_id
                        && old.rule_id == value.rule_id
                        && old.key == value.key)
                });
                retained.values.push(value.clone());
            }
        }
        *self = next;
        Ok(output)
    }
}

impl MetadataValue {
    pub(crate) fn missing_issue(&self) -> RuleIssue {
        let detail = match &self.value {
            MetadataState::Error(error) => format!("{:?}: {}", error.code, error.message),
            MetadataState::Unsupported => "field is unsupported".into(),
            _ => "field is missing".into(),
        };
        RuleIssue {
            code: RuleIssueCode::MetadataMissing,
            native_code: None,
            message: format!(
                "{} {} [{}]: {}",
                self.provider, self.version, self.source_key, detail
            ),
        }
    }
}

/// Opt-in controls. Keep has no compiled effect and never appears in summaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeChange {
    #[default]
    Keep,
    Set,
    Clear,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimestampChange {
    #[default]
    Keep,
    Set {
        #[serde(with = "vidlib_inventory::exact::i128_decimal")]
        value_ns: i128,
    },
    Offset {
        #[serde(with = "vidlib_inventory::exact::i128_decimal")]
        offset_ns: i128,
    },
    Increment {
        #[serde(with = "vidlib_inventory::exact::i128_decimal")]
        start_ns: i128,
        #[serde(with = "vidlib_inventory::exact::i128_decimal")]
        step_ns: i128,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetadataChangeSet {
    pub hidden: AttributeChange,
    pub read_only: AttributeChange,
    pub archive: AttributeChange,
    pub system: AttributeChange,
    pub created: TimestampChange,
    pub modified: TimestampChange,
    pub accessed: TimestampChange,
}
impl MetadataChangeSet {
    pub fn changes(&self) -> Vec<MetadataChange> {
        let mut out = Vec::new();
        let (mut set, mut clear) = (0, 0);
        for (action, bit) in [
            (self.read_only, 1),
            (self.hidden, 2),
            (self.system, 4),
            (self.archive, 32),
        ] {
            match action {
                AttributeChange::Keep => {}
                AttributeChange::Set => set |= bit,
                AttributeChange::Clear => clear |= bit,
            }
        }
        if set | clear != 0 {
            out.push(MetadataChange::WindowsAttributes { set, clear });
        }
        for (field, action) in [
            (TimestampField::Created, &self.created),
            (TimestampField::Modified, &self.modified),
            (TimestampField::Accessed, &self.accessed),
        ] {
            match action {
                TimestampChange::Keep => {}
                TimestampChange::Set { value_ns } => out.push(MetadataChange::Timestamp {
                    field,
                    value_ns: *value_ns,
                }),
                TimestampChange::Offset { offset_ns } => {
                    out.push(MetadataChange::TimestampOffset {
                        field,
                        offset_ns: *offset_ns,
                    })
                }
                TimestampChange::Increment { start_ns, step_ns } => {
                    out.push(MetadataChange::TimestampIncrement {
                        field,
                        start_ns: *start_ns,
                        step_ns: *step_ns,
                    })
                }
            }
        }
        out
    }
}
pub fn timestamp(stamp: &vidlib_inventory::FileStamp, field: TimestampField) -> Option<i128> {
    match field {
        TimestampField::Created => stamp.created_ns,
        TimestampField::Modified => stamp.modified_ns,
        TimestampField::Accessed => stamp.accessed_ns,
    }
}
/// Resolve intent once from immutable original facts and the full primary vector.
/// Only exact Set intents enter the execution journal; historical Set is unchanged.
pub fn resolved_changes(
    row: &PlanRow,
    recipe: &RecipeV1,
    preserve_equal: bool,
) -> Result<Vec<MetadataChange>, RenameError> {
    let ordinal = recipe
        .metadata_changes
        .iter()
        .any(|change| matches!(change, MetadataChange::TimestampIncrement { .. }))
        .then(|| {
            recipe
                .numbering_order
                .iter()
                .position(|id| *id == row.primary_id)
        })
        .flatten();
    resolved_changes_at(row, recipe, preserve_equal, ordinal)
}
fn resolved_changes_at(
    row: &PlanRow,
    recipe: &RecipeV1,
    preserve_equal: bool,
    ordinal: Option<usize>,
) -> Result<Vec<MetadataChange>, RenameError> {
    let invalid = |message| RenameError::new(RenameErrorCode::InvalidRuleInput, message);
    let mut out = Vec::new();
    for change in &recipe.metadata_changes {
        let resolved = match change {
            MetadataChange::TimestampOffset { field, offset_ns } => MetadataChange::Timestamp {
                field: *field,
                value_ns: timestamp(&row.stamp, *field)
                    .ok_or_else(|| invalid("metadata offset original timestamp unavailable"))?
                    .checked_add(*offset_ns)
                    .ok_or_else(|| invalid("metadata offset overflow"))?,
            },
            MetadataChange::TimestampIncrement {
                field,
                start_ns,
                step_ns,
            } => {
                let ordinal = ordinal.ok_or_else(|| {
                    invalid("metadata increment requires resolved frozen primary ordinal")
                })? as i128;
                MetadataChange::Timestamp {
                    field: *field,
                    value_ns: step_ns
                        .checked_mul(ordinal)
                        .and_then(|n| start_ns.checked_add(n))
                        .ok_or_else(|| invalid("metadata increment overflow"))?,
                }
            }
            other => other.clone(),
        };
        match resolved {
            MetadataChange::Timestamp { field, value_ns }
                if !preserve_equal && timestamp(&row.stamp, field) == Some(value_ns) => {}
            MetadataChange::WindowsAttributes { set, clear } => {
                // Each flag is independently intended/receipted; unknown bits survive.
                for bit in [1, 2, 4, 32] {
                    if set & bit != 0 && row.stamp.windows_attributes.is_none_or(|a| a & bit == 0) {
                        out.push(MetadataChange::WindowsAttributes { set: bit, clear: 0 });
                    }
                    if clear & bit != 0 && row.stamp.windows_attributes.is_none_or(|a| a & bit != 0)
                    {
                        out.push(MetadataChange::WindowsAttributes { set: 0, clear: bit });
                    }
                }
            }
            other => out.push(other),
        }
    }
    Ok(out)
}
// Per-call indexes preserve the original first ordinal and any-directory match.
// They do not become persisted native identity or metadata authority.
struct ChangeIndex {
    ordinals: BTreeMap<vidlib_inventory::ids::EntryId, usize>,
    directories: std::collections::BTreeSet<vidlib_inventory::ids::EntryId>,
}
impl ChangeIndex {
    fn new(inventory: &vidlib_inventory::Inventory, recipe: &RecipeV1) -> Self {
        let mut ordinals = BTreeMap::new();
        for (ordinal, id) in recipe.numbering_order.iter().enumerate() {
            ordinals.entry(*id).or_insert(ordinal);
        }
        Self {
            ordinals,
            directories: inventory
                .entries
                .iter()
                .filter(|e| e.kind == vidlib_inventory::EntryKind::Directory)
                .map(|e| e.id)
                .collect(),
        }
    }
    fn resolve(
        &self,
        row: &PlanRow,
        recipe: &RecipeV1,
    ) -> Result<Vec<MetadataChange>, RenameError> {
        resolved_changes_at(
            row,
            recipe,
            self.directories.contains(&row.entry_id),
            self.ordinals.get(&row.primary_id).copied(),
        )
    }
}
pub(crate) fn evaluate_changes(
    batch: &mut EvaluatedBatch,
    control: &JobControl,
) -> Result<(), RenameError> {
    let total = batch.rows.len();
    crate::evaluate::checkpoint(
        control,
        vidlib_inventory::ProgressPhase::Evaluating,
        0,
        total,
        None,
    )?;
    if batch.recipe.metadata_changes.is_empty() {
        return Ok(());
    }
    let index = ChangeIndex::new(&batch.inventory, &batch.recipe);
    for (done, row) in batch.rows.iter_mut().enumerate() {
        crate::evaluate::checkpoint(
            control,
            vidlib_inventory::ProgressPhase::Evaluating,
            done,
            total,
            Some(row.entry_id),
        )?;
        if matches!(row.status, RowStatus::Blocked | RowStatus::NeedsReview) {
            continue;
        }
        match index.resolve(row, &batch.recipe) {
            Ok(changes) if !changes.is_empty() => {
                row.status = RowStatus::Changed;
                for change in changes {
                    row.explanations.push(RuleExplanation {
                        rule_id: "filesystem-metadata".into(),
                        before: original_field(&row.stamp, &change).to_string(),
                        after: serde_json::to_string(&change).expect("intent serializes"),
                    });
                }
            }
            Ok(_) => {}
            Err(error) => crate::validate::issue(row, error.code, &error.message),
        }
    }
    Ok(())
}
/// Append field effects after path/content effects. Each field's original and
/// expected state is the exact observed prestate or named preceding receipt.
pub fn compile_changes(
    inventory: &vidlib_inventory::Inventory,
    rows: &mut [PlanRow],
    recipe: &RecipeV1,
    groups: &mut Vec<ExecutionGroup>,
) -> Result<(), RenameError> {
    if recipe.metadata_changes.is_empty() {
        return Ok(());
    }
    let index = ChangeIndex::new(inventory, recipe);
    let mut ordinal = groups.iter().map(|g| g.steps.len()).sum::<usize>() as u64;
    let mut order: Vec<_> = (0..rows.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(rows[i].target_path.components().count()));
    for row_index in order {
        let row = &mut rows[row_index];
        if matches!(row.status, RowStatus::Blocked | RowStatus::NeedsReview) {
            continue;
        }
        let changes = index.resolve(row, recipe)?;
        if changes.is_empty() {
            continue;
        }
        row.status = RowStatus::Changed;
        let group_index = groups
            .iter()
            .position(|g| g.steps.iter().any(|s| s.entry_id == row.entry_id))
            .or_else(|| groups.iter().position(|g| g.steps.iter().any(|step| {
                index.directories.contains(&step.entry_id)
                    && matches!(&step.action, ExecutionAction::RenameEntry { source, target, .. } if row.source_path.starts_with(&source.path) || row.target_path.starts_with(target))
            })))
            .or_else(|| groups.iter().position(|g| g.id == row.group_id))
            .unwrap_or_else(|| {
                groups.push(ExecutionGroup {
                    id: row.group_id,
                    depends_on: vec![],
                    steps: vec![],
                });
                groups.len() - 1
            });
        let group = &mut groups[group_index];
        row.group_id = group.id;
        let directory = index.directories.contains(&row.entry_id);
        let last = group.steps.iter().rev().find(|s| match &s.action {
            ExecutionAction::RenameEntry { target, .. }
            | ExecutionAction::PublishCopy { target, .. }
            | ExecutionAction::CreateDirectory { target } => *target == row.target_path,
            ExecutionAction::SetMetadata { entry, .. } => entry.path == row.target_path,
            _ => false,
        });
        let mut identity = if directory {
            let created_by = group.steps.iter().find_map(|s| match &s.action {
                ExecutionAction::CreateDirectory { target } if *target == row.target_path => {
                    Some(s.id)
                }
                _ => None,
            });
            // Preserve the stronger native-grant/full-namespace directory proof.
            ExpectedIdentity::DirectoryAfterEffects {
                original: row.stamp.clone(),
                created_by,
                effects: group.steps.iter().map(|s| s.id).collect(),
            }
        } else {
            last.map_or_else(
                || ExpectedIdentity::Observed(row.stamp.clone()),
                |s| ExpectedIdentity::FromReceipt { step_id: s.id },
            )
        };
        for change in changes {
            ordinal += 1;
            let id = format!("00000000-0000-0000-0001-{ordinal:012x}")
                .parse()
                .map_err(|_| {
                    RenameError::new(RenameErrorCode::InvalidRuleInput, "metadata step overflow")
                })?;
            group.steps.push(ExecutionStep {
                id,
                entry_id: row.entry_id,
                group_id: group.id,
                action: ExecutionAction::SetMetadata {
                    entry: EntryPrestate {
                        path: row.target_path.clone(),
                        identity,
                    },
                    changes: vec![change],
                },
            });
            identity = ExpectedIdentity::FromReceipt { step_id: id };
        }
    }
    Ok(())
}

fn original_field(
    stamp: &vidlib_inventory::FileStamp,
    change: &MetadataChange,
) -> serde_json::Value {
    match change {
        MetadataChange::Timestamp { field, .. } => {
            serde_json::json!({"field": field, "value_ns": timestamp(stamp, *field).map(|n| n.to_string())})
        }
        MetadataChange::WindowsAttributes { set, clear } => {
            serde_json::json!({"mask": set | clear, "value": stamp.windows_attributes.map(|bits| bits & (set | clear))})
        }
        _ => unreachable!("summaries use resolved intents"),
    }
}

#[cfg(test)]
mod tail_tests {
    use super::*;
    use vidlib_inventory::*;

    fn batch() -> (tempfile::TempDir, EvaluatedBatch) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("movie.mkv"), b"x").unwrap();
        let inventory = scan(
            &InventoryRequest::new(vec![RootBinding::new(dir.path().to_path_buf()).unwrap()]),
            &JobControl::default(),
        )
        .unwrap();
        let id = inventory.entries[0].id;
        let recipe: RecipeV1 = serde_json::from_value(serde_json::json!({
            "schema_version":1,"revision":"1","selection":[id],"numbering_order":[id],
            "rules":[],"metadata_changes":[],"companions":{"type":"none"},"destination":{"type":"same_directory"},"collision_policy":{"type":"block"}
        })).unwrap();
        let inputs = FrozenInputCache::default()
            .prepare(
                &InputRequirements::from_recipe(&recipe, FrozenTimezone::Utc).unwrap(),
                &inventory,
                &JobControl::default(),
            )
            .unwrap();
        let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
        (dir, batch)
    }

    #[test]
    fn metadata_tail_preserves_first_primary_ordinal_companions_and_explicit_directory_set() {
        let (_dir, mut batch) = batch();
        let first = batch.rows[0].entry_id;
        let second = vidlib_inventory::ids::EntryId::new();
        let companion = vidlib_inventory::ids::EntryId::new();
        let mut directory = batch.inventory.entries[0].clone();
        directory.id = second;
        directory.kind = EntryKind::Directory;
        batch.inventory.entries.push(directory);
        let sample = batch.rows[0].clone();
        batch.rows = vec![sample.clone(), sample.clone(), sample];
        batch.rows[1].entry_id = second;
        batch.rows[1].primary_id = second;
        batch.rows[2].entry_id = companion;
        batch.rows[2].primary_id = first;
        batch.recipe.numbering_order = vec![second, first, second];
        batch.recipe.metadata_changes = vec![
            MetadataChange::TimestampIncrement {
                field: TimestampField::Created,
                start_ns: 100,
                step_ns: 7,
            },
            MetadataChange::Timestamp {
                field: TimestampField::Modified,
                value_ns: batch.rows[0].stamp.modified_ns.unwrap(),
            },
        ];
        evaluate_changes(&mut batch, &JobControl::default()).unwrap();
        assert_eq!(
            batch.rows.iter().map(|r| r.entry_id).collect::<Vec<_>>(),
            vec![first, second, companion]
        );
        for (row, expected, effects) in [
            (&batch.rows[0], "107", 1),
            (&batch.rows[1], "100", 2),
            (&batch.rows[2], "107", 1),
        ] {
            assert_eq!(row.status, RowStatus::Changed);
            assert_eq!(row.explanations.len(), effects);
            let intent: serde_json::Value =
                serde_json::from_str(&row.explanations[0].after).unwrap();
            assert_eq!(intent["value_ns"], expected);
        }
        batch.recipe.metadata_changes.clear();
        let before = batch.rows.clone();
        evaluate_changes(&mut batch, &JobControl::default()).unwrap();
        assert_eq!(batch.rows, before, "Keep must add no metadata intent");
    }

    #[test]
    fn cancellation_interrupts_actual_metadata_tail_after_partial_work() {
        let (_dir, mut batch) = batch();
        batch.rows = vec![batch.rows[0].clone(); 30_000];
        batch.recipe.metadata_changes = [
            TimestampField::Created,
            TimestampField::Modified,
            TimestampField::Accessed,
        ]
        .into_iter()
        .map(|field| MetadataChange::TimestampIncrement {
            field,
            start_ns: 100,
            step_ns: 3,
        })
        .collect();
        let (control, updates) = JobControl::with_progress(16);
        let worker_control = control.clone();
        let worker = std::thread::spawn(move || {
            let result = evaluate_changes(&mut batch, &worker_control);
            (result, batch)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut partial = false;
        while std::time::Instant::now() < deadline {
            if let Ok(update) = updates.recv_timeout(std::time::Duration::from_millis(100)) {
                if update.done > 0 && update.done < update.total.unwrap() {
                    partial = true;
                    control.cancel();
                    break;
                }
            }
            if worker.is_finished() {
                break;
            }
        }
        let (result, batch) = worker.join().unwrap();
        assert!(
            partial,
            "actual metadata tail must publish progress before finishing"
        );
        assert_eq!(result.unwrap_err().code, RenameErrorCode::Cancelled);
        let processed = batch
            .rows
            .iter()
            .filter(|row| !row.explanations.is_empty())
            .count();
        assert!(
            processed > 0 && processed < batch.rows.len(),
            "cancellation must interrupt the tail between metadata rows"
        );
    }
}
