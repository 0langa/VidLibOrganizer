use vidlib_rename::*;

fn run(
    name: &str,
    segment: SegmentSelector,
    operation: TextOperation,
) -> Result<NameParts, RuleIssue> {
    apply_text(
        &NameParts::from_filename(name),
        &TextRule { segment, operation },
    )
}
fn g(start: i32, end: i32) -> SegmentSelector {
    SegmentSelector::Graphemes {
        target: NameSegment::Stem,
        start,
        end,
    }
}
fn case(mode: CaseMode) -> TextOperation {
    TextOperation::Case {
        mode,
        exceptions: vec![],
        roman: None,
    }
}
fn pair(occurrence: u32, include_delimiters: bool) -> SegmentSelector {
    SegmentSelector::Between {
        target: NameSegment::Stem,
        opening: "[".into(),
        closing: "]".into(),
        occurrence,
        include_delimiters,
    }
}

// Catches omitted operations, wrong ordering and accidental transformation outside the selected text.
#[test]
fn text_rule_goldens() {
    use TextOperation::*;
    let rows = vec![
        (
            "prefix",
            "abc.MKV",
            SegmentSelector::Stem,
            Prefix {
                text: "pre-".into(),
            },
            "pre-abc.MKV",
        ),
        (
            "suffix",
            "abc.MKV",
            SegmentSelector::Stem,
            Suffix {
                text: "-post".into(),
            },
            "abc-post.MKV",
        ),
        (
            "insert",
            "abc.MKV",
            SegmentSelector::Stem,
            Insert {
                at: InsertPosition::Before { position: 2 },
                text: "X".into(),
            },
            "aXbc.MKV",
        ),
        (
            "append",
            "abc.MKV",
            SegmentSelector::Stem,
            Insert {
                at: InsertPosition::End,
                text: "X".into(),
            },
            "abcX.MKV",
        ),
        (
            "ordered replacements",
            "a_a.MKV",
            SegmentSelector::Stem,
            Replace {
                replacements: vec![
                    LiteralReplacement {
                        from: "a".into(),
                        to: "b".into(),
                    },
                    LiteralReplacement {
                        from: "b_".into(),
                        to: "C".into(),
                    },
                ],
                scope: MatchScope::All,
                case_sensitive: true,
            },
            "Cb.MKV",
        ),
        (
            "first literal",
            "aa.MKV",
            SegmentSelector::Stem,
            Replace {
                replacements: vec![LiteralReplacement {
                    from: "a".into(),
                    to: "X".into(),
                }],
                scope: MatchScope::First,
                case_sensitive: true,
            },
            "Xa.MKV",
        ),
        (
            "folded literal",
            "Straße STRASSE.MKV",
            SegmentSelector::Stem,
            Replace {
                replacements: vec![LiteralReplacement {
                    from: "strasse".into(),
                    to: "X".into(),
                }],
                scope: MatchScope::All,
                case_sensitive: false,
            },
            "X X.MKV",
        ),
        (
            "characters",
            "a_b-c.MKV",
            SegmentSelector::Stem,
            RemoveCharacters {
                characters: "_-".into(),
            },
            "abc.MKV",
        ),
        (
            "range",
            "abcde.MKV",
            SegmentSelector::Stem,
            RemoveRange { start: 2, end: -2 },
            "ae.MKV",
        ),
        (
            "crop before",
            "one-two-three.MKV",
            SegmentSelector::Stem,
            Crop {
                delimiter: "-".into(),
                occurrence: 2,
                side: CropSide::Before,
                include_delimiter: false,
            },
            "one-two.MKV",
        ),
        (
            "crop after included",
            "one-two-three.MKV",
            SegmentSelector::Stem,
            Crop {
                delimiter: "-".into(),
                occurrence: 1,
                side: CropSide::After,
                include_delimiter: true,
            },
            "-two-three.MKV",
        ),
        (
            "move forward",
            "2024 Show.MKV",
            SegmentSelector::Stem,
            MoveCopy {
                start: 1,
                end: 5,
                destination: InsertPosition::End,
                copy: false,
            },
            "Show2024 .MKV",
        ),
        (
            "move backward",
            "abcXYZ.MKV",
            SegmentSelector::Stem,
            MoveCopy {
                start: 4,
                end: 6,
                destination: InsertPosition::Before { position: 1 },
                copy: false,
            },
            "XYZabc.MKV",
        ),
        (
            "copy",
            "abc.MKV",
            SegmentSelector::Stem,
            MoveCopy {
                start: 1,
                end: 2,
                destination: InsertPosition::End,
                copy: true,
            },
            "abcab.MKV",
        ),
        ("reverse selected", "abcd.MKV", g(2, 3), Reverse, "acbd.MKV"),
        (
            "trim",
            " \tShow\u{2003}.MKV",
            SegmentSelector::Stem,
            Trim,
            "Show.MKV",
        ),
        (
            "collapse",
            "a \t b\u{2003}c.MKV",
            SegmentSelector::Stem,
            CollapseWhitespace {
                replacement: "_".into(),
            },
            "a_b_c.MKV",
        ),
        (
            "translate simultaneous",
            "aéb.MKV",
            SegmentSelector::Stem,
            Translate {
                translations: vec![
                    CharacterTranslation {
                        from: 'a',
                        to: "é".into(),
                    },
                    CharacterTranslation {
                        from: 'é',
                        to: "e".into(),
                    },
                ],
            },
            "éeb.MKV",
        ),
        (
            "transliteration explicit",
            "影像 café.MKV",
            SegmentSelector::Stem,
            Transliterate,
            "Ying Xiang cafe.MKV",
        ),
        (
            "keep",
            "影像 café.MKV",
            SegmentSelector::Stem,
            Keep,
            "影像 café.MKV",
        ),
        (
            "fixed",
            "abc.MKV",
            SegmentSelector::Stem,
            Fixed {
                value: "New".into(),
            },
            "New.MKV",
        ),
        (
            "remove stem",
            "abc.MKV",
            SegmentSelector::Stem,
            Remove,
            ".MKV",
        ),
        (
            "case keep",
            "mIx.MKV",
            SegmentSelector::Stem,
            case(CaseMode::Keep),
            "mIx.MKV",
        ),
        (
            "uppercase expanded",
            "Straße.MKV",
            SegmentSelector::Stem,
            case(CaseMode::Upper),
            "STRASSE.MKV",
        ),
        (
            "lowercase context",
            "ΟΣ İ.MKV",
            SegmentSelector::Stem,
            case(CaseMode::Lower),
            "ος i\u{307}.MKV",
        ),
        (
            "title full",
            "ǳURO ßHOW l'été.MKV",
            SegmentSelector::Stem,
            case(CaseMode::Title),
            "ǲuro Sshow L'été.MKV",
        ),
        (
            "sentence",
            "hELLO WORLD! nEXT SHOW.MKV",
            SegmentSelector::Stem,
            case(CaseMode::Sentence),
            "Hello world! Next show.MKV",
        ),
        (
            "paired",
            "A [one] B [two] C.MKV",
            pair(2, false),
            case(CaseMode::Upper),
            "A [one] B [TWO] C.MKV",
        ),
    ];
    let failures: Vec<_> = rows
        .into_iter()
        .filter_map(|(label, name, segment, op, want)| {
            let got = run(name, segment, op).map(|v| v.filename());
            (got.as_deref() != Ok(want)).then(|| format!("{label}: expected {want:?}, got {got:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Catches byte/char slicing of combining sequences, emoji and flags.
#[test]
fn unicode_graphemes_are_not_split() {
    let reversed = run(
        "Cafe\u{301}.mkv",
        SegmentSelector::Stem,
        TextOperation::Reverse,
    )
    .unwrap();
    assert_eq!(reversed.stem, "e\u{301}faC");
    let selected = "A👩‍👩‍👧‍👦e\u{301}🇫🇷Z";
    assert_eq!(
        &selected[resolve_segment(selected, &g(2, -2)).unwrap()],
        "👩‍👩‍👧‍👦e\u{301}🇫🇷"
    );
    assert_eq!(
        run(
            "Ae\u{301}Z.mkv",
            g(2, 2),
            TextOperation::Fixed { value: "X".into() }
        )
        .unwrap()
        .filename(),
        "AXZ.mkv"
    );
    assert_eq!(
        run(
            "e\u{301}.mkv",
            SegmentSelector::Stem,
            TextOperation::Replace {
                replacements: vec![LiteralReplacement {
                    from: "e".into(),
                    to: "x".into()
                }],
                scope: MatchScope::All,
                case_sensitive: true
            }
        )
        .unwrap()
        .filename(),
        "e\u{301}.mkv"
    );
}

// Catches implicit Roman rewriting and substring exception matching.
#[test]
fn case_exceptions_and_roman_are_explicit() {
    let rows = [
        (None, "BBC Show Iii Bbcish.mkv"),
        (Some(RomanCase::Keep), "BBC Show iii Bbcish.mkv"),
        (Some(RomanCase::Upper), "BBC Show III Bbcish.mkv"),
        (Some(RomanCase::Lower), "BBC Show iii Bbcish.mkv"),
    ];
    let failures: Vec<_> = rows
        .into_iter()
        .filter_map(|(roman, want)| {
            let got = run(
                "bbc SHOW iii bbcish.mkv",
                SegmentSelector::Stem,
                TextOperation::Case {
                    mode: CaseMode::Title,
                    exceptions: vec!["BBC".into()],
                    roman,
                },
            )
            .unwrap()
            .filename();
            (got != want).then(|| format!("{roman:?}: expected {want:?}, got {got:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(
        run(
            "strasse iii iiv.mkv",
            SegmentSelector::Stem,
            TextOperation::Case {
                mode: CaseMode::Title,
                exceptions: vec!["Straße".into()],
                roman: Some(RomanCase::Upper)
            }
        )
        .unwrap()
        .filename(),
        "Straße III Iiv.mkv"
    );
}

// Catches extension changes from ordinary rules or failure to re-split explicit WholeName output.
#[test]
fn extension_requires_explicit_mode() {
    let ordinary = run("SHOW.MKV", SegmentSelector::Stem, case(CaseMode::Lower)).unwrap();
    assert_eq!(ordinary.ext, "MKV");
    let rows = [
        (
            "Show.MKV",
            SegmentSelector::Extension,
            case(CaseMode::Lower),
            "Show.mkv",
        ),
        (
            "Show",
            SegmentSelector::Extension,
            TextOperation::Fixed {
                value: "mp4".into(),
            },
            "Show.mp4",
        ),
        (
            "Show.avi",
            SegmentSelector::Extension,
            TextOperation::Fixed {
                value: "mp4".into(),
            },
            "Show.mp4",
        ),
        (
            "Show.avi",
            SegmentSelector::Extension,
            TextOperation::Remove,
            "Show",
        ),
        (
            "SHOW.MKV",
            SegmentSelector::WholeName,
            case(CaseMode::Lower),
            "show.mkv",
        ),
        (
            "old.MKV",
            SegmentSelector::WholeName,
            TextOperation::Fixed {
                value: "new.tar.gz".into(),
            },
            "new.tar.gz",
        ),
        (
            "trail.",
            SegmentSelector::Stem,
            TextOperation::Suffix { text: "X".into() },
            "trailX.",
        ),
        (
            ".hidden",
            SegmentSelector::Stem,
            TextOperation::Suffix { text: "X".into() },
            ".hiddenX",
        ),
    ];
    let failures: Vec<_> = rows
        .into_iter()
        .filter_map(|(name, segment, op, want)| {
            let got = run(name, segment, op).unwrap().filename();
            (got != want).then(|| format!("{name}: expected {want:?}, got {got:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Catches accepting invalid logical positions, delimiter boundaries or invalid output names.
#[test]
fn invalid_positions_delimiters_and_names_are_row_issues() {
    let selectors = vec![
        g(0, 1),
        g(1, 0),
        g(1, 4),
        g(-4, -1),
        g(3, 1),
        g(i32::MIN, -1),
        pair(0, false),
        pair(3, false),
        SegmentSelector::Between {
            target: NameSegment::Stem,
            opening: "".into(),
            closing: "]".into(),
            occurrence: 1,
            include_delimiters: false,
        },
        SegmentSelector::Delimited {
            target: NameSegment::Stem,
            delimiter: "".into(),
            occurrence: 1,
            include_delimiter: false,
        },
        SegmentSelector::Delimited {
            target: NameSegment::Stem,
            delimiter: "b".into(),
            occurrence: 0,
            include_delimiter: false,
        },
        SegmentSelector::Delimited {
            target: NameSegment::Stem,
            delimiter: "b".into(),
            occurrence: 2,
            include_delimiter: false,
        },
        SegmentSelector::Delimited {
            target: NameSegment::Stem,
            delimiter: "\u{301}".into(),
            occurrence: 1,
            include_delimiter: false,
        },
    ];
    let accepted: Vec<_> = selectors
        .into_iter()
        .filter(|s| resolve_segment("ab\u{301}c", s).is_ok())
        .collect();
    assert!(
        accepted.is_empty(),
        "accepted invalid selectors: {accepted:?}"
    );
    for value in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
        assert!(
            run(
                "abc",
                SegmentSelector::Stem,
                TextOperation::Fixed {
                    value: value.into()
                }
            )
            .is_err(),
            "{value:?}"
        );
    }
    for at in [0, 4, -4] {
        assert!(run(
            "abc",
            SegmentSelector::Stem,
            TextOperation::Insert {
                at: InsertPosition::Before { position: at },
                text: "X".into()
            }
        )
        .is_err());
    }
}

// Catches a segment's use of original coordinates instead of each rule's current input.
#[test]
fn segment_resolves_each_current_input() {
    let a = NameParts::from_filename("A [one] B [two] C.mkv");
    let first = apply_text(
        &a,
        &TextRule {
            segment: pair(1, false),
            operation: TextOperation::Fixed {
                value: "longer".into(),
            },
        },
    )
    .unwrap();
    let second = apply_text(
        &first,
        &TextRule {
            segment: pair(2, false),
            operation: case(CaseMode::Upper),
        },
    )
    .unwrap();
    assert_eq!(second.filename(), "A [longer] B [TWO] C.mkv");
    assert_eq!(a.filename(), "A [one] B [two] C.mkv");
    let current = "one--two--three";
    let select = SegmentSelector::Delimited {
        target: NameSegment::Stem,
        delimiter: "--".into(),
        occurrence: 1,
        include_delimiter: false,
    };
    assert_eq!(&current[resolve_segment(current, &select).unwrap()], "two");
    let included = SegmentSelector::Delimited {
        target: NameSegment::Stem,
        delimiter: "--".into(),
        occurrence: 1,
        include_delimiter: true,
    };
    assert_eq!(
        &current[resolve_segment(current, &included).unwrap()],
        "--two"
    );
    assert_eq!(
        &"[x]tail"[resolve_segment("[x]tail", &pair(1, true)).unwrap()],
        "[x]"
    );
}

fn evaluate_rules(rules: Vec<Rule>) -> EvaluatedBatch {
    use vidlib_inventory::*;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("A [one] B [two] C.MKV"), b"media").unwrap();
    let inventory = scan(
        &InventoryRequest::new(vec![RootBinding::new(dir.path().to_path_buf()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let id = inventory
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::File)
        .unwrap()
        .id;
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: vec![id],
        numbering_order: vec![id],
        rules,
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
        primary_order: vec![id],
        values: vec![],
    };
    let decoded = RecipeV1::from_json(&serde_json::to_vec(&recipe).unwrap()).unwrap();
    evaluate(&inventory, &decoded, &inputs).expect("text recipes must evaluate")
}
fn rule(id: &str, segment: SegmentSelector, operation: TextOperation) -> Rule {
    Rule {
        id: id.into(),
        enabled: true,
        segment,
        parameters: RuleParameters::Text { operation },
    }
}

// Catches a text engine disconnected from preview, lost original spelling or wrong explanations.
#[test]
fn evaluator_composes_text_segments_and_retains_original() {
    let batch = evaluate_rules(vec![
        rule(
            "first",
            pair(1, false),
            TextOperation::Fixed {
                value: "longer".into(),
            },
        ),
        rule("second", pair(2, false), case(CaseMode::Upper)),
        rule("ext", SegmentSelector::Extension, case(CaseMode::Lower)),
    ]);
    let row = &batch.rows[0];
    assert_eq!(row.old_display_name, "A [one] B [two] C.MKV");
    assert_eq!(row.new_display_name, "A [longer] B [TWO] C.mkv");
    assert_eq!(row.status, RowStatus::Changed);
    assert_eq!(
        row.explanations
            .iter()
            .map(|e| (e.rule_id.as_str(), e.before.as_str(), e.after.as_str()))
            .collect::<Vec<_>>(),
        [
            ("first", "A [one] B [two] C", "A [longer] B [two] C"),
            ("second", "A [longer] B [two] C", "A [longer] B [TWO] C"),
            ("ext", "MKV", "mkv"),
        ]
    );
}

// Catches treating row-dependent failures as a batch abort or publishing an invalid partial target.
#[test]
fn evaluator_blocks_row_on_rule_issue_with_rule_id() {
    let batch = evaluate_rules(vec![rule("bad", g(0, 2), TextOperation::Reverse)]);
    let row = &batch.rows[0];
    assert_eq!(row.status, RowStatus::Blocked);
    assert_eq!(row.issues[0].rule_id.as_deref(), Some("bad"));
    assert_eq!(row.new_display_name, row.old_display_name);
    assert_eq!(row.target_path, row.source_path);
}

// Catches silent fallback on malformed operations and ambiguous movement coordinates.
#[test]
fn invalid_operations_never_succeed_as_noops() {
    let operations = vec![
        TextOperation::RemoveRange { start: 0, end: 1 },
        TextOperation::MoveCopy {
            start: 1,
            end: 2,
            destination: InsertPosition::Before { position: 2 },
            copy: false,
        },
        TextOperation::Crop {
            delimiter: "-".into(),
            occurrence: 2,
            side: CropSide::After,
            include_delimiter: false,
        },
        TextOperation::Crop {
            delimiter: "".into(),
            occurrence: 1,
            side: CropSide::After,
            include_delimiter: false,
        },
        TextOperation::Replace {
            replacements: vec![LiteralReplacement {
                from: "".into(),
                to: "X".into(),
            }],
            scope: MatchScope::All,
            case_sensitive: true,
        },
        TextOperation::Translate {
            translations: vec![
                CharacterTranslation {
                    from: 'a',
                    to: "X".into(),
                },
                CharacterTranslation {
                    from: 'a',
                    to: "Y".into(),
                },
            ],
        },
        TextOperation::Case {
            mode: CaseMode::Title,
            exceptions: vec!["two words".into()],
            roman: None,
        },
        TextOperation::Case {
            mode: CaseMode::Title,
            exceptions: vec!["BBC".into(), "bbc".into()],
            roman: None,
        },
    ];
    let accepted: Vec<_> = operations
        .into_iter()
        .filter(|op| run("abc.mkv", SegmentSelector::Stem, op.clone()).is_ok())
        .collect();
    assert!(
        accepted.is_empty(),
        "invalid operations accepted: {accepted:?}"
    );
}

// Catches WholeName path sanitization before validation and implicit trailing-dot removal.
#[test]
fn whole_name_is_validated_before_splitting_and_noops_preserve_spelling() {
    let mut accepted = vec![];
    for name in ["sub/new.mkv", "sub\\new.mkv", "a\0b", "", ".", ".."] {
        if run(
            "old.mkv",
            SegmentSelector::WholeName,
            TextOperation::Fixed { value: name.into() },
        )
        .is_ok()
        {
            accepted.push(name);
        }
    }
    assert!(
        accepted.is_empty(),
        "invalid whole names accepted: {accepted:?}"
    );
    for op in [TextOperation::Keep, case(CaseMode::Lower)] {
        assert_eq!(
            run("trail.", SegmentSelector::Extension, op)
                .unwrap()
                .filename(),
            "trail."
        );
    }
    assert_eq!(
        run("trail.", SegmentSelector::Extension, TextOperation::Remove)
            .unwrap()
            .filename(),
        "trail"
    );
}

// Catches incomplete pairs, multi-byte delimiter splitting and failure to re-resolve a changing token.
#[test]
fn delimiters_are_flat_literal_grapheme_aligned_and_current() {
    for (input, selector) in [
        ("[open", pair(1, false)),
        ("[one] [open", pair(2, false)),
        (
            "[one]",
            SegmentSelector::Between {
                target: NameSegment::Stem,
                opening: "[".into(),
                closing: "".into(),
                occurrence: 1,
                include_delimiters: false,
            },
        ),
        ("[one]", pair(u32::MAX, false)),
        (
            "[one]",
            SegmentSelector::Delimited {
                target: NameSegment::Stem,
                delimiter: "[".into(),
                occurrence: u32::MAX,
                include_delimiter: false,
            },
        ),
    ] {
        assert!(
            resolve_segment(input, &selector).is_err(),
            "{input}: {selector:?}"
        );
    }
    let selector = SegmentSelector::Delimited {
        target: NameSegment::Stem,
        delimiter: "--".into(),
        occurrence: 1,
        include_delimiter: false,
    };
    let first = run(
        "a--b--c.MKV",
        selector.clone(),
        TextOperation::Fixed {
            value: "longer".into(),
        },
    )
    .unwrap();
    assert_eq!(
        apply_text(
            &first,
            &TextRule {
                segment: selector,
                operation: case(CaseMode::Upper)
            }
        )
        .unwrap()
        .filename(),
        "a--LONGER--c.MKV"
    );
    let selector = SegmentSelector::Between {
        target: NameSegment::Stem,
        opening: "«".into(),
        closing: "»".into(),
        occurrence: 2,
        include_delimiters: false,
    };
    assert_eq!(
        run(
            "«é»_«👩‍👩‍👧‍👦»_tail.mkv",
            selector,
            TextOperation::Fixed { value: "OK".into() }
        )
        .unwrap()
        .filename(),
        "«é»_«OK»_tail.mkv"
    );
    assert_eq!(
        run(
            "ab.mkv",
            SegmentSelector::Stem,
            TextOperation::Insert {
                at: InsertPosition::Before { position: -1 },
                text: "X".into()
            }
        )
        .unwrap()
        .filename(),
        "aXb.mkv"
    );
}

// Catches segment resolution failures escaping as batch errors or publishing earlier rule output.
#[test]
fn evaluator_preserves_source_target_after_a_later_rule_fails() {
    let batch = evaluate_rules(vec![
        rule(
            "first",
            pair(1, false),
            TextOperation::Fixed {
                value: "changed".into(),
            },
        ),
        rule("missing", pair(3, false), TextOperation::Reverse),
    ]);
    let row = &batch.rows[0];
    assert_eq!(row.status, RowStatus::Blocked);
    assert_eq!(row.issues[0].code, RenameErrorCode::InvalidRuleInput);
    assert_eq!(row.issues[0].rule_id.as_deref(), Some("missing"));
    assert_eq!(row.new_display_name, "A [one] B [two] C.MKV");
    assert_eq!(row.target_path, row.source_path);
    assert_eq!(row.explanations[0].after, "A [changed] B [two] C");
}

// Catches using platform path parsing to silently discard part of a filename component.
#[test]
fn input_components_are_never_implicitly_sanitized() {
    assert!(run("sub/new.mkv", SegmentSelector::Stem, TextOperation::Keep).is_err());
    assert_eq!(
        run(
            "drive:name.mkv",
            SegmentSelector::WholeName,
            TextOperation::Keep
        )
        .unwrap()
        .filename(),
        "drive:name.mkv"
    );
}

// Catches treating Unicode Lt letters as uncased when consuming sentence capitalization.
#[test]
fn sentence_case_consumes_titlecase_letters_and_preserves_context() {
    let rows = [
        ("ǅ hello.mkv", "ǅ hello.mkv"),
        ("ǈ hello.mkv", "ǈ hello.mkv"),
        ("ǋ hello.mkv", "ǋ hello.mkv"),
        ("ǲ hello.mkv", "ǲ hello.mkv"),
        ("123 ǅ HELLO.mkv", "123 ǅ hello.mkv"),
        ("ǅ HELLO! ǈ WORLD.mkv", "ǅ hello! ǈ world.mkv"),
        ("ǅ ΟΣ.mkv", "ǅ ος.mkv"),
        ("影像 hello WORLD.mkv", "影像 Hello world.mkv"),
        ("ß HELLO.mkv", "Ss hello.mkv"),
    ];
    for (input, expected) in rows {
        assert_eq!(
            run(input, SegmentSelector::Stem, case(CaseMode::Sentence))
                .unwrap()
                .filename(),
            expected,
            "sentence case for {input:?}"
        );
    }
    assert_eq!(
        run(
            "ǅ HELLO iii.mkv",
            SegmentSelector::Stem,
            TextOperation::Case {
                mode: CaseMode::Sentence,
                exceptions: vec!["ǅ".into()],
                roman: Some(RomanCase::Upper),
            },
        )
        .unwrap()
        .filename(),
        "ǅ hello III.mkv"
    );
    assert_eq!(
        run("ǅ hello.mkv", SegmentSelector::Stem, case(CaseMode::Title))
            .unwrap()
            .filename(),
        "ǅ Hello.mkv"
    );
    assert_eq!(
        run("ǅ hello.mkv", g(1, 1), case(CaseMode::Sentence))
            .unwrap()
            .filename(),
        "ǅ hello.mkv"
    );
}
