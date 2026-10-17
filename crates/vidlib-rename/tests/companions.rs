use std::path::{Path, PathBuf};
use vidlib_inventory::{ids::*, *};
use vidlib_rename::*;

fn root() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\media")
    } else {
        PathBuf::from("/media")
    }
}
fn fixture(names: &[&str]) -> Inventory {
    let binding = RootBinding::new(root()).unwrap();
    let entries = names
        .iter()
        .map(|name| EntrySnapshot {
            id: EntryId::new(),
            root_id: binding.id,
            absolute_path: root().join(name),
            relative_path: encode_path(Path::new(name)),
            display_name: Path::new(name)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .into(),
            display_path: "not authority".into(),
            kind: EntryKind::File,
            stamp: FileStamp {
                physical_id: Some(PhysicalId {
                    platform: encode_path(Path::new(name)).platform,
                    volume_identity: vec![1],
                    file_identity: name.as_bytes().to_vec(),
                }),
                size: Some(100),
                created_ns: Some(10),
                modified_ns: Some(20),
                accessed_ns: Some(30),
                windows_attributes: Some(2),
            },
            observed_ns: 0,
            issues: vec![],
        })
        .collect();
    Inventory {
        id: InventoryId::new(),
        generation: 1,
        roots: vec![binding],
        entries,
        coverage: vec![],
        window: Default::default(),
        issues: vec![],
    }
}
fn grants(i: &Inventory) -> GrantSet {
    GrantSet {
        grants: i.roots.iter().map(|r| r.source_grant.clone()).collect(),
    }
}
fn recipe(i: &Inventory) -> RecipeV1 {
    let ids = i.entries.iter().map(|e| e.id).collect::<Vec<_>>();
    RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: ids.clone(),
        selection_filter: None,
        numbering_order: ids,
        numbering_scope: Default::default(),
        rules: vec![],
        companions: CompanionPolicy::None,
        destination: Destination::SameDirectory,
        collision_policy: CollisionPolicy::Block,
        metadata_changes: vec![],
    }
}
fn frozen(i: &Inventory, r: &RecipeV1) -> FrozenInputs {
    FrozenInputs {
        inventory_id: i.id,
        generation: i.generation,
        now_ns: 0,
        timezone: FrozenTimezone::Utc,
        primary_order: r.numbering_order.clone(),
        numbering_scope: r.numbering_scope,
        values: vec![],
    }
}
fn namespace(i: &Inventory) -> NamespaceSnapshot {
    let mut result = NamespaceSnapshot {
        grants: grants(i),
        ..Default::default()
    };
    for entry in &i.entries {
        let parent = entry.absolute_path.parent().unwrap();
        if !result.parents.iter().any(|p| p.parent_path == parent) {
            result.parents.push(DirectoryFacts {
                parent_path: parent.into(),
                source_grants: vec![i.roots[0].source_grant.id],
                stamp: FileStamp {
                    physical_id: Some(PhysicalId {
                        platform: encode_path(parent).platform,
                        volume_identity: vec![1],
                        file_identity: vec![0],
                    }),
                    ..Default::default()
                },
                case_sensitivity: CaseSensitivity::Sensitive,
                normalization: NameNormalization::Preserved,
                max_component_units: Some(255),
                name_policy: if cfg!(windows) {
                    NamePolicy::Windows
                } else {
                    NamePolicy::Posix
                },
                trailing_dots_spaces_allowed: Some(!cfg!(windows)),
                capabilities: FilesystemCapabilities {
                    read_only: Some(false),
                    physical_identity: Support::Supported,
                    atomic_rename: Support::Supported,
                    ..Default::default()
                },
                children_complete: true,
                ..Default::default()
            });
        }
        result
            .parents
            .iter_mut()
            .find(|p| p.parent_path == parent)
            .unwrap()
            .entries
            .push(NamespaceEntry {
                name: encode_path(Path::new(entry.absolute_path.file_name().unwrap())),
                absolute_path: entry.absolute_path.clone(),
                kind: entry.kind,
                stamp: entry.stamp.clone(),
                protected: false,
            });
    }
    result
}
fn version(revision: u64) -> PlanRef {
    PlanRef {
        session_id: SessionId::new(),
        plan_id: PlanId::new(),
        revision,
    }
}

