use vidlib_inventory::{ids::*, *};
use vidlib_rename::*;

fn fixture() -> (tempfile::TempDir, Inventory, RecipeV1, FrozenInputs) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Original.MKV"), b"media").unwrap();
    let root = RootBinding::new(dir.path().to_path_buf()).unwrap();
    let inventory = scan(&InventoryRequest::new(vec![root]), &JobControl::default()).unwrap();
    let id = inventory
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::File)
        .unwrap()
        .id;
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 9,
        selection: vec![id],
        numbering_order: vec![id],
        rules: vec![Rule {
            id: "rename".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::FixedStem {
                value: "Renamed".into(),
            },
        }],
        companions: CompanionPolicy::None,
        destination: Destination::SameDirectory,
        collision_policy: CollisionPolicy::Block,
        metadata_changes: vec![],
    };
    let inputs = FrozenInputs {
        inventory_id: inventory.id,
        generation: inventory.generation,
        now_ns: 1_701_234_567_123_456_789,
        timezone: FrozenTimezone::Utc,
        primary_order: vec![id],
        values: vec![FrozenValue {
            entry_id: id,
            rule_id: Some("rename".into()),
            key: FieldKey::Uuid,
            value: FrozenFieldValue::Text("frozen-uuid".into()),
        }],
    };
    (dir, inventory, recipe, inputs)
}

// Catches accepting unsupported schema versions or silently ignoring unknown tags.
#[test]
fn unknown_recipe_version_or_rule_rejected() {
    let (_, _, recipe, _) = fixture();
    let mut json = serde_json::to_value(&recipe).unwrap();
    json["schema_version"] = 2.into();
    assert_eq!(
        RecipeV1::from_json(&serde_json::to_vec(&json).unwrap())
            .unwrap_err()
            .code,
        RenameErrorCode::UnknownRecipeVersion
    );
    json["schema_version"] = 1.into();
    json["rules"][0]["parameters"]["type"] = "execute_script".into();
    assert_eq!(
        RecipeV1::from_json(&serde_json::to_vec(&json).unwrap())
            .unwrap_err()
            .code,
        RenameErrorCode::InvalidRecipe
    );
    assert!(serde_json::from_value::<RecipeV1>(json).is_err());
}

// Catches generation during evaluation, changed timestamps, random IDs or output ordering.
#[test]
fn preview_freezes_clock_and_random() {
    let (_, inventory, recipe, inputs) = fixture();
    let first = evaluate(&inventory, &recipe, &inputs).unwrap();
    let repeated = evaluate(&inventory, &recipe, &inputs).unwrap();
    assert_eq!(first.rows, repeated.rows);
    assert_eq!(first.frozen_inputs, inputs);
    assert_eq!(first.rows[0].new_display_name, "Renamed.MKV");
    assert_eq!(first.rows[0].status, RowStatus::Changed);
    assert_eq!(
        serde_json::to_vec(&first.to_dto()).unwrap(),
        serde_json::to_vec(&repeated.to_dto()).unwrap()
    );
}

// Catches trusting arbitrary entry IDs or treating a display path as source authority.
#[test]
fn unselected_or_outside_grant_id_rejected() {
    let (_, mut inventory, mut recipe, mut inputs) = fixture();
    inputs.values[0].entry_id = EntryId::new();
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::UnselectedEntry
    );
    inputs.values[0].entry_id = recipe.selection[0];
    let entry = inventory
        .entries
        .iter_mut()
        .find(|e| e.id == recipe.selection[0])
        .unwrap();
    entry.absolute_path = inventory.roots[0]
        .path
        .parent()
        .unwrap()
        .join("outside.mkv");
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::OutsideGrant
    );
    recipe.selection = vec![EntryId::new()];
    recipe.numbering_order = recipe.selection.clone();
    inputs.primary_order = recipe.selection.clone();
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::UnknownEntry
    );
}

// Catches float/numeric JSON rounding at the projection boundary.
#[test]
fn json_preserves_large_sizes() {
    let (_, mut inventory, recipe, inputs) = fixture();
    let entry = inventory
        .entries
        .iter_mut()
        .find(|e| e.id == recipe.selection[0])
        .unwrap();
    entry.stamp.size = Some(9_007_199_254_740_993);
    entry.stamp.modified_ns = Some(-9_007_199_254_740_993);
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    let json_row = &batch.to_dto().rows[0];
    assert_eq!(json_row.size_bytes.as_deref(), Some("9007199254740993"));
    assert_eq!(json_row.modified_ns.as_deref(), Some("-9007199254740993"));
    let json = serde_json::to_value(batch.to_dto()).unwrap();
    assert_eq!(json["rows"][0]["size_bytes"], "9007199254740993");
    assert!(json.get("grants").is_none());
    assert!(json["rows"][0].get("source_path").is_none());
    assert!(json["rows"][0].get("target_path").is_none());
}

