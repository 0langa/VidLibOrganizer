//! Private on-disk representations: native units and exact quantities, never UI DTOs.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vidlib_rename::{
    EntryPrestate, ExecutionAction, ExpectedIdentity, MetadataChange, OwnedEntryPrestate,
};

pub(super) const VERSION: i64 = 12;
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

// Old records retain inspection version 1: missing namespace is unknown, never
// reconstructed from current disk state. New records require lossless version 2.
pub(super) const V4_SQL: &str = r#"
ALTER TABLE operations ADD COLUMN inspection_version INTEGER NOT NULL DEFAULT 1 CHECK(inspection_version IN (1,2));
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
    CreateStaging {
        #[serde(with = "native")]
        staging: PathBuf,
    },
    WriteStaging {
        #[serde(with = "EntryCodec")]
        source: EntryPrestate,
        #[serde(with = "boxed_owned")]
        staging: Box<OwnedEntryPrestate>,
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
    RemoveSourceDirectory {
        #[serde(with = "EntryCodec")]
        source: EntryPrestate,
        destination_created_by: StepId,
        required_descendants: Vec<StepId>,
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
    FromReceipt {
        step_id: StepId,
    },
    DirectoryAfterEffects {
        original: FileStamp,
        created_by: Option<StepId>,
        effects: Vec<StepId>,
    },
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
        "inverse_of": plan.inverse_of,
        "expected_namespace": namespace_inspection(&plan.expected_namespace)?,
        "approved_plan": p.approved_plan, "inventory": p.inventory, "recipe": p.recipe,
        "frozen_inputs": p.frozen_inputs, "namespace_epoch": p.namespace_epoch.to_string(), "policy_epoch": p.policy_epoch.to_string(),
        "grants": p.grants.grants.iter().map(|g| json!({"id": g.id, "path": encode_path(&g.path), "source": g.source})).collect::<Vec<_>>(),
        "native_roots": p.inventory.roots.iter().map(|r| json!({"id":r.id, "path": encode_path(&r.path), "grant_path": encode_path(&r.source_grant.path)})).collect::<Vec<_>>(),
        "native_entries": p.inventory.entries.iter().map(|e| json!({"id": e.id, "path": encode_path(&e.absolute_path)})).collect::<Vec<_>>(),
        "native_coverage": p.inventory.coverage.iter().map(|c| encode_path(&c.directory)).collect::<Vec<_>>()
    }))
}

/// Private inspection codec. Public namespace serde intentionally omits native
/// paths; supplement every omitted path using the lossless native-unit codec.
fn namespace_inspection(
    namespace: &vidlib_inventory::NamespaceSnapshot,
) -> Result<serde_json::Value, OpError> {
    use serde_json::json;
    use vidlib_inventory::encode_path;
    let mut value = serde_json::to_value(namespace).map_err(|e| invalid(e.to_string()))?;
    for (stored, parent) in value["parents"]
        .as_array_mut()
        .ok_or_else(|| invalid("invalid namespace codec"))?
        .iter_mut()
        .zip(&namespace.parents)
    {
        stored["parent_path"] = json!(encode_path(&parent.parent_path));
        for (stored, entry) in stored["entries"]
            .as_array_mut()
            .ok_or_else(|| invalid("invalid entry codec"))?
            .iter_mut()
            .zip(&parent.entries)
        {
            stored["absolute_path"] = json!(encode_path(&entry.absolute_path));
        }
    }
    for (stored, grant) in value["grants"]["grants"]
        .as_array_mut()
        .ok_or_else(|| invalid("invalid grant codec"))?
        .iter_mut()
        .zip(&namespace.grants.grants)
    {
        stored["path"] = json!(encode_path(&grant.path));
    }
    value["policy"]["guarded_roots"] = json!(namespace
        .policy
        .guarded_roots
        .iter()
        .map(|p| encode_path(p))
        .collect::<Vec<_>>());
    value["policy"]["protected_roots"] = json!(namespace
        .policy
        .protected_roots
        .iter()
        .map(|p| encode_path(p))
        .collect::<Vec<_>>());
    Ok(value)
}

