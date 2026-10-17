//! Immutable portable preset revisions. Disk paths and durability belong to the app.
use crate::{RecipeV1, RenameError, RenameErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const RECIPE_BYTE_LIMIT: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PresetId(uuid::Uuid);
impl std::fmt::Display for PresetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetRevision {
    pub id: PresetId,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub revision: u64,
    pub recipe: RecipeV1,
}
#[derive(Default)]
pub struct PresetStore {
    revisions: BTreeMap<PresetId, BTreeMap<u64, PresetRevision>>,
}
impl PresetStore {
    pub fn load(
        &self,
        id: PresetId,
        revision: Option<u64>,
    ) -> Result<&PresetRevision, RenameError> {
        let revisions = self
            .revisions
            .get(&id)
            .ok_or_else(|| crate::recipe::invalid("unknown preset"))?;
        revision
            .map_or_else(
                || revisions.last_key_value().map(|(_, v)| v),
                |r| revisions.get(&r),
            )
            .ok_or_else(|| crate::recipe::invalid("unknown preset revision"))
    }
    /// Restore a durable snapshot without granting authority to any saved IDs.
    pub fn from_snapshot(snapshot: PresetRevision) -> Result<Self, RenameError> {
        Self::export(&snapshot.recipe)?;
        if snapshot.revision == 0 {
            return Err(crate::recipe::invalid("preset revision must be positive"));
        }
        let mut store = Self::default();
        store
            .revisions
            .entry(snapshot.id)
            .or_default()
            .insert(snapshot.revision, snapshot);
        Ok(store)
    }
    pub fn save_revision(
        &mut self,
        id: Option<PresetId>,
        recipe: RecipeV1,
    ) -> Result<PresetRevision, RenameError> {
        Self::export(&recipe)?;
        let (id, revision) = match id {
            Some(id) => (
                id,
                self.load(id, None)?
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| crate::recipe::invalid("preset revision overflow"))?,
            ),
            None => (PresetId(uuid::Uuid::new_v4()), 1),
        };
        let snapshot = PresetRevision {
            id,
            revision,
            recipe,
        };
        self.revisions
            .entry(id)
            .or_default()
            .insert(revision, snapshot.clone());
        Ok(snapshot)
    }
    pub fn import(bytes: &[u8]) -> Result<RecipeV1, RenameError> {
        RecipeV1::from_json(bytes).map_err(import_error)
    }
    pub fn export(recipe: &RecipeV1) -> Result<Vec<u8>, RenameError> {
        recipe.validate().map_err(import_error)?;
        let bytes =
            serde_json::to_vec(recipe).map_err(|e| crate::recipe::invalid(e.to_string()))?;
        if bytes.len() > RECIPE_BYTE_LIMIT {
            return Err(RenameError::new(
                RenameErrorCode::ImportLimit,
                "recipe exceeds 1 MiB",
            ));
        }
        Ok(bytes)
    }
}
fn import_error(mut error: RenameError) -> RenameError {
    if matches!(
        error.code,
        RenameErrorCode::RecipeTooLarge | RenameErrorCode::TooManyRules
    ) {
        error.code = RenameErrorCode::ImportLimit;
    }
    error
}