// Catches applying to original input each time or losing ordered rule explanations.
#[test]
fn explanations_follow_rule_order() {
    let (_, inventory, mut recipe, inputs) = fixture();
    recipe.rules.push(Rule {
        id: "keep".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::Keep,
    });
    recipe.rules.push(Rule {
        id: "last".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::FixedStem {
            value: "Final".into(),
        },
    });
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    let row = &batch.rows[0];
    assert_eq!(row.new_display_name, "Final.MKV");
    assert_eq!(
        row.explanations
            .iter()
            .map(|e| (e.rule_id.as_str(), e.before.as_str(), e.after.as_str()))
            .collect::<Vec<_>>(),
        [
            ("rename", "Original", "Renamed"),
            ("keep", "Renamed", "Renamed"),
            ("last", "Renamed", "Final")
        ]
    );
}

// Catches silently defaulting malformed selections, revisions or future rule parameters.
#[test]
fn recipe_schema_round_trip_and_validation() {
    let (_, _, mut recipe, _) = fixture();
    let json = serde_json::to_vec(&recipe).unwrap();
    assert_eq!(RecipeV1::from_json(&json).unwrap(), recipe);
    let mut numeric = serde_json::to_value(&recipe).unwrap();
    numeric["revision"] = 9.into();
    assert!(RecipeV1::from_json(&serde_json::to_vec(&numeric).unwrap()).is_err());
    recipe.rules.push(recipe.rules[0].clone());
    assert_eq!(
        recipe.validate().unwrap_err().code,
        RenameErrorCode::InvalidRecipe
    );
}

// Catches accepting stale inputs and deriving counters from mutable display/inventory order.
#[test]
fn frozen_inventory_and_numbering_order_must_match() {
    let (_, inventory, recipe, mut inputs) = fixture();
    inputs.generation += 1;
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::StaleInputs
    );
    inputs.generation = inventory.generation;
    inputs.primary_order.clear();
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::InvalidNumberingOrder
    );
}

// Catches bypassing the bound or accepting unsupported active configurations as a no-op.
#[test]
fn bounded_recipe_import_and_unimplemented_rules_fail_closed() {
    assert_eq!(
        RecipeV1::from_json(&vec![b' '; 1_048_577])
            .unwrap_err()
            .code,
        RenameErrorCode::RecipeTooLarge
    );
    let (_, inventory, mut recipe, inputs) = fixture();
    recipe.rules[0].parameters = RuleParameters::Template {
        parts: vec![TemplatePart::Field {
            key: FieldKey::Uuid,
            format: ValueFormat::Text,
            fallback: None,
        }],
    };
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::UnsupportedRule
    );
    recipe.rules[0].enabled = false;
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].status,
        RowStatus::Unchanged
    );
}

// Catches permitting path traversal through a stem or implicit extension changes.
#[test]
fn invalid_fixed_stem_or_segment_rejected_before_evaluation() {
    let (_, inventory, mut recipe, inputs) = fixture();
    for value in ["", ".", "..", "../escape", "a/b", "a\\b", "a\0b"] {
        recipe.rules[0].parameters = RuleParameters::FixedStem {
            value: value.into(),
        };
        assert_eq!(
            evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
            RenameErrorCode::InvalidRecipe
        );
    }
    recipe.rules[0].parameters = RuleParameters::FixedStem {
        value: "good".into(),
    };
    recipe.rules[0].segment = SegmentSelector::Extension;
    assert_eq!(
        recipe.validate().unwrap_err().code,
        RenameErrorCode::InvalidRecipe
    );
}

