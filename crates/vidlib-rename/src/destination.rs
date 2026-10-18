//! Pure destination layout resolution after primary/companion evaluation.
use crate::*;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};
use vidlib_inventory::{ids::*, EntryKind, GrantSet, Inventory, NamespaceSnapshot};
fn invalid(message: &str) -> RenameError {
    RenameError::new(RenameErrorCode::InvalidPath, message)
}
pub fn resolve_destinations(
    mut evaluated: EvaluatedBatch,
    policy: &DestinationPolicy,
    grants: &GrantSet,
) -> Result<EvaluatedBatch, RenameError> {
    if policy != &evaluated.recipe.destination {
        return Err(invalid("destination policy differs from evaluated recipe"));
    }
    let (grant_id, layout) = match policy {
        Destination::SameDirectory => return Ok(evaluated),
        Destination::CopyTo { grant_id, layout } | Destination::MoveTo { grant_id, layout } => {
            (grant_id, layout)
        }
        Destination::Granted { .. } => {
            return Err(RenameError::new(
                RenameErrorCode::UnsupportedConfiguration,
                "choose explicit Copy or Move for the legacy destination",
            ))
        }
    };
    let root = &grants
        .grants
        .iter()
        .find(|g| g.id == *grant_id)
        .ok_or_else(|| {
            RenameError::new(
                RenameErrorCode::OutsideGrant,
                "current native destination grant required",
            )
        })?
        .path;
    if !crate::validate::clean(root)
        || evaluated
            .grants
            .grants
            .iter()
            .any(|g| !grants.grants.iter().any(|a| a == g))
    {
        return Err(invalid(
            "destination authority is not the current native grant",
        ));
    }
    let original_rows = evaluated.rows.clone();
    for row in &mut evaluated.rows {
        let entry = evaluated
            .inventory
            .entries
            .iter()
            .find(|e| e.id == row.entry_id)
            .ok_or_else(|| invalid("destination source missing"))?;
        let relative = match layout {
            DestinationLayout::PreserveHierarchy => {
                let source_root = evaluated
                    .inventory
                    .roots
                    .iter()
                    .find(|r| r.id == entry.root_id)
                    .ok_or_else(|| invalid("source root missing"))?;
                entry
                    .absolute_path
                    .strip_prefix(&source_root.path)
                    .ok()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| {
                        Path::new(entry.absolute_path.file_name().unwrap_or_default())
                    })
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_owned()
            }
            DestinationLayout::FolderTemplate { parts } => {
                let primary = original_rows
                    .iter()
                    .find(|r| r.entry_id == row.primary_id)
                    .ok_or_else(|| invalid("destination primary missing"))?;
                let primary_entry = evaluated
                    .inventory
                    .entries
                    .iter()
                    .find(|e| e.id == primary.entry_id)
                    .ok_or_else(|| invalid("destination primary inventory missing"))?;
                let original = NameParts::from_filename(&primary.old_display_name);
                let current = NameParts::from_filename(&primary.new_display_name);
                match render_template(
                    parts,
                    &TemplateContext {
                        scope: FrozenScope::Destination,
                        entry: primary_entry,
                        original: &original,
                        current: &current,
                        rule_id: "",
                        inputs: &evaluated.frozen_inputs,
                    },
                ) {
                    Ok(text) => PathBuf::from(text),
                    Err(error) => {
                        crate::validate::issue(
                            row,
                            if error.code == RuleIssueCode::OutputLimit {
                                RenameErrorCode::OutputLimit
                            } else {
                                RenameErrorCode::InvalidRuleInput
                            },
                            &error.message,
                        );
                        continue;
                    }
                }
            }
        };
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || relative.to_string_lossy().contains(['\\', '\0', ':'])
        {
            return Err(invalid(
                "folder template must be a relative path without traversal or drive syntax",
            ));
        }
        row.target_path = root.join(relative).join(
            row.target_path
                .file_name()
                .ok_or_else(|| invalid("missing destination name"))?,
        );
        if row.target_path == row.source_path
            || (entry.kind == EntryKind::Directory && row.target_path.starts_with(&row.source_path))
        {
            return Err(invalid("destination overlaps the source entry"));
        }
        row.new_display_path = row.target_path.to_string_lossy().into_owned();
        if !matches!(row.status, RowStatus::Blocked | RowStatus::NeedsReview) {
            row.status = RowStatus::Changed;
        }
    }
    if matches!(layout, DestinationLayout::PreserveHierarchy) {
        // A selected descendant follows its nearest selected directory's final
        // location, retaining intervening unselected directories. Resolve in
        // source depth order so renamed nested directories compose as well.
        let mut order: Vec<_> = (0..evaluated.rows.len()).collect();
        order.sort_by_key(|&i| evaluated.rows[i].source_path.components().count());
        for i in order {
            let row = &evaluated.rows[i];
            let ancestor = evaluated
                .rows
                .iter()
                .enumerate()
                .filter(|(j, r)| {
                    *j != i
                        && row.source_path.starts_with(&r.source_path)
                        && evaluated
                            .inventory
                            .entries
                            .iter()
                            .any(|e| e.id == r.entry_id && e.kind == EntryKind::Directory)
                })
                .max_by_key(|(_, r)| r.source_path.components().count());
            if let Some((_, ancestor)) = ancestor {
                let relative = row
                    .source_path
                    .parent()
                    .and_then(|p| p.strip_prefix(&ancestor.source_path).ok())
                    .ok_or_else(|| invalid("descendant parent missing"))?;
                let target = ancestor.target_path.join(relative).join(
                    row.target_path
                        .file_name()
                        .ok_or_else(|| invalid("descendant filename missing"))?,
                );
                evaluated.rows[i].target_path = target;
                evaluated.rows[i].new_display_path =
                    evaluated.rows[i].target_path.to_string_lossy().into_owned();
            }
        }
    }
    evaluated.grants = grants.clone();
    Ok(evaluated)
}
/// Include source descendants and destination ancestor candidates. The native
/// adapter captures the nearest existing parent without inventing a directory.
pub fn capture_candidates(batch: &EvaluatedBatch) -> Vec<PathBuf> {
    let dirs: Vec<_> = batch
        .rows
        .iter()
        .filter(|r| {
            batch
                .inventory
                .entries
                .iter()
                .any(|e| e.id == r.entry_id && e.kind == EntryKind::Directory)
        })
        .map(|r| &r.source_path)
        .collect();
    let carried = |p: &Path| dirs.iter().any(|d| p.starts_with(d));
    let explicit = matches!(
        batch.recipe.destination,
        Destination::CopyTo { .. } | Destination::MoveTo { .. }
    );
    let mut result: Vec<_> = batch
        .rows
        .iter()
        .flat_map(|r| {
            let covered_target = batch.rows.iter().any(|ancestor| {
                ancestor.entry_id != r.entry_id
                    && ancestor.target_path != r.target_path
                    && r.target_path.starts_with(&ancestor.target_path)
                    && dirs.contains(&&ancestor.source_path)
            });
            std::iter::once(r.source_path.clone())
                .chain((explicit && !covered_target).then(|| r.target_path.clone()))
        })
        .chain(
            batch
                .inventory
                .entries
                .iter()
                .filter(|e| carried(&e.absolute_path))
                .map(|e| e.absolute_path.clone()),
        )
        .chain(
            batch
                .inventory
                .coverage
                .iter()
                .filter(|c| carried(&c.directory))
                .map(|c| c.directory.join(".vidlib-capture")),
        )
        .collect();
    if let Destination::CopyTo { grant_id, .. } | Destination::MoveTo { grant_id, .. } =
        batch.recipe.destination
    {
        if let Some(grant) = batch.grants.grants.iter().find(|g| g.id == grant_id) {
            result.push(grant.path.join(".vidlib-capture"));
        }
    }
    result.sort();
    result.dedup();
    result
}
pub(crate) fn compile(
    inventory: &Inventory,
    rows: &[PlanRow],
    ns: &NamespaceSnapshot,
    move_to: bool,
) -> Result<Vec<ExecutionGroup>, String> {
    let mut ordinal = 0u64;
    let mut groups = BTreeMap::<GroupId, ExecutionGroup>::new();
    let mut directory_owners = BTreeMap::<PathBuf, GroupId>::new();
    // Common created parents make one compensation unit. Existing parents do not.
    let mut owners: Vec<GroupId> = rows.iter().map(|r| r.group_id).collect();
    for (i, row) in rows.iter().enumerate() {
        for (j, other) in rows.iter().enumerate() {
            let shared_created = row
                .target_path
                .ancestors()
                .skip(1)
                .take_while(|p| !ns.parents.iter().any(|f| f.parent_path == *p))
                .any(|p| other.target_path.parent().is_some_and(|q| q.starts_with(p)));
            if row.source_path.starts_with(&other.source_path) || shared_created {
                let old = owners[i].max(owners[j]);
                let new = owners[i].min(owners[j]);
                for owner in &mut owners {
                    if *owner == old {
                        *owner = new;
                    }
                }
            }
        }
    }
    let native_move = move_to
        && rows.iter().all(|row| {
            let parent = row
                .target_path
                .ancestors()
                .skip(1)
                .find_map(|path| ns.parents.iter().find(|p| p.parent_path == path));
            parent.is_some_and(|p| {
                p.capabilities.atomic_rename == vidlib_inventory::Support::Supported
                    && p.stamp
                        .physical_id
                        .as_ref()
                        .zip(row.stamp.physical_id.as_ref())
                        .is_some_and(|(a, b)| {
                            a.platform == b.platform && a.volume_identity == b.volume_identity
                        })
            }) && !ns
                .parents
                .iter()
                .flat_map(|p| &p.entries)
                .any(|e| e.absolute_path == row.target_path)
                && !rows.iter().any(|other| {
                    other.entry_id != row.entry_id
                        && (row.source_path.starts_with(&other.source_path)
                            || other.source_path.starts_with(&row.source_path))
                })
        });
    if native_move {
        for (i, row) in rows.iter().enumerate() {
            let group_id = owners[i];
            let group = groups.entry(group_id).or_insert_with(|| ExecutionGroup {
                id: group_id,
                depends_on: vec![],
                steps: vec![],
            });
            let mut missing: Vec<_> = row
                .target_path
                .ancestors()
                .skip(1)
                .take_while(|p| !ns.parents.iter().any(|f| f.parent_path == *p))
                .map(Path::to_owned)
                .collect();
            missing.reverse();
            for path in missing {
                if directory_owners.contains_key(&path) {
                    continue;
                }
                if ns
                    .parents
                    .iter()
                    .flat_map(|p| &p.entries)
                    .any(|e| e.absolute_path == path)
                {
                    return Err("native destination ancestor is occupied".into());
                }
                ordinal += 1;
                group.steps.push(ExecutionStep {
                    id: format!("00000000-0000-0000-0000-{ordinal:012x}")
                        .parse()
                        .map_err(|_| "step overflow")?,
                    entry_id: row.entry_id,
                    group_id,
                    action: ExecutionAction::CreateDirectory {
                        target: path.clone(),
                    },
                });
                directory_owners.insert(path, group_id);
            }
            ordinal += 1;
            group.steps.push(ExecutionStep {
                id: format!("00000000-0000-0000-0000-{ordinal:012x}")
                    .parse()
                    .map_err(|_| "step overflow")?,
                entry_id: row.entry_id,
                group_id,
                action: ExecutionAction::RenameEntry {
                    source: EntryPrestate {
                        path: row.source_path.clone(),
                        identity: ExpectedIdentity::Observed(row.stamp.clone()),
                    },
                    target: row.target_path.clone(),
                    temporary: false,
                },
            });
        }
        return Ok(groups.into_values().collect());
    }
    let mut materials = BTreeMap::<PathBuf, (usize, PathBuf)>::new();
    for (i, row) in rows.iter().enumerate() {
        materials.insert(row.source_path.clone(), (i, row.target_path.clone()));
        if inventory
            .entries
            .iter()
            .any(|e| e.id == row.entry_id && e.kind == EntryKind::Directory)
        {
            for child in inventory.entries.iter().filter(|e| {
                e.absolute_path != row.source_path && e.absolute_path.starts_with(&row.source_path)
            }) {
                let target = rows
                    .iter()
                    .find(|r| r.source_path == child.absolute_path)
                    .map(|r| r.target_path.clone())
                    .unwrap_or_else(|| {
                        row.target_path.join(
                            child
                                .absolute_path
                                .strip_prefix(&row.source_path)
                                .expect("child"),
                        )
                    });
                materials
                    .entry(child.absolute_path.clone())
                    .and_modify(|(owner, mapped)| {
                        if rows[*owner].source_path.components().count()
                            < row.source_path.components().count()
                        {
                            *owner = i;
                            *mapped = target.clone();
                        }
                    })
                    .or_insert((i, target));
            }
        }
    }
    for (source_path, (i, target)) in &materials {
        let row = &rows[*i];
        let group_id = owners[*i];
        let group = groups.entry(group_id).or_insert_with(|| ExecutionGroup {
            id: group_id,
            depends_on: vec![],
            steps: vec![],
        });
        let entry = inventory
            .entries
            .iter()
            .find(|e| e.absolute_path == *source_path)
            .ok_or("source missing")?;
        let mut push = |action| -> Result<StepId, String> {
            ordinal += 1;
            let id = format!("00000000-0000-0000-0000-{ordinal:012x}")
                .parse()
                .map_err(|_| "step overflow")?;
            group.steps.push(ExecutionStep {
                id,
                entry_id: row.entry_id,
                group_id,
                action,
            });
            Ok(id)
        };
        let mut missing: Vec<_> = target
            .ancestors()
            .skip(if entry.kind == EntryKind::Directory {
                0
            } else {
                1
            })
            .take_while(|p| !ns.parents.iter().any(|f| f.parent_path == *p))
            .map(Path::to_owned)
            .collect();
        missing.reverse();
        for directory in missing {
            if directory_owners.contains_key(&directory) {
                continue;
            }
            if ns
                .parents
                .iter()
                .flat_map(|f| &f.entries)
                .any(|e| e.absolute_path == directory)
            {
                return Err(
                    "destination ancestor already exists but has no complete directory capture"
                        .into(),
                );
            }
            push(ExecutionAction::CreateDirectory {
                target: directory.clone(),
            })?;
            directory_owners.insert(directory, group_id);
        }
        if entry.kind == EntryKind::Directory {
            continue;
        }
        if entry.kind != EntryKind::File {
            return Err("special entries cannot be copied".into());
        }
        let staging = target.with_file_name(format!(".vidlib-staging-{}", entry.id));
        if ns
            .parents
            .iter()
            .flat_map(|f| &f.entries)
            .any(|e| e.absolute_path == staging)
            || !ns.policy.allows_owned_staging(&staging)
        {
            return Err("compiled staging reservation is unavailable".into());
        }
        let source = EntryPrestate {
            path: source_path.clone(),
            identity: ExpectedIdentity::Observed(entry.stamp.clone()),
        };
        let create = push(ExecutionAction::CreateStaging {
            staging: staging.clone(),
        })?;
        let write = push(ExecutionAction::WriteStaging {
            source: source.clone(),
            staging: Box::new(OwnedEntryPrestate {
                entry: EntryPrestate {
                    path: staging.clone(),
                    identity: ExpectedIdentity::FromReceipt { step_id: create },
                },
                created_by: create,
            }),
        })?;
        let published = push(ExecutionAction::PublishCopy {
            staging: OwnedEntryPrestate {
                entry: EntryPrestate {
                    path: staging,
                    identity: ExpectedIdentity::FromReceipt { step_id: write },
                },
                created_by: create,
            },
            target: target.clone(),
            verified_by: write,
        })?;
        if move_to {
            let quarantine = source.path.with_file_name(format!(
                "{}remove-{}",
                vidlib_inventory::RENAME_TEMP_PREFIX,
                entry.id
            ));
            if ns
                .parents
                .iter()
                .flat_map(|f| &f.entries)
                .any(|e| e.absolute_path == quarantine)
                || !ns.policy.allows_rename_reservation(&quarantine)
            {
                return Err("source quarantine reservation is unavailable".into());
            }
            let moved = push(ExecutionAction::RenameEntry {
                source,
                target: quarantine.clone(),
                temporary: true,
            })?;
            push(ExecutionAction::RemoveSource {
                source: EntryPrestate {
                    path: quarantine,
                    identity: ExpectedIdentity::FromReceipt { step_id: moved },
                },
                published_by: published,
            })?;
        }
    }
    // Created directory metadata is finalized deepest first, after every child.
    let mut directories: Vec<_> = materials
        .iter()
        .filter(|(path, _)| {
            inventory
                .entries
                .iter()
                .any(|e| e.absolute_path == **path && e.kind == EntryKind::Directory)
        })
        .collect();
    directories.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    for (source, (i, target)) in directories {
        let group = groups
            .get_mut(&owners[*i])
            .ok_or("directory group missing")?;
        let Some(created) = group.steps.iter().find(|s| matches!(&s.action, ExecutionAction::CreateDirectory { target: p } if p == target)).map(|s| s.id) else { continue; };
        let original = inventory
            .entries
            .iter()
            .find(|e| e.absolute_path == *source)
            .ok_or("directory original missing")?
            .stamp
            .clone();
        let effects = group
            .steps
            .iter()
            .filter(|s| {
                action_paths(&s.action)
                    .iter()
                    .any(|p| *p != target && p.starts_with(target))
            })
            .map(|s| s.id)
            .collect();
        ordinal += 1;
        group.steps.push(ExecutionStep {
            id: format!("00000000-0000-0000-0000-{ordinal:012x}")
                .parse()
                .map_err(|_| "step overflow")?,
            entry_id: rows[*i].entry_id,
            group_id: owners[*i],
            action: ExecutionAction::SetMetadata {
                entry: EntryPrestate {
                    path: target.clone(),
                    identity: ExpectedIdentity::DirectoryAfterEffects {
                        original: original.clone(),
                        created_by: Some(created),
                        effects,
                    },
                },
                changes: directory_metadata_changes(&original),
            },
        });
        if move_to {
            let effects: Vec<_> = group
                .steps
                .iter()
                .filter(|s| {
                    action_paths(&s.action)
                        .iter()
                        .any(|p| *p != source && p.starts_with(source))
                })
                .map(|s| s.id)
                .collect();
            let required_descendants: Vec<_> = group
                .steps
                .iter()
                .filter(|s| {
                    action_paths(&s.action).iter().any(|p| {
                        (*p != source && p.starts_with(source))
                            || (*p != target && p.starts_with(target))
                    })
                })
                .map(|s| s.id)
                .collect();
            let quarantine = source.with_file_name(format!(
                "{}remove-directory-{}",
                vidlib_inventory::RENAME_TEMP_PREFIX,
                inventory
                    .entries
                    .iter()
                    .find(|e| e.absolute_path == *source)
                    .ok_or("directory missing")?
                    .id
            ));
            if ns
                .parents
                .iter()
                .flat_map(|p| &p.entries)
                .any(|e| e.absolute_path == quarantine)
                || !ns.policy.allows_rename_reservation(&quarantine)
            {
                return Err("source directory quarantine occupied or protected".into());
            }
            ordinal += 1;
            let moved = format!("00000000-0000-0000-0000-{ordinal:012x}")
                .parse()
                .map_err(|_| "step overflow")?;
            group.steps.push(ExecutionStep {
                id: moved,
                entry_id: rows[*i].entry_id,
                group_id: owners[*i],
                action: ExecutionAction::RenameEntry {
                    source: EntryPrestate {
                        path: source.clone(),
                        identity: ExpectedIdentity::DirectoryAfterEffects {
                            original,
                            created_by: None,
                            effects,
                        },
                    },
                    target: quarantine.clone(),
                    temporary: true,
                },
            });
            ordinal += 1;
            group.steps.push(ExecutionStep {
                id: format!("00000000-0000-0000-0000-{ordinal:012x}")
                    .parse()
                    .map_err(|_| "step overflow")?,
                entry_id: rows[*i].entry_id,
                group_id: owners[*i],
                action: ExecutionAction::RemoveSourceDirectory {
                    source: EntryPrestate {
                        path: quarantine,
                        identity: ExpectedIdentity::FromReceipt { step_id: moved },
                    },
                    destination_created_by: created,
                    required_descendants,
                },
            });
        }
    }
    Ok(groups.into_values().collect())
}

