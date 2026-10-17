//! Private on-disk representations: native units and exact quantities, never UI DTOs.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vidlib_rename::{
    EntryPrestate, ExecutionAction, ExpectedIdentity, MetadataChange, OwnedEntryPrestate,
};

pub(super) const VERSION: i64 = 3;
pub(super) const SQL: &str = r#"
CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, checksum TEXT NOT NULL);
CREATE TABLE operations(id TEXT PRIMARY KEY, state TEXT NOT NULL CHECK(state IN ('Prepared','Running','Completed','Cancelled','Failed','RecoveryRequired')), inspection TEXT NOT NULL, groups_json TEXT NOT NULL);
CREATE TABLE steps(operation_id TEXT NOT NULL REFERENCES operations(id), id TEXT NOT NULL, group_id TEXT NOT NULL, sequence INTEGER NOT NULL, state TEXT NOT NULL CHECK(state IN ('Planned','IntentDurable','CompletionDurable','Compensated','Unresolved')), compiled TEXT NOT NULL, PRIMARY KEY(operation_id,id), UNIQUE(operation_id,sequence));
CREATE TABLE receipts(operation_id TEXT NOT NULL, step_id TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(operation_id,step_id), FOREIGN KEY(operation_id,step_id) REFERENCES steps(operation_id,id));
CREATE TABLE operation_events(sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE, operation_id TEXT NOT NULL, step_id TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(operation_id,step_id), FOREIGN KEY(operation_id,step_id) REFERENCES receipts(operation_id,step_id));
CREATE TABLE outbox_ack(event_id TEXT PRIMARY KEY REFERENCES operation_events(id));
CREATE TABLE history_visibility(operation_id TEXT PRIMARY KEY REFERENCES operations(id), visible INTEGER NOT NULL DEFAULT 1 CHECK(visible IN (0,1)));
"#;

// Keep v1 SQL/checksum immutable so existing journals can upgrade transactionally.
pub(super) const V2_SQL: &str = r#"
ALTER TABLE operations ADD COLUMN approved_plan TEXT NOT NULL DEFAULT '';
UPDATE operations SET approved_plan = json_extract(inspection, '$.approved_plan');
CREATE INDEX operation_effect_header ON operations(id,state,approved_plan);
CREATE INDEX unfinished_steps ON steps(operation_id,sequence) WHERE state IN ('Planned','IntentDurable','Unresolved');
"#;

