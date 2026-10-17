//! Deterministic, read-only rename graph compilation.
use crate::{validate::*, *};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use vidlib_inventory::{ids::*, *};

/// Build the complete selection before projecting a 200-row UI page. A blocked
/// batch has no executable groups; approval cannot discard hidden conflicts.
pub fn plan(
    inventory: &Inventory,
    evaluated: &EvaluatedBatch,
    namespace: &NamespaceSnapshot,
    version: PlanRef,
) -> Result<RenamePlan, RenameError> {
    validate_batch(inventory, evaluated, namespace)?;
    let mut rows = evaluated.rows.clone();
    let parents: TargetParents = rows
        .iter()
        .filter_map(|r| r.target_path.parent().map(|p| (r.entry_id, p.to_owned())))
        .collect();
    resolve_paths(inventory, &mut rows);
    validate_rows(inventory, &mut rows, &parents, namespace);
    resolve_collisions(
        inventory,
        &mut rows,
        namespace,
        &parents,
        &evaluated.recipe.collision_policy,
    );
    validate_rows(inventory, &mut rows, &parents, namespace);
    validate_carried_paths(inventory, &mut rows, namespace);
    let mut groups = Vec::new();
    if rows
        .iter()
        .all(|r| matches!(r.status, RowStatus::Changed | RowStatus::Unchanged))
    {
        match compile(inventory, &rows, &parents, namespace) {
            Ok(compiled) => groups = compiled,
            Err(message) => {
                for row in rows.iter_mut().filter(|r| r.status == RowStatus::Changed) {
                    issue(row, RenameErrorCode::InvalidPath, &message);
                }
            }
        }
    }
    let counts = PlanCounts::from_rows(&rows)?;
    let issues = rows.iter().flat_map(|r| r.issues.clone()).collect();
    Ok(RenamePlan {
        inverse_of: None,
        version,
        inventory_id: evaluated.inventory_id,
        generation: evaluated.generation,
        recipe_revision: evaluated.recipe_revision,
        rows,
        counts,
        issues,
        execution: ExecutionPlan {
            inverse_of: None,
            groups,
            expected_namespace: namespace.clone(),
            provenance: InspectionProvenance {
                approved_plan: version,
                inventory: inventory.clone(),
                recipe: evaluated.recipe.clone(),
                frozen_inputs: evaluated.frozen_inputs.clone(),
                grants: evaluated.grants.clone(),
                namespace_epoch: namespace.namespace_epoch,
                policy_epoch: namespace.policy_epoch,
            },
        },
        frozen_inputs: evaluated.frozen_inputs.clone(),
        grants: evaluated.grants.clone(),
    })
}

