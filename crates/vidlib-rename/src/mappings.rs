//! Mapping imports are explicit data; native targets stay in this backend module.
//! Parsing is bounded before path resolution or publication, including a single
//! enormous quoted record. Report protection is never written into stored paths.
use crate::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, BufRead, BufReader, Read},
    path::{Component, Path, PathBuf},
};
use vidlib_inventory::{ids::*, GrantSet, Inventory};
pub const MAPPING_BYTE_LIMIT: u64 = 64 * 1024 * 1024;
pub const MAPPING_ROW_LIMIT: usize = 100_000;
#[derive(Clone, Debug)]
pub enum MappingFormat {
    Csv,
    Delimited { delimiter: u8 },
    NameList { order: Vec<EntryId> },
}
#[derive(Clone, Debug)]
pub struct MappingRow {
    pub entry_id: EntryId,
    pub target_path: PathBuf,
}
#[derive(Clone, Debug)]
pub struct MappingSet {
    pub inventory_id: InventoryId,
    pub generation: u64,
    pub rows: Vec<MappingRow>,
}
#[derive(Clone, Copy, Debug)]
pub enum ExportMode {
    SpreadsheetSafe,
    ExactMapping,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowOverride {
    pub entry_id: EntryId,
    pub new_name: String,
}
struct BoundedReader<R> {
    reader: R,
    remaining: u64,
    exceeded: bool,
}
impl<R: Read> Read for BoundedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut probe = [0];
            if self.reader.read(&mut probe)? == 0 {
                return Ok(0);
            }
            self.exceeded = true;
            return Err(io::Error::other("mapping byte limit exceeded"));
        }
        let count = buffer.len().min(self.remaining as usize);
        let n = self.reader.read(&mut buffer[..count])?;
        self.remaining -= n as u64;
        Ok(n)
    }
}
fn invalid(message: impl Into<String>) -> RenameError {
    RenameError::new(RenameErrorCode::InvalidMapping, message)
}
fn limit() -> RenameError {
    RenameError::new(
        RenameErrorCode::ImportLimit,
        "mapping exceeds 64 MiB or 100,000 data rows",
    )
}
fn csv_error(error: csv::Error, exceeded: bool) -> RenameError {
    if exceeded {
        limit()
    } else {
        invalid(error.to_string())
    }
}

