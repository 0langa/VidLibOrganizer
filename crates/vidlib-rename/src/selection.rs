//! Pure inventory filtering. Display strings are never used as path identity.
use crate::*;
use serde::{Deserialize, Serialize};
use vidlib_inventory::{ids::EntryId, EntryKind, Inventory};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct U64Range {
    #[serde(with = "vidlib_inventory::exact::option_u64_decimal")]
    pub min: Option<u64>,
    #[serde(with = "vidlib_inventory::exact::option_u64_decimal")]
    pub max: Option<u64>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct I128Range {
    #[serde(with = "vidlib_inventory::exact::option_i128_decimal")]
    pub min: Option<i128>,
    #[serde(with = "vidlib_inventory::exact::option_i128_decimal")]
    pub max: Option<i128>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeFilter {
    pub required: u32,
    pub forbidden: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionField {
    Name,
    RelativePath,
    FullPath,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternSyntax {
    Wildcard,
    Regex,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionPattern {
    pub field: SelectionField,
    pub syntax: PatternSyntax,
    pub value: String,
    pub case_sensitive: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SelectionFilter {
    pub excluded_ids: Vec<EntryId>,
    pub recursive: bool,
    pub max_depth: Option<u32>,
    pub kinds: Vec<EntryKind>,
    pub extensions: Vec<String>,
    pub name_length: Option<U64Range>,
    pub path_length: Option<U64Range>,
    pub size: Option<U64Range>,
    pub created_ns: Option<I128Range>,
    pub modified_ns: Option<I128Range>,
    pub accessed_ns: Option<I128Range>,
    pub attributes: Option<AttributeFilter>,
    pub include: Vec<SelectionPattern>,
    pub exclude: Vec<SelectionPattern>,
}
impl Default for SelectionFilter {
    fn default() -> Self {
        Self {
            excluded_ids: vec![],
            recursive: true,
            max_depth: None,
            kinds: vec![],
            extensions: vec![],
            name_length: None,
            path_length: None,
            size: None,
            created_ns: None,
            modified_ns: None,
            accessed_ns: None,
            attributes: None,
            include: vec![],
            exclude: vec![],
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberingScope {
    #[default]
    Files,
    Directories,
}

impl NumberingScope {
    pub fn includes(self, kind: EntryKind) -> bool {
        matches!(
            (self, kind),
            (Self::Files, EntryKind::File) | (Self::Directories, EntryKind::Directory)
        )
    }
}
/// A separate counter vector preserves manual primary order without consuming
/// ordinals for the other kind. Companion ownership is handled before this stage.
pub fn scoped_numbering_order(
    inventory: &Inventory,
    recipe: &RecipeV1,
) -> Result<Vec<EntryId>, RenameError> {
    scoped_order(inventory, &recipe.numbering_order, recipe.numbering_scope)
}
pub(crate) fn scoped_order(
    inventory: &Inventory,
    order: &[EntryId],
    scope: NumberingScope,
) -> Result<Vec<EntryId>, RenameError> {
    let entries = inventory
        .entries
        .iter()
        .map(|e| (e.id, e))
        .collect::<std::collections::BTreeMap<_, _>>();
    order
        .iter()
        .filter_map(|id| match entries.get(id) {
            Some(e) if scope.includes(e.kind) => Some(Ok(*id)),
            Some(_) => None,
            None => Some(Err(RenameError::new(
                RenameErrorCode::UnknownEntry,
                "numbered entry is absent",
            ))),
        })
        .collect()
}
pub fn select(
    inventory: &Inventory,
    filter: &SelectionFilter,
) -> Result<Vec<EntryId>, RenameError> {
    use std::collections::{BTreeMap, BTreeSet};
    use unicode_segmentation::UnicodeSegmentation;
    let entries = inventory
        .entries
        .iter()
        .map(|e| (e.id, e))
        .collect::<BTreeMap<_, _>>();
    let ids = inventory.entries.iter().map(|e| e.id).collect::<Vec<_>>();
    crate::evaluate::validate_inventory(inventory, &ids, &entries)?;
    let (include, exclude) = compiled_filter(filter)?;
    let mut excluded_paths = BTreeSet::new();
    let mut excluded_ids = BTreeSet::new();
    for id in &filter.excluded_ids {
        let entry = entries.get(id).ok_or_else(|| {
            RenameError::new(
                RenameErrorCode::UnknownEntry,
                "excluded ID is absent from inventory",
            )
        })?;
        if !excluded_ids.insert(id) {
            return Err(invalid_selection("duplicate excluded ID"));
        }
        excluded_paths.insert(&entry.absolute_path);
    }
    let mut seen = BTreeSet::new();
    let mut selected = vec![];
    for entry in &inventory.entries {
        if excluded_paths.contains(&entry.absolute_path) {
            continue;
        }
        let relative = vidlib_inventory::decode_path(&entry.relative_path)
            .map_err(|e| invalid_selection(e.to_string()))?;
        let depth = relative.components().count().saturating_sub(1);
        if (!filter.recursive && depth > 0)
            || filter.max_depth.is_some_and(|d| depth > d as usize)
            || (!filter.kinds.is_empty() && !filter.kinds.contains(&entry.kind))
        {
            continue;
        }
        let name = entry.absolute_path.file_name().and_then(|n| n.to_str());
        let relative_text = relative.to_str();
        if !filter.extensions.is_empty()
            && (entry.kind != EntryKind::File
                || !entry
                    .absolute_path
                    .extension()
                    .and_then(|n| n.to_str())
                    .is_some_and(|ext| {
                        filter
                            .extensions
                            .iter()
                            .any(|want| ext.eq_ignore_ascii_case(want.trim_start_matches('.')))
                    }))
        {
            continue;
        }
        if !matches_u64(
            &filter.name_length,
            name.map(|n| n.graphemes(true).count() as u64),
        ) || !matches_u64(
            &filter.path_length,
            relative_text.map(|p| p.graphemes(true).count() as u64),
        ) || !matches_u64(&filter.size, entry.stamp.size)
            || !matches_i128(&filter.created_ns, entry.stamp.created_ns)
            || !matches_i128(&filter.modified_ns, entry.stamp.modified_ns)
            || !matches_i128(&filter.accessed_ns, entry.stamp.accessed_ns)
        {
            continue;
        }
        if filter.attributes.as_ref().is_some_and(|a| {
            entry
                .stamp
                .windows_attributes
                .is_none_or(|v| v & a.required != a.required || v & a.forbidden != 0)
        }) {
            continue;
        }
        let matches =
            |patterns: &[(SelectionField, CompiledRegexRule)]| -> Result<bool, RenameError> {
                for (field, compiled) in patterns {
                    let input = match field {
                        SelectionField::Name => name,
                        SelectionField::RelativePath => relative_text,
                        SelectionField::FullPath => entry.absolute_path.to_str(),
                    };
                    if let Some(input) = input {
                        if compiled.is_match(input).map_err(|e| {
                            RenameError::new(
                                if e.code == RuleIssueCode::RegexLimit {
                                    RenameErrorCode::RegexLimit
                                } else {
                                    RenameErrorCode::RegexNative
                                },
                                format!("entry {} selection: {}", entry.id, e.message),
                            )
                        })? {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            };
        if (!include.is_empty() && !matches(&include)?) || matches(&exclude)? {
            continue;
        }
        if seen.insert(&entry.absolute_path) {
            selected.push(entry.id);
        }
    }
    Ok(selected)
}
/// Rebuild selection before input preparation; surviving manual order is kept,
/// new primaries append in inventory order. Callers must revoke published plans.
pub fn prepare_selection(
    inventory: &Inventory,
    recipe: &mut RecipeV1,
    filter: &SelectionFilter,
    scope: NumberingScope,
) -> Result<(), RenameError> {
    use std::collections::BTreeSet;
    recipe.validate()?;
    let known = inventory
        .entries
        .iter()
        .map(|e| e.id)
        .collect::<BTreeSet<_>>();
    if recipe.selection.iter().any(|id| !known.contains(id)) {
        return Err(RenameError::new(
            RenameErrorCode::UnknownEntry,
            "existing selection contains an unknown ID",
        ));
    }
    let selection = select(inventory, filter)?;
    let selected = selection.iter().copied().collect::<BTreeSet<_>>();
    let mut updated = recipe.clone();
    updated.selection = selection;
    updated.selection_filter = Some(filter.clone());
    if let CompanionPolicy::Explicit { associations }
    | CompanionPolicy::MatchExact { associations } = &mut updated.companions
    {
        associations.retain(|a| selected.contains(&a.primary));
        for a in associations {
            a.companions.retain(|id| selected.contains(id));
        }
    }
    let companions = match &updated.companions {
        CompanionPolicy::None => BTreeSet::new(),
        CompanionPolicy::Explicit { associations }
        | CompanionPolicy::MatchExact { associations } => associations
            .iter()
            .flat_map(|a| a.companions.iter().copied())
            .collect(),
    };
    updated
        .numbering_order
        .retain(|id| selected.contains(id) && !companions.contains(id));
    let mut ordered = updated
        .numbering_order
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    for id in &updated.selection {
        if !companions.contains(id) && ordered.insert(*id) {
            updated.numbering_order.push(*id);
        }
    }
    updated.numbering_scope = scope;
    updated.revision = updated.revision.checked_add(1).ok_or_else(|| {
        RenameError::new(
            RenameErrorCode::ArithmeticOverflow,
            "recipe revision overflow",
        )
    })?;
    prepare_associations(inventory, &mut updated)?;
    updated.validate()?;
    *recipe = updated;
    Ok(())
}
fn matches_u64(range: &Option<U64Range>, value: Option<u64>) -> bool {
    range.as_ref().is_none_or(|r| {
        value.is_some_and(|v| r.min.is_none_or(|m| v >= m) && r.max.is_none_or(|m| v <= m))
    })
}
fn matches_i128(range: &Option<I128Range>, value: Option<i128>) -> bool {
    range.as_ref().is_none_or(|r| {
        value.is_some_and(|v| r.min.is_none_or(|m| v >= m) && r.max.is_none_or(|m| v <= m))
    })
}
fn invalid_selection(message: impl Into<String>) -> RenameError {
    RenameError::new(RenameErrorCode::InvalidSelection, message)
}
fn compile_patterns(
    patterns: &[SelectionPattern],
) -> Result<Vec<(SelectionField, CompiledRegexRule)>, RenameError> {
    patterns
        .iter()
        .map(|p| {
            if p.value.len() > REGEX_LIMITS.pattern_bytes {
                return Err(invalid_selection("selection pattern exceeds 16 KiB"));
            }
            let pattern = match p.syntax {
                PatternSyntax::Regex => p.value.clone(),
                PatternSyntax::Wildcard => {
                    let mut out = String::from("\\A(?:");
                    for c in p.value.chars() {
                        match c {
                            '*' => out.push_str("[\\s\\S]*"),
                            '?' => out.push_str("[\\s\\S]"),
                            c => {
                                if "\\.^$|()[]{}+".contains(c) {
                                    out.push('\\');
                                }
                                out.push(c);
                            }
                        }
                    }
                    out.push_str(")\\z");
                    out
                }
            };
            let compiled = CompiledRegexRule::compile(&RegexRule {
                pattern,
                replacement: String::new(),
                scope: MatchScope::First,
                case_sensitive: p.case_sensitive,
            })?;
            Ok((p.field, compiled))
        })
        .collect()
}

impl SelectionFilter {
    pub fn validate(&self) -> Result<(), RenameError> {
        compiled_filter(self).map(|_| ())
    }
}
type CompiledPatterns = Vec<(SelectionField, CompiledRegexRule)>;
fn compiled_filter(
    filter: &SelectionFilter,
) -> Result<(CompiledPatterns, CompiledPatterns), RenameError> {
    for range in [&filter.name_length, &filter.path_length, &filter.size]
        .into_iter()
        .flatten()
    {
        if range.min.zip(range.max).is_some_and(|(a, b)| a > b) {
            return Err(invalid_selection("reversed range"));
        }
    }
    for range in [&filter.created_ns, &filter.modified_ns, &filter.accessed_ns]
        .into_iter()
        .flatten()
    {
        if range.min.zip(range.max).is_some_and(|(a, b)| a > b) {
            return Err(invalid_selection("reversed date range"));
        }
    }
    if filter
        .attributes
        .as_ref()
        .is_some_and(|a| a.required & a.forbidden != 0)
    {
        return Err(invalid_selection(
            "conflicting required/forbidden attributes",
        ));
    }
    Ok((
        compile_patterns(&filter.include)?,
        compile_patterns(&filter.exclude)?,
    ))
}