fn directory_ids(inventory: &Inventory) -> BTreeSet<EntryId> {
    inventory
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Directory)
        .map(|e| e.id)
        .collect()
}
fn resolve_paths(inventory: &Inventory, rows: &mut [PlanRow]) {
    let directories = directory_ids(inventory);
    let originals = rows.to_vec();
    fn resolve(
        index: usize,
        rows: &[PlanRow],
        dirs: &BTreeSet<EntryId>,
        visiting: &mut BTreeSet<usize>,
    ) -> Option<PathBuf> {
        if !visiting.insert(index) {
            return None;
        }
        let target = &rows[index].target_path;
        let parent = target.parent()?;
        let ancestor = rows
            .iter()
            .enumerate()
            .filter(|(i, r)| {
                *i != index && dirs.contains(&r.entry_id) && parent.starts_with(&r.source_path)
            })
            .max_by_key(|(_, r)| r.source_path.components().count());
        let result = if let Some((i, row)) = ancestor {
            resolve(i, rows, dirs, visiting)?.join(target.strip_prefix(&row.source_path).ok()?)
        } else {
            target.clone()
        };
        visiting.remove(&index);
        Some(result)
    }
    for (i, row) in rows.iter_mut().enumerate() {
        if directories.contains(&row.entry_id)
            && row.target_path != row.source_path
            && row.target_path.starts_with(&row.source_path)
        {
            issue(
                row,
                RenameErrorCode::InvalidPath,
                "directory cannot move into its own subtree",
            );
            continue;
        }
        match resolve(i, &originals, &directories, &mut BTreeSet::new()) {
            Some(target) => {
                row.target_path = target;
                refresh_row(row);
            }
            None => issue(
                row,
                RenameErrorCode::InvalidPath,
                "cyclic destination ancestry",
            ),
        }
    }
}
fn refresh_row(row: &mut PlanRow) {
    row.new_display_path = row.target_path.to_string_lossy().into_owned();
    row.new_display_name = row
        .target_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    if !matches!(row.status, RowStatus::Blocked | RowStatus::NeedsReview) {
        row.status = if row.target_path == row.source_path {
            RowStatus::Unchanged
        } else {
            RowStatus::Changed
        };
    }
}
fn resolve_collisions(
    inventory: &Inventory,
    rows: &mut [PlanRow],
    ns: &NamespaceSnapshot,
    parents: &TargetParents,
    policy: &CollisionPolicy,
) {
    let dirs = directory_ids(inventory);
    let selected: BTreeSet<_> = rows.iter().map(|r| r.source_path.clone()).collect();
    let mut processed = Vec::new();
    let mut order: Vec<_> = (0..rows.len()).collect();
    order.sort_by_key(|&i| {
        (
            !dirs.contains(&rows[i].entry_id),
            rows[i].source_path.components().count(),
            i,
        )
    });
    for i in order {
        let mut taken: Vec<(PathBuf, Option<usize>)> = ns
            .parents
            .iter()
            .flat_map(|f| &f.entries)
            .filter(|e| !selected.contains(&e.absolute_path))
            .map(|e| {
                (
                    carried_destination(&e.absolute_path, rows, &dirs)
                        .map_or_else(|| e.absolute_path.clone(), |(_, path)| path),
                    None,
                )
            })
            .collect();
        taken.extend(
            processed
                .iter()
                .map(|&j: &usize| (rows[j].target_path.clone(), Some(j))),
        );
        if rows[i].status == RowStatus::Blocked {
            continue;
        }
        let Some(facts) = facts_for(&rows[i], parents, ns) else {
            continue;
        };
        let collision = |path: &Path, occupied: &[(PathBuf, Option<usize>)]| {
            occupied.iter().position(|(p, _)| same_path(path, p, facts))
        };
        if let Some(conflict) = collision(&rows[i].target_path, &taken) {
            let mut replacement = None;
            if let CollisionPolicy::Suffix {
                separator,
                max_attempts,
            } = policy
            {
                let path = &rows[i].target_path;
                let filename = Path::new(path.file_name().unwrap());
                let (stem, ext) = if dirs.contains(&rows[i].entry_id) {
                    (filename.to_str().unwrap(), None)
                } else {
                    (
                        filename
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or_default(),
                        filename.extension().and_then(|e| e.to_str()),
                    )
                };
                for ordinal in 1..=*max_attempts {
                    let name = match ext {
                        Some(ext) => format!("{stem}{separator}{ordinal}.{ext}"),
                        None => format!("{stem}{separator}{ordinal}"),
                    };
                    let candidate = path.with_file_name(name);
                    if facts.validate_path(&candidate).is_ok()
                        && !ns.policy.excludes(&candidate)
                        && collision(&candidate, &taken).is_none()
                        && !rows
                            .iter()
                            .skip(i + 1)
                            .any(|r| same_path(&candidate, &r.target_path, facts))
                    {
                        replacement = Some(candidate);
                        break;
                    }
                }
            }
            if let Some(replacement) = replacement {
                let previous = rows[i].target_path.clone();
                rows[i].target_path = replacement.clone();
                refresh_row(&mut rows[i]);
                if dirs.contains(&rows[i].entry_id) {
                    let source = rows[i].source_path.clone();
                    for (j, row) in rows.iter_mut().enumerate().filter(|(j, _)| *j != i) {
                        if parents[&row.entry_id].starts_with(&source)
                            && j != i
                            && row.target_path != previous
                        {
                            if let Ok(relative) = row.target_path.strip_prefix(&previous) {
                                row.target_path = replacement.join(relative);
                                refresh_row(row);
                            }
                        }
                    }
                }
            } else {
                if let Some(other) = taken[conflict].1 {
                    issue(
                        &mut rows[other],
                        RenameErrorCode::Collision,
                        "multiple selected rows have the same final target",
                    );
                }
                issue(
                    &mut rows[i],
                    RenameErrorCode::Collision,
                    "final target is occupied",
                );
            }
        }
        processed.push(i);
    }
}