// Catches nonsensical template formats and loss of explicit missing-field decisions.
#[test]
fn template_types_and_frozen_values_round_trip() {
    let (_, _, mut recipe, mut inputs) = fixture();
    let parts = vec![
        TemplatePart::Literal("prefix".into()),
        TemplatePart::Optional(vec![TemplatePart::Field {
            key: FieldKey::SizeBytes,
            format: ValueFormat::Integer {
                radix: Radix::Decimal,
                padding: 4,
            },
            fallback: None,
        }]),
    ];
    recipe.rules[0].parameters = RuleParameters::Template { parts };
    assert_eq!(
        RecipeV1::from_json(&serde_json::to_vec(&recipe).unwrap()).unwrap(),
        recipe
    );
    recipe.rules[0].parameters = RuleParameters::Template {
        parts: vec![TemplatePart::Field {
            key: FieldKey::OriginalStem,
            format: ValueFormat::Integer {
                radix: Radix::HexLower,
                padding: 0,
            },
            fallback: None,
        }],
    };
    assert_eq!(
        recipe.validate().unwrap_err().code,
        RenameErrorCode::InvalidRecipe
    );
    inputs.values[0].value = FrozenFieldValue::Missing {
        decision: MissingDecision::Fallback("known".into()),
    };
    assert_eq!(
        serde_json::from_slice::<FrozenInputs>(&serde_json::to_vec(&inputs).unwrap()).unwrap(),
        inputs
    );
}

fn plan_fixture() -> (tempfile::TempDir, RenamePlan) {
    let (dir, inventory, recipe, inputs) = fixture();
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    let version = PlanRef {
        session_id: SessionId::new(),
        plan_id: PlanId::new(),
        revision: 1,
    };
    let execution = ExecutionPlan {
        groups: vec![],
        provenance: InspectionProvenance {
            approved_plan: version,
            inventory,
            recipe,
            frozen_inputs: inputs.clone(),
            grants: batch.grants.clone(),
            namespace_epoch: 0,
            policy_epoch: 0,
        },
    };
    let plan = RenamePlan {
        version,
        inventory_id: batch.inventory_id,
        generation: batch.generation,
        recipe_revision: batch.recipe_revision,
        counts: PlanCounts::from_rows(&batch.rows).unwrap(),
        rows: batch.rows,
        issues: vec![],
        execution,
        frozen_inputs: inputs,
        grants: batch.grants,
    };
    (dir, plan)
}

// Catches cross-session lookup, replayed confirmation, stale-ref revival and invalidation
// that leaves a usable confirmation behind. No commands or executor are mocked.
#[test]
fn plan_store_revokes_refs_and_confirmation_nonces() {
    let (_dir, plan) = plan_fixture();
    let version = plan.version;
    let mut store = PlanStore::new(version.session_id);
    store.publish(plan.clone()).unwrap();
    let mut foreign = version;
    foreign.session_id = SessionId::new();
    assert_eq!(
        store.current(foreign).unwrap_err().code,
        RenameErrorCode::InvalidPlanRef
    );
    store.confirm(version, "nonce-1".into()).unwrap();
    assert_eq!(
        store
            .consume_confirmation(version, "wrong")
            .unwrap_err()
            .code,
        RenameErrorCode::InvalidConfirmation
    );
    store.consume_confirmation(version, "nonce-1").unwrap();
    assert_eq!(
        store
            .consume_confirmation(version, "nonce-1")
            .unwrap_err()
            .code,
        RenameErrorCode::InvalidConfirmation
    );
    assert_eq!(
        store.confirm(version, "nonce-1".into()).unwrap_err().code,
        RenameErrorCode::InvalidConfirmation
    );
    store.confirm(version, "nonce-2".into()).unwrap();
    store.invalidate();
    assert_eq!(
        store.current(version).unwrap_err().code,
        RenameErrorCode::InvalidPlanRef
    );
    assert_eq!(
        store
            .consume_confirmation(version, "nonce-2")
            .unwrap_err()
            .code,
        RenameErrorCode::InvalidPlanRef
    );
    assert_eq!(
        store.publish(plan.clone()).unwrap_err().code,
        RenameErrorCode::InvalidPlanRef
    );
    let mut newer = plan;
    newer.version.revision = 2;
    newer.execution.provenance.approved_plan = newer.version;
    store.publish(newer).unwrap();
    assert_eq!(
        store.current(version).unwrap_err().code,
        RenameErrorCode::InvalidPlanRef
    );
}

// Catches a store accepting a plan from another process or mismatched provenance.
#[test]
fn plan_store_rejects_foreign_session_and_snapshot_mismatch() {
    let (_dir, mut plan) = plan_fixture();
    let mut store = PlanStore::new(SessionId::new());
    assert_eq!(
        store.publish(plan.clone()).unwrap_err().code,
        RenameErrorCode::InvalidPlanRef
    );
    let mut store = PlanStore::new(plan.version.session_id);
    plan.generation += 1;
    assert_eq!(
        store.publish(plan).unwrap_err().code,
        RenameErrorCode::InvalidPlanRef
    );
}