pub(super) fn source(action: &ExecutionAction) -> Option<&EntryPrestate> {
    match action {
        ExecutionAction::RenameEntry { source, .. }
        | ExecutionAction::WriteStaging { source, .. }
        | ExecutionAction::StageCopy { source, .. }
        | ExecutionAction::RemoveSource { source, .. }
        | ExecutionAction::RemoveSourceDirectory { source, .. } => Some(source),
        ExecutionAction::SetMetadata { entry, .. } => Some(entry),
        ExecutionAction::PublishCopy { staging, .. } => Some(&staging.entry),
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => Some(&owned.entry),
        ExecutionAction::CreateDirectory { .. } | ExecutionAction::CreateStaging { .. } => None,
    }
}
pub(super) fn output(action: &ExecutionAction) -> Option<&Path> {
    match action {
        ExecutionAction::RenameEntry { target, .. }
        | ExecutionAction::CreateDirectory { target }
        | ExecutionAction::PublishCopy { target, .. } => Some(target),
        ExecutionAction::StageCopy { staging, .. } | ExecutionAction::CreateStaging { staging } => {
            Some(staging)
        }
        ExecutionAction::WriteStaging { staging, .. } => Some(&staging.entry.path),
        ExecutionAction::SetMetadata { entry, .. } => Some(&entry.path),
        _ => None,
    }
}
pub(super) fn owned(action: &ExecutionAction) -> Option<&OwnedEntryPrestate> {
    match action {
        ExecutionAction::PublishCopy { staging, .. } => Some(staging),
        ExecutionAction::WriteStaging { staging, .. } => Some(staging),
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => Some(owned),
        _ => None,
    }
}
pub(super) fn phase(action: &ExecutionAction) -> CompletedPhase {
    match action {
        ExecutionAction::RenameEntry { .. } => CompletedPhase::Renamed,
        ExecutionAction::CreateDirectory { .. } => CompletedPhase::DirectoryCreated,
        ExecutionAction::CreateStaging { .. } => CompletedPhase::StagingCreated,
        ExecutionAction::StageCopy { .. } | ExecutionAction::WriteStaging { .. } => {
            CompletedPhase::CopyStaged
        }
        ExecutionAction::PublishCopy { .. } => CompletedPhase::CopyPublished,
        ExecutionAction::RemoveSource { .. } => CompletedPhase::SourceRemoved,
        ExecutionAction::RemoveSourceDirectory { .. } => CompletedPhase::SourceDirectoryRemoved,
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
            if let Some(EntryPrestate {
                path,
                identity:
                    ExpectedIdentity::DirectoryAfterEffects {
                        original,
                        created_by,
                        effects,
                    },
            }) = source(&step.action)
            {
                if original.physical_id.is_none() {
                    return Err(invalid("directory original identity missing"));
                }
                if let Some(created) = created_by {
                    if !matches!(&earlier(*created)?.action, ExecutionAction::CreateDirectory { target } if target == path)
                    {
                        return Err(invalid("directory authority must name its exact creator"));
                    }
                }
                let mut unique = BTreeSet::new();
                for effect in effects {
                    if !unique.insert(*effect) {
                        return Err(invalid("duplicate directory child effect"));
                    }
                    earlier(*effect)?;
                }
            }
            if let Some(path) = output(&step.action) {
                validate_path(path)?;
            }
            if let Some(owned) = owned(&step.action) {
                // Historical cleanup authority is checked by re-deriving the
                // entire inverse from its retained original in Journal::begin.
                if !steps.contains_key(&owned.created_by)
                    && plan.inverse_of.is_some()
                    && matches!(
                        step.action,
                        ExecutionAction::RemoveOwnedCopy { .. }
                            | ExecutionAction::RemoveOwnedEmptyDirectory { .. }
                    )
                {
                    steps.insert(step.id, step);
                    continue;
                }
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
                    _ => matches!(
                        creator.action,
                        ExecutionAction::StageCopy { .. } | ExecutionAction::CreateStaging { .. }
                    ),
                };
                if !right_kind {
                    return Err(invalid(
                        "ownership must name the corresponding creation action",
                    ));
                }
            }
            match &step.action {
                ExecutionAction::RemoveSourceDirectory {
                    source,
                    destination_created_by,
                    required_descendants,
                } => {
                    if !matches!(
                        earlier(*destination_created_by)?.action,
                        ExecutionAction::CreateDirectory { .. }
                    ) {
                        return Err(invalid(
                            "source directory needs owned destination directory",
                        ));
                    }
                    let ExpectedIdentity::FromReceipt { step_id } = source.identity else {
                        return Err(invalid("source directory needs durable quarantine"));
                    };
                    if !matches!(&earlier(step_id)?.action, ExecutionAction::RenameEntry { target, source: EntryPrestate { identity: ExpectedIdentity::DirectoryAfterEffects { created_by: None, .. }, .. }, temporary: true } if target == &source.path)
                    {
                        return Err(invalid("invalid source directory quarantine lineage"));
                    }
                    for id in required_descendants {
                        earlier(*id)?;
                    }
                }
                ExecutionAction::PublishCopy {
                    staging,
                    verified_by,
                    ..
                } => {
                    if !match &earlier(*verified_by)?.action {
                        ExecutionAction::StageCopy { .. } => *verified_by == staging.created_by,
                        ExecutionAction::WriteStaging {
                            staging: written, ..
                        } => {
                            written.created_by == staging.created_by
                                && written.entry.path == staging.entry.path
                        }
                        _ => false,
                    } {
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
                    let (ExecutionAction::StageCopy { source: copied, .. }
                    | ExecutionAction::WriteStaging { source: copied, .. }) = &verified.action
                    else {
                        return Err(invalid("publication has no staged source"));
                    };
                    if remove != copied {
                        let ExpectedIdentity::FromReceipt { step_id } = remove.identity else {
                            return Err(invalid("removed source lacks quarantine lineage"));
                        };
                        if !matches!(&earlier(step_id)?.action, ExecutionAction::RenameEntry { source, target, temporary: true } if source == copied && target == &remove.path)
                        {
                            return Err(invalid(
                                "removed source differs from verified source/quarantine",
                            ));
                        }
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

// Recovery adds evidence; original compiled steps and factual receipts are immutable.
pub(super) const V5_SQL: &str = r#"
CREATE TABLE recovery_audit(sequence INTEGER PRIMARY KEY AUTOINCREMENT, operation_id TEXT NOT NULL REFERENCES operations(id), step_id TEXT NOT NULL, resolution TEXT NOT NULL, evidence TEXT NOT NULL, FOREIGN KEY(operation_id,step_id) REFERENCES steps(operation_id,id));
CREATE TABLE recovery_effects(operation_id TEXT NOT NULL, original_step TEXT NOT NULL, inverse_step TEXT NOT NULL, parents TEXT NOT NULL, PRIMARY KEY(operation_id,original_step), UNIQUE(operation_id,inverse_step), FOREIGN KEY(operation_id,original_step) REFERENCES steps(operation_id,id), FOREIGN KEY(operation_id,inverse_step) REFERENCES steps(operation_id,id));
"#;

// Explicit Restore authority survives receipt/reset/flush transitions. An old
// reader must reject this protocol version rather than ignore the selection.
pub(super) const V6_SQL: &str = r#"
CREATE TABLE recovery_restore(operation_id TEXT PRIMARY KEY REFERENCES operations(id), group_id TEXT);
"#;

// New readers validate historical inverse lineage at begin; older readers must
// reject these journals rather than treating inverse effects as new originals.
pub(super) const V7_SQL: &str = r#"
CREATE TABLE operation_inverses(operation_id TEXT PRIMARY KEY REFERENCES operations(id), original_operation TEXT NOT NULL REFERENCES operations(id));
CREATE INDEX inverses_by_original ON operation_inverses(original_operation);
"#;

// A checked decimal counter changes in the same durable transaction as facts.
// Older readers reject v8 rather than ignoring this query freshness protocol.
pub(super) const V8_SQL: &str = r#"
CREATE TABLE history_revision(singleton INTEGER PRIMARY KEY CHECK(singleton=1), revision TEXT NOT NULL);
INSERT INTO history_revision VALUES(1,'0');
CREATE INDEX history_by_state ON operations(state);
"#;

// Protocol v9 separates create-new ownership from streaming verification.
pub(super) const V9_SQL: &str = "CREATE TABLE copy_protocol(singleton INTEGER PRIMARY KEY CHECK(singleton=1), version INTEGER NOT NULL CHECK(version=1)); INSERT INTO copy_protocol VALUES(1,1);";

// Preserve every v5 link as ordinal zero; a chain is durably compiled as a whole.
pub(super) const V10_SQL: &str = r#"
ALTER TABLE recovery_effects RENAME TO recovery_effects_v5;
CREATE TABLE recovery_effects(operation_id TEXT NOT NULL, original_step TEXT NOT NULL, inverse_step TEXT NOT NULL, parents TEXT NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal>=0), PRIMARY KEY(operation_id,original_step,ordinal), UNIQUE(operation_id,inverse_step), FOREIGN KEY(operation_id,original_step) REFERENCES steps(operation_id,id), FOREIGN KEY(operation_id,inverse_step) REFERENCES steps(operation_id,id));
INSERT INTO recovery_effects SELECT operation_id,original_step,inverse_step,parents,0 FROM recovery_effects_v5;
DROP TABLE recovery_effects_v5;
"#;

// Typed future-parent bindings and explicit cross-original inverse prerequisites.
// Existing parent payloads and ordered chain rows retain their exact meaning.
pub(super) const V11_SQL: &str = r#"
CREATE TABLE recovery_dependencies(operation_id TEXT NOT NULL, inverse_step TEXT NOT NULL, prerequisite_step TEXT NOT NULL, PRIMARY KEY(operation_id,inverse_step,prerequisite_step), FOREIGN KEY(operation_id,inverse_step) REFERENCES steps(operation_id,id), FOREIGN KEY(operation_id,prerequisite_step) REFERENCES steps(operation_id,id));
INSERT INTO recovery_dependencies SELECT current.operation_id,current.inverse_step,prior.inverse_step FROM recovery_effects current JOIN recovery_effects prior ON prior.operation_id=current.operation_id AND prior.original_step=current.original_step AND prior.ordinal=current.ordinal-1;
CREATE TABLE recovery_groups(operation_id TEXT NOT NULL REFERENCES operations(id), group_id TEXT NOT NULL, canonical_effects TEXT NOT NULL, PRIMARY KEY(operation_id,group_id));
"#;

// Boxing the in-memory write ownership keeps every execution step compact;
// the private durable representation remains the exact existing OwnedCodec.
mod boxed_owned {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &OwnedEntryPrestate,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        OwnedCodec::serialize(value, serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Box<OwnedEntryPrestate>, D::Error> {
        OwnedCodec::deserialize(deserializer).map(Box::new)
    }
}

// Exact resolved metadata prestate commits with intent, never reconstructed after an effect.
pub(super) const V12_SQL: &str = r#"
CREATE TABLE metadata_prestates(operation_id TEXT NOT NULL, step_id TEXT NOT NULL, stamp TEXT NOT NULL, PRIMARY KEY(operation_id,step_id), FOREIGN KEY(operation_id,step_id) REFERENCES steps(operation_id,id));
"#;