/// Iterative Kosaraju: dependency cycles share one recovery/temporary group.
fn components(edges: &[BTreeSet<usize>]) -> Vec<Vec<usize>> {
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for root in 0..edges.len() {
        let mut stack = vec![(root, false)];
        while let Some((node, finish)) = stack.pop() {
            if finish {
                order.push(node);
            } else if seen.insert(node) {
                stack.push((node, true));
                for &next in edges[node].iter().rev() {
                    if !seen.contains(&next) {
                        stack.push((next, false));
                    }
                }
            }
        }
    }
    let mut reverse = vec![BTreeSet::new(); edges.len()];
    for (node, next) in edges.iter().enumerate() {
        for &next in next {
            reverse[next].insert(node);
        }
    }
    seen.clear();
    let mut groups = Vec::new();
    for root in order.into_iter().rev() {
        if seen.contains(&root) {
            continue;
        }
        let mut members = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if seen.insert(node) {
                members.push(node);
                stack.extend(reverse[node].iter().copied());
            }
        }
        members.sort_unstable();
        groups.push(members);
    }
    groups
}

fn compile(
    inventory: &Inventory,
    rows: &[PlanRow],
    parents: &TargetParents,
    ns: &NamespaceSnapshot,
) -> Result<Vec<ExecutionGroup>, String> {
    let dirs = directory_ids(inventory);
    let mut edges = vec![BTreeSet::new(); rows.len()];
    for (i, row) in rows.iter().enumerate() {
        let facts = facts_for(row, parents, ns).ok_or("target policy missing")?;
        for (j, other) in rows.iter().enumerate() {
            if i == j {
                continue;
            }
            if same_path(&row.target_path, &other.source_path, facts)
                && other.status == RowStatus::Changed
            {
                edges[i].insert(j);
            }
            if row.group_id == other.group_id
                || (dirs.contains(&other.entry_id)
                    && (row.source_path.starts_with(&other.source_path)
                        || row
                            .target_path
                            .parent()
                            .is_some_and(|p| p.starts_with(&other.target_path))))
            {
                edges[i].insert(j);
                edges[j].insert(i);
            }
        }
    }
    let groups = components(&edges);
    let mut owner = vec![0; rows.len()];
    for (g, members) in groups.iter().enumerate() {
        for &i in members {
            owner[i] = g;
        }
    }
    let dependencies: Vec<BTreeSet<usize>> = groups
        .iter()
        .enumerate()
        .map(|(g, members)| {
            members
                .iter()
                .flat_map(|&i| edges[i].iter().map(|&j| owner[j]))
                .filter(|&other| other != g)
                .collect()
        })
        .collect();
    let group_ids: Vec<_> = groups
        .iter()
        .map(|members| {
            members
                .iter()
                .map(|&i| rows[i].group_id)
                .min()
                .expect("nonempty component")
        })
        .collect();
    let mut occupancy: BTreeMap<PathBuf, Option<EntryId>> = ns
        .parents
        .iter()
        .flat_map(|f| &f.entries)
        .map(|e| {
            (
                e.absolute_path.clone(),
                rows.iter()
                    .find(|r| r.source_path == e.absolute_path)
                    .map(|r| r.entry_id),
            )
        })
        .collect();
    let mut current: Vec<_> = rows.iter().map(|r| r.source_path.clone()).collect();
    let mut parent_locations: Vec<_> = ns.parents.iter().map(|f| f.parent_path.clone()).collect();
    let mut receipt: BTreeMap<EntryId, StepId> = BTreeMap::new();
    let mut finished = BTreeSet::new();
    let mut output = Vec::new();
    let mut step_ordinal = 0u64;
    while finished.len() < groups.len() {
        let g = (0..groups.len())
            .filter(|g| !finished.contains(g) && dependencies[*g].is_subset(&finished))
            .min_by_key(|g| groups[*g][0])
            .ok_or("unresolved dependency cycle")?;
        let mut group = ExecutionGroup {
            id: group_ids[g],
            depends_on: dependencies[g]
                .iter()
                .filter(|&&d| output.iter().any(|o: &ExecutionGroup| o.id == group_ids[d]))
                .map(|&d| group_ids[d])
                .collect(),
            steps: Vec::new(),
        };
        let mut pending: BTreeSet<_> = groups[g]
            .iter()
            .copied()
            .filter(|&i| current[i] != rows[i].target_path)
            .collect();
        let mut parked = BTreeSet::new();
        while !pending.is_empty() {
            // A descendant namespace write changes its directory's material
            // stamp even at a parked path. Finish every pending ancestor's final
            // move before consuming either observed or receipt-based child steps.
            let ancestor_finished = |i: usize| {
                !pending.iter().any(|&ancestor| {
                    ancestor != i
                        && dirs.contains(&rows[ancestor].entry_id)
                        && current[i].starts_with(&current[ancestor])
                })
            };
            let ready = pending
                .iter()
                .copied()
                .filter(|&i| {
                    if !ancestor_finished(i) {
                        return false;
                    }
                    let Some(parent) = rows[i].target_path.parent() else {
                        return false;
                    };
                    let Some(expected) = facts_for(&rows[i], parents, ns) else {
                        return false;
                    };
                    let Some(f) = ns
                        .parents
                        .iter()
                        .position(|f| f.parent_path == expected.parent_path)
                    else {
                        return false;
                    };
                    if parent_locations[f] != parent {
                        return false;
                    }
                    !occupancy
                        .keys()
                        .any(|p| same_path(p, &rows[i].target_path, &ns.parents[f]))
                })
                .min_by_key(|&i| (current[i].components().count(), i));
            let (i, target, temporary) = if let Some(i) = ready {
                (i, rows[i].target_path.clone(), false)
            } else {
                let i = pending
                    .iter()
                    .copied()
                    .filter(|i| !parked.contains(i) && ancestor_finished(*i))
                    .min_by_key(|&i| (current[i].components().count(), i))
                    .ok_or("no progress: pending ancestor moves cannot finish before descendant namespace writes")?;
                parked.insert(i);
                let parent = current[i].parent().ok_or("temporary parent absent")?;
                let f = parent_locations
                    .iter()
                    .position(|p| p == parent)
                    .ok_or("temporary parent policy missing")?;
                let facts = &ns.parents[f];
                let mut reserved = None;
                for suffix in 0..10_000u32 {
                    let candidate =
                        parent.join(format!("{RENAME_TEMP_PREFIX}{}-{suffix}", rows[i].entry_id));
                    if facts.validate_path(&candidate).is_ok()
                        && ns.policy.allows_rename_reservation(&candidate)
                        && !occupancy.keys().any(|p| same_path(p, &candidate, facts))
                        && !rows
                            .iter()
                            .any(|r| same_path(&r.target_path, &candidate, facts))
                    {
                        reserved = Some(candidate);
                        break;
                    }
                }
                (
                    i,
                    reserved.ok_or("no legal temporary reservation available")?,
                    true,
                )
            };
            step_ordinal += 1;
            let id: StepId = format!("00000000-0000-0000-0000-{step_ordinal:012x}")
                .parse()
                .map_err(|_| "step ID overflow")?;
            let from = current[i].clone();
            let identity = receipt.get(&rows[i].entry_id).map_or_else(
                || ExpectedIdentity::Observed(rows[i].stamp.clone()),
                |step_id| ExpectedIdentity::FromReceipt { step_id: *step_id },
            );
            group.steps.push(ExecutionStep {
                id,
                entry_id: rows[i].entry_id,
                group_id: group.id,
                action: ExecutionAction::RenameEntry {
                    source: EntryPrestate {
                        path: from.clone(),
                        identity,
                    },
                    target: target.clone(),
                    temporary,
                },
            });
            receipt.insert(rows[i].entry_id, id);
            let relocate = |path: &Path| {
                path.strip_prefix(&from).map_or_else(
                    |_| path.to_owned(),
                    |relative| {
                        if relative.as_os_str().is_empty() {
                            target.clone()
                        } else {
                            target.join(relative)
                        }
                    },
                )
            };
            occupancy = occupancy
                .into_iter()
                .map(|(p, id)| (relocate(&p), id))
                .collect();
            for path in &mut current {
                *path = relocate(path);
            }
            for path in &mut parent_locations {
                *path = relocate(path);
            }
            if !temporary {
                pending.remove(&i);
            }
            pending.retain(|&i| current[i] != rows[i].target_path);
        }
        finished.insert(g);
        if !group.steps.is_empty() {
            output.push(group);
        }
    }
    Ok(output)
}
