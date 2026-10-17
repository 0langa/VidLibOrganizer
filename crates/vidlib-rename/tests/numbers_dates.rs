use serde_json::{json, Value};
use vidlib_inventory::*;
use vidlib_rename::*;

fn parameter(value: Value) -> RuleParameters {
    let parsed = serde_json::from_value(value);
    assert!(
        parsed.is_ok(),
        "number/date recipe must be supported: {parsed:?}"
    );
    parsed.unwrap()
}
fn number(start: &str, step: &str, repeat: &str, radix: &str) -> RuleParameters {
    parameter(json!({"type":"number", "rule": {
        "start":start,"step":step,"repeat_each":repeat,"padding":"2",
        "radix":radix,"reset_by_folder":false,"at":{"type":"end"},"prefix":"-","suffix":""
    }}))
}
fn date(reformat: bool, input: Option<&str>) -> RuleParameters {
    let rule = json!({"input_format":input,"output_format":"%Y-%m-%d","timezone":null,
        "assume_timezone":null,"ambiguity":"reject","nonexistent":"reject"});
    parameter(if reformat {
        json!({"type":"date","operation":{"type":"reformat","rule":rule}})
    } else {
        json!({"type":"date","operation":{"type":"insert","source":"recorded","at":{"type":"end"},"prefix":"","suffix":"","rule":rule}})
    })
}
fn fixture(
    names: &[&str],
    parameters: RuleParameters,
) -> (tempfile::TempDir, Inventory, RecipeV1, FrozenInputs) {
    let dir = tempfile::tempdir().unwrap();
    for name in names {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"media").unwrap();
    }
    let inventory = scan(
        &InventoryRequest::new(vec![RootBinding::new(dir.path().to_owned()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let ids = names
        .iter()
        .map(|name| {
            inventory
                .entries
                .iter()
                .find(|e| e.absolute_path == dir.path().join(name))
                .unwrap()
                .id
        })
        .collect::<Vec<_>>();
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: ids.clone(),
        selection_filter: None,
        numbering_order: ids.clone(),
        numbering_scope: Default::default(),
        rules: vec![Rule {
            id: "r".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters,
        }],
        companions: CompanionPolicy::None,
        destination: Destination::SameDirectory,
        collision_policy: CollisionPolicy::Block,
        metadata_changes: vec![],
    };
    let inputs = FrozenInputs {
        inventory_id: inventory.id,
        generation: inventory.generation,
        now_ns: 0,
        timezone: FrozenTimezone::Utc,
        primary_order: ids,
        numbering_scope: Default::default(),
        values: vec![],
    };
    (dir, inventory, recipe, inputs)
}
#[test]
fn natural_display_sort_does_not_change_number_order() {
    let (_dir, mut inventory, recipe, inputs) =
        fixture(&["10.mkv", "2.mkv"], number("1", "1", "1", "decimal"));
    inventory.entries.reverse();
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    assert_eq!(
        batch
            .rows
            .iter()
            .map(|r| r.new_display_name.as_str())
            .collect::<Vec<_>>(),
        ["10-01.mkv", "2-02.mkv"]
    );
}
#[test]
fn counter_repeat_step_reset_and_formats() {
    let (_dir, inventory, recipe, inputs) = fixture(
        &["a.mkv", "b.mkv", "c.mkv", "d.mkv"],
        number("1", "2", "2", "decimal"),
    );
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    assert_eq!(
        batch
            .rows
            .iter()
            .map(|r| r.new_display_name.as_str())
            .collect::<Vec<_>>(),
        ["a-01.mkv", "b-01.mkv", "c-03.mkv", "d-03.mkv"]
    );
}
#[test]
fn pad_existing_number_in_segment() {
    let pad = parameter(json!({"type":"pad_numbers","padding":"3","scope":"all"}));
    let (_dir, inventory, recipe, inputs) = fixture(&["S2E10.MKV"], pad);
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
        "S002E010.MKV"
    );
}
#[test]
fn ambiguous_date_requires_format() {
    let (_dir, inventory, recipe, inputs) = fixture(&["10-11-2026.mkv"], date(true, None));
    let row = &evaluate(&inventory, &recipe, &inputs).unwrap().rows[0];
    assert_eq!(format!("{:?}", row.issues[0].code), "DateNeedsFormat");
}
#[test]
fn explicit_date_reformat() {
    let (_dir, inventory, recipe, inputs) =
        fixture(&["17-10-2026.MKV"], date(true, Some("%d-%m-%Y")));
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
        "2026-10-17.MKV"
    );
}
#[test]
fn missing_recording_timezone_stays_unknown() {
    let (_dir, inventory, recipe, mut inputs) = fixture(&["clip.mkv"], date(false, None));
    inputs.values.push(FrozenValue {
        scope: Default::default(),
        entry_id: recipe.selection[0],
        rule_id: Some("r".into()),
        key: FieldKey::Date {
            source: DateSource::Recorded,
        },
        value: FrozenFieldValue::Timestamp {
            ns: 0,
            timezone: None,
        },
    });
    let row = &evaluate(&inventory, &recipe, &inputs).unwrap().rows[0];
    assert_eq!(format!("{:?}", row.issues[0].code), "DateTimezoneUnknown");
}
#[test]
fn dst_ambiguity_is_explicit() {
    for (input, policy, nonexistent, want) in [
        ("2026-10-25_02-30-00", "reject", "reject", "DateAmbiguous"),
        (
            "2026-10-25_02-30-00",
            "earliest",
            "reject",
            "2026-10-25_00-30-00",
        ),
        (
            "2026-10-25_02-30-00",
            "latest",
            "reject",
            "2026-10-25_01-30-00",
        ),
        ("2026-03-29_02-30-00", "reject", "reject", "DateNonexistent"),
        (
            "2026-03-29_02-30-00",
            "reject",
            "next_valid",
            "2026-03-29_01-00-00",
        ),
        (
            "2026-03-29_02-30-00",
            "reject",
            "previous_valid",
            "2026-03-29_00-59-59",
        ),
    ] {
        let p = parameter(json!({"type":"date","operation":{"type":"reformat","rule":{
            "input_format":"%Y-%m-%d_%H-%M-%S","output_format":"%Y-%m-%d_%H-%M-%S",
            "timezone":{"type":"utc"},"assume_timezone":{"type":"iana","name":"Europe/Berlin"},
            "ambiguity":policy,"nonexistent":nonexistent}}}));
        let name = format!("{input}.mkv");
        let (_dir, inventory, recipe, inputs) = fixture(&[&name], p);
        let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
        let row = &batch.rows[0];
        if want.starts_with("Date") {
            assert_eq!(format!("{:?}", row.issues[0].code), want);
        } else {
            assert_eq!(row.new_display_name, format!("{want}.mkv"));
        }
    }
}
#[test]
fn counter_alphabetic_signed_padding_and_source_parent_resets() {
    for (start, step, radix, want) in [
        (
            "25",
            "1",
            "alphabetic_upper",
            vec!["a-Y.mkv", "b-Z.mkv", "c-AA.mkv"],
        ),
        (
            "-15",
            "-1",
            "hex_upper",
            vec!["a--0F.mkv", "b--10.mkv", "c--11.mkv"],
        ),
        (
            "1",
            "-2",
            "decimal",
            vec!["a-01.mkv", "b--01.mkv", "c--03.mkv"],
        ),
    ] {
        let (_dir, inventory, recipe, inputs) = fixture(
            &["a.mkv", "b.mkv", "c.mkv"],
            number(start, step, "1", radix),
        );
        assert_eq!(
            evaluate(&inventory, &recipe, &inputs)
                .unwrap()
                .rows
                .iter()
                .map(|r| r.new_display_name.as_str())
                .collect::<Vec<_>>(),
            want
        );
    }
    let mut p = serde_json::to_value(number("1", "1", "1", "decimal")).unwrap();
    p["rule"]["reset_by_folder"] = true.into();
    let (_dir, inventory, recipe, inputs) =
        fixture(&["A/a.mkv", "B/b.mkv", "A/c.mkv"], parameter(p));
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs)
            .unwrap()
            .rows
            .iter()
            .map(|r| r.new_display_name.as_str())
            .collect::<Vec<_>>(),
        ["a-01.mkv", "b-01.mkv", "c-02.mkv"]
    );
}
#[test]
fn counters_reject_invalid_repeat_and_checked_overflow() {
    for (start, step, repeat, radix, want) in [
        ("1", "1", "0", "decimal", RenameErrorCode::InvalidRecipe),
        (
            "0",
            "1",
            "1",
            "alphabetic_upper",
            RenameErrorCode::InvalidRuleInput,
        ),
        (
            "170141183460469231731687303715884105727",
            "1",
            "1",
            "decimal",
            RenameErrorCode::ArithmeticOverflow,
        ),
        (
            "0",
            "170141183460469231731687303715884105727",
            "1",
            "decimal",
            RenameErrorCode::ArithmeticOverflow,
        ),
    ] {
        let (_dir, inventory, recipe, inputs) = fixture(
            &["a.mkv", "b.mkv", "c.mkv"],
            number(start, step, repeat, radix),
        );
        assert_eq!(
            evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
            want
        );
    }
}
#[test]
fn explicit_dates_restore_original_target_and_prior_explanations() {
    let (_dir, inventory, mut recipe, inputs) = fixture(&["10-11-2026.MKV"], date(true, None));
    recipe.rules.insert(
        0,
        Rule {
            id: "prefix".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::Text {
                operation: TextOperation::Prefix { text: "X".into() },
            },
        },
    );
    let row = &evaluate(&inventory, &recipe, &inputs).unwrap().rows[0];
    assert_eq!(row.status, RowStatus::Blocked);
    assert_eq!(row.new_display_name, "10-11-2026.MKV");
    assert_eq!(row.explanations.len(), 1);
    assert_eq!(row.explanations[0].after, "X10-11-2026");
}
#[test]
fn exact_numeric_json_and_bounded_configuration() {
    let mut serialized =
        serde_json::to_value(number("9007199254740993", "-2", "1", "decimal")).unwrap();
    assert_eq!(serialized["rule"]["start"], "9007199254740993");
    for field in ["start", "step", "repeat_each", "padding"] {
        let mut invalid = serialized.clone();
        invalid["rule"][field] = 1.into();
        assert!(
            serde_json::from_value::<RuleParameters>(invalid).is_err(),
            "accepted JS numeric {field}"
        );
    }
    serialized["rule"]["at"] = json!({"type":"before","position":"1"});
    assert!(
        serde_json::from_value::<RuleParameters>(serialized).is_ok(),
        "number insertion indices must cross JSON exactly"
    );
    let zone = serde_json::to_value(FrozenTimezone::FixedOffset { seconds: 3600 }).unwrap();
    assert_eq!(zone["seconds"], "3600");
}
#[test]
fn file_dates_are_distinct_now_is_frozen_and_absence_blocks() {
    for (source, want) in [
        ("created", "clip1970-01-01.mkv"),
        ("modified", "clip1970-01-02.mkv"),
        ("accessed", "clip1970-01-03.mkv"),
        ("now", "clip1970-01-04.mkv"),
        ("recorded", "missing"),
    ] {
        let mut p = serde_json::to_value(date(false, None)).unwrap();
        p["operation"]["source"] = source.into();
        let (_dir, mut inventory, recipe, mut inputs) = fixture(&["clip.mkv"], parameter(p));
        let entry = inventory
            .entries
            .iter_mut()
            .find(|e| e.id == recipe.selection[0])
            .unwrap();
        entry.stamp.created_ns = Some(0);
        entry.stamp.modified_ns = Some(86_400_000_000_000);
        entry.stamp.accessed_ns = Some(172_800_000_000_000);
        inputs.now_ns = 259_200_000_000_000;
        let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
        if want == "missing" {
            assert_eq!(format!("{:?}", batch.rows[0].issues[0].code), "DateMissing");
        } else {
            assert_eq!(batch.rows[0].new_display_name, want);
            assert_eq!(
                evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
                want
            );
        }
        let entry = inventory
            .entries
            .iter_mut()
            .find(|e| e.id == recipe.selection[0])
            .unwrap();
        entry.stamp.created_ns = None;
        entry.stamp.modified_ns = None;
        entry.stamp.accessed_ns = None;
        if source != "now" {
            assert_eq!(
                format!(
                    "{:?}",
                    evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].issues[0].code
                ),
                "DateMissing"
            );
        }
    }
}
#[test]
fn date_fixed_iana_utc_and_recorded_assumption() {
    for (zone, want) in [
        (json!({"type":"utc"}), "clip1970-01-01_00-00-00_+0000.mkv"),
        (
            json!({"type":"fixed_offset","seconds":"-3600"}),
            "clip1969-12-31_23-00-00_-0100.mkv",
        ),
        (
            json!({"type":"iana","name":"Europe/Berlin"}),
            "clip1970-01-01_01-00-00_+0100.mkv",
        ),
    ] {
        let mut p = serde_json::to_value(date(false, None)).unwrap();
        p["operation"]["rule"]["assume_timezone"] = json!({"type":"utc"});
        p["operation"]["rule"]["timezone"] = zone;
        p["operation"]["rule"]["output_format"] = "%Y-%m-%d_%H-%M-%S_%z".into();
        let (_dir, inventory, recipe, mut inputs) = fixture(&["clip.mkv"], parameter(p));
        inputs.values.push(FrozenValue {
            scope: Default::default(),
            entry_id: recipe.selection[0],
            rule_id: None,
            key: FieldKey::Date {
                source: DateSource::Recorded,
            },
            value: FrozenFieldValue::Timestamp {
                ns: 0,
                timezone: None,
            },
        });
        assert_eq!(
            evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
            want
        );
    }
}
#[test]
fn number_date_rules_use_current_segments_and_keep_extensions() {
    let (_dir, inventory, mut recipe, inputs) = fixture(
        &["x[2]Z.MKV"],
        parameter(json!({"type":"pad_numbers","padding":"3","scope":"all"})),
    );
    recipe.rules[0].segment = SegmentSelector::Between {
        target: NameSegment::Stem,
        opening: "[".into(),
        closing: "]".into(),
        occurrence: 1,
        include_delimiters: false,
    };
    recipe.rules.push(Rule {
        id: "n".into(),
        enabled: true,
        segment: SegmentSelector::Delimited {
            target: NameSegment::Stem,
            delimiter: "]".into(),
            occurrence: 1,
            include_delimiter: false,
        },
        parameters: number("1", "1", "1", "decimal"),
    });
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
        "x[002]Z-01.MKV"
    );
    let (_dir, inventory, mut recipe, inputs) =
        fixture(&["x[17-10-2026]Z.MKV"], date(true, Some("%d-%m-%Y")));
    recipe.rules[0].segment = SegmentSelector::Between {
        target: NameSegment::Stem,
        opening: "[".into(),
        closing: "]".into(),
        occurrence: 1,
        include_delimiters: false,
    };
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
        "x[2026-10-17]Z.MKV"
    );
    recipe.rules[0].segment = SegmentSelector::Extension;
    recipe.rules[0].parameters =
        parameter(json!({"type":"pad_numbers","padding":"3","scope":"all"}));
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
        "x[17-10-2026]Z.MKV"
    );
}
#[test]
fn typed_recording_local_time_uses_explicit_zone_and_policy() {
    let mut p = serde_json::to_value(date(false, None)).unwrap();
    p["operation"]["rule"]["output_format"] = "%Y-%m-%d_%H-%M-%S".into();
    p["operation"]["rule"]["ambiguity"] = "latest".into();
    let (_dir, inventory, recipe, mut inputs) = fixture(&["clip.mkv"], parameter(p));
    let value: Result<FrozenFieldValue, _> =
        serde_json::from_value(json!({"type":"local_timestamp","value":{
        "value":"2026-10-25T02:30:00","timezone":{"type":"iana","name":"Europe/Berlin"}}}));
    assert!(
        value.is_ok(),
        "typed local recording timestamps must be accepted: {value:?}"
    );
    inputs.values.push(FrozenValue {
        scope: Default::default(),
        entry_id: recipe.selection[0],
        rule_id: None,
        key: FieldKey::Date {
            source: DateSource::Recorded,
        },
        value: value.unwrap(),
    });
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
        "clip2026-10-25_01-30-00.mkv"
    );
}
#[test]
fn assignment_rejects_duplicate_inventory_and_skips_nonprimary_entries() {
    let parameters = number("1", "1", "1", "decimal");
    let (_dir, mut inventory, recipe, _inputs) =
        fixture(&["a.mkv", "a.srt", "b.mkv"], parameters.clone());
    let RuleParameters::Number { rule } = parameters else {
        unreachable!()
    };
    let assignments = assign_numbers(
        &inventory.entries,
        &vec![recipe.selection[0], recipe.selection[2]],
        &rule,
    )
    .unwrap();
    assert_eq!(assignments[&recipe.selection[2]], "02");
    assert!(!assignments.contains_key(&recipe.selection[1]));
    inventory.entries.push(
        inventory
            .entries
            .iter()
            .find(|e| e.id == recipe.selection[0])
            .unwrap()
            .clone(),
    );
    assert_eq!(
        assign_numbers(&inventory.entries, &recipe.numbering_order, &rule)
            .unwrap_err()
            .code,
        RenameErrorCode::InvalidInventory
    );
}
#[test]
fn direct_formats_handle_minimum_value_negative_epoch_and_invalid_patterns() {
    assert_eq!(
        format_number(i128::MIN, Radix::Decimal, 40).unwrap(),
        "-0170141183460469231731687303715884105728"
    );
    assert_eq!(format_number(27, Radix::AlphabeticLower, 0).unwrap(), "aa");
    assert!(format_number(-1, Radix::AlphabeticLower, 0).is_err());
    assert!(format_number(1, Radix::Decimal, 4097).is_err());
    assert_eq!(
        pad_existing_numbers("-2 0004 １２", 3, MatchScope::First).unwrap(),
        "-002 0004 １２"
    );
    let env = FrozenEnvironment::freeze(
        -1,
        FrozenTimezone::Utc,
        DstAmbiguity::Reject,
        NonexistentTime::Reject,
    )
    .unwrap();
    let mut rule:DateRule=serde_json::from_value(json!({"input_format":null,"output_format":"%Y-%m-%d_%H-%M-%S_%f","timezone":null,"assume_timezone":null,"ambiguity":"reject","nonexistent":"reject"})).unwrap();
    assert_eq!(
        format_date(&DateValue::Instant { ns: -1 }, &rule, &env).unwrap(),
        "1969-12-31_23-59-59_999999999"
    );
    assert!(format_date(&DateValue::Instant { ns: i128::MAX }, &rule, &env).is_err());
    rule.output_format = "%c".into();
    assert!(format_date(&DateValue::Instant { ns: 0 }, &rule, &env).is_err());
    assert!(FrozenEnvironment::freeze(
        0,
        FrozenTimezone::Iana {
            name: "Europe/Unconfirmed".into()
        },
        DstAmbiguity::Reject,
        NonexistentTime::Reject
    )
    .is_err());
}
#[test]
fn insert_positions_prefix_suffix_and_invalid_rows_use_shared_graphemes() {
    let mut p = serde_json::to_value(number("1", "1", "1", "decimal")).unwrap();
    p["rule"]["at"] = json!({"type":"before","position":"-1"});
    p["rule"]["prefix"] = "[".into();
    p["rule"]["suffix"] = "]".into();
    let (_dir, inventory, recipe, inputs) = fixture(&["e\u{301}🦀.mkv"], parameter(p.clone()));
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap().rows[0].new_display_name,
        "e\u{301}[01]🦀.mkv"
    );
    p["rule"]["at"] = json!({"type":"before","position":"0"});
    let (_dir, inventory, recipe, inputs) = fixture(&["clip.mkv"], parameter(p));
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    assert_eq!(batch.rows[0].status, RowStatus::Blocked);
    assert_eq!(batch.rows[0].new_display_name, "clip.mkv");
}
#[test]
fn date_format_literals_are_not_mistaken_for_time_or_offset_fields() {
    let env = FrozenEnvironment::freeze(
        0,
        FrozenTimezone::Utc,
        DstAmbiguity::Reject,
        NonexistentTime::Reject,
    )
    .unwrap();
    for (input, pattern, want) in [
        ("2026-10-17%H", "%Y-%m-%d%%H", "2026-10-17"),
        ("2026-10-17%z", "%Y-%m-%d%%z", "2026-10-17"),
    ] {
        let rule = DateRule {
            input_format: Some(pattern.into()),
            output_format: "%Y-%m-%d".into(),
            timezone: None,
            assume_timezone: None,
            ambiguity: DstAmbiguity::Reject,
            nonexistent: NonexistentTime::Reject,
        };
        assert_eq!(
            format_date(
                &DateValue::Text {
                    value: input.into()
                },
                &rule,
                &env
            )
            .unwrap(),
            want
        );
    }
}
#[test]
fn date_parsing_preserves_explicit_offsets_and_rejects_lossy_leap_seconds() {
    let env = FrozenEnvironment::freeze(
        0,
        FrozenTimezone::Utc,
        DstAmbiguity::Reject,
        NonexistentTime::Reject,
    )
    .unwrap();
    let rule = DateRule {
        input_format: None,
        output_format: "%Y-%m-%d_%H-%M-%S".into(),
        timezone: None,
        assume_timezone: None,
        ambiguity: DstAmbiguity::Reject,
        nonexistent: NonexistentTime::Reject,
    };
    assert_eq!(
        format_date(
            &DateValue::Text {
                value: "2026-10-17T01:00:00+02:00".into()
            },
            &rule,
            &env
        )
        .unwrap(),
        "2026-10-16_23-00-00"
    );
    let leap = format_date(
        &DateValue::Text {
            value: "2016-12-31T23:59:60Z".into(),
        },
        &rule,
        &env,
    );
    assert!(
        leap.is_err(),
        "epoch nanoseconds cannot represent a leap second without changing it: {leap:?}"
    );
}
