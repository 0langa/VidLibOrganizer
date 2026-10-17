use vidlib_rename::*;

fn regex(pattern: &str, replacement: &str) -> RegexRule {
    RegexRule {
        pattern: pattern.into(),
        replacement: replacement.into(),
        scope: MatchScope::All,
        case_sensitive: true,
    }
}
fn run(pattern: &str, replacement: &str, input: &str) -> String {
    CompiledRegexRule::compile(&regex(pattern, replacement))
        .unwrap()
        .apply(input)
        .unwrap()
}
// Catch loss of PCRE-only syntax, captures, and Unicode mode.
#[test]
fn lookarounds_pattern_backrefs_and_capture_replacement() {
    assert_eq!(
        run(
            r"(?<=Show )(?<season>\d\d)x(\d\d)(?= END)",
            "- S${season}E$2",
            "Show 02x03 END"
        ),
        "Show - S02E03 END"
    );
    assert_eq!(run(r"^(\w+)\s+\1$", "$1", "Show Show"), "Show");
    assert_eq!(run(r"^\w+$", "yes", "影像é"), "yes");
}
#[test]
fn literal_dollar_and_unmatched_optional_capture() {
    assert_eq!(run(r"^(a)?(b)$", "$$-$1-${2}-$0", "b"), "$--b-b");
    assert_eq!(run(r"(x)", r"\$1$$", "x"), r"\x$");
}
#[test]
fn first_all_and_case_modes() {
    for (scope, case_sensitive, expected) in [
        (MatchScope::First, true, "A-x-a"),
        (MatchScope::All, true, "A-x-x"),
        (MatchScope::First, false, "x-a-a"),
        (MatchScope::All, false, "x-x-x"),
    ] {
        let mut rule = regex("a", "x");
        rule.scope = scope;
        rule.case_sensitive = case_sensitive;
        assert_eq!(
            CompiledRegexRule::compile(&rule)
                .unwrap()
                .apply("A-a-a")
                .unwrap(),
            expected
        );
    }
    assert_eq!(run("z", "x", "é"), "é");
}
#[test]
fn empty_matches_progress_without_losing_nonempty_alternative() {
    for (pattern, input, want) in [
        ("", "é🦀", "XéX🦀X"),
        ("(?:|a)", "a", "XXX"),
        ("a*", "a", "XX"),
        ("$", "abc", "abcX"),
        ("(?=b)", "ab", "aXb"),
        ("^|$", "", "X"),
    ] {
        assert_eq!(run(pattern, "X", input), want, "{pattern:?} on {input:?}");
    }
}
#[test]
fn invalid_references_and_patterns_are_recipe_errors() {
    for (pattern, replacement) in [
        ("(a)", "$2"),
        ("(?<a>x)", "${missing}"),
        ("a", "$"),
        ("a", "$x"),
        ("a", "${a"),
        ("a", "${}"),
        ("a", "$99999999999999999999999"),
        ("(", "x"),
        (r"\C", "x"),
    ] {
        assert!(
            CompiledRegexRule::compile(&regex(pattern, replacement)).is_err(),
            "accepted {pattern:?} {replacement:?}"
        );
    }
    assert!(CompiledRegexRule::compile(&regex(&"é".repeat(8193), "x")).is_err());
    assert!(CompiledRegexRule::compile(&regex(&"a".repeat(16384), "x")).is_ok());
}
// Actual native exhaustion, isolated so a broken limit cannot hang the test runner.
// The deadline only diagnoses a hung test; it is not an engine timeout contract.
#[test]
fn resource_limits_are_real() {
    const CHILD: &str = "VIDLIB_REGEX_LIMIT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let limits = REGEX_LIMITS;
        assert_eq!(limits.match_limit, 1_000_000);
        assert_eq!(limits.depth_limit, 1000);
        assert_eq!(limits.heap_limit_kib, 8192);
        assert_eq!(limits.pattern_bytes, 16384);
        let compiled = CompiledRegexRule::compile(&regex(r"(*NO_START_OPT)(a+)+$", "x")).unwrap();
        let exhausted = compiled.apply(&format!("{}!", "a".repeat(30))).unwrap_err();
        assert_eq!(exhausted.code, RuleIssueCode::RegexLimit);
        assert_eq!(
            exhausted.native_code,
            Some(pcre2_sys::PCRE2_ERROR_MATCHLIMIT)
        );
        let depth = CompiledRegexRule::compile(&regex(r"^(a(?1)?b)$", "x"))
            .unwrap()
            .apply(&format!("{}{}", "a".repeat(600), "b".repeat(600)))
            .unwrap_err();
        assert_eq!(depth.native_code, Some(pcre2_sys::PCRE2_ERROR_DEPTHLIMIT));
        let heap_pattern = format!("^(a(?1)?b){}$", "()".repeat(1000));
        let heap = CompiledRegexRule::compile(&regex(&heap_pattern, "x"))
            .unwrap()
            .apply(&format!("{}{}", "a".repeat(400), "b".repeat(400)))
            .unwrap_err();
        assert_eq!(heap.native_code, Some(pcre2_sys::PCRE2_ERROR_HEAPLIMIT));
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "resource_limits_are_real", "--nocapture"])
        .env(CHILD, "1")
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "native resource fixture failed: {status}");
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(15) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native fixture hung beyond harness deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn rule(id: &str, pattern: &str, replacement: &str, segment: SegmentSelector) -> Rule {
    Rule {
        id: id.into(),
        enabled: true,
        segment,
        parameters: RuleParameters::Regex {
            rule: regex(pattern, replacement),
        },
    }
}
fn fixture(
    names: &[&str],
    rules: Vec<Rule>,
) -> (
    tempfile::TempDir,
    vidlib_inventory::Inventory,
    RecipeV1,
    FrozenInputs,
) {
    use vidlib_inventory::*;
    let dir = tempfile::tempdir().unwrap();
    for name in names {
        std::fs::write(dir.path().join(name), b"media").unwrap();
    }
    let inventory = scan(
        &InventoryRequest::new(vec![RootBinding::new(dir.path().to_owned()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let ids = inventory
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::File)
        .map(|e| e.id)
        .collect::<Vec<_>>();
    let recipe = RecipeV1 {
        schema_version: 1,
        revision: 1,
        selection: ids.clone(),
        selection_filter: None,
        numbering_order: ids.clone(),
        numbering_scope: Default::default(),
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
        primary_order: ids,
        numbering_scope: Default::default(),
        values: vec![],
    };
    (dir, inventory, recipe, inputs)
}
#[test]
fn disabled_and_repeated_rules() {
    let mut disabled = rule("disabled", "(", "$99", SegmentSelector::Stem);
    disabled.enabled = false;
    let rules = vec![
        disabled,
        rule("first", "a", "aa", SegmentSelector::Stem),
        rule("second", "a", "aa", SegmentSelector::Stem),
    ];
    let (_dir, inventory, recipe, inputs) = fixture(&["a.MKV", "ba.MKV"], rules);
    let recipe = RecipeV1::from_json(&serde_json::to_vec(&recipe).unwrap()).unwrap();
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    for row in batch.rows {
        assert_eq!(
            row.new_display_name,
            if row.old_display_name == "a.MKV" {
                "aaaa.MKV"
            } else {
                "baaaa.MKV"
            }
        );
        assert_eq!(row.explanations.len(), 2);
        assert!(row.source_path.exists());
    }
}
#[test]
fn current_segments_and_explicit_extensions_use_shared_reassembly() {
    let rules = vec![
        rule(
            "grow",
            "one",
            "longer",
            SegmentSelector::Between {
                target: NameSegment::Stem,
                opening: "[".into(),
                closing: "]".into(),
                occurrence: 1,
                include_delimiters: false,
            },
        ),
        rule(
            "second",
            "two",
            "TWO",
            SegmentSelector::Between {
                target: NameSegment::Stem,
                opening: "[".into(),
                closing: "]".into(),
                occurrence: 2,
                include_delimiters: false,
            },
        ),
        rule("ext", "MKV", "mkv", SegmentSelector::Extension),
    ];
    let (_dir, inventory, recipe, inputs) = fixture(&["A [one] B [two] C.MKV"], rules);
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    let row = &batch.rows[0];
    assert_eq!(row.new_display_name, "A [longer] B [TWO] C.mkv");
    assert_eq!(row.explanations[1].before, "A [longer] B [two] C");
    assert_eq!(row.old_display_name, "A [one] B [two] C.MKV");
}
#[test]
fn failed_regex_restores_original_target_and_keeps_prior_explanation() {
    let rules = vec![
        rule("prior", "^", "prefix", SegmentSelector::Stem),
        rule("bad", r"(*NO_START_OPT)(a+)+$", "x", SegmentSelector::Stem),
    ];
    let (_dir, inventory, recipe, inputs) =
        fixture(&["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!.mkv", "ok.mkv"], rules);
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    let row = batch
        .rows
        .iter()
        .find(|r| r.old_display_name.starts_with('a'))
        .unwrap();
    assert_eq!(row.status, RowStatus::Blocked);
    assert_eq!(row.issues[0].code, RenameErrorCode::RegexLimit);
    assert_eq!(row.issues[0].rule_id.as_deref(), Some("bad"));
    assert_eq!(row.target_path, row.source_path);
    assert_eq!(row.new_display_name, row.old_display_name);
    assert_eq!(row.explanations.len(), 1);
    assert_eq!(
        batch
            .rows
            .iter()
            .find(|r| r.old_display_name == "ok.mkv")
            .unwrap()
            .new_display_name,
        "prefixok.mkv"
    );
}
#[test]
fn invalid_replacement_is_rejected_even_without_selected_rows() {
    let (_dir, inventory, recipe, inputs) =
        fixture(&[], vec![rule("bad", "(a)", "$2", SegmentSelector::Stem)]);
    assert_eq!(
        evaluate(&inventory, &recipe, &inputs).unwrap_err().code,
        RenameErrorCode::InvalidRecipe
    );
    assert_eq!(
        RecipeV1::from_json(&serde_json::to_vec(&recipe).unwrap())
            .unwrap_err()
            .code,
        RenameErrorCode::InvalidRecipe
    );
}
#[test]
fn native_errors_are_not_nonmatches() {
    let error = CompiledRegexRule::compile(&regex("(?R)", "x"))
        .unwrap()
        .apply("a")
        .unwrap_err();
    assert_eq!(error.code, RuleIssueCode::RegexNative);
    assert_eq!(error.native_code, Some(pcre2_sys::PCRE2_ERROR_RECURSELOOP));
}

#[test]
fn named_optional_duplicate_names_and_adjacent_literals() {
    assert_eq!(run(r"^(?<optional>a)?b$", "${optional}$$1", "b"), "$1");
    assert_eq!(
        run(r"(?J)(?<letter>a)|(?<letter>b)", "${letter}${letter}", "ab"),
        "aabb"
    );
    assert_eq!(
        run(
            "(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)",
            "$10-$1x-${2}0",
            "abcdefghij"
        ),
        "j-ax-b0"
    );
    assert_eq!(run("(x)", "$$$1$$$$", "x"), "$x$$");
    assert_eq!(run("a", "long replacement", "a"), "long replacement");
}

#[test]
fn generated_components_are_validated_without_sanitization() {
    let (_dir, inventory, recipe, inputs) = fixture(
        &["original.MKV"],
        vec![rule(
            "bad",
            "original",
            "dir/name",
            SegmentSelector::WholeName,
        )],
    );
    let batch = evaluate(&inventory, &recipe, &inputs).unwrap();
    let row = &batch.rows[0];
    assert_eq!(row.status, RowStatus::Blocked);
    assert_eq!(row.target_path, row.source_path);
    assert_eq!(row.issues[0].code, RenameErrorCode::InvalidRuleInput);
    assert_eq!(row.issues[0].rule_id.as_deref(), Some("bad"));
}
