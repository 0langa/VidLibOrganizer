use crate::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Component,
};
use vidlib_inventory::{decode_path, ids::*, EntryKind, GrantSet, Inventory};

/// Pure preview evaluation. All validation completes before constructing rows.
/// Later rule implementations extend this pipeline; no input is prepared here.
pub fn evaluate(
    inventory: &Inventory,
    recipe: &RecipeV1,
    inputs: &FrozenInputs,
) -> Result<EvaluatedBatch, RenameError> {
    let entries = inventory
        .entries
        .iter()
        .map(|entry| (entry.id, entry))
        .collect::<BTreeMap<_, _>>();
    validate_inventory(inventory, recipe, &entries)?;
    let compiled_regex = recipe.validate_and_compile_regex()?;
    validate_inputs(inventory, recipe, inputs)?;
    if recipe.companions != CompanionPolicy::None
        || recipe.destination != Destination::SameDirectory
        || !recipe.metadata_changes.is_empty()
    {
        return Err(RenameError::new(
            RenameErrorCode::UnsupportedConfiguration,
            "configuration requires a later evaluator",
        ));
    }
    for rule in recipe.rules.iter().filter(|r| r.enabled) {
        if !matches!(
            rule.parameters,
            RuleParameters::Keep
                | RuleParameters::FixedStem { .. }
                | RuleParameters::Text { .. }
                | RuleParameters::Regex { .. }
                | RuleParameters::Number { .. }
                | RuleParameters::PadNumbers { .. }
                | RuleParameters::Date { .. }
                | RuleParameters::Template { .. }
        ) {
            return Err(RenameError::new(
                RenameErrorCode::UnsupportedRule,
                "rule or segment is not implemented",
            ));
        }
    }
    let numbered = recipe
        .rules
        .iter()
        .filter(|rule| rule.enabled)
        .filter_map(|rule| {
            if let RuleParameters::Number { rule: parameters } = &rule.parameters {
                Some(
                    assign_numbers(&inventory.entries, &inputs.primary_order, parameters)
                        .map(|values| (rule.id.as_str(), values)),
                )
            } else {
                None
            }
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut rows = Vec::with_capacity(recipe.numbering_order.len());
    for id in &recipe.numbering_order {
        let entry = entries[id];
        let native_name = entry.absolute_path.file_name().and_then(|n| n.to_str());
        // Unsupported native names remain visible and blocked without lossy reconstruction.
        let mut row = PlanRow {
            carried_descendants: 0,
            entry_id: *id,
            group_id: GroupId::from_uuid(*id.as_uuid()),
            primary_id: *id,
            source_path: entry.absolute_path.clone(),
            target_path: entry.absolute_path.clone(),
            old_display_name: entry.display_name.clone(),
            new_display_name: entry.display_name.clone(),
            old_display_path: entry.display_path.clone(),
            new_display_path: entry.display_path.clone(),
            stamp: entry.stamp.clone(),
            status: RowStatus::Unchanged,
            explanations: vec![],
            issues: vec![],
        };
        if native_name.is_none()
            || !matches!(entry.kind, EntryKind::File | EntryKind::Directory)
            || !entry.issues.is_empty()
        {
            row.status = RowStatus::Blocked;
            row.issues.push(PlanIssue {
                code: RenameErrorCode::UnsupportedName,
                entry_id: Some(*id),
                rule_id: None,
                message: "entry is not a supported regular file with a clean observation".into(),
            });
            rows.push(row);
            continue;
        }
        let original = if entry.kind == EntryKind::Directory {
            NameParts::stem_only(native_name.expect("checked above"))
        } else {
            NameParts::from_filename(native_name.expect("checked above"))
        };
        let mut current = original.clone();
        for rule in recipe.rules.iter().filter(|r| r.enabled) {
            let before = current.segment(rule.segment.target());
            let result = match &rule.parameters {
                RuleParameters::Regex { .. } => {
                    crate::text::apply_segment(&current, &rule.segment, false, |selected| {
                        compiled_regex[rule.id.as_str()].apply(selected)
                    })
                }
                RuleParameters::Number { rule: parameters } => {
                    crate::text::apply_segment(&current, &rule.segment, false, |selected| {
                        crate::numbers::insert_value(
                            selected,
                            &parameters.at,
                            format!(
                                "{}{}{}",
                                parameters.prefix,
                                numbered[rule.id.as_str()][id],
                                parameters.suffix
                            ),
                        )
                    })
                }
                RuleParameters::PadNumbers { padding, scope } => {
                    crate::text::apply_segment(&current, &rule.segment, false, |selected| {
                        pad_existing_numbers(selected, *padding, *scope)
                    })
                }
                RuleParameters::Date { operation } => {
                    crate::text::apply_segment(&current, &rule.segment, false, |selected| {
                        crate::dates::apply_date(selected, operation, entry, &rule.id, inputs)
                    })
                }
                RuleParameters::Template { parts } => {
                    crate::text::apply_segment(&current, &rule.segment, false, |_| {
                        render_template(
                            parts,
                            &TemplateContext {
                                entry,
                                original: &original,
                                current: &current,
                                rule_id: &rule.id,
                                inputs,
                            },
                        )
                    })
                }
                parameters => {
                    let operation = match parameters {
                        RuleParameters::Keep => TextOperation::Keep,
                        RuleParameters::FixedStem { value } => TextOperation::Fixed {
                            value: value.clone(),
                        },
                        RuleParameters::Text { operation } => operation.clone(),
                        _ => unreachable!("rejected before evaluating rows"),
                    };
                    apply_text(
                        &current,
                        &TextRule {
                            segment: rule.segment.clone(),
                            operation,
                        },
                    )
                }
            };
            match result {
                Ok(next) => {
                    row.explanations.push(RuleExplanation {
                        rule_id: rule.id.clone(),
                        before,
                        after: next.segment(rule.segment.target()),
                    });
                    current = next;
                }
                Err(issue) => {
                    row.status = RowStatus::Blocked;
                    row.issues.push(PlanIssue {
                        code: match issue.code {
                            RuleIssueCode::InvalidInput => RenameErrorCode::InvalidRuleInput,
                            RuleIssueCode::RegexLimit => RenameErrorCode::RegexLimit,
                            RuleIssueCode::RegexNative => RenameErrorCode::RegexNative,
                            RuleIssueCode::DateNeedsFormat => RenameErrorCode::DateNeedsFormat,
                            RuleIssueCode::DateTimezoneUnknown => {
                                RenameErrorCode::DateTimezoneUnknown
                            }
                            RuleIssueCode::DateAmbiguous => RenameErrorCode::DateAmbiguous,
                            RuleIssueCode::DateNonexistent => RenameErrorCode::DateNonexistent,
                            RuleIssueCode::DateMissing => RenameErrorCode::DateMissing,
                            RuleIssueCode::MetadataMissing => RenameErrorCode::MetadataMissing,
                            RuleIssueCode::DateInvalid => RenameErrorCode::DateInvalid,
                        },
                        entry_id: Some(*id),
                        rule_id: Some(rule.id.clone()),
                        message: issue.message,
                    });
                    break;
                }
            }
        }
        if row.status == RowStatus::Blocked {
            rows.push(row);
            continue;
        }
        let new_name = current.filename();
        row.target_path.set_file_name(&new_name);
        row.new_display_name = new_name;
        row.new_display_path = row.target_path.to_string_lossy().into_owned();
        if row.target_path != row.source_path {
            row.status = RowStatus::Changed;
        }
        rows.push(row);
    }
    Ok(EvaluatedBatch {
        recipe: recipe.clone(),
        inventory_id: inventory.id,
        generation: inventory.generation,
        recipe_revision: recipe.revision,
        rows,
        frozen_inputs: inputs.clone(),
        grants: GrantSet {
            grants: inventory
                .roots
                .iter()
                .map(|r| r.source_grant.clone())
                .collect(),
        },
    })
}

fn validate_inventory(
    inventory: &Inventory,
    recipe: &RecipeV1,
    entries: &BTreeMap<EntryId, &vidlib_inventory::EntrySnapshot>,
) -> Result<(), RenameError> {
    let mut roots = BTreeSet::new();
    let mut grants = BTreeSet::new();
    for root in &inventory.roots {
        if !roots.insert(root.id)
            || !grants.insert(root.source_grant.id)
            || !clean_absolute(&root.path)
            || !clean_absolute(&root.source_grant.path)
            || !root.path.starts_with(&root.source_grant.path)
        {
            return Err(RenameError::new(
                RenameErrorCode::InvalidInventory,
                "invalid root/grant binding",
            ));
        }
    }
    if entries.len() != inventory.entries.len() {
        return Err(RenameError::new(
            RenameErrorCode::InvalidInventory,
            "duplicate inventory entry ID",
        ));
    }
    for id in &recipe.selection {
        let entry = entries.get(id).ok_or_else(|| {
            RenameError::new(RenameErrorCode::UnknownEntry, "selected entry is absent")
        })?;
        let root = inventory
            .roots
            .iter()
            .find(|r| r.id == entry.root_id)
            .ok_or_else(|| {
                RenameError::new(RenameErrorCode::OutsideGrant, "entry has no root binding")
            })?;
        let relative = entry
            .relative_path
            .validate_relative()
            .ok()
            .and_then(|()| decode_path(&entry.relative_path).ok());
        if !clean_absolute(&entry.absolute_path)
            || !entry.absolute_path.starts_with(&root.path)
            || !entry.absolute_path.starts_with(&root.source_grant.path)
            || relative.is_none_or(|relative| {
                let base = if entry.absolute_path == root.path {
                    root.path.parent().unwrap_or(&root.path)
                } else {
                    &root.path
                };
                base.join(relative) != entry.absolute_path
            })
        {
            return Err(RenameError::new(
                RenameErrorCode::OutsideGrant,
                "entry is outside its native root/grant binding",
            ));
        }
    }
    Ok(())
}
fn clean_absolute(path: &std::path::Path) -> bool {
    path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir))
}
fn validate_inputs(
    inventory: &Inventory,
    recipe: &RecipeV1,
    inputs: &FrozenInputs,
) -> Result<(), RenameError> {
    if inputs.inventory_id != inventory.id || inputs.generation != inventory.generation {
        return Err(RenameError::new(
            RenameErrorCode::StaleInputs,
            "frozen inputs belong to another inventory generation",
        ));
    }
    if inputs.primary_order != recipe.numbering_order {
        return Err(RenameError::new(
            RenameErrorCode::InvalidNumberingOrder,
            "frozen primary vector differs from the recipe",
        ));
    }
    if !valid_timezone(&inputs.timezone) {
        return Err(RenameError::new(
            RenameErrorCode::InvalidFrozenInputs,
            "invalid frozen timezone",
        ));
    }
    let selected: BTreeSet<_> = recipe.selection.iter().copied().collect();
    let mut seen = BTreeSet::new();
    for value in &inputs.values {
        if !selected.contains(&value.entry_id) {
            return Err(RenameError::new(
                RenameErrorCode::UnselectedEntry,
                "frozen value refers to an unselected entry",
            ));
        }
        if value
            .rule_id
            .as_ref()
            .is_some_and(|id| !recipe.rules.iter().any(|r| &r.id == id))
            || !seen.insert((value.entry_id, &value.rule_id, &value.key))
            || !valid_frozen_value(&value.key, &value.value)
            || matches!(&value.value, FrozenFieldValue::Timestamp { timezone: Some(zone), .. } | FrozenFieldValue::LocalTimestamp { timezone: Some(zone), .. } if !valid_timezone(zone))
        {
            return Err(RenameError::new(
                RenameErrorCode::InvalidFrozenInputs,
                "unknown rule, duplicate value or invalid timezone",
            ));
        }
    }
    Ok(())
}
fn valid_timezone(zone: &FrozenTimezone) -> bool {
    crate::dates::valid_timezone(zone)
}

fn valid_frozen_value(key: &FieldKey, value: &FrozenFieldValue) -> bool {
    if matches!(value, FrozenFieldValue::Missing { .. }) {
        return true;
    }
    match key {
        FieldKey::Date { .. } => matches!(
            value,
            FrozenFieldValue::Timestamp { .. } | FrozenFieldValue::LocalTimestamp { .. }
        ),
        FieldKey::Counter => matches!(value, FrozenFieldValue::Integer(_)),
        FieldKey::SizeBytes
        | FieldKey::Random
        | FieldKey::Media {
            property:
                MediaProperty::Width
                | MediaProperty::Height
                | MediaProperty::DurationNs
                | MediaProperty::AudioChannels
                | MediaProperty::BitRate,
        } => matches!(value, FrozenFieldValue::Integer(v) if *v >= 0),
        FieldKey::Media {
            property: MediaProperty::FrameRate,
        } => {
            matches!(value, FrozenFieldValue::Decimal { coefficient, scale } if *coefficient >= 0 && *scale <= 18)
        }
        FieldKey::Parent { level: 0 } => false,
        _ => matches!(value, FrozenFieldValue::Text(_)),
    }
}
