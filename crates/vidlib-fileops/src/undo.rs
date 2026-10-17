//! Historical receipts authorize inverse candidates; a fresh namespace and the
//! ordinary graph compiler validate them. Effects use the ordinary executor.
use crate::{
    journal::{OperationRecord, OperationState, StepState},
    platform::check_stamp,
    recovery::evidence::{parent_map, rebase, transformed_namespace},
    ErrorCode, OpError, Phase,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use vidlib_inventory::{ids::*, *};
use vidlib_rename::*;

fn invalid(message: impl Into<String>) -> OpError {
    OpError::new(ErrorCode::StaleState, Phase::Verify, message)
}
fn path(value: &serde_json::Value) -> Result<PathBuf, OpError> {
    decode_path(
        &serde_json::from_value::<NativePathRecord>(value.clone())
            .map_err(|_| invalid("invalid historical native path"))?,
    )
    .map_err(|_| invalid("historical native path unavailable on this platform"))
}
fn decode<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Result<T, OpError> {
    serde_json::from_value(value.clone())
        .map_err(|_| invalid("invalid historical inspection value"))
}
fn factual_moves(op: &OperationRecord) -> impl Iterator<Item = (&Path, &Path)> {
    op.steps
        .iter()
        .chain(op.recovery_effects.iter().map(|e| &e.inverse))
        .filter(|s| !s.receipts.is_empty())
        .filter_map(|s| match &s.step.action {
            ExecutionAction::RenameEntry { source, target, .. } => {
                Some((source.path.as_path(), target.as_path()))
            }
            _ => None,
        })
}
fn current_path(op: &OperationRecord, original: &Path) -> PathBuf {
    let mut current = original.to_path_buf();
    for (source, target) in factual_moves(op) {
        rebase(&mut current, source, target);
    }
    current
}

// Explicit destination operations retain both selected source and destination
// grants. Their quarantines do not move the original path's restore authority.
fn grant_path(op: &OperationRecord, original: &Path) -> PathBuf {
    if matches!(
        op.inspection["recipe"]["destination"]["type"].as_str(),
        Some("copy_to" | "move_to")
    ) {
        original.to_owned()
    } else {
        current_path(op, original)
    }
}

/// Backend-only capture inputs. This is read-only context, never mutation
/// authority; `plan_undo` still validates historical lineage, complete subtree
/// evidence and the supplied fresh namespace before the ordinary executor.
pub struct UndoCaptureContext {
    pub grants: GrantSet,
    pub candidates: Vec<PathBuf>,
}
pub fn capture_context(completed: &OperationRecord) -> Result<UndoCaptureContext, OpError> {
    let mut grants = GrantSet::default();
    for value in completed.inspection["grants"]
        .as_array()
        .ok_or_else(|| invalid("historical grants unavailable"))?
    {
        grants.grants.push(SourceGrant {
            id: decode(&value["id"])?,
            path: grant_path(completed, &path(&value["path"])?),
            source: decode(&value["source"])?,
        });
    }
    let (parents, entries, _) = transformed_namespace(completed)?;
    let grant_candidates = grants
        .grants
        .iter()
        .filter(|g| !parents.iter().any(|(p, _)| p == &g.path))
        .map(|g| g.path.clone())
        .collect::<Vec<_>>();
    let mut candidates: Vec<_> = entries
        .into_keys()
        .chain(parents.into_iter().map(|(p, _)| p.join(".vidlib-capture")))
        .filter(|p| grants.grants.iter().any(|g| p.starts_with(&g.path)))
        .chain(grant_candidates)
        .collect();
    candidates.sort();
    candidates.dedup();
    Ok(UndoCaptureContext { grants, candidates })
}

/// A terminal batch can retain completed earlier groups even when its final
/// label is Failed/Cancelled. Recovery inverse receipts are never new originals.
pub fn plan_undo(
    completed: &OperationRecord,
    namespace: &NamespaceSnapshot,
    version: PlanRef,
) -> Result<RenamePlan, OpError> {
    if !matches!(
        completed.state,
        OperationState::Completed | OperationState::Cancelled | OperationState::Failed
    ) || completed
        .steps
        .iter()
        .chain(completed.recovery_effects.iter().map(|e| &e.inverse))
        .any(|s| matches!(s.state, StepState::IntentDurable | StepState::Unresolved))
    {
        return Err(OpError::new(
            ErrorCode::RecoveryRequired,
            Phase::Verify,
            "unfinished operation requires recovery",
        ));
    }
    if !completed.history_visible || !completed.active_inverses.is_empty() {
        return Err(invalid(
            "undo support removed or an inverse already retains effects",
        ));
    }
    if completed.steps.iter().any(|s| {
        s.state == StepState::CompletionDurable
            && matches!(
                s.step.action,
                ExecutionAction::PublishCopy { .. } | ExecutionAction::CreateDirectory { .. }
            )
    }) {
        return plan_copy_undo(completed, namespace, version);
    }
    let mut retained = BTreeMap::new();
    for pending in &completed.steps {
        if pending.state != StepState::CompletionDurable {
            continue;
        }
        if !matches!(pending.step.action, ExecutionAction::RenameEntry { .. }) {
            return Err(OpError::new(
                ErrorCode::Unsupported,
                Phase::Verify,
                "this phase requires its owning inverse planner",
            ));
        }
        let receipt = pending
            .receipts
            .last()
            .ok_or_else(|| invalid("retained effect has no receipt"))?;
        if receipt.phase != crate::events::CompletedPhase::Renamed
            || receipt.after.is_none()
            || receipt.after_path.is_none()
        {
            return Err(invalid("retained rename has no poststate"));
        }
        retained.insert(pending.step.entry_id, (pending, receipt));
    }
    if retained.is_empty() {
        return Err(invalid(
            "operation has no retained completed rename effects",
        ));
    }
    let mut grants = GrantSet::default();
    for value in completed.inspection["grants"]
        .as_array()
        .ok_or_else(|| invalid("historical grants unavailable"))?
    {
        grants.grants.push(SourceGrant {
            id: decode(&value["id"])?,
            path: grant_path(completed, &path(&value["path"])?),
            source: decode(&value["source"])?,
        });
    }
    if grants != namespace.grants {
        return Err(invalid("fresh namespace does not carry historical grants"));
    }
    let historical_paths: BTreeMap<EntryId, PathBuf> = completed.inspection["native_entries"]
        .as_array()
        .ok_or_else(|| invalid("historical entry paths unavailable"))?
        .iter()
        .map(|v| Ok((decode(&v["id"])?, path(&v["path"])?)))
        .collect::<Result<_, OpError>>()?;
    let historical_kinds: BTreeMap<EntryId, EntryKind> = completed.inspection["inventory"]
        ["entries"]
        .as_array()
        .ok_or_else(|| invalid("historical entry kinds unavailable"))?
        .iter()
        .map(|v| Ok((decode(&v["id"])?, decode(&v["kind"])?)))
        .collect::<Result<_, OpError>>()?;
    let parents = parent_map(completed, None)?;
    let mut problems = BTreeMap::new();
    let mut selected = Vec::new();
    let mut expected = BTreeMap::new();
    for (id, (pending, receipt)) in &retained {
        let original = historical_paths
            .get(id)
            .ok_or_else(|| invalid("original source path missing"))?;
        let current = current_path(completed, original);
        // The last receipt may precede a later ancestor carry. Track exact paths
        // through all subsequent original/recovery facts, not display names.
        let mut receipt_path = decode_path(receipt.after_path.as_ref().unwrap())
            .map_err(|_| invalid("historical receipt path unavailable"))?;
        for later in completed
            .steps
            .iter()
            .filter(|s| s.sequence > pending.sequence)
            .chain(completed.recovery_effects.iter().map(|e| &e.inverse))
            .filter(|s| !s.receipts.is_empty())
        {
            if let ExecutionAction::RenameEntry { source, target, .. } = &later.step.action {
                rebase(&mut receipt_path, &source.path, target);
            }
        }
        if current != receipt_path {
            return Err(invalid(
                "historical receipt does not own current source path",
            ));
        }
        let kind = *historical_kinds
            .get(id)
            .ok_or_else(|| invalid("historical source kind missing"))?;
        let stamp = receipt.after.as_ref().unwrap();
        let observed = namespace
            .parents
            .iter()
            .flat_map(|p| &p.entries)
            .find(|e| e.absolute_path == current);
        let mut verified = stamp.clone();
        let material = match observed {
            Some(actual) if actual.kind == kind => {
                if kind == EntryKind::Directory {
                    directory_material(completed, namespace, &current, stamp)
                } else {
                    check_stamp(&actual.stamp, stamp, Phase::Verify).map(|_| actual.stamp.clone())
                }
            }
            _ => Err(invalid("current poststate is missing or has changed kind")),
        };
        match material {
            Ok(s) => verified = s,
            Err(e) => {
                problems.insert(*id, e.message);
            }
        }
        let original_parent = original
            .parent()
            .ok_or_else(|| invalid("original parent missing"))?;
        let target_parent = current_path(completed, original_parent);
        for p in [current.parent().unwrap(), target_parent.as_path()] {
            if !parents.iter().any(|(path, id)| {
                path == p
                    && namespace
                        .parents
                        .iter()
                        .any(|f| f.parent_path == p && f.stamp.physical_id.as_ref() == Some(id))
            }) {
                problems.insert(*id, "reviewed parent identity missing or changed".into());
            }
        }
        selected.push((
            *id,
            pending.step.group_id,
            current,
            target_parent.join(original.file_name().unwrap()),
            kind,
        ));
        expected.insert(*id, verified);
    }
    let roots: Vec<_> = grants
        .grants
        .iter()
        .map(|g| RootBinding {
            id: RootId::from_uuid(*g.id.as_uuid()),
            source_grant: g.clone(),
            path: g.path.clone(),
            evidence: RootEvidence::default(),
            capabilities: FilesystemCapabilities::default(),
        })
        .collect();
    let root_for = |p: &Path| {
        roots
            .iter()
            .filter(|r| p.starts_with(&r.path))
            .max_by_key(|r| r.path.components().count())
            .ok_or_else(|| invalid("source outside historical grant"))
    };
    let make_entry = |id, path: PathBuf, kind, stamp| -> Result<EntrySnapshot, OpError> {
        let root = root_for(&path)?;
        // Root entries use their filename relative to the root's parent, just
        // like scanned inventories. In particular an individually granted file
        // must not acquire an empty relative path in an inverse inventory.
        let relative_base = if path == root.path {
            root.path.parent().unwrap_or(&root.path)
        } else {
            &root.path
        };
        Ok(EntrySnapshot {
            id,
            root_id: root.id,
            relative_path: encode_path(path.strip_prefix(relative_base).unwrap()),
            display_name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            display_path: path.to_string_lossy().into_owned(),
            absolute_path: path,
            kind,
            stamp,
            observed_ns: namespace.window.finished_ns,
            issues: vec![],
        })
    };
    let mut entries = Vec::new();
    for item in namespace.parents.iter().flat_map(|p| &p.entries) {
        if !grants
            .grants
            .iter()
            .any(|g| item.absolute_path.starts_with(&g.path))
        {
            continue;
        }
        let selected = selected
            .iter()
            .find(|(_, _, p, _, _)| *p == item.absolute_path);
        let id = match selected {
            Some((id, ..)) => *id,
            None => format!("{:x}", md5::compute(encode_path(&item.absolute_path).bytes))
                .parse()
                .map_err(|_| invalid("cannot derive namespace entry ID"))?,
        };
        let stamp = expected.get(&id).unwrap_or(&item.stamp).clone();
        entries.push(make_entry(
            id,
            item.absolute_path.clone(),
            item.kind,
            stamp,
        )?);
    }
    for (id, _, source, _, kind) in &selected {
        if !entries.iter().any(|e| e.id == *id) {
            entries.push(make_entry(
                *id,
                source.clone(),
                *kind,
                expected[id].clone(),
            )?);
        }
    }
    let mut coverage = Vec::new();
    for parent in &namespace.parents {
        let Ok(root) = root_for(&parent.parent_path) else {
            continue;
        };
        let complete = parent.children_complete
            && parent.issues.is_empty()
            && subtree_complete(&parent.parent_path, namespace);
        coverage.push(ScopeCoverage {
            root_id: root.id,
            directory: parent.parent_path.clone(),
            relative_directory: Some(encode_path(
                parent.parent_path.strip_prefix(&root.path).unwrap(),
            )),
            listed_names: parent.entries.iter().map(|e| e.name.clone()).collect(),
            children_complete: parent.children_complete,
            subtree_complete: complete,
            outcome: if complete {
                CoverageOutcome::Complete
            } else {
                CoverageOutcome::Failed
            },
            filter_fingerprint: "undo-reviewed-namespace".into(),
            window: parent.window.clone(),
            issues: parent.issues.clone(),
        });
    }
    let inventory = Inventory {
        id: decode(&completed.inspection["inventory"]["id"])?,
        generation: namespace.namespace_epoch,
        roots,
        entries,
        coverage,
        window: namespace.window.clone(),
        issues: vec![],
    };
    let ids: Vec<_> = selected.iter().map(|(id, ..)| *id).collect();
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: ids.clone(),
        selection_filter: None,
        numbering_order: ids.clone(),
        numbering_scope: Default::default(),
        rules: vec![],
        companions: CompanionPolicy::None,
        destination: Destination::SameDirectory,
        collision_policy: CollisionPolicy::Block,
        metadata_changes: vec![],
    };
    let frozen = FrozenInputs {
        inventory_id: inventory.id,
        generation: inventory.generation,
        now_ns: namespace.window.finished_ns,
        timezone: FrozenTimezone::Utc,
        primary_order: ids,
        numbering_scope: Default::default(),
        values: vec![],
    };
    let rows = selected
        .into_iter()
        .map(|(id, group, source, target, _)| {
            let issues = problems
                .get(&id)
                .map(|m| {
                    vec![PlanIssue {
                        code: RenameErrorCode::InvalidNamespace,
                        entry_id: Some(id),
                        rule_id: None,
                        message: m.clone(),
                    }]
                })
                .unwrap_or_default();
            PlanRow {
                carried_descendants: 0,
                entry_id: id,
                group_id: group,
                primary_id: id,
                old_display_name: source
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                new_display_name: target
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                old_display_path: source.to_string_lossy().into_owned(),
                new_display_path: target.to_string_lossy().into_owned(),
                source_path: source,
                target_path: target,
                stamp: expected[&id].clone(),
                status: if issues.is_empty() {
                    RowStatus::Changed
                } else {
                    RowStatus::Blocked
                },
                explanations: vec![],
                issues,
            }
        })
        .collect();
    let batch = EvaluatedBatch {
        inventory: inventory.clone(),
        recipe,
        inventory_id: inventory.id,
        generation: inventory.generation,
        recipe_revision: 1,
        rows,
        frozen_inputs: frozen,
        grants,
    };
    let mut result =
        plan(&inventory, &batch, namespace, version).map_err(|e| invalid(e.to_string()))?;
    result.inverse_of = Some(completed.operation_id);
    result.execution.inverse_of = result.inverse_of;
    Ok(result)
}
fn subtree_complete(root: &Path, ns: &NamespaceSnapshot) -> bool {
    ns.parents
        .iter()
        .flat_map(|p| &p.entries)
        .filter(|e| e.kind == EntryKind::Directory && e.absolute_path.starts_with(root))
        .all(|e| {
            ns.parents.iter().any(|p| {
                p.parent_path == e.absolute_path && p.children_complete && p.issues.is_empty()
            })
        })
}
fn directory_material(
    op: &OperationRecord,
    ns: &NamespaceSnapshot,
    path: &Path,
    expected: &FileStamp,
) -> Result<FileStamp, OpError> {
    let (directories, entries, changed) = transformed_namespace(op)?;
    // A fresh scan cannot supply missing historical child authority. Every
    // carried directory needs a recorded complete listing before stamp refresh.
    if entries.iter().any(|(child, (_, kind))| {
        *kind == EntryKind::Directory
            && child.starts_with(path)
            && !directories.iter().any(|(directory, _)| directory == child)
    }) {
        return Err(invalid(
            "historical carried subtree namespace is incomplete",
        ));
    }

    for (directory, id) in &directories {
        let actual = ns
            .parents
            .iter()
            .find(|p| p.parent_path == *directory)
            .ok_or_else(|| invalid("complete historical directory namespace unavailable"))?;
        if actual.stamp.physical_id.as_ref() != Some(id)
            || !actual.children_complete
            || !actual.issues.is_empty()
        {
            return Err(invalid("directory namespace incomplete or replaced"));
        }
        let children: Vec<_> = entries
            .iter()
            .filter(|(p, _)| p.parent() == Some(directory.as_path()))
            .collect();
        if children.len() != actual.entries.len() {
            return Err(invalid("unknown directory child blocks undo"));
        }
        for (p, (stamp, kind)) in children {
            let child = actual
                .entries
                .iter()
                .find(|e| e.absolute_path == *p)
                .ok_or_else(|| invalid("historical child is missing"))?;
            if child.kind != *kind || child.stamp.physical_id != stamp.physical_id {
                return Err(invalid("historical child identity changed"));
            }
            if *kind == EntryKind::Directory
                && stamp
                    .physical_id
                    .as_ref()
                    .is_some_and(|id| changed.contains(id))
                && directories.iter().any(|(d, _)| d == p)
            {
                if child.stamp.created_ns != stamp.created_ns
                    || child.stamp.windows_attributes != stamp.windows_attributes
                {
                    return Err(invalid("directory metadata changed"));
                }
            } else {
                check_stamp(&child.stamp, stamp, Phase::Verify)?;
            }
        }
    }
    let actual = ns
        .parents
        .iter()
        .find(|p| p.parent_path == path)
        .ok_or_else(|| invalid("directory itself must be captured"))?;
    if check_stamp(&actual.stamp, expected, Phase::Verify).is_err()
        && !(expected
            .physical_id
            .as_ref()
            .is_some_and(|id| changed.contains(id))
            && actual.stamp.physical_id == expected.physical_id
            && actual.stamp.created_ns == expected.created_ns
            && actual.stamp.windows_attributes == expected.windows_attributes)
    {
        return Err(invalid(
            "directory material changed without owned child effects",
        ));
    }
    Ok(actual.stamp.clone())
}

