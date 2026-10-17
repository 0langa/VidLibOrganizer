//! Exact native entry requests. Process arguments can only enqueue a selection;
//! the normal inventory, preview and confirmation protocol remains mandatory.
use crate::{
    app_state::AppState,
    commands::rename::{CommandError, EmptyRequest},
    selection::{complete_picker_at, SelectionDto},
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Component, PathBuf},
};
use vidlib_inventory::ids::StepId;

fn invalid(message: impl Into<String>) -> CommandError {
    vidlib_fileops::OpError::new(
        vidlib_fileops::ErrorCode::InvalidPath,
        vidlib_fileops::Phase::Read,
        message,
    )
    .into()
}
pub fn validate_paths(paths: &[PathBuf]) -> Result<(), CommandError> {
    if paths.is_empty() || paths.len() > 10_000 {
        return Err(invalid("Choose between 1 and 10000 native sources"));
    }
    let mut bytes = 0usize;
    for path in paths {
        let record = vidlib_inventory::encode_path(path);
        bytes = bytes
            .checked_add(record.bytes.len())
            .ok_or_else(|| invalid("Native selection is too large"))?;
        if bytes > 4 * 1024 * 1024
            || !path.is_absolute()
            || path.components().any(|c| matches!(c, Component::ParentDir))
        {
            return Err(invalid("Native source must be an absolute entry path without traversal; selection limit is 4 MiB"));
        }
        #[cfg(unix)]
        let nul = record.bytes.contains(&0);
        #[cfg(windows)]
        let nul = record.bytes.chunks_exact(2).any(|u| u == [0, 0]);
        if nul {
            return Err(invalid("Native path contains a null unit"));
        }
    }
    Ok(())
}
/// Arguments exclude argv[0]. Only `--select -- PATH...` is accepted.
pub fn parse_selection_arguments(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Option<Vec<PathBuf>>, CommandError> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(None);
    };
    if first != "--select" || args.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
        return Err(invalid(
            "Only --select -- PATH... is supported; arguments never apply changes",
        ));
    }
    let mut paths = Vec::new();
    let mut size = 0usize;
    for arg in args {
        let path = PathBuf::from(arg);
        size = size
            .checked_add(vidlib_inventory::encode_path(&path).bytes.len())
            .ok_or_else(|| invalid("Native selection too large"))?;
        if paths.len() >= 10_000 || size > 4 * 1024 * 1024 {
            return Err(invalid("Native selection exceeds limits"));
        }
        paths.push(path);
    }
    validate_paths(&paths)?;
    Ok(Some(paths))
}
pub struct PendingSelection {
    pub id: StepId,
    paths: Vec<PathBuf>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSelectionDto {
    pub selection_id: StepId,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptSelectionRequest {
    pub selection_id: StepId,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub revision: u64,
}
pub fn queue_native_paths(state: &AppState, paths: Vec<PathBuf>) -> Result<StepId, CommandError> {
    validate_paths(&paths)?;
    let id = StepId::new();
    *state
        .native_selection
        .lock()
        .map_err(|_| invalid("Native selection unavailable"))? =
        Some(PendingSelection { id, paths });
    Ok(id)
}
pub fn pending_owned(
    state: &AppState,
    _: EmptyRequest,
) -> Result<Option<PendingSelectionDto>, CommandError> {
    let pending = state
        .native_selection
        .lock()
        .map_err(|_| invalid("Native selection unavailable"))?;
    if pending.is_none() {
        return Ok(None);
    }
    let _lease = state
        .coordinator
        .try_acquire(vidlib_fileops::coordinator::LeaseKind::Read)?;
    Ok(pending
        .as_ref()
        .map(|v| PendingSelectionDto { selection_id: v.id }))
}
pub fn accept_owned(
    state: &AppState,
    request: AcceptSelectionRequest,
) -> Result<Option<SelectionDto>, CommandError> {
    let _lease = state
        .coordinator
        .try_acquire(vidlib_fileops::coordinator::LeaseKind::Read)?;
    let mut pending = state
        .native_selection
        .lock()
        .map_err(|_| invalid("Native selection unavailable"))?;
    let value = pending
        .as_ref()
        .filter(|v| v.id == request.selection_id)
        .ok_or_else(|| invalid("Native selection was superseded"))?;
    let selected = complete_picker_at(state, Some(value.paths.clone()), request.revision)?;
    pending.take();
    Ok(selected)
}
#[tauri::command]
pub fn pending_native_selection(
    request: EmptyRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Option<PendingSelectionDto>, CommandError> {
    pending_owned(&state, request)
}
#[tauri::command]
pub fn accept_native_selection(
    request: AcceptSelectionRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Option<SelectionDto>, CommandError> {
    accept_owned(&state, request)
}

/// Discard only queued paths, including while another instance holds a writer.
/// The current selection, revisions and any active operation stay untouched.
pub fn discard_owned(state: &AppState, _: EmptyRequest) -> Result<(), CommandError> {
    state
        .native_selection
        .lock()
        .map_err(|_| invalid("Native selection unavailable"))?
        .take();
    Ok(())
}
#[tauri::command]
pub fn discard_native_selection(
    request: EmptyRequest,
    state: tauri::State<'_, AppState>,
) -> Result<(), CommandError> {
    discard_owned(&state, request)
}