/// Existing native parents and same-volume evidence allow the normal graph
/// compiler to preserve identities (including a selected directory subtree).
pub(crate) fn can_rename(rows: &[PlanRow], ns: &NamespaceSnapshot, policy: &Destination) -> bool {
    matches!(policy, Destination::MoveTo { .. })
        && rows.iter().all(|row| {
            ns.parents.iter().any(|p| {
                Some(p.parent_path.as_path()) == row.target_path.parent()
                    && p.capabilities.atomic_rename == vidlib_inventory::Support::Supported
                    && p.stamp
                        .physical_id
                        .as_ref()
                        .zip(row.stamp.physical_id.as_ref())
                        .is_some_and(|(a, b)| {
                            a.platform == b.platform && a.volume_identity == b.volume_identity
                        })
            })
        })
}
/// Verify the complete native expansion against its immutable row/namespace
/// inputs, including carried descendants and all compiled reservations.
pub fn validate_compiled(plan: &RenamePlan) -> Result<(), RenameError> {
    let mut groups = compile(
        &plan.execution.provenance.inventory,
        &plan.rows,
        &plan.execution.expected_namespace,
        matches!(
            plan.execution.provenance.recipe.destination,
            Destination::MoveTo { .. }
        ),
    )
    .map_err(|m| invalid(&m))?;
    crate::metadata::compile_changes(
        &plan.execution.provenance.inventory,
        &mut plan.rows.clone(),
        &plan.execution.provenance.recipe,
        &mut groups,
    )?;
    if groups.len() != plan.execution.groups.len()
        || groups
            .iter()
            .zip(&plan.execution.groups)
            .any(|(a, b)| a.id != b.id || a.depends_on != b.depends_on || a.steps != b.steps)
    {
        return Err(invalid(
            "destination graph differs from complete reviewed expansion",
        ));
    }
    Ok(())
}

