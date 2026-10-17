use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};
use vidlib_inventory::*;
use vidlib_rename::{metadata::*, *};

struct Provider {
    calls: AtomicUsize,
    version: String,
    fields: Mutex<Vec<Vec<PropertyKey>>>,
}
impl Provider {
    fn new(version: &str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            version: version.into(),
            fields: Mutex::new(vec![]),
        }
    }
}
impl MetadataProvider for Provider {
    fn version(&self) -> &str {
        &self.version
    }
    fn probe(
        &self,
        _: &Path,
        fields: &[PropertyKey],
        _: &JobControl,
    ) -> Result<MetadataRecord, MetadataIssue> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.fields.lock().unwrap().push(fields.to_vec());
        Ok(MetadataRecord {
            values: fields
                .iter()
                .map(|key| {
                    (
                        key.clone(),
                        MetadataValue {
                            value: MetadataState::Value(FrozenFieldValue::Integer(64)),
                            source_key: "Video.Width".into(),
                            provider: "fixture".into(),
                            version: self.version.clone(),
                            timezone: TimezoneEvidence::NotApplicable,
                        },
                    )
                })
                .collect(),
        })
    }
}
fn fixture() -> (tempfile::TempDir, Inventory, InputRequirements) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("one.mp4"), b"media").unwrap();
    let inv = scan(
        &InventoryRequest::new(vec![RootBinding::new(d.path().to_owned()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let id = inv
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::File)
        .unwrap()
        .id;
    let req = InputRequirements {
        primary_order: vec![id],
        numbering_scope: Default::default(),
        timezone: FrozenTimezone::Utc,
        fields: vec![RequestedInput {
            entry_id: id,
            rule_id: Some("r".into()),
            key: FieldKey::Media {
                property: MediaProperty::Width,
            },
        }],
    };
    (d, inv, req)
}
#[test]
fn no_probe_for_text_recipe() {
    let (_d, inv, mut req) = fixture();
    req.fields[0].key = FieldKey::OriginalStem;
    let p = Provider::new("1");
    let output = FrozenInputCache::default()
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &p)
        .unwrap();
    assert!(output.values.is_empty());
    assert_eq!(p.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn requested_fields_only_and_cache_invalidates() {
    let (d, mut inv, mut req) = fixture();
    let p = Provider::new("1");
    let mut cache = FrozenInputCache::default();
    // A pre-existing Missing placeholder must not prevent real metadata production.
    cache.prepare(&req, &inv, &JobControl::default()).unwrap();
    let output = cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &p)
        .unwrap();
    assert_eq!(
        output.values[0].value.resolved(),
        FrozenFieldValue::Integer(64)
    );
    cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &p)
        .unwrap();
    assert_eq!(p.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        p.fields.lock().unwrap()[0],
        vec![PropertyKey::Media(MediaProperty::Width)]
    );
    req.fields.push(RequestedInput {
        key: FieldKey::Media {
            property: MediaProperty::Height,
        },
        ..req.fields[0].clone()
    });
    cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &p)
        .unwrap();
    assert_eq!(
        p.calls.load(Ordering::SeqCst),
        2,
        "requested set is part of the key"
    );
    let p2 = Provider::new("2");
    cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &p2)
        .unwrap();
    assert_eq!(
        p2.calls.load(Ordering::SeqCst),
        1,
        "provider upgrade invalidates"
    );
    std::fs::write(d.path().join("one.mp4"), b"changed media").unwrap();
    assert!(
        cache
            .prepare_with_metadata(&req, &inv, &JobControl::default(), &p2)
            .is_err(),
        "stale inventory cannot authorize a probe or cached result"
    );
    let fresh = scan(
        &InventoryRequest::new(vec![RootBinding::new(d.path().to_owned()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    inv.entries[0].stamp = fresh.entries[0].stamp.clone();
    cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &p2)
        .unwrap();
    assert_eq!(
        p2.calls.load(Ordering::SeqCst),
        2,
        "material stamp invalidates"
    );
}

#[test]
fn title_language_and_canonical_windows_fields_are_typed_recipe_inputs() {
    for (json, format) in [
        (r#"{"type":"media","property":"title"}"#, ValueFormat::Text),
        (
            r#"{"type":"media","property":"language"}"#,
            ValueFormat::Text,
        ),
        (
            r#"{"type":"windows_property","property":"System.Author"}"#,
            ValueFormat::Text,
        ),
        (
            r#"{"type":"windows_property","property":"System.Title"}"#,
            ValueFormat::Text,
        ),
        (
            r#"{"type":"windows_property","property":"System.Media.DateEncoded"}"#,
            ValueFormat::Date {
                pattern: "%Y-%m-%d".into(),
            },
        ),
        (
            r#"{"type":"windows_property","property":"System.Photo.DateTaken"}"#,
            ValueFormat::Date {
                pattern: "%Y-%m-%d".into(),
            },
        ),
    ] {
        let key: FieldKey = serde_json::from_str(json).expect("approved typed metadata field");
        assert!(PropertyKey::from_field(&key).is_some());
        let (_d, inv, _) = fixture();
        let id = inv.entries[0].id;
        let recipe = RecipeV1 {
            schema_version: 1,
            revision: 1,
            selection: vec![id],
            selection_filter: None,
            numbering_order: vec![id],
            numbering_scope: Default::default(),
            rules: vec![Rule {
                id: "r".into(),
                enabled: true,
                segment: SegmentSelector::Stem,
                parameters: RuleParameters::Template {
                    parts: vec![TemplatePart::Field {
                        key: key.clone(),
                        format,
                        fallback: None,
                    }],
                },
            }],
            companions: CompanionPolicy::None,
            destination: Destination::SameDirectory,
            collision_policy: CollisionPolicy::Block,
            metadata_changes: vec![],
        };
        let requirements = InputRequirements::from_recipe(&recipe, FrozenTimezone::Utc).unwrap();
        assert_eq!(requirements.fields[0].key, key);
    }
    assert!(serde_json::from_str::<FieldKey>(
        r#"{"type":"windows_property","property":"System.Arbitrary"}"#
    )
    .is_err());
}

#[test]
fn absent_identity_and_hard_link_aliases_never_merge_exact_paths() {
    let (d, inv, req) = fixture();
    let base = MetadataCacheKey {
        physical_id: None,
        path: d.path().join("one.mp4"),
        size: Some(5),
        modified_ns: Some(1),
        provider_version: "1".into(),
        fields: vec![PropertyKey::Media(MediaProperty::Width)],
    };
    let mut other = base.clone();
    other.path = d.path().join("two.mp4");
    assert_ne!(base, other);
    std::fs::hard_link(&base.path, &other.path).unwrap();
    let fresh = scan(
        &InventoryRequest::new(vec![RootBinding::new(d.path().to_owned()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let mut req = req;
    req.primary_order = fresh.entries.iter().map(|e| e.id).collect();
    req.fields = req
        .primary_order
        .iter()
        .map(|id| RequestedInput {
            entry_id: *id,
            ..req.fields[0].clone()
        })
        .collect();
    let provider = Provider::new("1");
    FrozenInputCache::default()
        .prepare_with_metadata(&req, &fresh, &JobControl::default(), &provider)
        .unwrap();
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        2,
        "aliases remain separate exact paths"
    );
    let mut absent = inv.clone();
    absent.entries[0].stamp.physical_id = None;
    req.primary_order = vec![absent.entries[0].id];
    req.fields = vec![RequestedInput {
        entry_id: absent.entries[0].id,
        ..req.fields[0].clone()
    }];
    assert!(
        FrozenInputCache::default()
            .prepare_with_metadata(&req, &absent, &JobControl::default(), &provider)
            .is_err(),
        "missing identity cannot bypass fresh native verification"
    );
}
#[test]
fn metadata_refresh_preserves_generated_values_and_explicit_missing_decisions() {
    let (_d, inv, mut req) = fixture();
    let p = Provider::new("1");
    req.fields.push(RequestedInput {
        key: FieldKey::Uuid,
        ..req.fields[0].clone()
    });
    req.fields.push(RequestedInput {
        key: FieldKey::Random,
        ..req.fields[0].clone()
    });
    let mut cache = FrozenInputCache::default();
    let first = cache.prepare(&req, &inv, &JobControl::default()).unwrap();
    let produced = cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &p)
        .unwrap();
    assert_eq!(produced.now_ns, first.now_ns);
    assert_eq!(&produced.values[1..], &first.values[1..]);
    for decision in [
        MissingDecision::Fallback("chosen".into()),
        MissingDecision::OmitOptional,
    ] {
        let mut inputs = first.clone();
        inputs.values[0].value = FrozenFieldValue::Missing {
            decision: decision.clone(),
        };
        let output = FrozenInputCache::from_inputs(inputs)
            .prepare_with_metadata(&req, &inv, &JobControl::default(), &p)
            .unwrap();
        assert_eq!(
            output.values[0].value,
            FrozenFieldValue::Missing { decision }
        );
    }
}
#[test]
fn cancelled_probe_preserves_previous_successful_cache() {
    struct Cancel;
    impl MetadataProvider for Cancel {
        fn version(&self) -> &str {
            "2"
        }
        fn probe(
            &self,
            _: &Path,
            _: &[PropertyKey],
            control: &JobControl,
        ) -> Result<MetadataRecord, MetadataIssue> {
            control.cancel();
            Err(MetadataIssue {
                code: MetadataIssueCode::Cancelled,
                message: "cancelled".into(),
            })
        }
    }
    let (_d, inv, req) = fixture();
    let p = Provider::new("1");
    let mut cache = FrozenInputCache::default();
    let first = cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &p)
        .unwrap();
    assert_eq!(
        cache
            .prepare_with_metadata(&req, &inv, &JobControl::default(), &Cancel)
            .unwrap_err()
            .code,
        RenameErrorCode::Cancelled
    );
    let restored = cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &p)
        .unwrap();
    assert_eq!(restored, first);
    assert_eq!(p.calls.load(Ordering::SeqCst), 1);
}

fn template_recipe(inv: &Inventory, parts: Vec<TemplatePart>) -> RecipeV1 {
    let id = inv.entries[0].id;
    RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: vec![id],
        selection_filter: None,
        numbering_order: vec![id],
        numbering_scope: Default::default(),
        rules: vec![Rule {
            id: "r".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::Template { parts },
        }],
        companions: CompanionPolicy::None,
        destination: Destination::SameDirectory,
        collision_policy: CollisionPolicy::Block,
        metadata_changes: vec![],
    }
}
#[test]
fn helper_failure_is_clear_and_explicit_fallbacks_still_work() {
    let (_d, inv, req) = fixture();
    let mut input = FrozenInputCache::default()
        .prepare(&req, &inv, &JobControl::default())
        .unwrap();
    input.values[0].value = FrozenFieldValue::Metadata(Box::new(MetadataValue {
        value: MetadataState::Error(MetadataIssue {
            code: MetadataIssueCode::Timeout,
            message: "metadata probe exceeded its deadline".into(),
        }),
        source_key: "Video.Width".into(),
        provider: "MediaInfo".into(),
        version: "25.10".into(),
        timezone: TimezoneEvidence::NotApplicable,
    }));
    let field = TemplatePart::Field {
        key: req.fields[0].key.clone(),
        format: ValueFormat::Text,
        fallback: None,
    };
    let recipe = template_recipe(&inv, vec![field.clone()]);
    let result = evaluate(&inv, &recipe, &input).unwrap();
    assert!(
        result.rows[0]
            .issues
            .iter()
            .any(|i| i.message.contains("MediaInfo") && i.message.contains("Timeout")),
        "missing-field issue must retain provider failure: {:?}",
        result.rows[0].issues
    );
    let recipe = template_recipe(
        &inv,
        vec![
            TemplatePart::Literal("optional".into()),
            TemplatePart::Optional(vec![field]),
        ],
    );
    assert_eq!(
        evaluate(&inv, &recipe, &input).unwrap().rows[0].new_display_name,
        "optional.mp4"
    );
    let recipe = template_recipe(
        &inv,
        vec![TemplatePart::Field {
            key: req.fields[0].key.clone(),
            format: ValueFormat::Text,
            fallback: Some("fallback".into()),
        }],
    );
    assert_eq!(
        evaluate(&inv, &recipe, &input).unwrap().rows[0].new_display_name,
        "fallback.mp4"
    );
}
#[test]
fn metadata_wrapping_cannot_hide_invalid_timezone_or_nested_values() {
    let (_d, inv, mut req) = fixture();
    req.fields[0].key = FieldKey::Date {
        source: DateSource::Recorded,
    };
    let mut input = FrozenInputCache::default()
        .prepare(&req, &inv, &JobControl::default())
        .unwrap();
    let value = MetadataValue {
        value: MetadataState::Value(FrozenFieldValue::Timestamp {
            ns: 0,
            timezone: Some(FrozenTimezone::Iana {
                name: "Not/AZone".into(),
            }),
        }),
        source_key: "Recorded_Date".into(),
        provider: "fixture".into(),
        version: "1".into(),
        timezone: TimezoneEvidence::Unknown,
    };
    input.values[0].value = FrozenFieldValue::Metadata(Box::new(value.clone()));
    let recipe = template_recipe(
        &inv,
        vec![TemplatePart::Field {
            key: req.fields[0].key.clone(),
            format: ValueFormat::Date {
                pattern: "%Y".into(),
            },
            fallback: None,
        }],
    );
    assert_eq!(
        evaluate(&inv, &recipe, &input).unwrap_err().code,
        RenameErrorCode::InvalidFrozenInputs
    );
    input.values[0].value = FrozenFieldValue::Metadata(Box::new(MetadataValue {
        value: MetadataState::Value(input.values[0].value.clone()),
        ..value
    }));
    assert_eq!(
        evaluate(&inv, &recipe, &input).unwrap_err().code,
        RenameErrorCode::InvalidFrozenInputs
    );
}

#[test]
fn transient_helper_failure_is_retried_without_regenerating_uuid() {
    struct Failing;
    impl MetadataProvider for Failing {
        fn version(&self) -> &str {
            "1"
        }
        fn probe(
            &self,
            _: &Path,
            _: &[PropertyKey],
            _: &JobControl,
        ) -> Result<MetadataRecord, MetadataIssue> {
            Err(MetadataIssue {
                code: MetadataIssueCode::Unavailable,
                message: "helper missing".into(),
            })
        }
    }
    let (_d, inv, mut req) = fixture();
    req.fields.push(RequestedInput {
        key: FieldKey::Uuid,
        ..req.fields[0].clone()
    });
    let mut cache = FrozenInputCache::default();
    let failed = cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &Failing)
        .unwrap();
    let good = cache
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &Provider::new("1"))
        .unwrap();
    assert_eq!(
        good.values[0].value.resolved(),
        FrozenFieldValue::Integer(64)
    );
    assert_eq!(good.values[1], failed.values[1]);
}
