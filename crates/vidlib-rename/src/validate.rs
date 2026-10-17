//! Pure validation of observed native paths. No filesystem calls.
use crate::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};
use vidlib_inventory::{ids::EntryId, *};

pub(crate) fn issue(row: &mut PlanRow, code: RenameErrorCode, message: &str) {
    row.status = RowStatus::Blocked;
    row.issues.push(PlanIssue {
        code,
        entry_id: Some(row.entry_id),
        rule_id: None,
        message: message.into(),
    });
}
pub(crate) fn clean(path: &Path) -> bool {
    path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir))
}
pub(crate) fn validate_batch(
    inventory: &Inventory,
    batch: &EvaluatedBatch,
    ns: &NamespaceSnapshot,
) -> Result<(), RenameError> {
    let invalid = |message| RenameError::new(RenameErrorCode::InvalidInventory, message);
    batch.recipe.validate()?;
    if !batch.recipe.metadata_changes.is_empty() {
        return Err(RenameError::new(
            RenameErrorCode::UnsupportedConfiguration,
            "metadata effects require their owning compiler",
        ));
    }
    if inventory.id != batch.inventory_id
        || inventory.generation != batch.generation
        || batch.recipe.revision != batch.recipe_revision
        || batch.frozen_inputs.inventory_id != inventory.id
        || batch.frozen_inputs.generation != inventory.generation
        || batch.frozen_inputs.primary_order != batch.recipe.numbering_order
        || batch.frozen_inputs.numbering_scope != batch.recipe.numbering_scope
    {
        return Err(invalid(
            "evaluated provenance differs from inventory/recipe",
        ));
    }
    if batch.grants != ns.grants {
        return Err(invalid("namespace grants differ from evaluated authority"));
    }
    let entries: BTreeMap<_, _> = inventory.entries.iter().map(|e| (e.id, e)).collect();
    let selected: BTreeSet<_> = batch.recipe.selection.iter().copied().collect();
    let row_ids: BTreeSet<_> = batch.rows.iter().map(|r| r.entry_id).collect();
    if batch
        .rows
        .iter()
        .map(|r| &r.source_path)
        .collect::<BTreeSet<_>>()
        .len()
        != batch.rows.len()
        || entries.len() != inventory.entries.len()
        || row_ids.len() != batch.rows.len()
        || row_ids != selected
    {
        return Err(invalid("complete selected entry set is required"));
    }
    let associations = associate(inventory, &batch.recipe.selection, &batch.recipe.companions)?;
    if associations
        .primary_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        != batch.recipe.numbering_order.iter().copied().collect()
    {
        return Err(invalid(
            "primary order differs from resolved companion ownership",
        ));
    }
    for row in &batch.rows {
        let owner = associations
            .assets
            .iter()
            .find(|a| a.asset_id == row.entry_id)
            .and_then(|a| a.owner)
            .unwrap_or(row.entry_id);
        // Receipt-derived inverse/cycle/ancestor groups can legitimately merge
        // independent primaries. Companion ownership requires the actual owner
        // row's group, not a freshly derived ID that discards historical lineage.
        let owner_group = batch
            .rows
            .iter()
            .find(|r| r.entry_id == owner)
            .ok_or_else(|| invalid("missing companion primary"))?
            .group_id;
        if row.primary_id != owner || (owner != row.entry_id && row.group_id != owner_group) {
            return Err(invalid(
                "row companion lineage differs from resolved ownership",
            ));
        }
        if let Some(asset) = associations
            .assets
            .iter()
            .find(|a| a.asset_id == row.entry_id && a.owner.is_some())
        {
            let primary = batch
                .rows
                .iter()
                .find(|r| r.entry_id == owner)
                .ok_or_else(|| invalid("missing companion primary"))?;
            if !matches!(row.status, RowStatus::Blocked | RowStatus::NeedsReview) {
                let stem = primary
                    .target_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| invalid("unsupported primary name"))?;
                let expected = row
                    .source_path
                    .with_file_name(format!("{stem}{}", asset.retained_suffix));
                if row.target_path != expected {
                    return Err(invalid(
                        "companion target differs from its final primary name",
                    ));
                }
            }
        }
        let entry = entries
            .get(&row.entry_id)
            .ok_or_else(|| invalid("unknown source"))?;
        let root = inventory
            .roots
            .iter()
            .find(|r| r.id == entry.root_id)
            .ok_or_else(|| invalid("unknown source root"))?;
        if row.source_path != entry.absolute_path
            || row.stamp != entry.stamp
            || !clean(&entry.absolute_path)
            || !entry.absolute_path.starts_with(&root.path)
            || !batch.grants.grants.iter().any(|g| {
                g.id == root.source_grant.id
                    && g.path == root.source_grant.path
                    && entry.absolute_path.starts_with(&g.path)
            })
        {
            return Err(invalid("source differs from inventory/grant"));
        }
    }
    let mut parents = BTreeSet::new();
    for facts in &ns.parents {
        if !clean(&facts.parent_path) || !parents.insert(&facts.parent_path) {
            return Err(RenameError::new(
                RenameErrorCode::InvalidNamespace,
                "duplicate or invalid namespace parent",
            ));
        }
        let mut paths = BTreeSet::new();
        for entry in &facts.entries {
            if entry.absolute_path.parent() != Some(facts.parent_path.as_path())
                || entry
                    .absolute_path
                    .file_name()
                    .map(|n| encode_path(Path::new(n)))
                    != Some(entry.name.clone())
                || !paths.insert(&entry.absolute_path)
            {
                return Err(RenameError::new(
                    RenameErrorCode::InvalidNamespace,
                    "namespace entry path/name mismatch",
                ));
            }
        }
    }
    Ok(())
}