pub fn import_mappings(
    reader: impl Read,
    format: MappingFormat,
    inventory: &Inventory,
    grants: &GrantSet,
) -> Result<MappingSet, RenameError> {
    let entries = inventory
        .entries
        .iter()
        .map(|e| (e.id, e))
        .collect::<BTreeMap<_, _>>();
    let roots = inventory
        .roots
        .iter()
        .map(|r| (r.id, r))
        .collect::<BTreeMap<_, _>>();
    let mut by_path = BTreeMap::<&Path, Vec<&vidlib_inventory::EntrySnapshot>>::new();
    let mut by_relative = BTreeMap::<PathBuf, Vec<&vidlib_inventory::EntrySnapshot>>::new();
    let mut by_basename =
        BTreeMap::<&std::ffi::OsStr, Vec<&vidlib_inventory::EntrySnapshot>>::new();
    for entry in &inventory.entries {
        by_path.entry(&entry.absolute_path).or_default().push(entry);
        if let Some(root) = roots.get(&entry.root_id) {
            if let Ok(relative) = entry.absolute_path.strip_prefix(&root.path) {
                if !relative.as_os_str().is_empty() {
                    by_relative
                        .entry(relative.to_owned())
                        .or_default()
                        .push(entry);
                }
            }
        }
        if let Some(name) = entry.absolute_path.file_name() {
            by_basename.entry(name).or_default().push(entry);
        }
    }
    let ids = inventory.entries.iter().map(|e| e.id).collect::<Vec<_>>();
    crate::evaluate::validate_inventory(inventory, &ids, &entries)?;
    let bounded = BoundedReader {
        reader,
        remaining: MAPPING_BYTE_LIMIT,
        exceeded: false,
    };
    let mut rows = vec![];
    match format {
        MappingFormat::NameList { order } => {
            let mut seen = BTreeSet::new();
            if order.len() > MAPPING_ROW_LIMIT {
                return Err(limit());
            }
            for id in &order {
                if !entries.contains_key(id) {
                    return Err(RenameError::new(
                        RenameErrorCode::UnknownEntry,
                        "paste order has an unknown ID",
                    ));
                }
                if !seen.insert(*id) {
                    return Err(invalid("duplicate paste order ID"));
                }
            }
            let mut reader = BufReader::new(bounded);
            let mut names = vec![];
            loop {
                let mut name = String::new();
                let n = reader.read_line(&mut name).map_err(|e| {
                    if reader.get_ref().exceeded {
                        limit()
                    } else {
                        invalid(e.to_string())
                    }
                })?;
                if n == 0 {
                    break;
                }
                if names.len() == MAPPING_ROW_LIMIT {
                    return Err(limit());
                }
                if name.ends_with('\n') {
                    name.pop();
                    if name.ends_with('\r') {
                        name.pop();
                    }
                }
                names.push(name);
            }
            if names.len() != order.len() {
                return Err(RenameError::new(
                    RenameErrorCode::MappingCardinality,
                    "paste names must exactly match the frozen explicit order",
                ));
            }
            for (id, name) in order.into_iter().zip(names) {
                if !crate::recipe::valid_component(&name) {
                    return Err(invalid("pasted name must be one literal component"));
                }
                let entry = entries[&id];
                rows.push(MappingRow {
                    entry_id: id,
                    target_path: entry.absolute_path.with_file_name(name),
                });
            }
        }
        format => {
            let delimiter = match format {
                MappingFormat::Csv => b',',
                MappingFormat::Delimited { delimiter } => delimiter,
                _ => unreachable!(),
            };
            if !delimiter.is_ascii() || matches!(delimiter, 0 | b'\r' | b'\n' | b'"') {
                return Err(invalid("invalid delimiter"));
            }
            let mut parser = csv::ReaderBuilder::new()
                .delimiter(delimiter)
                .from_reader(bounded);
            let headers = parser
                .headers()
                .cloned()
                .map_err(|e| csv_error(e, parser.get_ref().exceeded))?;
            if headers.iter().ne(["old_path", "new_path"]) {
                return Err(invalid("mapping v1 requires old_path,new_path headers"));
            }
            let mut records = vec![];
            let mut record = csv::StringRecord::new();
            loop {
                let available = parser
                    .read_record(&mut record)
                    .map_err(|e| csv_error(e, parser.get_ref().exceeded))?;
                if !available {
                    break;
                }
                if records.len() == MAPPING_ROW_LIMIT {
                    return Err(limit());
                }
                records.push((record[0].to_owned(), record[1].to_owned()));
            }
            // Resolve only after bounded parsing: no partially accepted mapping set.
            for (index, (old, new)) in records.into_iter().enumerate() {
                let resolve = || -> Result<MappingRow, RenameError> {
                    let path = import_path(&old)?;
                    let exact = if path.is_absolute() {
                        by_path.get(path.as_path())
                    } else {
                        by_relative.get(&path)
                    };
                    let candidates = exact
                        .or_else(|| {
                            if path.components().count() == 1 {
                                path.file_name().and_then(|n| by_basename.get(n))
                            } else {
                                None
                            }
                        })
                        .cloned()
                        .unwrap_or_default();
                    let native = candidates
                        .iter()
                        .map(|e| &e.absolute_path)
                        .collect::<BTreeSet<_>>();
                    if native.len() > 1 {
                        return Err(RenameError::new(
                            RenameErrorCode::AmbiguousMapping,
                            format!("ambiguous source: {old}"),
                        ));
                    }
                    let entry = candidates.first().ok_or_else(|| {
                        RenameError::new(
                            RenameErrorCode::UnknownEntry,
                            format!("mapping source absent: {old}"),
                        )
                    })?;
                    let new = import_path(&new)?;
                    let target = if new.is_absolute() {
                        new
                    } else if new.components().count() == 1 {
                        entry.absolute_path.with_file_name(new)
                    } else {
                        inventory
                            .roots
                            .iter()
                            .find(|r| r.id == entry.root_id)
                            .expect("validated root")
                            .path
                            .join(new)
                    };
                    Ok(MappingRow {
                        entry_id: entry.id,
                        target_path: target,
                    })
                };
                rows.push(resolve().map_err(|mut e| {
                    e.message = format!("mapping row {}: {}", index + 2, e.message);
                    e
                })?);
            }
        }
    }
    let mappings = MappingSet {
        inventory_id: inventory.id,
        generation: inventory.generation,
        rows,
    };
    validate_mappings(&mappings, inventory, grants, None)?;
    Ok(mappings)
}
fn import_path(text: &str) -> Result<PathBuf, RenameError> {
    if text.is_empty()
        || text.contains('\0')
        || text.split(['/', '\\']).any(|p| p == ".." || p == ".")
        || text.starts_with(r"\\?\")
        || text.starts_with(r"\\.\")
    {
        return Err(invalid("empty, traversal or device path"));
    }
    let path = PathBuf::from(text);
    #[cfg(windows)]
    if text.contains(':') {
        let valid = path.is_absolute()
            && text.as_bytes().get(1) == Some(&b':')
            && text.as_bytes().iter().filter(|b| **b == b':').count() == 1;
        if !valid {
            return Err(invalid("alternate stream or foreign device path"));
        }
    }
    Ok(path)
}
fn validate_mappings(
    mappings: &MappingSet,
    inventory: &Inventory,
    grants: &GrantSet,
    selection: Option<&[EntryId]>,
) -> Result<(), RenameError> {
    if mappings.inventory_id != inventory.id || mappings.generation != inventory.generation {
        return Err(RenameError::new(
            RenameErrorCode::StaleInputs,
            "mapping belongs to another inventory generation",
        ));
    }
    if mappings.rows.len() > MAPPING_ROW_LIMIT {
        return Err(limit());
    }
    let mut sources = BTreeSet::new();
    let mut native_sources = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let entries = inventory
        .entries
        .iter()
        .map(|e| (e.id, e))
        .collect::<BTreeMap<_, _>>();
    let selected = selection.map(|ids| ids.iter().copied().collect::<BTreeSet<_>>());
    let roots = inventory
        .roots
        .iter()
        .map(|r| (r.id, r))
        .collect::<BTreeMap<_, _>>();
    let root_paths = inventory
        .roots
        .iter()
        .map(|r| (r.path.as_path(), r))
        .collect::<BTreeMap<_, _>>();
    let grant_ids = grants
        .grants
        .iter()
        .map(|g| (g.id, g))
        .collect::<BTreeMap<_, _>>();
    let file_roots = inventory
        .entries
        .iter()
        .filter(|e| {
            e.kind == vidlib_inventory::EntryKind::File
                && roots
                    .get(&e.root_id)
                    .is_some_and(|r| r.path == e.absolute_path)
        })
        .map(|e| e.absolute_path.as_path())
        .collect::<BTreeSet<_>>();
    let authorized = |path: &Path| {
        path.ancestors().any(|parent| {
            root_paths.get(parent).is_some_and(|root| {
                grant_ids.get(&root.source_grant.id).is_some_and(|g| {
                    g.path == root.source_grant.path
                        && path.starts_with(&g.path)
                        && (!file_roots.contains(root.path.as_path()) || path == root.path)
                })
            })
        })
    };
    for row in &mappings.rows {
        let entry = entries.get(&row.entry_id).ok_or_else(|| {
            RenameError::new(RenameErrorCode::UnknownEntry, "mapping has an unknown ID")
        })?;
        if selected.as_ref().is_some_and(|s| !s.contains(&entry.id)) {
            return Err(RenameError::new(
                RenameErrorCode::UnselectedEntry,
                "mapping refers to an unselected entry",
            ));
        }
        if !sources.insert(row.entry_id)
            || !native_sources.insert(&entry.absolute_path)
            || !targets.insert(&row.target_path)
        {
            return Err(invalid("duplicate source or target mapping"));
        }
        let path = &row.target_path;
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            || path
                .file_name()
                .and_then(|n| n.to_str())
                .is_none_or(|n| !crate::recipe::valid_component(n))
        {
            return Err(invalid("invalid native mapping target"));
        }
        let source_root = roots
            .get(&entry.root_id)
            .ok_or_else(|| invalid("source root is absent"))?;
        let explicit_file_target = entry.kind == vidlib_inventory::EntryKind::File
            && source_root.path == entry.absolute_path
            && path.parent() == entry.absolute_path.parent();
        if !authorized(&entry.absolute_path)
            || (!authorized(path) && !explicit_file_target)
            || !grant_ids
                .get(&source_root.source_grant.id)
                .is_some_and(|g| **g == source_root.source_grant)
        {
            return Err(RenameError::new(
                RenameErrorCode::OutsideGrant,
                "mapping source/target is outside an authorized root",
            ));
        }
    }
    Ok(())
}
/// Explicit targets override generated names, followed by literal per-row edits.
/// This remains pure evaluation; callers must capture a fresh namespace and plan
/// the entire resulting batch before publishing/confirming it.
pub fn evaluate_with_mappings(
    inventory: &Inventory,
    recipe: &RecipeV1,
    inputs: &FrozenInputs,
    mappings: &MappingSet,
    overrides: &[RowOverride],
) -> Result<EvaluatedBatch, RenameError> {
    evaluate_with_mappings_controlled(
        inventory,
        recipe,
        inputs,
        mappings,
        overrides,
        &vidlib_inventory::JobControl::default(),
    )
}
pub fn evaluate_with_mappings_controlled(
    inventory: &Inventory,
    recipe: &RecipeV1,
    inputs: &FrozenInputs,
    mappings: &MappingSet,
    overrides: &[RowOverride],
    control: &vidlib_inventory::JobControl,
) -> Result<EvaluatedBatch, RenameError> {
    let mut batch = crate::evaluate::evaluate_controlled(inventory, recipe, inputs, control)?;
    validate_mappings(mappings, inventory, &batch.grants, Some(&recipe.selection))?;
    let mut targets = mappings
        .rows
        .iter()
        .map(|r| (r.entry_id, r.target_path.clone()))
        .collect::<BTreeMap<_, _>>();
    if overrides.len() > MAPPING_ROW_LIMIT
        || overrides
            .iter()
            .try_fold(0usize, |sum, r| sum.checked_add(r.new_name.len()))
            .is_none_or(|n| n as u64 > MAPPING_BYTE_LIMIT)
    {
        return Err(limit());
    }
    let entries = inventory
        .entries
        .iter()
        .map(|e| (e.id, e))
        .collect::<BTreeMap<_, _>>();
    let selected = recipe.selection.iter().copied().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for (ordinal, edit) in overrides.iter().enumerate() {
        crate::evaluate::checkpoint(
            control,
            vidlib_inventory::ProgressPhase::Evaluating,
            ordinal,
            overrides.len(),
            Some(edit.entry_id),
        )?;
        let entry = entries.get(&edit.entry_id).ok_or_else(|| {
            RenameError::new(RenameErrorCode::UnknownEntry, "override has unknown ID")
        })?;
        if !selected.contains(&edit.entry_id) {
            return Err(RenameError::new(
                RenameErrorCode::UnselectedEntry,
                "override has unselected ID",
            ));
        }
        if !seen.insert(edit.entry_id) || !crate::recipe::valid_component(&edit.new_name) {
            return Err(invalid("duplicate override or invalid literal name"));
        }
        let path = targets
            .get(&edit.entry_id)
            .unwrap_or(&entry.absolute_path)
            .with_file_name(&edit.new_name);
        targets.insert(edit.entry_id, path);
    }
    let associations = associate(inventory, &recipe.selection, &recipe.companions)?;
    if associations
        .assets
        .iter()
        .any(|a| targets.contains_key(&a.asset_id))
    {
        return Err(invalid(
            "unbind companion ownership before editing an asset independently",
        ));
    }
    let total = batch.rows.len();
    for (ordinal, row) in batch.rows.iter_mut().enumerate() {
        crate::evaluate::checkpoint(
            control,
            vidlib_inventory::ProgressPhase::Evaluating,
            ordinal,
            total,
            Some(row.entry_id),
        )?;
        let Some(target) = targets.get(&row.entry_id) else {
            continue;
        };
        // Native-source problems cannot be repaired by a display/name edit.
        if row
            .issues
            .iter()
            .any(|i| i.code == RenameErrorCode::UnsupportedName)
        {
            continue;
        }
        row.issues.clear();
        row.explanations.push(RuleExplanation {
            rule_id: "explicit_mapping".into(),
            before: row.new_display_name.clone(),
            after: target
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| invalid("unsupported native target"))?
                .into(),
        });
        row.target_path = target.clone();
        row.new_display_name = target
            .file_name()
            .and_then(|n| n.to_str())
            .expect("validated target")
            .into();
        row.new_display_path = target.to_string_lossy().into_owned();
        row.status = if row.target_path == row.source_path {
            RowStatus::Unchanged
        } else {
            RowStatus::Changed
        };
    }
    propagate_names(&mut batch, &associations)?;
    Ok(batch)
}
pub fn export_report(plan: &RenamePlan, mode: ExportMode) -> Result<Vec<u8>, RenameError> {
    export_rows(&plan.rows, mode)
}
pub fn export_rows(rows: &[PlanRow], mode: ExportMode) -> Result<Vec<u8>, RenameError> {
    let mut writer = csv::Writer::from_writer(vec![]);
    match mode {
        ExportMode::ExactMapping => {
            writer
                .write_record(["old_path", "new_path"])
                .map_err(|e| invalid(e.to_string()))?;
            for row in rows {
                // Exact exports fail instead of silently replacing native units.
                let old = row.source_path.to_str().ok_or_else(|| {
                    RenameError::new(
                        RenameErrorCode::UnsupportedName,
                        "source path cannot be exported losslessly as UTF-8",
                    )
                })?;
                let new = row.target_path.to_str().ok_or_else(|| {
                    RenameError::new(
                        RenameErrorCode::UnsupportedName,
                        "target path cannot be exported losslessly as UTF-8",
                    )
                })?;
                for (path, text) in [(&row.source_path, old), (&row.target_path, new)] {
                    if !path.is_absolute()
                        || import_path(text).is_err()
                        || path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_none_or(|n| !crate::recipe::valid_component(n))
                    {
                        return Err(RenameError::new(
                            RenameErrorCode::UnsupportedName,
                            "native path is outside the lossless mapping format",
                        ));
                    }
                }
                writer
                    .write_record([old, new])
                    .map_err(|e| invalid(e.to_string()))?;
            }
        }
        ExportMode::SpreadsheetSafe => {
            writer
                .write_record([
                    "entry_id",
                    "old_name",
                    "new_name",
                    "old_display_path",
                    "new_display_path",
                    "status",
                    "size_bytes",
                    "created_ns",
                    "modified_ns",
                    "accessed_ns",
                ])
                .map_err(|e| invalid(e.to_string()))?;
            for row in rows {
                let cells = [
                    row.entry_id.to_string(),
                    row.old_display_name.clone(),
                    row.new_display_name.clone(),
                    row.old_display_path.clone(),
                    row.new_display_path.clone(),
                    format!("{:?}", row.status),
                    row.stamp.size.map(|n| n.to_string()).unwrap_or_default(),
                    row.stamp
                        .created_ns
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    row.stamp
                        .modified_ns
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    row.stamp
                        .accessed_ns
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                ];
                writer
                    .write_record(cells.iter().map(|s| spreadsheet_cell(s)))
                    .map_err(|e| invalid(e.to_string()))?;
            }
        }
    }
    writer.into_inner().map_err(|e| invalid(e.to_string()))
}
fn spreadsheet_cell(value: &str) -> String {
    if value.starts_with(['\t', '\r', '\n']) || value.trim_start().starts_with(['=', '+', '-', '@'])
    {
        format!("'{value}")
    } else {
        value.into()
    }
}
