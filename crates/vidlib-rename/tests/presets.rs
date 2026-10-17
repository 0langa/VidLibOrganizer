use vidlib_rename::{presets::*, *};
fn recipe() -> RecipeV1 {
    RecipeV1::from_json(br#"{"schema_version":1,"revision":"9007199254740993","selection":[],"numbering_order":[],"numbering_scope":"directories","selection_filter":{"recursive":false},"rules":[{"id":"a","enabled":false,"segment":{"type":"between","target":"stem","opening":"[","closing":"]","occurrence":2,"include_delimiters":false},"parameters":{"type":"text","operation":{"type":"fixed","value":"new"}}}],"companions":{"type":"none"},"destination":{"type":"same_directory"},"collision_policy":{"type":"block"},"metadata_changes":[]}"#).unwrap()
}
#[test]
fn recipe_round_trip_and_version_rejection() {
    let original = recipe();
    let mut store = PresetStore::default();
    let saved = store.save_revision(None, original.clone()).unwrap();
    let reloaded = PresetStore::import(&PresetStore::export(&saved.recipe).unwrap()).unwrap();
    assert_eq!(reloaded, original);
    let mut edited = original.clone();
    edited.rules[0].enabled = true;
    let next = store.save_revision(Some(saved.id), edited).unwrap();
    assert_eq!(next.revision, 2);
    assert_eq!(store.load(saved.id, Some(1)).unwrap().recipe, original);
    assert_eq!(store.load(saved.id, None).unwrap(), &next);
    let bytes = PresetStore::export(&original).unwrap();
    let invalid = String::from_utf8(bytes)
        .unwrap()
        .replace("\"schema_version\":1", "\"schema_version\":2");
    assert_eq!(
        PresetStore::import(invalid.as_bytes()).unwrap_err().code,
        RenameErrorCode::UnknownRecipeVersion
    );
}
#[test]
fn import_limits_and_rule_count() {
    assert_eq!(
        PresetStore::import(&vec![b' '; 1024 * 1024 + 1])
            .unwrap_err()
            .code,
        RenameErrorCode::ImportLimit
    );
    let mut candidate = recipe();
    candidate.rules = (0..101)
        .map(|n| {
            let mut r = candidate.rules[0].clone();
            r.id = n.to_string();
            r
        })
        .collect();
    assert_eq!(
        PresetStore::import(&serde_json::to_vec(&candidate).unwrap())
            .unwrap_err()
            .code,
        RenameErrorCode::ImportLimit
    );
    candidate.rules.pop();
    assert_eq!(
        PresetStore::import(&serde_json::to_vec(&candidate).unwrap())
            .unwrap()
            .rules
            .len(),
        100
    );
    let mut value = serde_json::to_value(recipe()).unwrap();
    value["rules"][0]["parameters"]["type"] = "execute".into();
    assert_eq!(
        PresetStore::import(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .code,
        RenameErrorCode::InvalidRecipe
    );
}
#[test]
fn failed_save_preserves_previous() {
    let mut store = PresetStore::default();
    let saved = store.save_revision(None, recipe()).unwrap();
    let mut invalid = recipe();
    invalid.schema_version = 2;
    assert!(store.save_revision(Some(saved.id), invalid).is_err());
    assert_eq!(store.load(saved.id, None).unwrap(), &saved);
}

#[test]
fn exact_byte_limit_accepts_boundary_and_snapshot_revision_is_not_recipe_revision() {
    let original = recipe();
    let mut bytes = PresetStore::export(&original).unwrap();
    bytes.resize(1024 * 1024, b' ');
    assert_eq!(PresetStore::import(&bytes).unwrap(), original);
    bytes.push(b' ');
    assert_eq!(
        PresetStore::import(&bytes).unwrap_err().code,
        RenameErrorCode::ImportLimit
    );
    let snapshot = PresetStore::default()
        .save_revision(None, original)
        .unwrap();
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.recipe.revision, 9007199254740993);
}