/// Targets are bound to the originally observed parent identity before ancestor
/// spelling changes. Inverting a final path is ambiguous during swaps/suffixes.
pub(crate) type TargetParents = BTreeMap<EntryId, PathBuf>;
pub(crate) fn facts_for<'a>(
    row: &PlanRow,
    parents: &TargetParents,
    ns: &'a NamespaceSnapshot,
) -> Option<&'a DirectoryFacts> {
    let parent = parents.get(&row.entry_id)?;
    ns.parents.iter().find(|f| &f.parent_path == parent)
}
pub(crate) fn same_path(left: &Path, right: &Path, facts: &DirectoryFacts) -> bool {
    left.parent() == right.parent()
        && left
            .file_name()
            .zip(right.file_name())
            .is_some_and(|(a, b)| facts.names_equal(a, b).unwrap_or(true))
}

pub(crate) fn validate_rows(
    inventory: &Inventory,
    rows: &mut [PlanRow],
    parents: &TargetParents,
    ns: &NamespaceSnapshot,
) {
    for row in rows {
        let entry = inventory
            .entries
            .iter()
            .find(|e| e.id == row.entry_id)
            .expect("validated");
        if row.status == RowStatus::Blocked {
            continue;
        }
        if !entry.issues.is_empty()
            || !matches!(entry.kind, EntryKind::File | EntryKind::Directory)
            || row
                .source_path
                .file_name()
                .and_then(|n| n.to_str())
                .is_none()
            || row
                .target_path
                .file_name()
                .and_then(|n| n.to_str())
                .is_none()
        {
            issue(
                row,
                RenameErrorCode::UnsupportedName,
                "unsupported source observation/name",
            );
            continue;
        }
        if !clean(&row.target_path) {
            issue(
                row,
                RenameErrorCode::InvalidPath,
                "destination must be absolute without traversal",
            );
            continue;
        }
        let Some(original_target_parent) = parents.get(&row.entry_id) else {
            issue(row, RenameErrorCode::InvalidPath, "target has no parent");
            continue;
        };
        let authorized = ns.grants.grants.iter().any(|g| {
            original_target_parent.starts_with(&g.path)
                || (g.path == row.source_path
                    && row.target_path.parent() == row.source_path.parent()
                    && entry.kind == EntryKind::File)
        });
        if !authorized {
            issue(
                row,
                RenameErrorCode::OutsideGrant,
                "destination is outside grants",
            );
            continue;
        }
        if ns.policy.guards_move(&row.source_path)
            || ns.policy.excludes(&row.target_path)
            || ns
                .policy
                .guarded_roots
                .iter()
                .any(|r| r == &row.target_path)
            || (entry.kind == EntryKind::Directory
                && inventory
                    .roots
                    .iter()
                    .any(|r| r.path == row.source_path || r.path.starts_with(&row.source_path)))
        {
            issue(
                row,
                RenameErrorCode::ProtectedEntry,
                "root, excluded subtree or active marker is protected",
            );
            continue;
        }
        if entry.kind == EntryKind::Directory
            && row.target_path != row.source_path
            && row.target_path.starts_with(&row.source_path)
        {
            issue(
                row,
                RenameErrorCode::InvalidPath,
                "directory cannot move into its descendant",
            );
            continue;
        }
        if entry.kind == EntryKind::Directory {
            row.carried_descendants = inventory
                .entries
                .iter()
                .filter(|e| {
                    e.absolute_path != row.source_path
                        && e.absolute_path.starts_with(&row.source_path)
                })
                .count() as u64;
            if !inventory.coverage.iter().any(|c| {
                c.directory == row.source_path
                    && c.outcome == CoverageOutcome::Complete
                    && c.subtree_complete
                    && c.children_complete
                    && c.issues.is_empty()
            }) || inventory.coverage.iter().any(|c| {
                c.directory.starts_with(&row.source_path)
                    && (c.outcome != CoverageOutcome::Complete || !c.subtree_complete)
            }) || inventory.entries.iter().any(|e| {
                e.absolute_path.starts_with(&row.source_path)
                    && (!e.issues.is_empty() || ns.policy.excludes(&e.absolute_path))
            }) || ns.parents.iter().flat_map(|f| &f.entries).any(|e| {
                e.absolute_path.starts_with(&row.source_path)
                    && (e.protected || ns.policy.excludes(&e.absolute_path))
            }) {
                issue(
                    row,
                    RenameErrorCode::IncompleteCoverage,
                    "move would carry excluded, protected or unobserved descendants",
                );
                continue;
            }
        }
        let Some(source_facts) = ns
            .parents
            .iter()
            .find(|f| Some(f.parent_path.as_path()) == row.source_path.parent())
        else {
            issue(
                row,
                RenameErrorCode::InvalidNamespace,
                "source parent not observed",
            );
            continue;
        };
        let Some(source) = source_facts
            .entries
            .iter()
            .find(|e| e.absolute_path == row.source_path)
        else {
            issue(
                row,
                RenameErrorCode::InvalidNamespace,
                "source absent from namespace",
            );
            continue;
        };
        if !material_stamp_matches(&source.stamp, &row.stamp)
            || source.kind != entry.kind
            || source.protected
        {
            issue(
                row,
                RenameErrorCode::InvalidNamespace,
                "source observation changed or protected",
            );
            continue;
        }
        let Some(facts) = facts_for(row, parents, ns) else {
            issue(
                row,
                RenameErrorCode::InvalidNamespace,
                "target directory not observed",
            );
            continue;
        };
        if !source_facts.children_complete
            || !source_facts.issues.is_empty()
            || !facts.children_complete
            || !facts.issues.is_empty()
            || facts.case_sensitivity == CaseSensitivity::Unknown
            || facts.normalization != NameNormalization::Preserved
            || facts.capabilities.read_only != Some(false)
            || facts.capabilities.atomic_rename != Support::Supported
            || source_facts.capabilities.read_only != Some(false)
            || source_facts.capabilities.atomic_rename != Support::Supported
            || !facts
                .source_grants
                .iter()
                .any(|id| ns.grants.grants.iter().any(|g| &g.id == id))
        {
            issue(
                row,
                RenameErrorCode::InvalidNamespace,
                "incomplete or unsupported directory policy/capabilities",
            );
            continue;
        }
        match (&row.stamp.physical_id, &facts.stamp.physical_id) {
            (Some(source), Some(parent))
                if source.platform == parent.platform
                    && source.volume_identity == parent.volume_identity => {}
            _ => {
                issue(row, RenameErrorCode::UnsupportedConfiguration, "same-volume identity evidence required; cross-volume copy is not compiled here");
                continue;
            }
        }
        if facts.validate_path(&row.target_path).is_err() {
            issue(
                row,
                RenameErrorCode::InvalidPath,
                "target violates native component/path policy",
            );
        }
    }
}