fn action_paths(action: &ExecutionAction) -> Vec<&PathBuf> {
    match action {
        ExecutionAction::RenameEntry { source, target, .. } => vec![&source.path, target],
        ExecutionAction::CreateDirectory { target } => vec![target],
        ExecutionAction::CreateStaging { staging } => vec![staging],
        ExecutionAction::WriteStaging { source, staging } => {
            vec![&source.path, &staging.entry.path]
        }
        ExecutionAction::StageCopy { source, staging } => vec![&source.path, staging],
        ExecutionAction::PublishCopy {
            staging, target, ..
        } => vec![&staging.entry.path, target],
        ExecutionAction::RemoveSource { source, .. }
        | ExecutionAction::RemoveSourceDirectory { source, .. } => vec![&source.path],
        ExecutionAction::SetMetadata { entry, .. } => vec![&entry.path],
        ExecutionAction::RemoveOwnedCopy { owned }
        | ExecutionAction::RemoveOwnedEmptyDirectory { owned } => vec![&owned.entry.path],
    }
}
pub fn directory_metadata_changes(stamp: &vidlib_inventory::FileStamp) -> Vec<MetadataChange> {
    let mut changes = Vec::new();
    for (field, value) in [
        (TimestampField::Modified, stamp.modified_ns),
        (TimestampField::Accessed, stamp.accessed_ns),
        (
            TimestampField::Created,
            if cfg!(windows) {
                stamp.created_ns
            } else {
                None
            },
        ),
    ] {
        if let Some(value_ns) = value {
            changes.push(MetadataChange::Timestamp { field, value_ns });
        }
    }
    if let Some(attributes) = stamp.windows_attributes {
        changes.push(MetadataChange::WindowsAttributes {
            set: attributes,
            clear: !attributes,
        });
    }
    changes
}
