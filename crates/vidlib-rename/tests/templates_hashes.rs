use vidlib_inventory::*;
use vidlib_rename::*;

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
        numbering_order: ids.clone(),
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
        values: vec![],
    };
    (dir, inventory, recipe, inputs)
}
fn field(key: FieldKey) -> TemplatePart {
    TemplatePart::Field {
        key,
        format: ValueFormat::Text,
        fallback: None,
    }
}
fn template(parts: Vec<TemplatePart>) -> RuleParameters {
    RuleParameters::Template { parts }
}
fn result(inventory: &Inventory, recipe: &RecipeV1, inputs: &FrozenInputs) -> PlanRow {
    evaluate(inventory, recipe, inputs)
        .expect("typed template evaluation")
        .rows
        .remove(0)
}
#[test]
fn original_and_current_stem_differ() {
    let (_dir, inv, mut recipe, inputs) = fixture(
        &["Movie.MKV"],
        template(vec![
            field(FieldKey::OriginalStem),
            TemplatePart::Literal("-".into()),
            field(FieldKey::CurrentStem),
        ]),
    );
    recipe.rules.insert(
        0,
        Rule {
            id: "first".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::FixedStem {
                value: "Edited".into(),
            },
        },
    );
    assert_eq!(
        result(&inv, &recipe, &inputs).new_display_name,
        "Movie-Edited.MKV"
    );
}
#[test]
fn missing_required_optional_and_fallback() {
    let resolution = field(FieldKey::Media {
        property: MediaProperty::Width,
    });
    let (_dir, inv, mut recipe, inputs) = fixture(
        &["Movie.mkv"],
        template(vec![
            field(FieldKey::OriginalStem),
            TemplatePart::Literal(" (2020)".into()),
            TemplatePart::Optional(vec![
                TemplatePart::Literal(" [".into()),
                resolution.clone(),
                TemplatePart::Literal("]".into()),
            ]),
        ]),
    );
    assert_eq!(
        result(&inv, &recipe, &inputs).new_display_name,
        "Movie (2020).mkv"
    );
    recipe.rules[0].parameters = template(vec![resolution]);
    let row = result(&inv, &recipe, &inputs);
    assert_eq!(row.status, RowStatus::Blocked);
    assert_eq!(
        serde_json::to_value(row.issues[0].code).unwrap(),
        "metadata_missing"
    );
    recipe.rules[0].parameters = template(vec![TemplatePart::Field {
        key: FieldKey::Media {
            property: MediaProperty::Width,
        },
        format: ValueFormat::Integer {
            radix: Radix::Decimal,
            padding: 4,
        },
        fallback: Some("unknown".into()),
    }]);
    assert_eq!(
        result(&inv, &recipe, &inputs).new_display_name,
        "unknown.mkv"
    );
}
#[test]
fn all_template_field_families() {
    let (_dir, mut inv, recipe, mut inputs) = fixture(
        &["films/Movie.mkv"],
        template(vec![
            field(FieldKey::OriginalStem),
            TemplatePart::Literal("_".into()),
            field(FieldKey::Extension),
            TemplatePart::Literal("_".into()),
            field(FieldKey::Parent { level: 1 }),
            TemplatePart::Literal("_".into()),
            field(FieldKey::RelativeDirectory),
            TemplatePart::Literal("_".into()),
            field(FieldKey::SizeBytes),
            TemplatePart::Literal("_".into()),
            TemplatePart::Field {
                key: FieldKey::Date {
                    source: DateSource::Now,
                },
                format: ValueFormat::Date {
                    pattern: "%Y-%m-%d".into(),
                },
                fallback: None,
            },
            TemplatePart::Literal("_".into()),
            field(FieldKey::Counter),
            TemplatePart::Literal("_".into()),
            TemplatePart::Field {
                key: FieldKey::Media {
                    property: MediaProperty::FrameRate,
                },
                format: ValueFormat::Decimal { places: 2 },
                fallback: None,
            },
        ]),
    );
    inv.entries
        .iter_mut()
        .find(|e| e.id == inputs.primary_order[0])
        .unwrap()
        .stamp
        .size = Some(9_007_199_254_740_993);
    for (key, value) in [
        (FieldKey::Counter, FrozenFieldValue::Integer(27)),
        (
            FieldKey::Media {
                property: MediaProperty::FrameRate,
            },
            FrozenFieldValue::Decimal {
                coefficient: 23976,
                scale: 3,
            },
        ),
    ] {
        inputs.values.push(FrozenValue {
            entry_id: inputs.primary_order[0],
            rule_id: Some("r".into()),
            key,
            value,
        });
    }
    assert_eq!(
        result(&inv, &recipe, &inputs).new_display_name,
        "Movie_mkv_films_films_9007199254740993_1970-01-01_27_23.98.mkv"
    );
}
fn hash_recipe(algorithm: HashAlgorithm) -> RuleParameters {
    template(vec![field(FieldKey::Hash { algorithm })])
}
fn request(recipe: &RecipeV1) -> InputRequirements {
    InputRequirements::from_recipe(recipe, FrozenTimezone::Utc).unwrap()
}
#[test]
fn hashes_match_independent_full_file_vectors() {
    // Published standard "abc" vectors, not expectations derived by production code.
    for (algorithm,expected) in [
        (HashAlgorithm::Crc32,"352441c2"),
        (HashAlgorithm::Md5,"900150983cd24fb0d6963f7d28e17f72"),
        (HashAlgorithm::Sha1,"a9993e364706816aba3e25717850c26c9cd0d89d"),
        (HashAlgorithm::Sha256,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        (HashAlgorithm::Sha512,"ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f")
    ] {
        let (dir,_,recipe,_) = fixture(&["abc.mkv"],hash_recipe(algorithm));
        std::fs::write(dir.path().join("abc.mkv"),b"abc").unwrap();
        let mut inv=scan(&InventoryRequest::new(vec![RootBinding::new(dir.path().to_owned()).unwrap()]),&JobControl::default()).unwrap();
        let mut recipe=recipe;
        recipe.selection=inv.entries.iter().filter(|e|e.kind==EntryKind::File).map(|e|e.id).collect(); recipe.numbering_order=recipe.selection.clone();
        let inputs=freeze_inputs(&request(&recipe),&inv,&JobControl::default()).unwrap();
        // Pure evaluation remains possible after the source disappears.
        std::fs::remove_file(dir.path().join("abc.mkv")).unwrap();
        assert_eq!(result(&inv,&recipe,&inputs).new_display_name,format!("{expected}.mkv"),"{algorithm:?}");
        inv.entries.clear();
    }
}
#[test]
fn text_recipe_reads_no_contents() {
    let (dir, inv, recipe, _) = fixture(
        &["Movie.mkv"],
        template(vec![field(FieldKey::OriginalStem)]),
    );
    std::fs::remove_file(dir.path().join("Movie.mkv")).unwrap();
    let mut cache = FrozenInputCache::default();
    let inputs = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_eq!(cache.content_bytes_read, 0);
    assert_eq!(result(&inv, &recipe, &inputs).new_display_name, "Movie.mkv");
}
#[test]
fn generated_tokens_survive_repeated_preview() {
    let (_dir, inv, recipe, _) = fixture(
        &["Movie.mkv"],
        template(vec![
            field(FieldKey::Uuid),
            TemplatePart::Literal("_".into()),
            field(FieldKey::Random),
        ]),
    );
    let mut cache = FrozenInputCache::default();
    let first = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_eq!(
        first.values.len(),
        2,
        "requested UUID and random must be frozen"
    );
    let second = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        result(&inv, &recipe, &first).new_display_name,
        result(&inv, &recipe, &second).new_display_name
    );
    cache.regenerate();
    let third = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_ne!(first.values, third.values);
}
#[test]
fn hash_cancel_and_stale_source() {
    let (dir, inv, recipe, _) = fixture(&["Movie.mkv"], hash_recipe(HashAlgorithm::Sha256));
    let control = JobControl::default();
    control.cancel();
    assert!(
        freeze_inputs(&request(&recipe), &inv, &control).is_err(),
        "cancel must fail preparation"
    );
    std::fs::write(dir.path().join("Movie.mkv"), b"changed content").unwrap();
    assert!(
        freeze_inputs(&request(&recipe), &inv, &JobControl::default()).is_err(),
        "stale source must fail preparation"
    );
}
#[test]
fn full_file_hashes_include_bytes_after_first_mebibyte_and_share_one_read() {
    let (dir, _, mut recipe, _) = fixture(&["a.mkv", "b.mkv"], hash_recipe(HashAlgorithm::Sha256));
    let mut bytes = vec![b'x'; 1024 * 1024 + 1];
    std::fs::write(dir.path().join("a.mkv"), &bytes).unwrap();
    bytes[1024 * 1024] = b'y';
    std::fs::write(dir.path().join("b.mkv"), &bytes).unwrap();
    let inv = scan(
        &InventoryRequest::new(vec![RootBinding::new(dir.path().to_owned()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    recipe.selection = inv
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::File)
        .map(|e| e.id)
        .collect();
    recipe.numbering_order = recipe.selection.clone();
    let mut second = recipe.rules[0].clone();
    second.id = "again".into();
    recipe.rules.push(second);
    let mut req = request(&recipe);
    for id in &recipe.numbering_order {
        for algorithm in [
            HashAlgorithm::Crc32,
            HashAlgorithm::Md5,
            HashAlgorithm::Sha1,
            HashAlgorithm::Sha512,
        ] {
            req.fields.push(RequestedInput {
                entry_id: *id,
                rule_id: Some("r".into()),
                key: FieldKey::Hash { algorithm },
            });
        }
    }
    let mut cache = FrozenInputCache::default();
    let inputs = cache.prepare(&req, &inv, &JobControl::default()).unwrap();
    for algorithm in [
        HashAlgorithm::Crc32,
        HashAlgorithm::Md5,
        HashAlgorithm::Sha1,
        HashAlgorithm::Sha256,
        HashAlgorithm::Sha512,
    ] {
        let values = inputs
            .values
            .iter()
            .filter(|v| v.key == FieldKey::Hash { algorithm })
            .map(|v| &v.value)
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 2);
        assert_ne!(values[0], values[1], "{algorithm:?} must include the tail");
    }
    let rows = evaluate(&inv, &recipe, &inputs).unwrap().rows;
    assert_ne!(rows[0].new_display_name, rows[1].new_display_name);
    assert_eq!(cache.content_bytes_read, 2 * (1024 * 1024 + 1));
    cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_eq!(
        cache.content_bytes_read,
        2 * (1024 * 1024 + 1),
        "repeat preview must not read again"
    );
}
#[test]
fn counter_scope_order_and_explicit_override_retention() {
    let (_dir, inv, mut recipe, mut seed) = fixture(
        &["a.mkv", "b.mkv", "unused.mkv"],
        template(vec![field(FieldKey::Counter)]),
    );
    recipe.selection.truncate(2);
    recipe.numbering_order.truncate(2);
    seed.primary_order = recipe.numbering_order.clone();
    seed.values.push(FrozenValue {
        entry_id: recipe.selection[0],
        rule_id: Some("r".into()),
        key: FieldKey::Counter,
        value: FrozenFieldValue::Integer(42),
    });
    let mut cache = FrozenInputCache::from_inputs(seed);
    let first = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_eq!(
        evaluate(&inv, &recipe, &first)
            .unwrap()
            .rows
            .iter()
            .map(|r| r.new_display_name.as_str())
            .collect::<Vec<_>>(),
        ["42.mkv", "2.mkv"]
    );
    recipe.numbering_order.reverse();
    let second = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_eq!(
        evaluate(&inv, &recipe, &second)
            .unwrap()
            .rows
            .iter()
            .map(|r| r.new_display_name.as_str())
            .collect::<Vec<_>>(),
        ["1.mkv", "42.mkv"]
    );
    assert_eq!(
        second.values.len(),
        2,
        "nonprimaries must not consume or acquire counters"
    );
}
#[test]
fn cache_retains_rule_tokens_across_selection_edits_and_drops_them_on_refresh() {
    let (_dir, mut inv, mut recipe, _) =
        fixture(&["a.mkv", "b.mkv"], template(vec![field(FieldKey::Uuid)]));
    let mut cache = FrozenInputCache::default();
    let first = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    recipe.numbering_order.reverse();
    recipe.rules[0].enabled = false;
    let disabled = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert!(disabled.values.is_empty());
    assert_eq!(first.now_ns, disabled.now_ns);
    recipe.rules[0].enabled = true;
    recipe.selection.truncate(1);
    recipe.numbering_order = recipe.selection.clone();
    let selected = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_eq!(selected.values, [first.values[0].clone()]);
    inv.generation += 1;
    let refreshed = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_ne!(selected.values, refreshed.values);
}
#[test]
fn every_canonical_media_property_is_typed_and_missing_is_blocking() {
    for (property, value, expected) in [
        (
            MediaProperty::Width,
            FrozenFieldValue::Integer(3840),
            "3840",
        ),
        (
            MediaProperty::Height,
            FrozenFieldValue::Integer(2160),
            "2160",
        ),
        (
            MediaProperty::DurationNs,
            FrozenFieldValue::Integer(9_007_199_254_740_993),
            "9007199254740993",
        ),
        (
            MediaProperty::FrameRate,
            FrozenFieldValue::Decimal {
                coefficient: 23976,
                scale: 3,
            },
            "23.976",
        ),
        (
            MediaProperty::VideoCodec,
            FrozenFieldValue::Text("hevc".into()),
            "hevc",
        ),
        (
            MediaProperty::AudioCodec,
            FrozenFieldValue::Text("aac".into()),
            "aac",
        ),
        (
            MediaProperty::AudioChannels,
            FrozenFieldValue::Integer(6),
            "6",
        ),
        (
            MediaProperty::BitRate,
            FrozenFieldValue::Integer(80000000),
            "80000000",
        ),
        (
            MediaProperty::Container,
            FrozenFieldValue::Text("matroska".into()),
            "matroska",
        ),
    ] {
        let (_dir, inv, recipe, mut inputs) = fixture(
            &["movie.mkv"],
            template(vec![field(FieldKey::Media { property })]),
        );
        let frozen = freeze_inputs(&request(&recipe), &inv, &JobControl::default()).unwrap();
        assert_eq!(
            result(&inv, &recipe, &frozen).issues[0].code,
            RenameErrorCode::MetadataMissing
        );
        inputs.values.push(FrozenValue {
            entry_id: inputs.primary_order[0],
            rule_id: None,
            key: FieldKey::Media { property },
            value,
        });
        assert_eq!(
            result(&inv, &recipe, &inputs).new_display_name,
            format!("{expected}.mkv")
        );
    }
}
#[test]
fn date_sources_are_distinct_and_unknown_recording_zone_is_not_optional_missing() {
    let (_dir, mut inv, mut recipe, mut inputs) = fixture(&["movie.mkv"], RuleParameters::Keep);
    let entry = inv
        .entries
        .iter_mut()
        .find(|e| e.id == inputs.primary_order[0])
        .unwrap();
    entry.stamp.created_ns = Some(0);
    entry.stamp.modified_ns = Some(86_400_000_000_000);
    entry.stamp.accessed_ns = Some(172_800_000_000_000);
    for (source, expected) in [
        (DateSource::Created, "1970-01-01"),
        (DateSource::Modified, "1970-01-02"),
        (DateSource::Accessed, "1970-01-03"),
        (DateSource::Now, "1970-01-01"),
    ] {
        recipe.rules[0].parameters = template(vec![TemplatePart::Field {
            key: FieldKey::Date { source },
            format: ValueFormat::Date {
                pattern: "%Y-%m-%d".into(),
            },
            fallback: None,
        }]);
        assert_eq!(
            result(&inv, &recipe, &inputs).new_display_name,
            format!("{expected}.mkv")
        );
    }
    recipe.rules[0].parameters =
        template(vec![TemplatePart::Optional(vec![TemplatePart::Field {
            key: FieldKey::Date {
                source: DateSource::Recorded,
            },
            format: ValueFormat::Date {
                pattern: "%Y".into(),
            },
            fallback: None,
        }])]);
    inputs.values.push(FrozenValue {
        entry_id: inputs.primary_order[0],
        rule_id: None,
        key: FieldKey::Date {
            source: DateSource::Recorded,
        },
        value: FrozenFieldValue::LocalTimestamp {
            value: chrono::NaiveDate::from_ymd_opt(2020, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            timezone: None,
        },
    });
    assert_eq!(
        result(&inv, &recipe, &inputs).issues[0].code,
        RenameErrorCode::DateTimezoneUnknown
    );
}
#[test]
fn frozen_missing_decisions_nested_optional_and_segment_boundaries() {
    let (_dir, inv, mut recipe, mut inputs) = fixture(
        &["Movie.MKV"],
        template(vec![
            TemplatePart::Literal("Title".into()),
            TemplatePart::Optional(vec![
                TemplatePart::Literal(" [".into()),
                TemplatePart::Optional(vec![field(FieldKey::Media {
                    property: MediaProperty::Width,
                })]),
                field(FieldKey::Media {
                    property: MediaProperty::Height,
                }),
                TemplatePart::Literal("]".into()),
            ]),
        ]),
    );
    assert_eq!(result(&inv, &recipe, &inputs).new_display_name, "Title.MKV");
    recipe.rules[0].parameters = template(vec![field(FieldKey::Media {
        property: MediaProperty::Height,
    })]);
    inputs.values.push(FrozenValue {
        entry_id: inputs.primary_order[0],
        rule_id: Some("r".into()),
        key: FieldKey::Media {
            property: MediaProperty::Height,
        },
        value: FrozenFieldValue::Missing {
            decision: MissingDecision::Fallback("HD".into()),
        },
    });
    recipe.rules[0].segment = SegmentSelector::Extension;
    assert_eq!(result(&inv, &recipe, &inputs).new_display_name, "Movie.HD");
}
#[test]
fn all_requested_algorithms_share_one_full_read_and_cached_hashes_recheck_source() {
    let (dir, inv, recipe, _) = fixture(&["movie.mkv"], RuleParameters::Keep);
    let mut req = request(&recipe);
    for algorithm in [
        HashAlgorithm::Crc32,
        HashAlgorithm::Md5,
        HashAlgorithm::Sha1,
        HashAlgorithm::Sha256,
        HashAlgorithm::Sha512,
        HashAlgorithm::Blake3,
    ] {
        req.fields.push(RequestedInput {
            entry_id: recipe.selection[0],
            rule_id: Some("r".into()),
            key: FieldKey::Hash { algorithm },
        });
    }
    let mut cache = FrozenInputCache::default();
    let inputs = cache.prepare(&req, &inv, &JobControl::default()).unwrap();
    assert_eq!(inputs.values.len(), 6);
    assert_eq!(cache.content_bytes_read, 5);
    std::fs::remove_file(dir.path().join("movie.mkv")).unwrap();
    std::fs::write(dir.path().join("movie.mkv"), b"media").unwrap();
    assert!(
        cache.prepare(&req, &inv, &JobControl::default()).is_err(),
        "cached hash cannot authorize replacement"
    );
    assert_eq!(cache.content_bytes_read, 5);
}
#[test]
fn different_rules_receive_distinct_tokens_and_cancelled_preparation_keeps_prior_tokens() {
    let (_dir, inv, mut recipe, _) = fixture(&["movie.mkv"], template(vec![field(FieldKey::Uuid)]));
    let mut next = recipe.rules[0].clone();
    next.id = "other".into();
    recipe.rules.push(next);
    let mut cache = FrozenInputCache::default();
    let first = cache
        .prepare(&request(&recipe), &inv, &JobControl::default())
        .unwrap();
    assert_ne!(first.values[0].value, first.values[1].value);
    let cancel = JobControl::default();
    cancel.cancel();
    assert_eq!(
        cache
            .prepare(&request(&recipe), &inv, &cancel)
            .unwrap_err()
            .code,
        RenameErrorCode::Cancelled
    );
    assert_eq!(
        first,
        cache
            .prepare(&request(&recipe), &inv, &JobControl::default())
            .unwrap()
    );
}
#[test]
fn lazy_preparation_rejects_oversized_recipes_before_opening_sources() {
    let (dir, inv, recipe, _) = fixture(
        &["movie.mkv"],
        template(vec![TemplatePart::Literal("x".repeat(1024 * 1024))]),
    );
    std::fs::remove_file(dir.path().join("movie.mkv")).unwrap();
    assert_eq!(
        InputRequirements::from_recipe(&recipe, FrozenTimezone::Utc)
            .unwrap_err()
            .code,
        RenameErrorCode::RecipeTooLarge
    );
    assert_eq!(inv.entries.len(), 1);
}

#[test]
fn cached_hash_revalidation_rejects_stale_selected_anchor_root() {
    let (_dir, mut inventory, recipe, _) =
        fixture(&["movie.mkv"], hash_recipe(HashAlgorithm::Sha256));
    let prior_root_identity = inventory.roots[0].evidence.observed_identity.clone();
    let anchor = inventory.roots[0]
        .path
        .ancestors()
        .last()
        .unwrap()
        .to_owned();
    let mut root = RootBinding::new(anchor.clone()).unwrap();
    root.evidence.observed_identity = OsInventoryFs
        .open_directory(&anchor)
        .unwrap()
        .observation()
        .unwrap()
        .stamp
        .physical_id;
    assert_ne!(root.evidence.observed_identity, prior_root_identity);
    let live_root_identity = root.evidence.observed_identity.clone();
    for entry in &mut inventory.entries {
        entry.root_id = root.id;
        entry.relative_path = encode_path(entry.absolute_path.strip_prefix(&anchor).unwrap());
    }
    inventory.roots = vec![root];
    let requirements = request(&recipe);
    let mut cache = FrozenInputCache::default();
    let first = cache
        .prepare(&requirements, &inventory, &JobControl::default())
        .unwrap();
    assert_eq!(cache.content_bytes_read, 5);
    assert_eq!(
        first,
        cache
            .prepare(&requirements, &inventory, &JobControl::default())
            .unwrap()
    );
    assert_eq!(
        cache.content_bytes_read, 5,
        "valid cached hash needs no further content read"
    );

    inventory.roots[0].evidence.observed_identity = prior_root_identity;
    let error = cache
        .prepare(&requirements, &inventory, &JobControl::default())
        .expect_err("cached descendant hash must not bypass scanned anchor-root identity");
    assert_eq!(error.code, RenameErrorCode::StaleInputs);
    assert_eq!(cache.content_bytes_read, 5);
    inventory.roots[0].evidence.observed_identity = live_root_identity;
    assert_eq!(
        first,
        cache
            .prepare(&requirements, &inventory, &JobControl::default())
            .unwrap()
    );
    assert_eq!(
        cache.content_bytes_read, 5,
        "failed revalidation must preserve the prior cache"
    );
}