fn plan_copy_undo(
    op: &OperationRecord,
    ns: &NamespaceSnapshot,
    version: PlanRef,
) -> Result<RenamePlan, OpError> {
    let context = capture_context(op)?;
    if context.grants != ns.grants {
        return Err(invalid("historical destination grants changed"));
    }
    let grants = context.grants;
    let roots: Vec<_> = grants
        .grants
        .iter()
        .map(|g| RootBinding {
            id: RootId::from_uuid(*g.id.as_uuid()),
            source_grant: g.clone(),
            path: g.path.clone(),
            evidence: RootEvidence::default(),
            capabilities: FilesystemCapabilities::default(),
        })
        .collect();
    let mut rows = Vec::new();
    let mut entries = Vec::new();
    let mut groups = Vec::new();
    let mut sequence = 0u64;
    let mut next = || -> StepId {
        sequence += 1;
        format!(
            "{:x}",
            md5::compute(format!(
                "copy-undo:{}:{}:{sequence}",
                op.operation_id, version.plan_id
            ))
        )
        .parse()
        .expect("digest ID")
    };
    let observed = |path: &Path, stamp: &FileStamp, dir: bool| -> Result<FileStamp, OpError> {
        let actual = ns
            .parents
            .iter()
            .flat_map(|p| &p.entries)
            .find(|e| e.absolute_path == path)
            .ok_or_else(|| invalid("owned copy is missing"))?;
        if dir {
            if actual.kind != EntryKind::Directory || actual.stamp.physical_id != stamp.physical_id
            {
                return Err(invalid("created directory identity changed"));
            }
        } else {
            check_stamp(&actual.stamp, stamp, Phase::Verify)?;
        }
        Ok(actual.stamp.clone())
    };
    for historical_group in op.groups.iter().rev() {
        let mut steps = Vec::new();
        // Recreate missing source ancestors before any descendant copy-back.
        let mut restored = Vec::new();
        for removed in op.steps.iter().rev().filter(|s| {
            s.step.group_id == historical_group.id && s.state == StepState::CompletionDurable
        }) {
            let ExecutionAction::RemoveSourceDirectory {
                source,
                destination_created_by,
                ..
            } = &removed.step.action
            else {
                continue;
            };
            let ExpectedIdentity::FromReceipt { step_id } = source.identity else {
                return Err(invalid("removed directory quarantine missing"));
            };
            let prior = op
                .steps
                .iter()
                .find(|s| s.step.id == step_id)
                .ok_or_else(|| invalid("directory quarantine missing"))?;
            let ExecutionAction::RenameEntry {
                source:
                    EntryPrestate {
                        path,
                        identity:
                            ExpectedIdentity::DirectoryAfterEffects {
                                original,
                                created_by: None,
                                ..
                            },
                    },
                ..
            } = &prior.step.action
            else {
                return Err(invalid("directory original metadata missing"));
            };
            if ns
                .parents
                .iter()
                .flat_map(|p| &p.entries)
                .any(|e| e.absolute_path == *path)
            {
                return Err(invalid("original source directory recreated; undo refused"));
            }
            let create = next();
            let entry_id = EntryId::from_uuid(*destination_created_by.as_uuid());
            steps.push(ExecutionStep {
                id: create,
                entry_id,
                group_id: historical_group.id,
                action: ExecutionAction::CreateDirectory {
                    target: path.clone(),
                },
            });
            restored.push((
                path.clone(),
                original.clone(),
                create,
                entry_id,
                *destination_created_by,
            ));
        }
        for original in op.steps.iter().rev().filter(|s| {
            s.step.group_id == historical_group.id && s.state == StepState::CompletionDurable
        }) {
            let native_move = matches!(
                original.step.action,
                ExecutionAction::RenameEntry {
                    temporary: false,
                    ..
                }
            );
            let (source, created_by, directory, moving, target, expected) = match &original
                .step
                .action
            {
                ExecutionAction::RenameEntry {
                    source,
                    target,
                    temporary: false,
                } => {
                    let stamp = original
                        .receipts
                        .last()
                        .and_then(|r| r.after.as_ref())
                        .ok_or_else(|| invalid("native move receipt missing"))?;
                    let directory = ns.parents.iter().any(|p| p.parent_path == *target);
                    if ns
                        .parents
                        .iter()
                        .flat_map(|p| &p.entries)
                        .any(|e| e.absolute_path == source.path)
                    {
                        return Err(invalid("original native source recreated"));
                    }
                    (
                        target.clone(),
                        original.step.id,
                        directory,
                        true,
                        source.path.clone(),
                        if directory {
                            directory_material(op, ns, target, stamp)?
                        } else {
                            observed(target, stamp, false)?
                        },
                    )
                }
                ExecutionAction::PublishCopy {
                    staging, target, ..
                } => {
                    let fact = original
                        .receipts
                        .last()
                        .ok_or_else(|| invalid("publication receipt missing"))?;
                    let stamp = fact
                        .after
                        .as_ref()
                        .ok_or_else(|| invalid("publication identity missing"))?;
                    if !fact
                        .digest
                        .as_ref()
                        .is_some_and(|d| d.algorithm == "sha256" && d.bytes.len() == 32)
                    {
                        return Err(invalid("full SHA256 ownership required for copy undo"));
                    }
                    let removed=op.steps.iter().find(|s|s.state==StepState::CompletionDurable && matches!(&s.step.action,ExecutionAction::RemoveSource{published_by,..} if *published_by==original.step.id));
                    let original_source = if let Some(remove) = removed {
                        let ExecutionAction::RemoveSource { source, .. } = &remove.step.action
                        else {
                            unreachable!()
                        };
                        let ExpectedIdentity::FromReceipt { step_id } = source.identity else {
                            return Err(invalid("move quarantine lineage missing"));
                        };
                        match &op
                            .steps
                            .iter()
                            .find(|s| s.step.id == step_id)
                            .ok_or_else(|| invalid("quarantine missing"))?
                            .step
                            .action
                        {
                            ExecutionAction::RenameEntry { source, .. } => source.path.clone(),
                            _ => return Err(invalid("quarantine source invalid")),
                        }
                    } else {
                        target.clone()
                    };
                    (
                        target.clone(),
                        staging.created_by,
                        false,
                        removed.is_some(),
                        original_source,
                        observed(target, stamp, false)?,
                    )
                }
                ExecutionAction::CreateDirectory { target } => {
                    let stamp = original
                        .receipts
                        .last()
                        .and_then(|r| r.after.as_ref())
                        .ok_or_else(|| invalid("created directory receipt missing"))?;
                    (
                        target.clone(),
                        original.step.id,
                        true,
                        restored
                            .iter()
                            .any(|(_, _, _, _, creator)| *creator == original.step.id),
                        restored
                            .iter()
                            .find(|(_, _, _, _, creator)| *creator == original.step.id)
                            .map_or_else(|| target.clone(), |(path, ..)| path.clone()),
                        directory_material(op, ns, target, stamp)?,
                    )
                }
                _ => continue,
            };
            if !grants.grants.iter().any(|g| source.starts_with(&g.path))
                || !grants.grants.iter().any(|g| target.starts_with(&g.path))
            {
                return Err(invalid("inverse path outside historical grant"));
            }
            if ns.policy.guards_move(&source) {
                return Err(invalid("owned copy is protected"));
            }
            let id = EntryId::from_uuid(*original.step.id.as_uuid());
            let group = historical_group.id;
            let mk = |id_step, action| ExecutionStep {
                id: id_step,
                entry_id: id,
                group_id: group,
                action,
            };
            if moving && !directory && !native_move {
                let future_parent = restored
                    .iter()
                    .any(|(p, ..)| Some(p.as_path()) == target.parent());
                let parent = ns
                    .parents
                    .iter()
                    .find(|p| Some(p.parent_path.as_path()) == target.parent());
                if parent.is_none() && !future_parent {
                    return Err(invalid("original source parent unavailable"));
                }
                if parent.is_some_and(|p| p.entries.iter().any(|e| e.absolute_path == target)) {
                    return Err(invalid("original source recreated; undo refused"));
                }
                let staging =
                    target.with_file_name(format!(".vidlib-staging-undo-{}", original.step.id));
                if parent.is_some_and(|p| p.entries.iter().any(|e| e.absolute_path == staging)) {
                    return Err(invalid("undo staging reservation occupied"));
                }
                let create = next();
                let write = next();
                steps.push(mk(
                    create,
                    ExecutionAction::CreateStaging {
                        staging: staging.clone(),
                    },
                ));
                steps.push(mk(
                    write,
                    ExecutionAction::WriteStaging {
                        source: EntryPrestate {
                            path: source.clone(),
                            identity: ExpectedIdentity::Observed(expected.clone()),
                        },
                        staging: Box::new(OwnedEntryPrestate {
                            entry: EntryPrestate {
                                path: staging.clone(),
                                identity: ExpectedIdentity::FromReceipt { step_id: create },
                            },
                            created_by: create,
                        }),
                    },
                ));
                steps.push(mk(
                    next(),
                    ExecutionAction::PublishCopy {
                        staging: OwnedEntryPrestate {
                            entry: EntryPrestate {
                                path: staging,
                                identity: ExpectedIdentity::FromReceipt { step_id: write },
                            },
                            created_by: create,
                        },
                        target: target.clone(),
                        verified_by: write,
                    },
                ));
            }
            if native_move {
                steps.push(mk(
                    next(),
                    ExecutionAction::RenameEntry {
                        source: EntryPrestate {
                            path: source.clone(),
                            identity: ExpectedIdentity::Observed(expected.clone()),
                        },
                        target: target.clone(),
                        temporary: false,
                    },
                ));
            } else {
                let owned = if directory {
                    OwnedEntryPrestate {
                        entry: EntryPrestate {
                            path: source.clone(),
                            identity: ExpectedIdentity::Observed(expected.clone()),
                        },
                        created_by,
                    }
                } else {
                    let quarantine = source
                        .with_file_name(format!("{}undo-{}", RENAME_TEMP_PREFIX, original.step.id));
                    if ns
                        .parents
                        .iter()
                        .flat_map(|p| &p.entries)
                        .any(|e| e.absolute_path == quarantine)
                        || !ns.policy.allows_rename_reservation(&quarantine)
                    {
                        return Err(invalid("undo quarantine unavailable"));
                    }
                    let moved = next();
                    steps.push(mk(
                        moved,
                        ExecutionAction::RenameEntry {
                            source: EntryPrestate {
                                path: source.clone(),
                                identity: ExpectedIdentity::Observed(expected.clone()),
                            },
                            target: quarantine.clone(),
                            temporary: true,
                        },
                    ));
                    OwnedEntryPrestate {
                        entry: EntryPrestate {
                            path: quarantine,
                            identity: ExpectedIdentity::FromReceipt { step_id: moved },
                        },
                        created_by,
                    }
                };
                steps.push(mk(
                    next(),
                    if directory {
                        ExecutionAction::RemoveOwnedEmptyDirectory { owned }
                    } else {
                        ExecutionAction::RemoveOwnedCopy { owned }
                    },
                ));
            }
            let root = roots
                .iter()
                .filter(|r| source.starts_with(&r.path))
                .max_by_key(|r| r.path.components().count())
                .ok_or_else(|| invalid("inverse source grant missing"))?;
            let display = source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            entries.push(EntrySnapshot {
                id,
                root_id: root.id,
                relative_path: encode_path(source.strip_prefix(&root.path).unwrap()),
                absolute_path: source.clone(),
                display_name: display.clone(),
                display_path: source.to_string_lossy().into_owned(),
                kind: if directory {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                stamp: expected.clone(),
                observed_ns: ns.window.finished_ns,
                issues: vec![],
            });
            rows.push(PlanRow {
                carried_descendants: 0,
                entry_id: id,
                group_id: group,
                primary_id: id,
                source_path: source.clone(),
                target_path: target.clone(),
                old_display_name: display,
                new_display_name: if moving {
                    target
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    "Delete created copy".into()
                },
                old_display_path: source.to_string_lossy().into_owned(),
                new_display_path: if moving {
                    target.to_string_lossy().into_owned()
                } else {
                    "Delete created copy".into()
                },
                stamp: expected,
                status: RowStatus::Changed,
                explanations: vec![RuleExplanation {
                    rule_id: if native_move { "undo-native-move" } else { "undo-delete-created-copy" }.into(),
                    before: source.to_string_lossy().into_owned(),
                    after: if native_move { "Move back using the recorded native identity".into() }
                    else if directory { "Remove only the operation-owned directory after its children are undone and it is empty".into() }
                    else if moving {
                        "Copy back, verify, then delete the created destination copy".into()
                    } else {
                        "Delete only the created copy after identity and full SHA256 verification"
                            .into()
                    },
                }],
                issues: vec![],
            });
        }
        if !steps.is_empty() {
            for (path, original, create, entry_id, _) in restored.iter().rev() {
                let effects = steps
                    .iter()
                    .filter(|s| {
                        let (source, target) = crate::recovery::evidence::paths(&s.action);
                        source
                            .map(|e| e.path.as_path())
                            .into_iter()
                            .chain(target)
                            .any(|p| p != path && p.starts_with(path))
                    })
                    .map(|s| s.id)
                    .collect();
                steps.push(ExecutionStep {
                    id: next(),
                    entry_id: *entry_id,
                    group_id: historical_group.id,
                    action: ExecutionAction::SetMetadata {
                        entry: EntryPrestate {
                            path: path.clone(),
                            identity: ExpectedIdentity::DirectoryAfterEffects {
                                original: original.clone(),
                                created_by: Some(*create),
                                effects,
                            },
                        },
                        changes: vidlib_rename::destination::directory_metadata_changes(original),
                    },
                });
            }
            groups.push(ExecutionGroup {
                id: historical_group.id,
                depends_on: vec![],
                steps,
            });
        }
    }
    if rows.is_empty() {
        return Err(invalid("no retained copies to undo"));
    }
    let ids: Vec<_> = rows.iter().map(|r| r.entry_id).collect();
    let inventory = Inventory {
        id: decode(&op.inspection["inventory"]["id"])?,
        generation: ns.namespace_epoch,
        roots,
        entries,
        coverage: vec![],
        window: ns.window.clone(),
        issues: vec![],
    };
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: ids.clone(),
        selection_filter: None,
        numbering_order: ids.clone(),
        numbering_scope: Default::default(),
        rules: vec![],
        companions: CompanionPolicy::None,
        destination: Destination::SameDirectory,
        collision_policy: CollisionPolicy::Block,
        metadata_changes: vec![],
    };
    let frozen = FrozenInputs {
        inventory_id: inventory.id,
        generation: inventory.generation,
        now_ns: ns.window.finished_ns,
        timezone: FrozenTimezone::Utc,
        primary_order: ids,
        numbering_scope: Default::default(),
        values: vec![],
    };
    Ok(RenamePlan {
        inverse_of: Some(op.operation_id),
        version,
        inventory_id: inventory.id,
        generation: inventory.generation,
        recipe_revision: 1,
        counts: PlanCounts::from_rows(&rows).map_err(|e| invalid(e.to_string()))?,
        rows,
        issues: vec![],
        execution: ExecutionPlan {
            inverse_of: Some(op.operation_id),
            expected_namespace: ns.clone(),
            groups,
            provenance: InspectionProvenance {
                approved_plan: version,
                inventory,
                recipe,
                frozen_inputs: frozen.clone(),
                grants: grants.clone(),
                namespace_epoch: ns.namespace_epoch,
                policy_epoch: ns.policy_epoch,
            },
        },
        frozen_inputs: frozen,
        grants,
    })
}
