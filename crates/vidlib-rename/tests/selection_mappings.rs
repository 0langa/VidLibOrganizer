use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};
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
fn pattern(field: SelectionField, syntax: PatternSyntax, value: &str) -> SelectionPattern {
    SelectionPattern {
        field,
        syntax,
        value: value.into(),
        case_sensitive: true,
    }
}
#[test]
fn every_filter_dimension() {
    let mut i = fixture(&["film.mkv", "deep/film.MKV", "ignore.txt", "folder"]);
    i.entries[3].kind = EntryKind::Directory;
    let want = i.entries[0].id;
    let filters = vec![
        SelectionFilter {
            recursive: false,
            ..Default::default()
        },
        SelectionFilter {
            max_depth: Some(0),
            ..Default::default()
        },
        SelectionFilter {
            kinds: vec![EntryKind::File],
            ..Default::default()
        },
        SelectionFilter {
            extensions: vec!["mkv".into()],
            ..Default::default()
        },
        SelectionFilter {
            name_length: Some(U64Range {
                min: Some(8),
                max: Some(8),
            }),
            ..Default::default()
        },
        SelectionFilter {
            path_length: Some(U64Range {
                min: Some(8),
                max: Some(8),
            }),
            ..Default::default()
        },
        SelectionFilter {
            size: Some(U64Range {
                min: Some(100),
                max: Some(100),
            }),
            ..Default::default()
        },
        SelectionFilter {
            created_ns: Some(I128Range {
                min: Some(10),
                max: Some(10),
            }),
            ..Default::default()
        },
        SelectionFilter {
            modified_ns: Some(I128Range {
                min: Some(20),
                max: Some(20),
            }),
            ..Default::default()
        },
        SelectionFilter {
            accessed_ns: Some(I128Range {
                min: Some(30),
                max: Some(30),
            }),
            ..Default::default()
        },
        SelectionFilter {
            attributes: Some(AttributeFilter {
                required: 2,
                forbidden: 1,
            }),
            ..Default::default()
        },
        SelectionFilter {
            include: vec![pattern(
                SelectionField::Name,
                PatternSyntax::Wildcard,
                "film.*",
            )],
            ..Default::default()
        },
        SelectionFilter {
            include: vec![pattern(
                SelectionField::RelativePath,
                PatternSyntax::Regex,
                "(?<=^)film\\.mkv$",
            )],
            ..Default::default()
        },
        SelectionFilter {
            include: vec![pattern(
                SelectionField::FullPath,
                PatternSyntax::Regex,
                "film\\.mkv$",
            )],
            ..Default::default()
        },
    ];
    for (index, f) in filters.iter().enumerate() {
        assert!(
            select(&i, f).unwrap().contains(&want),
            "dimension {index} excludes boundary"
        );
    }
    // An independent negative observation must be rejected by each dimension.
    i.entries[0].absolute_path = root().join("deep/no.txt");
    i.entries[0].relative_path = encode_path(Path::new("deep/no.txt"));
    i.entries[0].kind = EntryKind::Directory;
    i.entries[0].stamp = FileStamp::default();
    for (index, f) in filters.iter().enumerate() {
        assert!(
            !select(&i, f).unwrap().contains(&want),
            "dimension {index} ignored"
        );
    }
    let f = SelectionFilter {
        exclude: vec![pattern(SelectionField::Name, PatternSyntax::Regex, "film")],
        ..Default::default()
    };
    assert!(!select(&i, &f).unwrap().contains(&i.entries[1].id));
}
#[test]
fn exclude_rebuilds_numbers_and_collisions() {
    let i = fixture(&["a.mkv", "b.mkv", "2.mkv"]);
    let mut r = recipe(&i);
    r.rules = vec![Rule {
        id: "n".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::Template {
            parts: vec![TemplatePart::Field {
                key: FieldKey::Counter,
                format: ValueFormat::Text,
                fallback: None,
            }],
        },
    }];
    let before_inputs = FrozenInputCache::default()
        .prepare(
            &InputRequirements::from_recipe(&r, FrozenTimezone::Utc).unwrap(),
            &i,
            &JobControl::default(),
        )
        .unwrap();
    let before = evaluate(&i, &r, &before_inputs).unwrap();
    let before_plan = plan(&i, &before, &namespace(&i), version(1)).unwrap();
    assert_eq!(before_plan.counts.blocked, 0);
    let f = SelectionFilter {
        exclude: vec![pattern(
            SelectionField::Name,
            PatternSyntax::Wildcard,
            "2.*",
        )],
        ..Default::default()
    };
    prepare_selection(&i, &mut r, &f, NumberingScope::Files).unwrap();
    let inputs = FrozenInputCache::default()
        .prepare(
            &InputRequirements::from_recipe(&r, FrozenTimezone::Utc).unwrap(),
            &i,
            &JobControl::default(),
        )
        .unwrap();
    let b = evaluate(&i, &r, &inputs).unwrap();
    assert_eq!(
        b.rows
            .iter()
            .map(|r| r.new_display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["1.mkv", "2.mkv"]
    );
    assert_eq!(r.selection.len(), 2);
    let after_plan = plan(&i, &b, &namespace(&i), version(2)).unwrap();
    assert!(after_plan.counts.blocked > 0);
    assert!(after_plan.rows[1]
        .issues
        .iter()
        .any(|e| e.code == RenameErrorCode::Collision));
    // Excluding b instead leaves 2 selected and renumbers it, without stale names.
    prepare_selection(
        &i,
        &mut r,
        &SelectionFilter {
            exclude: vec![pattern(
                SelectionField::Name,
                PatternSyntax::Wildcard,
                "b.*",
            )],
            ..Default::default()
        },
        NumberingScope::Files,
    )
    .unwrap();
    let inputs = FrozenInputCache::default()
        .prepare(
            &InputRequirements::from_recipe(&r, FrozenTimezone::Utc).unwrap(),
            &i,
            &JobControl::default(),
        )
        .unwrap();
    assert_eq!(
        evaluate(&i, &r, &inputs).unwrap().rows[1].new_display_name,
        "2.mkv"
    );
}
#[test]
fn ambiguous_basename_and_outside_root_rejected() {
    let i = fixture(&["a/movie.mkv", "b/movie.mkv"]);
    let err = import_mappings(
        "old_path,new_path\nmovie.mkv,new.mkv\n".as_bytes(),
        MappingFormat::Csv,
        &i,
        &grants(&i),
    )
    .unwrap_err();
    assert_eq!(err.code, RenameErrorCode::AmbiguousMapping);
    let input = format!(
        "old_path,new_path\na/movie.mkv,{}\n",
        root().parent().unwrap().join("escape.mkv").display()
    );
    assert_eq!(
        import_mappings(input.as_bytes(), MappingFormat::Csv, &i, &grants(&i))
            .unwrap_err()
            .code,
        RenameErrorCode::OutsideGrant
    );
    let m = import_mappings(
        "old_path,new_path\na/movie.mkv,a/new.mkv\n".as_bytes(),
        MappingFormat::Csv,
        &i,
        &grants(&i),
    )
    .unwrap();
    assert_eq!(m.rows[0].entry_id, i.entries[0].id);
}
#[test]
fn quoted_csv_and_paste_cardinality() {
    let i = fixture(&["movie,é.mkv", "second.mkv"]);
    let m = import_mappings(
        "old_path;new_path\n\"movie,é.mkv\";\"line\n名.mkv\"\n".as_bytes(),
        MappingFormat::Delimited { delimiter: b';' },
        &i,
        &grants(&i),
    )
    .unwrap();
    assert_eq!(m.rows[0].target_path, root().join("line\n名.mkv"));
    let order = vec![i.entries[1].id, i.entries[0].id];
    let format = MappingFormat::NameList {
        order: order.clone(),
    };
    assert_eq!(
        import_mappings("one.mkv\n".as_bytes(), format.clone(), &i, &grants(&i))
            .unwrap_err()
            .code,
        RenameErrorCode::MappingCardinality
    );
    let m = import_mappings(
        "=SUM(1,2).mkv\ntwo.mkv\n".as_bytes(),
        format,
        &i,
        &grants(&i),
    )
    .unwrap();
    assert_eq!(m.rows[0].entry_id, order[0]);
    assert_eq!(m.rows[0].target_path, root().join("=SUM(1,2).mkv"));
}
struct Repeated {
    left: u64,
    byte: u8,
}
impl Read for Repeated {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let n = (self.left as usize).min(b.len());
        b[..n].fill(self.byte);
        self.left -= n as u64;
        Ok(n)
    }
}
#[test]
fn mapping_limits_before_allocation() {
    let i = fixture(&["a.mkv"]);
    let err = import_mappings(
        Repeated {
            left: 64 * 1024 * 1024 + 1,
            byte: b'x',
        },
        MappingFormat::Csv,
        &i,
        &grants(&i),
    )
    .unwrap_err();
    assert_eq!(err.code, RenameErrorCode::ImportLimit);
}
#[test]
fn spreadsheet_safe_export_does_not_change_recorded_path() {
    let i = fixture(&["=SUM(1,2).mkv"]);
    let r = recipe(&i);
    let batch = evaluate(&i, &r, &frozen(&i, &r)).unwrap();
    let stored_original_name = &batch.rows[0].old_display_name;
    assert_eq!(stored_original_name, "=SUM(1,2).mkv");
    let reviewed = plan(&i, &batch, &namespace(&i), version(1)).unwrap();
    let safe = export_report(&reviewed, ExportMode::SpreadsheetSafe).unwrap();
    let text = String::from_utf8(safe).unwrap();
    assert!(text.contains("'=SUM(1,2).mkv"));
    let mut report = csv::Reader::from_reader(text.as_bytes());
    let record = report.records().next().unwrap().unwrap();
    assert!(record[1].starts_with('\''));
    let exact = export_rows(&batch.rows, ExportMode::ExactMapping).unwrap();
    let round = import_mappings(exact.as_slice(), MappingFormat::Csv, &i, &grants(&i)).unwrap();
    assert_eq!(round.rows[0].target_path, i.entries[0].absolute_path);
    assert_eq!(batch.rows[0].old_display_name, "=SUM(1,2).mkv");
}
#[test]
fn duplicate_traversal_unknown_and_unselected_mappings_are_rejected() {
    let i = fixture(&["a.mkv", "b.mkv"]);
    let g = grants(&i);
    for input in [
        "old_path,new_path\na.mkv,../escape\n",
        "old_path,new_path\n../a.mkv,new.mkv\n",
        "old_path,new_path\na.mkv,x.mkv\na.mkv,y.mkv\n",
        "old_path,new_path\na.mkv,x.mkv\nb.mkv,x.mkv\n",
        "old_path,new_path\nmissing/a.mkv,x.mkv\n",
    ] {
        assert!(
            import_mappings(input.as_bytes(), MappingFormat::Csv, &i, &g).is_err(),
            "{input}"
        );
    }
    assert_eq!(
        import_mappings(
            b"x.mkv".as_slice(),
            MappingFormat::NameList {
                order: vec![EntryId::new()]
            },
            &i,
            &g
        )
        .unwrap_err()
        .code,
        RenameErrorCode::UnknownEntry
    );
}
#[test]
fn mappings_and_row_override_enter_authoritative_evaluation() {
    let i = fixture(&["a.mkv", "b.mkv"]);
    let r = recipe(&i);
    let f = frozen(&i, &r);
    let m = import_mappings(
        b"old_path,new_path\na.mkv,mapped.mkv\n".as_slice(),
        MappingFormat::Csv,
        &i,
        &grants(&i),
    )
    .unwrap();
    let b = evaluate_with_mappings(
        &i,
        &r,
        &f,
        &m,
        &[RowOverride {
            entry_id: i.entries[0].id,
            new_name: "=literal.mkv".into(),
        }],
    )
    .unwrap();
    assert_eq!(b.rows[0].target_path, root().join("=literal.mkv"));
    assert_eq!(b.rows[1].target_path, root().join("b.mkv"));
    assert!(evaluate_with_mappings(
        &i,
        &r,
        &f,
        &m,
        &[RowOverride {
            entry_id: EntryId::new(),
            new_name: "x".into()
        }]
    )
    .is_err());
    assert!(evaluate_with_mappings(
        &i,
        &r,
        &f,
        &m,
        &[RowOverride {
            entry_id: i.entries[0].id,
            new_name: "../x".into()
        }]
    )
    .is_err());
    let mut r2 = r.clone();
    r2.selection.remove(0);
    r2.numbering_order.remove(0);
    assert_eq!(
        evaluate_with_mappings(&i, &r2, &frozen(&i, &r2), &m, &[])
            .unwrap_err()
            .code,
        RenameErrorCode::UnselectedEntry
    );
    let mut stale = m;
    stale.generation += 1;
    assert_eq!(
        evaluate_with_mappings(&i, &r, &f, &stale, &[])
            .unwrap_err()
            .code,
        RenameErrorCode::StaleInputs
    );
}
#[test]
fn mixed_kinds_have_explicit_counter_scope_and_cache_recomputes_scope_switch() {
    let mut i = fixture(&["folder", "a.mkv", "b.mkv"]);
    i.entries[0].kind = EntryKind::Directory;
    let mut r = recipe(&i);
    r.rules = vec![Rule {
        id: "t".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::Template {
            parts: vec![TemplatePart::Field {
                key: FieldKey::Counter,
                format: ValueFormat::Text,
                fallback: None,
            }],
        },
    }];
    let mut cache = FrozenInputCache::default();
    let input = cache
        .prepare(
            &InputRequirements::from_recipe(&r, FrozenTimezone::Utc).unwrap(),
            &i,
            &JobControl::default(),
        )
        .unwrap();
    let b = evaluate(&i, &r, &input).unwrap();
    assert_eq!(b.rows.len(), 3);
    assert_eq!(b.rows[0].status, RowStatus::Blocked);
    assert_eq!(b.rows[1].new_display_name, "1.mkv");
    assert_eq!(b.rows[2].new_display_name, "2.mkv");
    r.numbering_scope = NumberingScope::Directories;
    let input = cache
        .prepare(
            &InputRequirements::from_recipe(&r, FrozenTimezone::Utc).unwrap(),
            &i,
            &JobControl::default(),
        )
        .unwrap();
    let b = evaluate(&i, &r, &input).unwrap();
    assert_eq!(b.rows[0].new_display_name, "1");
    assert_eq!(b.rows[1].status, RowStatus::Blocked);
    let RuleParameters::Template { parts } = &mut r.rules[0].parameters else {
        unreachable!()
    };
    let TemplatePart::Field { fallback, .. } = &mut parts[0] else {
        unreachable!()
    };
    *fallback = Some("outside".into());
    let input = cache
        .prepare(
            &InputRequirements::from_recipe(&r, FrozenTimezone::Utc).unwrap(),
            &i,
            &JobControl::default(),
        )
        .unwrap();
    assert_eq!(
        evaluate(&i, &r, &input).unwrap().rows[1].new_display_name,
        "outside.mkv"
    );
}
#[test]
fn byte_and_row_limits_accept_boundary_and_reject_one_over() {
    let i = fixture(&["a.mkv"]);
    let format = MappingFormat::NameList {
        order: vec![i.entries[0].id],
    };
    assert!(import_mappings(
        Repeated {
            left: 64 * 1024 * 1024,
            byte: b'x'
        },
        format.clone(),
        &i,
        &grants(&i)
    )
    .is_ok());
    assert_eq!(
        import_mappings(
            Repeated {
                left: 64 * 1024 * 1024 + 1,
                byte: b'x'
            },
            format,
            &i,
            &grants(&i)
        )
        .unwrap_err()
        .code,
        RenameErrorCode::ImportLimit
    );
    let names = (0..100_000).map(|n| format!("{n}.mkv")).collect::<Vec<_>>();
    let many = fixture(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let mut at = "old_path,new_path\n".to_owned();
    for n in 0..100_000 {
        at.push_str(&format!("{n}.mkv,new{n}.mkv\n"));
    }
    assert_eq!(
        import_mappings(at.as_bytes(), MappingFormat::Csv, &many, &grants(&many))
            .unwrap()
            .rows
            .len(),
        100_000
    );
    let over = at + "a.mkv,x.mkv\n";
    assert_eq!(
        import_mappings(over.as_bytes(), MappingFormat::Csv, &many, &grants(&many))
            .unwrap_err()
            .code,
        RenameErrorCode::ImportLimit
    );
}
#[test]
fn old_scope_inputs_are_not_valid_after_scope_edit() {
    let mut i = fixture(&["folder", "a.mkv"]);
    i.entries[0].kind = EntryKind::Directory;
    let mut r = recipe(&i);
    let f = frozen(&i, &r);
    r.numbering_scope = NumberingScope::Directories;
    assert_eq!(
        evaluate(&i, &r, &f).unwrap_err().code,
        RenameErrorCode::StaleInputs
    );
}

#[test]
fn override_collision_on_hidden_row_blocks_complete_plan() {
    let names = (0..205).map(|n| format!("{n}.mkv")).collect::<Vec<_>>();
    let i = fixture(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let r = recipe(&i);
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
            entry_id: i.entries[204].id,
            new_name: "0.mkv".into(),
        }],
    )
    .unwrap();
    let p = plan(&i, &b, &namespace(&i), version(1)).unwrap();
    assert_eq!(p.rows.len(), 205);
    assert!(p.counts.blocked > 0);
    assert!(p.rows[204]
        .issues
        .iter()
        .any(|e| e.code == RenameErrorCode::Collision));
}
#[test]
fn number_rule_skips_other_kind_and_companions_do_not_consume_ordinals() {
    let mut i = fixture(&["folder", "a.mkv", "a.srt", "b.mkv"]);
    i.entries[0].kind = EntryKind::Directory;
    let mut r = recipe(&i);
    r.numbering_order = vec![i.entries[3].id, i.entries[0].id, i.entries[1].id];
    r.companions = CompanionPolicy::Explicit {
        associations: vec![CompanionAssociation {
            primary: i.entries[1].id,
            companions: vec![i.entries[2].id],
        }],
    };
    assert_eq!(
        scoped_numbering_order(&i, &r).unwrap(),
        vec![i.entries[3].id, i.entries[1].id]
    );
    // Association naming remains Task18; the existing evaluator rejects it clearly.
    assert_eq!(
        evaluate(&i, &r, &frozen(&i, &r)).unwrap_err().code,
        RenameErrorCode::UnsupportedConfiguration
    );
    r.companions = CompanionPolicy::None;
    r.selection.retain(|id| *id != i.entries[2].id);
    r.rules = vec![Rule {
        id: "n".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::Number {
            rule: NumberRule {
                start: 1,
                step: 1,
                repeat_each: 1,
                padding: 0,
                radix: Radix::Decimal,
                reset_by_folder: false,
                at: InsertPosition::Before { position: 1 },
                prefix: String::new(),
                suffix: String::new(),
            },
        },
    }];
    let b = evaluate(&i, &r, &frozen(&i, &r)).unwrap();
    assert_eq!(
        b.rows
            .iter()
            .map(|r| r.new_display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["1b.mkv", "folder", "2a.mkv"]
    );
    r.numbering_scope = NumberingScope::Directories;
    let b = evaluate(&i, &r, &frozen(&i, &r)).unwrap();
    assert_eq!(
        b.rows
            .iter()
            .map(|r| r.new_display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["b.mkv", "1folder", "a.mkv"]
    );
}

#[test]
fn filters_deduplicate_overlapping_roots_and_preserve_order() {
    let mut i = fixture(&["a.mkv", "nested/b.mkv"]);
    let nested = RootBinding::new(root().join("nested")).unwrap();
    let mut alias = i.entries[1].clone();
    alias.id = EntryId::new();
    alias.root_id = nested.id;
    alias.relative_path = encode_path(Path::new("b.mkv"));
    i.roots.push(nested);
    i.entries.push(alias);
    assert_eq!(
        select(&i, &SelectionFilter::default()).unwrap(),
        vec![i.entries[0].id, i.entries[1].id]
    );
    let f = SelectionFilter {
        include: vec![pattern(
            SelectionField::Name,
            PatternSyntax::Wildcard,
            "*.MKV",
        )],
        exclude: vec![pattern(SelectionField::Name, PatternSyntax::Regex, "^b")],
        ..Default::default()
    };
    assert!(select(&i, &f).unwrap().is_empty());
    let mut f = f;
    f.include[0].case_sensitive = false;
    assert_eq!(select(&i, &f).unwrap(), vec![i.entries[0].id]);
}
#[test]
fn invalid_and_exhausted_filter_expressions_report_errors() {
    let i = fixture(&["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!"]);
    let f = SelectionFilter {
        include: vec![pattern(
            SelectionField::Name,
            PatternSyntax::Regex,
            "(*NO_AUTO_POSSESS)(a+)+$",
        )],
        ..Default::default()
    };
    assert_eq!(
        select(&i, &f).unwrap_err().code,
        RenameErrorCode::RegexLimit
    );
    for f in [
        SelectionFilter {
            size: Some(U64Range {
                min: Some(2),
                max: Some(1),
            }),
            ..Default::default()
        },
        SelectionFilter {
            include: vec![pattern(SelectionField::Name, PatternSyntax::Regex, "(")],
            ..Default::default()
        },
        SelectionFilter {
            attributes: Some(AttributeFilter {
                required: 1,
                forbidden: 1,
            }),
            ..Default::default()
        },
    ] {
        assert!(select(&i, &f).is_err());
    }
}
#[test]
fn persisted_filters_reject_inconsistent_frozen_selection_and_keep_explicit_exclusions() {
    let i = fixture(&["a.mkv", "b.txt"]);
    let mut r = recipe(&i);
    r.selection_filter = Some(SelectionFilter {
        extensions: vec!["mkv".into()],
        ..Default::default()
    });
    assert_eq!(
        evaluate(&i, &r, &frozen(&i, &r)).unwrap_err().code,
        RenameErrorCode::InvalidSelection
    );
    r.selection.pop();
    r.numbering_order.pop();
    let saved = serde_json::to_vec(&r).unwrap();
    let restored = RecipeV1::from_json(&saved).unwrap();
    assert_eq!(restored.selection_filter, r.selection_filter);
    assert_eq!(
        evaluate(&i, &restored, &frozen(&i, &restored))
            .unwrap()
            .rows
            .len(),
        1
    );
    // Explicit selection may be a subset of a broader persisted filter.
    r.selection_filter = Some(SelectionFilter::default());
    assert_eq!(evaluate(&i, &r, &frozen(&i, &r)).unwrap().rows.len(), 1);
}
#[test]
fn explicit_exclusion_overlay_persists_and_unknown_ids_fail_closed() {
    let i = fixture(&["a.mkv", "b.mkv"]);
    let mut r = recipe(&i);
    let f = SelectionFilter {
        excluded_ids: vec![i.entries[1].id],
        ..Default::default()
    };
    prepare_selection(&i, &mut r, &f, NumberingScope::Files).unwrap();
    assert_eq!(r.selection, vec![i.entries[0].id]);
    assert_eq!(r.selection_filter, Some(f));
    let f = SelectionFilter {
        excluded_ids: vec![EntryId::new()],
        ..Default::default()
    };
    assert_eq!(
        select(&i, &f).unwrap_err().code,
        RenameErrorCode::UnknownEntry
    );
}

#[test]
fn self_review_mapping_honors_explicit_file_grant_without_authorizing_siblings() {
    let mut i = fixture(&["a.mkv"]);
    let binding = RootBinding::new(i.entries[0].absolute_path.clone()).unwrap();
    i.entries[0].root_id = binding.id;
    i.roots = vec![binding];
    let m = import_mappings(
        b"old_path,new_path\na.mkv,b.mkv\n".as_slice(),
        MappingFormat::Csv,
        &i,
        &grants(&i),
    )
    .unwrap();
    assert_eq!(m.rows[0].target_path, root().join("b.mkv"));
    assert!(import_mappings(
        b"old_path,new_path\na.mkv,nested/b.mkv\n".as_slice(),
        MappingFormat::Csv,
        &i,
        &grants(&i)
    )
    .is_err());
    assert!(import_mappings(
        b"old_path,new_path\nother.mkv,b.mkv\n".as_slice(),
        MappingFormat::Csv,
        &i,
        &grants(&i)
    )
    .is_err());
}
#[test]
fn self_review_duplicate_native_sources_are_rejected_even_with_distinct_ids() {
    let mut i = fixture(&["a.mkv"]);
    let mut alias = i.entries[0].clone();
    alias.id = EntryId::new();
    i.entries.push(alias);
    let r = recipe(&i);
    let m = MappingSet {
        inventory_id: i.id,
        generation: i.generation,
        rows: vec![
            MappingRow {
                entry_id: i.entries[0].id,
                target_path: root().join("b.mkv"),
            },
            MappingRow {
                entry_id: i.entries[1].id,
                target_path: root().join("c.mkv"),
            },
        ],
    };
    assert_eq!(
        evaluate_with_mappings(&i, &r, &frozen(&i, &r), &m, &[])
            .unwrap_err()
            .code,
        RenameErrorCode::InvalidMapping
    );
}
#[test]
fn self_review_planner_checks_scope_provenance() {
    let i = fixture(&["a.mkv"]);
    let r = recipe(&i);
    let mut b = evaluate(&i, &r, &frozen(&i, &r)).unwrap();
    b.frozen_inputs.numbering_scope = NumberingScope::Directories;
    assert_eq!(
        plan(&i, &b, &namespace(&i), version(1)).unwrap_err().code,
        RenameErrorCode::InvalidInventory
    );
}
#[cfg(unix)]
#[test]
fn self_review_exact_mapping_export_round_trips_native_colons() {
    let i = fixture(&["movie:part.mkv"]);
    let r = recipe(&i);
    let b = evaluate(&i, &r, &frozen(&i, &r)).unwrap();
    let bytes = export_rows(&b.rows, ExportMode::ExactMapping).unwrap();
    let m = import_mappings(bytes.as_slice(), MappingFormat::Csv, &i, &grants(&i)).unwrap();
    assert_eq!(m.rows[0].target_path, i.entries[0].absolute_path);
}
#[cfg(unix)]
#[test]
fn self_review_exact_export_fails_when_native_name_cannot_be_reimported() {
    let i = fixture(&[r"literal\backslash.mkv"]);
    let r = recipe(&i);
    let b = evaluate(&i, &r, &frozen(&i, &r)).unwrap();
    assert_eq!(
        export_rows(&b.rows, ExportMode::ExactMapping)
            .unwrap_err()
            .code,
        RenameErrorCode::UnsupportedName
    );
}
#[test]
fn filter_json_keeps_exact_quantities_and_accepts_one_sided_ranges() {
    let parsed = serde_json::from_value::<SelectionFilter>(
        serde_json::json!({"size":{"min":"9007199254740993"},"created_ns":{"max":"1700000000000000000123456789"}}),
    );
    assert!(
        parsed.is_ok(),
        "one-sided exact ranges must deserialize: {:?}",
        parsed.err()
    );
    let filter = parsed.unwrap();
    assert_eq!(
        filter.size.as_ref().unwrap().min,
        Some(9_007_199_254_740_993)
    );
    let serialized = serde_json::to_value(filter).unwrap();
    assert_eq!(serialized["size"]["min"], "9007199254740993");
    assert_eq!(
        serialized["created_ns"]["max"],
        "1700000000000000000123456789"
    );
    assert!(serde_json::from_value::<SelectionFilter>(
        serde_json::json!({"size":{"min":9007199254740993u64,"max":null}})
    )
    .is_err());
}