/// The nearest selected directory owns an unselected descendant's final location.
/// Shared by collision occupancy and final carried-path validation so both use
/// the same frozen directory targets after hierarchy resolution and suffixing.
pub(crate) fn carried_destination(
    path: &Path,
    rows: &[PlanRow],
    directories: &BTreeSet<EntryId>,
) -> Option<(usize, PathBuf)> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| {
            directories.contains(&row.entry_id)
                && path != row.source_path
                && path.starts_with(&row.source_path)
        })
        .max_by_key(|(_, row)| row.source_path.components().count())
        .map(|(index, row)| {
            (
                index,
                row.target_path
                    .join(path.strip_prefix(&row.source_path).expect("prefix checked")),
            )
        })
}

/// Selected descendants have their own row/target-parent validation. Validate
/// every other carried inventory entry under its original parent's observed
/// identity and policy, after all directory destinations and suffixes are frozen.
pub(crate) fn validate_carried_paths(
    inventory: &Inventory,
    rows: &mut [PlanRow],
    ns: &NamespaceSnapshot,
) {
    let selected: BTreeSet<_> = rows.iter().map(|row| row.entry_id).collect();
    let directories: BTreeSet<_> = inventory
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Directory)
        .map(|entry| entry.id)
        .collect();
    for entry in inventory
        .entries
        .iter()
        .filter(|entry| !selected.contains(&entry.id))
    {
        let Some((owner, final_path)) =
            carried_destination(&entry.absolute_path, rows, &directories)
        else {
            continue;
        };
        if final_path == entry.absolute_path || rows[owner].status == RowStatus::Blocked {
            continue;
        }
        let facts = ns
            .parents
            .iter()
            .find(|facts| Some(facts.parent_path.as_path()) == entry.absolute_path.parent());
        let Some(facts) = facts.filter(|facts| {
            facts.stamp.physical_id.is_some()
                && facts.children_complete
                && facts.issues.is_empty()
                && facts.name_policy != NamePolicy::Unknown
                && facts.case_sensitivity != CaseSensitivity::Unknown
                && facts.normalization == NameNormalization::Preserved
        }) else {
            issue(
                &mut rows[owner],
                RenameErrorCode::InvalidNamespace,
                &format!(
                    "original parent identity/name facts unavailable for carried entry {}",
                    entry.id
                ),
            );
            continue;
        };
        if facts.validate_path(&final_path).is_err() {
            issue(
                &mut rows[owner],
                RenameErrorCode::InvalidPath,
                &format!(
                    "final path of carried entry {} violates its original parent policy",
                    entry.id
                ),
            );
        }
    }
}