// Catches unstable DTO plans, imprecise revision/count encoding and backend leakage.
#[test]
fn plan_dto_is_stable_exact_and_contains_no_execution_authority() {
    let (_dir, mut plan) = plan_fixture();
    assert_eq!(plan.counts.changed, 1);
    assert_eq!(plan.counts.unchanged, 0);
    plan.version.revision = 9_007_199_254_740_993;
    let first = serde_json::to_value(plan.to_dto()).unwrap();
    assert_eq!(first["version"]["revision"], "9007199254740993");
    assert_eq!(first["counts"]["changed"], "1");
    assert!(first.get("execution").is_none());
    assert!(first.get("grants").is_none());
    assert!(first.get("frozen_inputs").is_none());
    assert_eq!(first, serde_json::to_value(plan.to_dto()).unwrap());
    plan.version.plan_id = PlanId::new();
    let mut second = serde_json::to_value(plan.to_dto()).unwrap();
    second["version"] = first["version"].clone();
    assert_eq!(first, second);
}

// Catches prematurely rejecting row-dependent grapheme positions, invalid date directives and duplicated
// association ownership before any rule implementation or output rows run.
#[test]
fn invalid_segments_templates_and_associations_rejected() {
    let (_, _, mut recipe, _) = fixture();
    recipe.rules[0].parameters = RuleParameters::Keep;
    recipe.rules[0].segment = SegmentSelector::Graphemes {
        target: NameSegment::Stem,
        start: 0,
        end: 1,
    };
    assert!(recipe.validate().is_ok()); // zero is a row issue when a segment evaluator exists
    recipe.rules[0].segment = SegmentSelector::Delimited {
        target: NameSegment::Stem,
        delimiter: "-".into(),
        occurrence: 0,
        include_delimiter: false,
    };
    assert!(recipe.validate().is_err());
    recipe.rules[0].segment = SegmentSelector::Stem;
    recipe.rules[0].parameters = RuleParameters::Template {
        parts: vec![TemplatePart::Field {
            key: FieldKey::Date {
                source: DateSource::Now,
            },
            format: ValueFormat::Date {
                pattern: "%Q".into(),
            },
            fallback: None,
        }],
    };
    assert!(recipe.validate().is_err());
    recipe.rules[0].parameters = RuleParameters::Keep;
    recipe.companions = CompanionPolicy::Explicit {
        associations: vec![CompanionAssociation {
            primary: recipe.selection[0],
            companions: vec![recipe.selection[0]],
        }],
    };
    assert!(recipe.validate().is_err());
}

// Catches duplicate values being chosen by iteration accident and invalid offsets.
#[test]
fn duplicate_frozen_values_and_invalid_timezone_rejected() {
    let (_, inventory, recipe, mut inputs) = fixture();
    inputs.values.push(inputs.values[0].clone());
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::InvalidFrozenInputs
    );
    inputs.values.pop();
    inputs.timezone = FrozenTimezone::FixedOffset { seconds: 86_400 };
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::InvalidFrozenInputs
    );
}

// Catches bypassing version rejection by deserializing RecipeV1 directly.
#[test]
fn direct_recipe_deserialization_rejects_unknown_version() {
    let (_, _, recipe, _) = fixture();
    let mut json = serde_json::to_value(recipe).unwrap();
    json["schema_version"] = 99.into();
    assert!(serde_json::from_value::<RecipeV1>(json).is_err());
}

// Catches a frozen field carrying an incompatible primitive, which later renderers
// could otherwise silently interpret differently from the preview producer.
#[test]
fn frozen_field_types_are_checked_before_rows() {
    let (_, inventory, recipe, mut inputs) = fixture();
    inputs.values[0].key = FieldKey::SizeBytes;
    inputs.values[0].value = FrozenFieldValue::Text("not a size".into());
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::InvalidFrozenInputs
    );
}

// Catches assuming every grant is a directory; individual native file selection
// must remain a valid source while its neighboring files gain no authority.
#[test]
fn individual_file_grant_is_supported() {
    let (dir, _, mut recipe, mut inputs) = fixture();
    let inventory = scan(
        &InventoryRequest::new(vec![
            RootBinding::new(dir.path().join("Original.MKV")).unwrap()
        ]),
        &JobControl::default(),
    )
    .unwrap();
    let id = inventory.entries[0].id;
    recipe.selection = vec![id];
    recipe.numbering_order = vec![id];
    inputs.inventory_id = inventory.id;
    inputs.primary_order = vec![id];
    inputs.values[0].entry_id = id;
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
        "Renamed.MKV"
    );
}
