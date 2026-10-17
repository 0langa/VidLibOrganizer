//! Native picker and desktop filter adapters. Only backend PathBuf values enter source authority.
use crate::{
    app_state::AppState,
    commands::rename::{CommandError, InputRevision},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vidlib_inventory::{GrantSource, RootBinding};
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PickerKind {
    Folders,
    Files,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PickerRequest {
    pub kind: PickerKind,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub revision: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionDto {
    pub sources: Vec<SourceDto>,
    pub revision: String,
    pub source_count: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDto {
    pub grant_id: vidlib_inventory::ids::GrantId,
    pub display: String,
}
/// Desktop command data has camelCase members/tags. Portable recipe structs
/// keep their existing schema; conversion happens before backend selection.
#[derive(Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionFilterDto {
    pub excluded_ids: Vec<vidlib_inventory::ids::EntryId>,
    pub recursive: Option<bool>,
    pub max_depth: Option<u32>,
    pub kinds: Vec<SelectionKindDto>,
    pub extensions: Vec<String>,
    // These shared exact ranges/attributes already have camelCase-compatible
    // min/max and required/forbidden fields, with strict decimal decoding.
    pub name_length: Option<vidlib_rename::U64Range>,
    pub path_length: Option<vidlib_rename::U64Range>,
    pub size: Option<vidlib_rename::U64Range>,
    pub created_ns: Option<vidlib_rename::I128Range>,
    pub modified_ns: Option<vidlib_rename::I128Range>,
    pub accessed_ns: Option<vidlib_rename::I128Range>,
    pub attributes: Option<vidlib_rename::AttributeFilter>,
    pub include: Vec<SelectionPatternDto>,
    pub exclude: Vec<SelectionPatternDto>,
}
impl From<SelectionFilterDto> for vidlib_rename::SelectionFilter {
    fn from(value: SelectionFilterDto) -> Self {
        Self {
            excluded_ids: value.excluded_ids,
            recursive: value.recursive.unwrap_or_else(|| Self::default().recursive),
            max_depth: value.max_depth,
            kinds: value.kinds.into_iter().map(Into::into).collect(),
            extensions: value.extensions,
            name_length: value.name_length,
            path_length: value.path_length,
            size: value.size,
            created_ns: value.created_ns,
            modified_ns: value.modified_ns,
            accessed_ns: value.accessed_ns,
            attributes: value.attributes,
            include: value.include.into_iter().map(Into::into).collect(),
            exclude: value.exclude.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionPatternDto {
    pub field: SelectionFieldDto,
    pub syntax: PatternSyntaxDto,
    pub value: String,
    pub case_sensitive: bool,
}
impl From<SelectionPatternDto> for vidlib_rename::SelectionPattern {
    fn from(value: SelectionPatternDto) -> Self {
        Self {
            field: value.field.into(),
            syntax: value.syntax.into(),
            value: value.value,
            case_sensitive: value.case_sensitive,
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectionKindDto {
    File,
    Directory,
    Symlink,
    ReparsePoint,
    Other,
    Unknown,
}
impl From<SelectionKindDto> for vidlib_inventory::EntryKind {
    fn from(value: SelectionKindDto) -> Self {
        match value {
            SelectionKindDto::File => Self::File,
            SelectionKindDto::Directory => Self::Directory,
            SelectionKindDto::Symlink => Self::Symlink,
            SelectionKindDto::ReparsePoint => Self::ReparsePoint,
            SelectionKindDto::Other => Self::Other,
            SelectionKindDto::Unknown => Self::Unknown,
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectionFieldDto {
    Name,
    RelativePath,
    FullPath,
}
impl From<SelectionFieldDto> for vidlib_rename::SelectionField {
    fn from(value: SelectionFieldDto) -> Self {
        match value {
            SelectionFieldDto::Name => Self::Name,
            SelectionFieldDto::RelativePath => Self::RelativePath,
            SelectionFieldDto::FullPath => Self::FullPath,
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatternSyntaxDto {
    Wildcard,
    Regex,
}
impl From<PatternSyntaxDto> for vidlib_rename::PatternSyntax {
    fn from(value: PatternSyntaxDto) -> Self {
        match value {
            PatternSyntaxDto::Wildcard => Self::Wildcard,
            PatternSyntaxDto::Regex => Self::Regex,
        }
    }
}
/// Owned native adapter seam for native pickers and desktop integration tests.
/// This is not an IPC command and never accepts UI path strings.
pub fn install_selection(
    state: &AppState,
    paths: Vec<PathBuf>,
) -> Result<SelectionDto, CommandError> {
    install_selection_at(state, paths, None)
}
pub fn install_selection_at(
    state: &AppState,
    paths: Vec<PathBuf>,
    expected_revision: Option<u64>,
) -> Result<SelectionDto, CommandError> {
    let roots = paths
        .into_iter()
        .map(|path| {
            let mut root = RootBinding::new(path).map_err(|e| {
                vidlib_fileops::OpError::new(
                    vidlib_fileops::ErrorCode::InvalidPath,
                    vidlib_fileops::Phase::Read,
                    e.to_string(),
                )
            })?;
            root.source_grant.source = GrantSource::NativeSelection;
            Ok(root)
        })
        .collect::<Result<Vec<_>, vidlib_fileops::OpError>>()?;
    let mut r = state.rename.lock().map_err(|_| {
        vidlib_fileops::OpError::new(
            vidlib_fileops::ErrorCode::StaleState,
            vidlib_fileops::Phase::Read,
            "Rename state unavailable",
        )
    })?;
    if expected_revision.is_some_and(|revision| revision != r.revision) {
        return Err(vidlib_fileops::OpError::new(
            vidlib_fileops::ErrorCode::StaleState,
            vidlib_fileops::Phase::Verify,
            "Native selection was superseded by newer inputs",
        )
        .into());
    }
    state
        .plans
        .lock()
        .map_err(|_| {
            vidlib_fileops::OpError::new(
                vidlib_fileops::ErrorCode::StaleState,
                vidlib_fileops::Phase::Read,
                "Plan state unavailable",
            )
        })?
        .invalidate();
    let result = SelectionDto {
        sources: roots
            .iter()
            .take(200)
            .map(|r| SourceDto {
                grant_id: r.source_grant.id,
                display: r.path.to_string_lossy().into_owned(),
            })
            .collect(),
        revision: r.revision.to_string(),
        source_count: roots.len().to_string(),
    };
    r.replace_sources(roots)?;
    Ok(result)
}
#[tauri::command]
pub async fn select_sources(
    request: PickerRequest,
    state: tauri::State<'_, AppState>,
) -> Result<SelectionDto, CommandError> {
    let picker = rfd::AsyncFileDialog::new().set_title("Choose Rename sources");
    let paths = match request.kind {
        PickerKind::Folders => picker.pick_folders().await,
        PickerKind::Files => picker.pick_files().await,
    };
    crate::commands::rename::update_inputs_owned(
        &state,
        InputRevision {
            revision: request.revision,
        },
    )?;
    if let Some(files) = paths {
        install_selection_at(
            &state,
            files.into_iter().map(|f| f.path().to_path_buf()).collect(),
            Some(request.revision),
        )
    } else {
        Ok(SelectionDto {
            sources: vec![],
            source_count: "0".into(),
            revision: request.revision.to_string(),
        })
    }
}