/// Pure preflight comparison; the executor supplies a fresh capture under its
/// write lease, then enforces no-replace again at every native mutation.
pub fn preflight(execution: &ExecutionPlan, fresh: &NamespaceSnapshot) -> Result<(), RenameError> {
    let expected = &execution.expected_namespace;
    let mismatch = || {
        RenameError::new(
            RenameErrorCode::InvalidNamespace,
            "namespace or protected policy changed since preview",
        )
    };
    if fresh.namespace_epoch != execution.provenance.namespace_epoch
        || fresh.policy_epoch != execution.provenance.policy_epoch
        || fresh.namespace_epoch != expected.namespace_epoch
        || fresh.policy_epoch != expected.policy_epoch
        || fresh.grants != expected.grants
        || fresh.grants != execution.provenance.grants
        || fresh.policy.guarded_roots != expected.policy.guarded_roots
        || fresh.policy.protected_roots != expected.policy.protected_roots
        || fresh.policy.marker_name != expected.policy.marker_name
        || fresh.policy.staging_prefixes != expected.policy.staging_prefixes
    {
        return Err(mismatch());
    }
    fn normalized(facts: &[DirectoryFacts]) -> Vec<DirectoryFacts> {
        let mut facts = facts.to_vec();
        for f in &mut facts {
            f.window = ObservationWindow::default();
            f.entries
                .sort_by(|a, b| a.absolute_path.cmp(&b.absolute_path));
            f.source_grants.sort();
        }
        facts.sort_by(|a, b| a.parent_path.cmp(&b.parent_path));
        facts
    }
    let mut actual_parents = normalized(&fresh.parents);
    let expected_parents = normalized(&expected.parents);
    if actual_parents.len() != expected_parents.len() {
        return Err(mismatch());
    }
    for (actual, expected) in actual_parents.iter_mut().zip(&expected_parents) {
        if !material_stamp_matches(&actual.stamp, &expected.stamp)
            || actual.entries.len() != expected.entries.len()
        {
            return Err(mismatch());
        }
        // Compare all other native namespace/policy facts exactly. These are
        // local comparison copies; retain complete observed stamps for inspection.
        actual.stamp = expected.stamp.clone();
        for (actual_entry, expected_entry) in actual.entries.iter_mut().zip(&expected.entries) {
            if !material_stamp_matches(&actual_entry.stamp, &expected_entry.stamp) {
                return Err(mismatch());
            }
            actual_entry.stamp = expected_entry.stamp.clone();
        }
        if actual != expected {
            return Err(mismatch());
        }
    }
    Ok(())
}

/// Same material freshness contract as the native adapter's check_stamp, kept
/// pure here to preserve inventory <- rename <- fileops dependency direction.
/// Our own observations/content reads may advance access time. A supplied
/// creation time must remain available; an initially absent one is optional.
fn material_stamp_matches(actual: &FileStamp, expected: &FileStamp) -> bool {
    expected.physical_id.is_some()
        && actual.physical_id == expected.physical_id
        && actual.size == expected.size
        && actual.modified_ns == expected.modified_ns
        && expected
            .created_ns
            .is_none_or(|t| actual.created_ns == Some(t))
        && actual.windows_attributes == expected.windows_attributes
}