fn matching() -> CompanionPolicy {
    serde_json::from_str(r#"{"type":"match_exact","associations":[]}"#).unwrap()
}
fn fixed(r: &mut RecipeV1) {
    r.rules.push(Rule {
        id: "name".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::FixedStem {
            value: "Harbor Watch - S02E02 [1080p]".into(),
        },
    });
}
fn prepared(i: &Inventory, mut r: RecipeV1) -> EvaluatedBatch {
    prepare_associations(i, &mut r).unwrap();
    evaluate(i, &r, &frozen(i, &r)).unwrap()
}
#[test]
fn language_and_role_suffixes_preserved() {
    let i = fixture(&["old.mkv", "old.en.forced.srt", "old.PT.sdh.ASS"]);
    let mut r = recipe(&i);
    r.companions = matching();
    fixed(&mut r);
    let b = prepared(&i, r);
    assert_eq!(
        b.rows[1].new_display_name,
        "Harbor Watch - S02E02 [1080p].en.forced.srt"
    );
    assert_eq!(
        b.rows[2].new_display_name,
        "Harbor Watch - S02E02 [1080p].PT.sdh.ASS"
    );
    assert_eq!(b.frozen_inputs.primary_order, vec![i.entries[0].id]);
}
#[test]
fn idx_sub_pair_is_one_dependency_group() {
    let i = fixture(&["old.mkv", "old.en.idx", "old.en.sub"]);
    let mut r = recipe(&i);
    r.companions = matching();
    fixed(&mut r);
    let b = prepared(&i, r);
    let p = plan(&i, &b, &namespace(&i), version(1)).unwrap();
    assert_eq!(p.counts.blocked, 0, "{:?}", p.issues);
    assert_eq!(p.rows[1].group_id, p.rows[2].group_id);
    assert_eq!(p.execution.groups.len(), 1);
    assert_eq!(p.execution.groups[0].steps.len(), 3);
}
#[test]
fn two_possible_owners_require_choice() {
    let i = fixture(&["old.mkv", "old.mp4", "old.en.srt"]);
    let a = associate(&i, &recipe(&i).selection, &matching()).unwrap();
    assert_eq!(
        a.assets
            .iter()
            .find(|a| a.asset_id == i.entries[2].id)
            .unwrap()
            .owner,
        None
    );
    assert!(!a.warnings.is_empty());
    let mut r = recipe(&i);
    r.companions = matching();
    fixed(&mut r);
    let b = prepared(&i, r);
    assert_eq!(b.rows[2].new_display_name, "old.en.srt");
    assert_eq!(b.rows[2].status, RowStatus::NeedsReview);
}
#[test]
fn folder_artwork_not_assigned_automatically() {
    let i = fixture(&["poster.mkv", "poster.jpg", "tvshow.nfo", "fanart.png"]);
    let mut r = recipe(&i);
    r.companions = matching();
    fixed(&mut r);
    let b = prepared(&i, r);
    for n in 1..4 {
        assert_eq!(b.rows[n].new_display_name, i.entries[n].display_name);
        assert_eq!(b.rows[n].status, RowStatus::NeedsReview);
    }
}
#[test]
fn cross_folder_requires_explicit_pair() {
    let i = fixture(&["one/old.mkv", "two/old.en.srt"]);
    let mut r = recipe(&i);
    r.companions = matching();
    fixed(&mut r);
    let b = prepared(&i, r.clone());
    assert_eq!(b.rows[1].new_display_name, "old.en.srt");
    r.companions = CompanionPolicy::Explicit {
        associations: vec![CompanionAssociation {
            primary: i.entries[0].id,
            companions: vec![i.entries[1].id],
        }],
    };
    let b = prepared(&i, r);
    assert_eq!(
        b.rows[1].new_display_name,
        "Harbor Watch - S02E02 [1080p].en.srt"
    );
    assert_eq!(
        b.rows[1].target_path.parent(),
        i.entries[1].absolute_path.parent()
    );
}
#[test]
fn primary_only_warns_without_mutating_asset() {
    let i = fixture(&["old.mkv", "old.en.srt", "poster.jpg"]);
    let mut r = recipe(&i);
    r.selection.truncate(1);
    r.numbering_order.truncate(1);
    fixed(&mut r);
    let b = prepared(&i, r);
    assert_eq!(b.rows.len(), 1);
    assert!(b.rows[0]
        .explanations
        .iter()
        .any(|e| e.rule_id == "companion_warning"));
    assert_eq!(b.recipe.selection, vec![i.entries[0].id]);
}
#[test]
fn primary_override_propagates_and_companion_override_requires_unbinding() {
    let i = fixture(&["old.mkv", "old.en.srt"]);
    let mut r = recipe(&i);
    r.companions = matching();
    fixed(&mut r);
    prepare_associations(&i, &mut r).unwrap();
    let mappings = MappingSet {
        inventory_id: i.id,
        generation: i.generation,
        rows: vec![],
    };
    let b = evaluate_with_mappings(
        &i,
        &r,
        &frozen(&i, &r),
        &mappings,
        &[RowOverride {
            entry_id: i.entries[0].id,
            new_name: "Override.mp4".into(),
        }],
    )
    .unwrap();
    assert_eq!(b.rows[1].new_display_name, "Override.en.srt");
    assert!(evaluate_with_mappings(
        &i,
        &r,
        &frozen(&i, &r),
        &mappings,
        &[RowOverride {
            entry_id: i.entries[1].id,
            new_name: "Wrong.srt".into()
        }]
    )
    .is_err());
}
#[test]
fn collision_suffix_keeps_one_primary_stem_for_entire_group() {
    let i = fixture(&["old.mkv", "old.en.srt", "New.en.srt"]);
    let mut r = recipe(&i);
    r.selection.truncate(2);
    r.numbering_order.truncate(2);
    r.companions = matching();
    r.rules.push(Rule {
        id: "fixed".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::FixedStem {
            value: "New".into(),
        },
    });
    r.collision_policy = CollisionPolicy::Suffix {
        separator: "-".into(),
        max_attempts: 5,
    };
    let b = prepared(&i, r);
    let p = plan(&i, &b, &namespace(&i), version(1)).unwrap();
    assert_eq!(p.counts.blocked, 0, "{:?}", p.issues);
    assert_eq!(p.rows[0].new_display_name, "New-1.mkv");
    assert_eq!(p.rows[1].new_display_name, "New-1.en.srt");
    assert_eq!(p.execution.groups.len(), 1);
}
#[test]
fn graph_rejects_forged_companion_lineage() {
    let i = fixture(&["old.mkv", "old.en.srt"]);
    let mut r = recipe(&i);
    r.companions = matching();
    fixed(&mut r);
    let mut b = prepared(&i, r);
    b.rows[1].primary_id = b.rows[1].entry_id;
    b.rows[1].group_id = GroupId::new();
    assert!(plan(&i, &b, &namespace(&i), version(1)).is_err());
}
#[test]
fn selected_none_assets_are_independent_and_exact_matching_does_not_guess() {
    let i = fixture(&[
        "old.mkv",
        "old.en.srt",
        "old.english.srt",
        "old.zz.srt",
        "OLD.en.srt",
        "old.en.txt",
    ]);
    let mut r = recipe(&i);
    fixed(&mut r);
    let b = prepared(&i, r.clone());
    assert_eq!(b.frozen_inputs.primary_order.len(), 6);
    assert_eq!(
        b.rows[1].new_display_name,
        "Harbor Watch - S02E02 [1080p].srt"
    );
    r.companions = matching();
    let b = prepared(&i, r);
    assert_eq!(b.frozen_inputs.primary_order.len(), 5);
    for n in 2..6 {
        assert_eq!(b.rows[n].primary_id, b.rows[n].entry_id);
    }
}
#[test]
fn invalid_manual_authority_and_multiple_owners_fail_closed() {
    let i = fixture(&[
        "one.mkv",
        "two.mp4",
        "one.en.srt",
        "one.en.idx",
        "one.en.sub",
    ]);
    let ids = recipe(&i).selection;
    let bind = |primary, companions| CompanionAssociation {
        primary,
        companions,
    };
    let policy = |associations| CompanionPolicy::Explicit { associations };
    assert!(associate(&i, &ids[..2], &policy(vec![bind(ids[0], vec![ids[2]])])).is_err());
    assert!(associate(&i, &ids, &policy(vec![bind(ids[0], vec![EntryId::new()])])).is_err());
    assert!(associate(
        &i,
        &ids,
        &policy(vec![bind(ids[0], vec![ids[2]]), bind(ids[1], vec![ids[2]])])
    )
    .is_err());
    assert!(associate(
        &i,
        &ids,
        &policy(vec![bind(ids[0], vec![ids[3]]), bind(ids[1], vec![ids[4]])])
    )
    .is_err());
    let mut forged = i.clone();
    forged.entries[2].absolute_path = root().parent().unwrap().join("outside.srt");
    assert_eq!(
        associate(&forged, &ids, &policy(vec![bind(ids[0], vec![ids[2]])]))
            .unwrap_err()
            .code,
        RenameErrorCode::OutsideGrant
    );
}
#[test]
fn companions_consume_no_number_or_template_counter_in_either_scope() {
    let mut i = fixture(&["directory", "a.mkv", "a.en.srt", "b.mkv", "b.en.srt"]);
    i.entries[0].kind = EntryKind::Directory;
    for scope in [NumberingScope::Files, NumberingScope::Directories] {
        let mut r = recipe(&i);
        r.numbering_scope = scope;
        r.companions = matching();
        r.rules.push(Rule {
            id: "counter".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::Template {
                parts: vec![TemplatePart::Field {
                    key: FieldKey::Counter,
                    format: ValueFormat::Text,
                    fallback: Some("outside".into()),
                }],
            },
        });
        r.rules.push(Rule {
            id: "number".into(),
            enabled: true,
            segment: SegmentSelector::Stem,
            parameters: RuleParameters::Number {
                rule: NumberRule {
                    start: 10,
                    step: 10,
                    repeat_each: 1,
                    padding: 0,
                    radix: Radix::Decimal,
                    reset_by_folder: false,
                    at: InsertPosition::End,
                    prefix: "-".into(),
                    suffix: "".into(),
                },
            },
        });
        prepare_associations(&i, &mut r).unwrap();
        let req = InputRequirements::from_recipe(&r, FrozenTimezone::Utc).unwrap();
        assert_eq!(
            req.primary_order,
            vec![i.entries[0].id, i.entries[1].id, i.entries[3].id]
        );
        let inputs = FrozenInputCache::default()
            .prepare(&req, &i, &JobControl::default())
            .unwrap();
        let b = evaluate(&i, &r, &inputs).unwrap();
        if scope == NumberingScope::Files {
            assert_eq!(b.rows[0].new_display_name, "outside");
            assert_eq!(b.rows[1].new_display_name, "1-10.mkv");
            assert_eq!(b.rows[2].new_display_name, "1-10.en.srt");
            assert_eq!(b.rows[3].new_display_name, "2-20.mkv");
            assert_eq!(b.rows[4].new_display_name, "2-20.en.srt");
        } else {
            assert_eq!(b.rows[0].new_display_name, "1-10");
            assert_eq!(b.rows[1].new_display_name, "outside.mkv");
            assert_eq!(b.rows[2].new_display_name, "outside.en.srt");
        }
    }
}
#[test]
fn all_supported_assets_retain_suffix_spelling_and_explicit_choice_resolves_ambiguity() {
    let names = [
        "old.mkv",
        "old.mp4",
        "old.ENG.CC.srt",
        "old.en.ass",
        "old.ssa",
        "old.vtt",
        "old.sup",
        "old.idx",
        "old.sub",
        "old.nfo",
        "old.poster.jpg",
        "old.fanart.jpeg",
        "old.thumb.png",
        "old.webp",
    ];
    let i = fixture(&names);
    let mut r = recipe(&i);
    r.companions = CompanionPolicy::MatchExact {
        associations: vec![CompanionAssociation {
            primary: i.entries[0].id,
            companions: i.entries[2..].iter().map(|e| e.id).collect(),
        }],
    };
    fixed(&mut r);
    let b = prepared(&i, r);
    for (n, name) in names.iter().enumerate().skip(2) {
        let row = b
            .rows
            .iter()
            .find(|r| r.entry_id == i.entries[n].id)
            .unwrap();
        assert_eq!(
            row.new_display_name,
            format!(
                "Harbor Watch - S02E02 [1080p]{}",
                name.strip_prefix("old").unwrap()
            )
        );
        assert_eq!(row.primary_id, i.entries[0].id);
    }
}
#[test]
fn explicit_vobsub_binding_cannot_leave_selected_peer_independent() {
    let i = fixture(&["old.mkv", "old.en.idx", "old.en.sub"]);
    let r = recipe(&i);
    let policy = CompanionPolicy::Explicit {
        associations: vec![CompanionAssociation {
            primary: i.entries[0].id,
            companions: vec![i.entries[1].id],
        }],
    };
    assert!(associate(&i, &r.selection, &policy).is_err());
}
#[test]
fn graph_rejects_independently_forged_companion_target() {
    let i = fixture(&["old.mkv", "old.en.srt"]);
    let mut r = recipe(&i);
    r.companions = matching();
    fixed(&mut r);
    let mut b = prepared(&i, r);
    b.rows[1].target_path.set_file_name("unrelated.srt");
    assert!(plan(&i, &b, &namespace(&i), version(1)).is_err());
}
#[test]
fn manual_primary_override_cannot_erase_warning_or_unresolved_asset_review() {
    let i = fixture(&["old.mkv", "old.en.srt"]);
    let mut r = recipe(&i);
    r.selection.truncate(1);
    r.numbering_order.truncate(1);
    prepare_associations(&i, &mut r).unwrap();
    let m = MappingSet {
        inventory_id: i.id,
        generation: i.generation,
        rows: vec![],
    };
    let b = evaluate_with_mappings(
        &i,
        &r,
        &frozen(&i, &r),
        &m,
        &[RowOverride {
            entry_id: i.entries[0].id,
            new_name: "new.mkv".into(),
        }],
    )
    .unwrap();
    assert!(b.rows[0]
        .explanations
        .iter()
        .any(|e| e.rule_id == "companion_warning"));
}
#[test]
fn overriding_blocked_primary_repairs_its_companions_too() {
    let i = fixture(&["old.mkv", "old.en.srt"]);
    let mut r = recipe(&i);
    r.companions = matching();
    r.rules.push(Rule {
        id: "missing".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::Template {
            parts: vec![TemplatePart::Field {
                key: FieldKey::Media {
                    property: MediaProperty::Title,
                },
                format: ValueFormat::Text,
                fallback: None,
            }],
        },
    });
    prepare_associations(&i, &mut r).unwrap();
    let b = evaluate_with_mappings(
        &i,
        &r,
        &frozen(&i, &r),
        &MappingSet {
            inventory_id: i.id,
            generation: i.generation,
            rows: vec![],
        },
        &[RowOverride {
            entry_id: i.entries[0].id,
            new_name: "fixed.mkv".into(),
        }],
    )
    .unwrap();
    let p = plan(&i, &b, &namespace(&i), version(1)).unwrap();
    assert!(p.issues.is_empty(), "{:?}", p.issues);
    assert_eq!(p.rows[1].new_display_name, "fixed.en.srt");
}
#[test]
fn collision_suffix_never_renames_an_unresolved_shared_asset() {
    let i = fixture(&["old.mkv", "poster.jpg"]);
    let mut r = recipe(&i);
    r.companions = matching();
    r.rules.push(Rule {
        id: "name".into(),
        enabled: true,
        segment: SegmentSelector::WholeName,
        parameters: RuleParameters::Text {
            operation: TextOperation::Fixed {
                value: "poster.jpg".into(),
            },
        },
    });
    r.collision_policy = CollisionPolicy::Suffix {
        separator: "-".into(),
        max_attempts: 4,
    };
    let b = prepared(&i, r);
    let p = plan(&i, &b, &namespace(&i), version(1)).unwrap();
    assert_eq!(p.rows[1].new_display_name, "poster.jpg");
    assert_eq!(p.rows[1].status, RowStatus::NeedsReview);
    assert!(p.execution.groups.is_empty());
}

#[test]
fn companion_collision_beyond_first_page_blocks_complete_selection() {
    let mut names = (0..101)
        .flat_map(|n| [format!("v{n:03}.mkv"), format!("v{n:03}.en.srt")])
        .collect::<Vec<_>>();
    names.push("new-v100.en.srt".into());
    let i = fixture(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let mut r = recipe(&i);
    r.selection.pop();
    r.numbering_order.pop();
    r.companions = matching();
    r.rules.push(Rule {
        id: "name".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::Text {
            operation: TextOperation::Prefix {
                text: "new-".into(),
            },
        },
    });
    let b = prepared(&i, r);
    let p = plan(&i, &b, &namespace(&i), version(1)).unwrap();
    assert_eq!(p.rows.len(), 202);
    assert_eq!(p.rows[201].status, RowStatus::Blocked);
    assert!(p.execution.groups.is_empty());
    assert_eq!(p.rows[201].new_display_name, "new-v100.en.srt");
}

#[test]
fn explicit_language_role_only_asset_retains_all_suffix_tokens() {
    let i = fixture(&["movie.mkv", "subtitles/EN.forced.srt", "art/poster.jpg"]);
    let mut r = recipe(&i);
    fixed(&mut r);
    r.companions = CompanionPolicy::Explicit {
        associations: vec![CompanionAssociation {
            primary: i.entries[0].id,
            companions: vec![i.entries[1].id, i.entries[2].id],
        }],
    };
    let b = prepared(&i, r);
    assert_eq!(
        b.rows[1].new_display_name,
        "Harbor Watch - S02E02 [1080p].EN.forced.srt"
    );
    assert_eq!(
        b.rows[2].new_display_name,
        "Harbor Watch - S02E02 [1080p].poster.jpg"
    );
}
