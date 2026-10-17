//! App-local immutable recipe snapshots. No caller-supplied filesystem paths.
use crate::{app_state::AppState, commands::rename::CommandError};
use serde::{Deserialize, Serialize};
use vidlib_rename::presets::{PresetId, PresetRevision, PresetStore};
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavePresetRequest {
    pub preset_id: Option<PresetId>,
    pub expected_revision: Option<String>,
    pub recipe_json: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadPresetRequest {
    pub preset_id: PresetId,
    pub revision: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetDto {
    pub preset_id: PresetId,
    pub revision: String,
    pub recipe_json: String,
}
fn error(message: impl Into<String>) -> CommandError {
    CommandError {
        code: "presetStorage".into(),
        message: message.into(),
        phase: "write".into(),
        item_id: None,
        operation_id: None,
        retryable: false,
    }
}
fn storage(state: &AppState) -> Result<std::path::PathBuf, CommandError> {
    let directory = state
        .operations
        .parent()
        .ok_or_else(|| error("Missing app-local storage"))?
        .join("rename-presets");
    match std::fs::create_dir(&directory) {
        Ok(()) => sync_directory(directory.parent().unwrap())?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(error(e.to_string())),
    }
    let metadata = std::fs::symlink_metadata(&directory).map_err(|e| error(e.to_string()))?;
    if !metadata.is_dir() || linked(&metadata) {
        return Err(error("Preset storage must be an app-owned directory"));
    }
    Ok(directory)
}
fn linked(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn sync_directory(path: &std::path::Path) -> Result<(), CommandError> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|e| error(e.to_string()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
fn read_snapshot(path: &std::path::Path) -> Result<PresetRevision, CommandError> {
    use std::io::Read;
    let metadata = std::fs::symlink_metadata(path).map_err(|e| error(e.to_string()))?;
    if !metadata.is_file() || linked(&metadata) {
        return Err(error("Preset record is not a regular file"));
    }
    // The enclosing record adds only a UUID, revision and field names.
    let limit = vidlib_rename::presets::RECIPE_BYTE_LIMIT + 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take((limit + 1) as u64).read_to_end(&mut bytes))
        .map_err(|e| error(e.to_string()))?;
    if bytes.len() > limit {
        return Err(error("Preset record exceeds its byte limit"));
    }
    let snapshot: PresetRevision =
        serde_json::from_slice(&bytes).map_err(|e| error(e.to_string()))?;
    PresetStore::from_snapshot(snapshot.clone())?;
    Ok(snapshot)
}
fn dto(snapshot: PresetRevision) -> Result<PresetDto, CommandError> {
    Ok(PresetDto {
        preset_id: snapshot.id,
        revision: snapshot.revision.to_string(),
        recipe_json: String::from_utf8(PresetStore::export(&snapshot.recipe)?)
            .map_err(|e| error(e.to_string()))?,
    })
}
fn revision(value: &str) -> Result<u64, CommandError> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| error("Invalid preset revision"))?;
    if parsed == 0 || parsed.to_string() != value {
        return Err(error("Invalid preset revision"));
    }
    Ok(parsed)
}
fn write_snapshot(
    directory: &std::path::Path,
    path: &std::path::Path,
    bytes: &[u8],
    replace: bool,
    sync: impl FnOnce(&std::path::Path) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    use std::io::Write;
    // NamedTempFile uses exclusive create-new. It cleans up on every precommit error.
    let mut pending =
        tempfile::NamedTempFile::new_in(directory).map_err(|e| error(e.to_string()))?;
    pending
        .write_all(bytes)
        .and_then(|_| pending.flush())
        .and_then(|_| pending.as_file().sync_all())
        .map_err(|e| error(e.to_string()))?;
    #[cfg(not(windows))]
    {
        if replace {
            pending.persist(path).map_err(|e| error(e.to_string()))?;
        } else {
            pending
                .persist_noclobber(path)
                .map_err(|e| error(e.to_string()))?;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, MOVEFILE_REPLACE_EXISTING,
            MOVEFILE_WRITE_THROUGH,
        };
        let temporary = pending.into_temp_path();
        let from = temporary
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let to = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let flags = MOVEFILE_WRITE_THROUGH
            | if replace {
                MOVEFILE_REPLACE_EXISTING
            } else {
                0
            };
        // App-owned generated paths only. UTF-16 buffers are NUL-terminated and live through the calls.
        unsafe {
            if SetFileAttributesW(from.as_ptr(), FILE_ATTRIBUTE_NORMAL) == 0
                || MoveFileExW(from.as_ptr(), to.as_ptr(), flags) == 0
            {
                return Err(error(std::io::Error::last_os_error().to_string()));
            }
        }
    }
    sync(directory).map_err(|mut error| {
        if !replace {
            return error;
        }
        // Rename is the commit point. Do not report an ordinary failed save after it.
        error.code = "presetCommitUncertain".into();
        error.message = format!(
            "Preset record replaced but directory flush failed; reload before retrying: {}",
            error.message
        );
        error
    })
}
pub fn save_owned(state: &AppState, request: SavePresetRequest) -> Result<PresetDto, CommandError> {
    let recipe = PresetStore::import(request.recipe_json.as_bytes())?;
    let _guard = state
        .rename_preferences_lock
        .lock()
        .map_err(|_| error("Preset storage unavailable"))?;
    let directory = storage(state)?;
    let mut store = match request.preset_id {
        Some(id) => {
            let previous = read_snapshot(&directory.join(format!("{id}.current.json")))?;
            if previous.id != id
                || request
                    .expected_revision
                    .as_deref()
                    .map(revision)
                    .transpose()?
                    != Some(previous.revision)
            {
                return Err(error(
                    "Preset changed. Reload before saving another revision.",
                ));
            }
            PresetStore::from_snapshot(previous)?
        }
        None => {
            if request.expected_revision.is_some() {
                return Err(error("New preset cannot have an expected revision"));
            }
            PresetStore::default()
        }
    };
    let mut snapshot = store.save_revision(request.preset_id, recipe)?;
    let bytes = loop {
        let archive = directory.join(format!("{}-{}.json", snapshot.id, snapshot.revision));
        let bytes = serde_json::to_vec(&snapshot).map_err(|e| error(e.to_string()))?;
        if archive.try_exists().map_err(|e| error(e.to_string()))? {
            let previous = read_snapshot(&archive)?;
            if previous.id != snapshot.id || previous.revision != snapshot.revision {
                return Err(error("Immutable preset identity mismatch"));
            }
            if previous == snapshot {
                // Retry an interrupted publication without modifying its archive.
                sync_directory(&directory)?;
                break bytes;
            }
            // A different orphan from an interrupted publication stays immutable.
            // Advance past it instead of overwriting it or blocking all later edits.
            snapshot = store.save_revision(Some(snapshot.id), snapshot.recipe)?;
        } else {
            write_snapshot(&directory, &archive, &bytes, false, sync_directory)?;
            break bytes;
        }
    };
    let current = directory.join(format!("{}.current.json", snapshot.id));
    if let Ok(metadata) = std::fs::symlink_metadata(&current) {
        if !metadata.is_file() || linked(&metadata) {
            return Err(error("Current preset record is not a regular file"));
        }
    }
    // Prepare every fallible response step before the current-record commit point.
    let response = dto(snapshot)?;
    write_snapshot(&directory, &current, &bytes, true, sync_directory)?;
    Ok(response)
}
pub fn load_owned(state: &AppState, request: LoadPresetRequest) -> Result<PresetDto, CommandError> {
    let _guard = state
        .rename_preferences_lock
        .lock()
        .map_err(|_| error("Preset storage unavailable"))?;
    let directory = storage(state)?;
    let suffix = match request.revision.as_deref() {
        Some(value) => format!("-{}.json", revision(value)?),
        None => ".current.json".into(),
    };
    let snapshot = read_snapshot(&directory.join(format!("{}{suffix}", request.preset_id)))?;
    if snapshot.id != request.preset_id
        || request
            .revision
            .as_deref()
            .is_some_and(|r| r != snapshot.revision.to_string())
    {
        return Err(error("Preset identity mismatch"));
    }
    dto(snapshot)
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeDocumentRequest {
    pub recipe_json: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeDocumentDto {
    pub recipe_json: String,
}
pub fn import_owned(request: RecipeDocumentRequest) -> Result<RecipeDocumentDto, CommandError> {
    let recipe = PresetStore::import(request.recipe_json.as_bytes())?;
    Ok(RecipeDocumentDto {
        recipe_json: String::from_utf8(PresetStore::export(&recipe)?)
            .map_err(|e| error(e.to_string()))?,
    })
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListPresetRequest {
    pub cursor: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetPage {
    pub items: Vec<PresetSummary>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetSummary {
    pub preset_id: PresetId,
    pub revision: String,
}
pub fn list_owned(
    state: &AppState,
    request: ListPresetRequest,
) -> Result<PresetPage, CommandError> {
    let _guard = state
        .rename_preferences_lock
        .lock()
        .map_err(|_| error("Preset storage unavailable"))?;
    let directory = storage(state)?;
    let mut paths = std::fs::read_dir(directory)
        .map_err(|e| error(e.to_string()))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| error(e.to_string()))?;
    paths.retain(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".current.json"))
    });
    paths.sort();
    let offset = request
        .cursor
        .as_deref()
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| error("Invalid preset cursor"))?;
    if offset > paths.len() {
        return Err(error("Preset cursor expired"));
    }
    let end = offset.saturating_add(200).min(paths.len());
    let items = paths[offset..end]
        .iter()
        .map(|path| {
            let value = read_snapshot(path)?;
            Ok(PresetSummary {
                preset_id: value.id,
                revision: value.revision.to_string(),
            })
        })
        .collect::<Result<Vec<_>, CommandError>>()?;
    Ok(PresetPage {
        items,
        next_cursor: (end < paths.len()).then(|| end.to_string()),
    })
}
#[tauri::command]
pub async fn save_rename_preset(
    state: tauri::State<'_, AppState>,
    request: SavePresetRequest,
) -> Result<PresetDto, CommandError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || save_owned(&state, request))
        .await
        .map_err(|e| error(e.to_string()))?
}
#[tauri::command]
pub async fn load_rename_preset(
    state: tauri::State<'_, AppState>,
    request: LoadPresetRequest,
) -> Result<PresetDto, CommandError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || load_owned(&state, request))
        .await
        .map_err(|e| error(e.to_string()))?
}
#[tauri::command]
pub async fn list_rename_presets(
    state: tauri::State<'_, AppState>,
    request: ListPresetRequest,
) -> Result<PresetPage, CommandError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list_owned(&state, request))
        .await
        .map_err(|e| error(e.to_string()))?
}
#[tauri::command]
pub async fn import_rename_recipe(
    request: RecipeDocumentRequest,
) -> Result<RecipeDocumentDto, CommandError> {
    tauri::async_runtime::spawn_blocking(move || import_owned(request))
        .await
        .map_err(|e| error(e.to_string()))?
}
#[tauri::command]
pub async fn export_rename_recipe(
    request: RecipeDocumentRequest,
) -> Result<RecipeDocumentDto, CommandError> {
    tauri::async_runtime::spawn_blocking(move || import_owned(request))
        .await
        .map_err(|e| error(e.to_string()))?
}
#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> String {
        r#"{"schema_version":1,"revision":"1","selection":[],"numbering_order":[],"rules":[],"companions":{"type":"none"},"destination":{"type":"same_directory"},"collision_policy":{"type":"block"},"metadata_changes":[]}"#.into()
    }
    #[test]
    fn failed_save_preserves_previous() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(dir.path(), dir.path().join("libraries.json")).unwrap();
        let old = save_owned(
            &state,
            SavePresetRequest {
                preset_id: None,
                expected_revision: None,
                recipe_json: document(),
            },
        )
        .unwrap();
        let obstruction = state
            .operations
            .parent()
            .unwrap()
            .join("rename-presets")
            .join(format!("{}-2.json", old.preset_id));
        std::fs::create_dir(&obstruction).unwrap();
        assert!(save_owned(
            &state,
            SavePresetRequest {
                preset_id: Some(old.preset_id),
                expected_revision: Some("1".into()),
                recipe_json: document()
            }
        )
        .is_err());
        let active = load_owned(
            &state,
            LoadPresetRequest {
                preset_id: old.preset_id,
                revision: None,
            },
        )
        .unwrap();
        assert_eq!(active.revision, "1");
        assert_eq!(active.recipe_json, old.recipe_json);
        std::fs::remove_dir(obstruction).unwrap();
        let next = save_owned(
            &state,
            SavePresetRequest {
                preset_id: Some(old.preset_id),
                expected_revision: Some("1".into()),
                recipe_json: document(),
            },
        )
        .unwrap();
        assert_eq!(next.revision, "2");
        drop(state);
        let reopened = AppState::open(dir.path(), dir.path().join("libraries.json")).unwrap();
        assert_eq!(
            load_owned(
                &reopened,
                LoadPresetRequest {
                    preset_id: old.preset_id,
                    revision: None
                }
            )
            .unwrap()
            .revision,
            "2"
        );
        assert_eq!(
            load_owned(
                &reopened,
                LoadPresetRequest {
                    preset_id: old.preset_id,
                    revision: Some("1".into())
                }
            )
            .unwrap()
            .recipe_json,
            old.recipe_json
        );
        assert!(save_owned(
            &reopened,
            SavePresetRequest {
                preset_id: Some(old.preset_id),
                expected_revision: Some("1".into()),
                recipe_json: document()
            }
        )
        .is_err());
    }
    #[test]
    fn post_publication_flush_fault_is_uncertain_and_reload_observes_new_revision() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(dir.path(), dir.path().join("libraries.json")).unwrap();
        let old = save_owned(
            &state,
            SavePresetRequest {
                preset_id: None,
                expected_revision: None,
                recipe_json: document(),
            },
        )
        .unwrap();
        let directory = storage(&state).unwrap();
        let current = directory.join(format!("{}.current.json", old.preset_id));
        let mut store = PresetStore::from_snapshot(read_snapshot(&current).unwrap()).unwrap();
        let next = store
            .save_revision(
                Some(old.preset_id),
                PresetStore::import(document().as_bytes()).unwrap(),
            )
            .unwrap();
        let bytes = serde_json::to_vec(&next).unwrap();
        write_snapshot(
            &directory,
            &directory.join(format!("{}-2.json", old.preset_id)),
            &bytes,
            false,
            sync_directory,
        )
        .unwrap();
        let failure = write_snapshot(&directory, &current, &bytes, true, |_| {
            Err(error("injected directory flush I/O failure"))
        })
        .unwrap_err();
        assert_eq!(failure.code, "presetCommitUncertain");
        assert!(failure.message.contains("reload"));
        assert_eq!(
            load_owned(
                &state,
                LoadPresetRequest {
                    preset_id: old.preset_id,
                    revision: None
                }
            )
            .unwrap()
            .revision,
            "2"
        );
        assert_eq!(
            load_owned(
                &state,
                LoadPresetRequest {
                    preset_id: old.preset_id,
                    revision: Some("1".into())
                }
            )
            .unwrap()
            .recipe_json,
            old.recipe_json
        );
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 3);
    }
    #[test]
    fn orphan_revision_after_interrupted_publication_does_not_block_future_edits() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(dir.path(), dir.path().join("libraries.json")).unwrap();
        let old = save_owned(
            &state,
            SavePresetRequest {
                preset_id: None,
                expected_revision: None,
                recipe_json: document(),
            },
        )
        .unwrap();
        let directory = storage(&state).unwrap();
        let current = directory.join(format!("{}.current.json", old.preset_id));
        let mut store = PresetStore::from_snapshot(read_snapshot(&current).unwrap()).unwrap();
        let mut different = PresetStore::import(document().as_bytes()).unwrap();
        different.revision = 9;
        let orphan = store.save_revision(Some(old.preset_id), different).unwrap();
        write_snapshot(
            &directory,
            &directory.join(format!("{}-2.json", old.preset_id)),
            &serde_json::to_vec(&orphan).unwrap(),
            false,
            sync_directory,
        )
        .unwrap();
        let saved = save_owned(
            &state,
            SavePresetRequest {
                preset_id: Some(old.preset_id),
                expected_revision: Some("1".into()),
                recipe_json: document(),
            },
        )
        .unwrap();
        assert_eq!(saved.revision, "3");
        assert_eq!(
            load_owned(
                &state,
                LoadPresetRequest {
                    preset_id: old.preset_id,
                    revision: Some("2".into())
                }
            )
            .unwrap()
            .recipe_json,
            String::from_utf8(PresetStore::export(&orphan.recipe).unwrap()).unwrap()
        );
    }
    #[cfg(unix)]
    #[test]
    fn create_new_permission_failure_keeps_active_bytes_and_cleans_temporary_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(dir.path(), dir.path().join("libraries.json")).unwrap();
        let saved = save_owned(
            &state,
            SavePresetRequest {
                preset_id: None,
                expected_revision: None,
                recipe_json: document(),
            },
        )
        .unwrap();
        let directory = dir.path().join("rename-presets");
        let before =
            std::fs::read(directory.join(format!("{}.current.json", saved.preset_id))).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
        let outcome = save_owned(
            &state,
            SavePresetRequest {
                preset_id: Some(saved.preset_id),
                expected_revision: Some("1".into()),
                recipe_json: document(),
            },
        );
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(outcome.is_err(), "real create-new failure must be reported");
        assert_eq!(
            std::fs::read(directory.join(format!("{}.current.json", saved.preset_id))).unwrap(),
            before
        );
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 2);
    }
    #[test]
    fn malformed_oversized_and_conflicting_documents_never_replace_active() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(dir.path(), dir.path().join("libraries.json")).unwrap();
        let saved = save_owned(
            &state,
            SavePresetRequest {
                preset_id: None,
                expected_revision: None,
                recipe_json: document(),
            },
        )
        .unwrap();
        for value in [
            "{broken".into(),
            " ".repeat(1024 * 1024 + 1),
            document().replace("\"schema_version\":1", "\"schema_version\":2"),
        ] {
            assert!(save_owned(
                &state,
                SavePresetRequest {
                    preset_id: Some(saved.preset_id),
                    expected_revision: Some("1".into()),
                    recipe_json: value
                }
            )
            .is_err());
            assert_eq!(
                load_owned(
                    &state,
                    LoadPresetRequest {
                        preset_id: saved.preset_id,
                        revision: None
                    }
                )
                .unwrap()
                .recipe_json,
                saved.recipe_json
            );
        }
        let listed = list_owned(&state, ListPresetRequest { cursor: None }).unwrap();
        assert_eq!(listed.items.len(), 1);
        assert_eq!(listed.items[0].revision, "1");
        assert!(serde_json::from_value::<SavePresetRequest>(
            serde_json::json!({"preset_id":null,"expectedRevision":null,"recipeJson":document()})
        )
        .is_err());
        assert!(serde_json::from_value::<LoadPresetRequest>(
            serde_json::json!({"presetId":saved.preset_id,"revision":1})
        )
        .is_err());
    }
    #[cfg(unix)]
    #[test]
    fn preset_storage_rejects_links_and_preserves_unrelated_file() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(dir.path(), dir.path().join("libraries.json")).unwrap();
        let outside = dir.path().join("outside.json");
        std::fs::write(&outside, "untouched").unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("rename-presets")).unwrap();
        assert!(save_owned(
            &state,
            SavePresetRequest {
                preset_id: None,
                expected_revision: None,
                recipe_json: document()
            }
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "untouched");
    }
}