// Epochs stay in the operation store. Decimal text preserves the full u64 range.
pub(super) const V3_SQL: &str = r#"
CREATE TABLE coordination(singleton INTEGER PRIMARY KEY CHECK(singleton=1), namespace_epoch TEXT NOT NULL, policy_epoch TEXT NOT NULL, interrupted_write INTEGER NOT NULL CHECK(interrupted_write IN (0,1)));
INSERT INTO coordination VALUES(1,'0','0',0);
CREATE INDEX pending_operations ON operations(state) WHERE state IN ('Prepared','Running','RecoveryRequired');
"#;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredStep {
    pub id: StepId,
    pub entry_id: EntryId,
    pub group_id: GroupId,
    #[serde(with = "ActionCodec")]
    pub action: ExecutionAction,
}
impl From<&ExecutionStep> for StoredStep {
    fn from(s: &ExecutionStep) -> Self {
        Self {
            id: s.id,
            entry_id: s.entry_id,
            group_id: s.group_id,
            action: s.action.clone(),
        }
    }
}
impl From<StoredStep> for ExecutionStep {
    fn from(s: StoredStep) -> Self {
        Self {
            id: s.id,
            entry_id: s.entry_id,
            group_id: s.group_id,
            action: s.action,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ExecutionAction", deny_unknown_fields)]
enum ActionCodec {
    RenameEntry {
        #[serde(with = "EntryCodec")]
        source: EntryPrestate,
        #[serde(with = "native")]
        target: PathBuf,
        temporary: bool,
    },
    CreateDirectory {
        #[serde(with = "native")]
        target: PathBuf,
    },
    StageCopy {
        #[serde(with = "EntryCodec")]
        source: EntryPrestate,
        #[serde(with = "native")]
        staging: PathBuf,
    },
    PublishCopy {
        #[serde(with = "OwnedCodec")]
        staging: OwnedEntryPrestate,
        #[serde(with = "native")]
        target: PathBuf,
        verified_by: StepId,
    },
    RemoveSource {
        #[serde(with = "EntryCodec")]
        source: EntryPrestate,
        published_by: StepId,
    },
    SetMetadata {
        #[serde(with = "EntryCodec")]
        entry: EntryPrestate,
        changes: Vec<MetadataChange>,
    },
    RemoveOwnedCopy {
        #[serde(with = "OwnedCodec")]
        owned: OwnedEntryPrestate,
    },
    RemoveOwnedEmptyDirectory {
        #[serde(with = "OwnedCodec")]
        owned: OwnedEntryPrestate,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "EntryPrestate", deny_unknown_fields)]
struct EntryCodec {
    #[serde(with = "native")]
    path: PathBuf,
    #[serde(with = "IdentityCodec")]
    identity: ExpectedIdentity,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ExpectedIdentity", deny_unknown_fields)]
enum IdentityCodec {
    Observed(FileStamp),
    FromReceipt { step_id: StepId },
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "OwnedEntryPrestate", deny_unknown_fields)]
struct OwnedCodec {
    #[serde(with = "EntryCodec")]
    entry: EntryPrestate,
    created_by: StepId,
}
mod native {
    use super::*;
    pub fn serialize<S: serde::Serializer>(path: &Path, s: S) -> Result<S::Ok, S::Error> {
        vidlib_inventory::encode_path(path).serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        vidlib_inventory::decode_path(&NativePathRecord::deserialize(d)?)
            .map_err(serde::de::Error::custom)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "Durability")]
enum DurabilityCodec {
    Synced,
    Unsupported,
    Failed(String),
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "StepReceipt", deny_unknown_fields)]
pub(super) struct ReceiptCodec {
    step_id: StepId,
    phase: CompletedPhase,
    before_path: Option<NativePathRecord>,
    after_path: Option<NativePathRecord>,
    before: Option<FileStamp>,
    after: Option<FileStamp>,
    digest: Option<ContentDigest>,
    #[serde(with = "vidlib_inventory::exact::i128_decimal")]
    observed_ns: i128,
    #[serde(with = "DurabilityCodec")]
    durability: Durability,
}
#[derive(Serialize, Deserialize)]
pub(super) struct StoredReceipt(#[serde(with = "ReceiptCodec")] pub StepReceipt);
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredGroup {
    pub id: GroupId,
    pub depends_on: Vec<GroupId>,
}

pub(super) fn inspection(plan: &ExecutionPlan) -> Result<serde_json::Value, OpError> {
    use serde_json::json;
    use vidlib_inventory::encode_path;
    let p = &plan.provenance;
    // The inventory's public Serialize intentionally omits absolute paths. Keep
    // exact native supplements here for inspection; never reconstruct from UI text.
    Ok(json!({
        "approved_plan": p.approved_plan, "inventory": p.inventory, "recipe": p.recipe,
        "frozen_inputs": p.frozen_inputs, "namespace_epoch": p.namespace_epoch.to_string(), "policy_epoch": p.policy_epoch.to_string(),
        "grants": p.grants.grants.iter().map(|g| json!({"id": g.id, "path": encode_path(&g.path), "source": g.source})).collect::<Vec<_>>(),
        "native_roots": p.inventory.roots.iter().map(|r| json!({"id":r.id, "path": encode_path(&r.path), "grant_path": encode_path(&r.source_grant.path)})).collect::<Vec<_>>(),
        "native_entries": p.inventory.entries.iter().map(|e| json!({"id": e.id, "path": encode_path(&e.absolute_path)})).collect::<Vec<_>>(),
        "native_coverage": p.inventory.coverage.iter().map(|c| encode_path(&c.directory)).collect::<Vec<_>>()
    }))
}

pub(super) fn source(action: &ExecutionAction) -> Option<&EntryPrestate> {
    match action {
        ExecutionAction::RenameEntry { source, .. }
        | ExecutionAction::StageCopy { source, .. }
        | ExecutionAction::RemoveSource { source, .. } => Some(source),
        ExecutionAction::SetMetadata { entry, .. } => Some(entry),
        ExecutionAction::PublishCopy { staging, .. } => Some(&staging.entry),
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => Some(&owned.entry),
        ExecutionAction::CreateDirectory { .. } => None,
    }
}
pub(super) fn output(action: &ExecutionAction) -> Option<&Path> {
    match action {
        ExecutionAction::RenameEntry { target, .. }
        | ExecutionAction::CreateDirectory { target }
        | ExecutionAction::PublishCopy { target, .. } => Some(target),
        ExecutionAction::StageCopy { staging, .. } => Some(staging),
        ExecutionAction::SetMetadata { entry, .. } => Some(&entry.path),
        _ => None,
    }
}
pub(super) fn owned(action: &ExecutionAction) -> Option<&OwnedEntryPrestate> {
    match action {
        ExecutionAction::PublishCopy { staging, .. } => Some(staging),
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => Some(owned),
        _ => None,
    }
}
pub(super) fn phase(action: &ExecutionAction) -> CompletedPhase {
    match action {
        ExecutionAction::RenameEntry { .. } => CompletedPhase::Renamed,
        ExecutionAction::CreateDirectory { .. } => CompletedPhase::DirectoryCreated,
        ExecutionAction::StageCopy { .. } => CompletedPhase::CopyStaged,
        ExecutionAction::PublishCopy { .. } => CompletedPhase::CopyPublished,
        ExecutionAction::RemoveSource { .. } => CompletedPhase::SourceRemoved,
        ExecutionAction::SetMetadata { .. } => CompletedPhase::MetadataSet,
        ExecutionAction::RemoveOwnedCopy { .. } => CompletedPhase::OwnedCopyRemoved,
        ExecutionAction::RemoveOwnedEmptyDirectory { .. } => {
            CompletedPhase::OwnedEmptyDirectoryRemoved
        }
    }
}
pub(super) fn validate_plan(plan: &ExecutionPlan) -> Result<(), OpError> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut groups: BTreeMap<GroupId, BTreeSet<GroupId>> = BTreeMap::new();
    let mut steps: BTreeMap<StepId, &ExecutionStep> = BTreeMap::new();
    if plan.groups.is_empty() {
        return Err(invalid("execution plan is empty"));
    }
    for group in &plan.groups {
        if group.steps.is_empty() || groups.contains_key(&group.id) {
            return Err(invalid("empty or duplicate execution group"));
        }
        let mut dependencies = BTreeSet::new();
        let mut direct_dependencies = BTreeSet::new();
        for dependency in &group.depends_on {
            let ancestors = groups
                .get(dependency)
                .ok_or_else(|| invalid("dependency is not an earlier group"))?;
            if !direct_dependencies.insert(*dependency) {
                return Err(invalid("duplicate dependency"));
            }
            dependencies.insert(*dependency);
            dependencies.extend(ancestors);
        }
        for step in &group.steps {
            if step.group_id != group.id || steps.contains_key(&step.id) {
                return Err(invalid("duplicate or misgrouped step"));
            }
            let earlier = |id: StepId| -> Result<&ExecutionStep, OpError> {
                let previous = steps
                    .get(&id)
                    .copied()
                    .ok_or_else(|| invalid("reference must name an earlier compiled step"))?;
                if previous.group_id != group.id && !dependencies.contains(&previous.group_id) {
                    return Err(invalid("reference crosses an undeclared group dependency"));
                }
                Ok(previous)
            };
            if let Some(entry) = source(&step.action) {
                validate_path(&entry.path)?;
                if let ExpectedIdentity::FromReceipt { step_id } = entry.identity {
                    if output(&earlier(step_id)?.action) != Some(entry.path.as_path()) {
                        return Err(invalid(
                            "receipt identity path does not match referenced poststate",
                        ));
                    }
                }
            }
            if let Some(path) = output(&step.action) {
                validate_path(path)?;
            }
            if let Some(owned) = owned(&step.action) {
                let creator = earlier(owned.created_by)?;
                if matches!(
                    step.action,
                    ExecutionAction::RemoveOwnedCopy { .. }
                        | ExecutionAction::RemoveOwnedEmptyDirectory { .. }
                ) && creator.group_id != step.group_id
                {
                    return Err(invalid(
                        "compensation ownership must stay within its dependency group",
                    ));
                }
                let right_kind = match &step.action {
                    ExecutionAction::RemoveOwnedEmptyDirectory { .. } => {
                        matches!(creator.action, ExecutionAction::CreateDirectory { .. })
                    }
                    _ => matches!(creator.action, ExecutionAction::StageCopy { .. }),
                };
                if !right_kind {
                    return Err(invalid(
                        "ownership must name the corresponding creation action",
                    ));
                }
            }
            match &step.action {
                ExecutionAction::PublishCopy {
                    staging,
                    verified_by,
                    ..
                } => {
                    if *verified_by != staging.created_by
                        || !matches!(
                            earlier(*verified_by)?.action,
                            ExecutionAction::StageCopy { .. }
                        )
                    {
                        return Err(invalid(
                            "publication requires its staging verification receipt",
                        ));
                    }
                }
                ExecutionAction::RemoveSource {
                    source: remove,
                    published_by,
                } => {
                    let published = earlier(*published_by)?;
                    let ExecutionAction::PublishCopy { verified_by, .. } = &published.action else {
                        return Err(invalid("source removal requires publication"));
                    };
                    let verified = earlier(*verified_by)?;
                    let ExecutionAction::StageCopy { source: copied, .. } = &verified.action else {
                        return Err(invalid("publication has no staged source"));
                    };
                    if remove != copied {
                        return Err(invalid("removed source differs from verified copy source"));
                    }
                }
                _ => {}
            }
            steps.insert(step.id, step);
        }
        groups.insert(group.id, dependencies);
    }
    Ok(())
}
fn validate_path(path: &Path) -> Result<(), OpError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(invalid(
            "compiled native paths must be absolute without parent traversal",
        ));
    }
    Ok(())
}
