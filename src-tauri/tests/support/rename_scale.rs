//! Owning desktop benchmark: metadata only; synthetic plans never execute.
//! Included inside commands::rename to exercise its real authorized page API.
use super::*;
use std::{io::Write, time::Instant};

fn seed(count: usize, fixture: Option<&std::path::Path>) -> (tempfile::TempDir, AppState, PlanRef) {
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("media");
    std::fs::create_dir(&media).unwrap();
    std::fs::write(media.join("seed.mkv"), b"x").unwrap();
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    crate::selection::install_selection(&state, vec![media.clone()]).unwrap();
    fn wait(state: &AppState, id: JobId) -> Value {
        let deadline = Instant::now() + std::time::Duration::from_secs(120);
        loop {
            assert!(Instant::now() < deadline, "seed preview timeout");
            let result = status_owned(state, JobRequest { job_id: id }).unwrap();
            if result.state != "running" {
                assert_eq!(result.state, "completed");
                return result.result.unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    let selected = lock(&state).unwrap().inventory.as_ref().unwrap().entries[0].id;
    let request =
        serde_json::from_value(json!({"revision":"2","selectedIds":[selected],"rule":"keep"}))
            .unwrap();
    let result = wait(&state, preview_owned(&state, request).unwrap());
    let reference: UiPlanRef = serde_json::from_value(result["planRef"].clone()).unwrap();
    let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
    let mut plan = (*plans(&state)
        .unwrap()
        .snapshot(reference.into(), &lease)
        .unwrap())
    .clone();
    let sample_entry = plan.execution.provenance.inventory.entries[0].clone();
    let sample_row = plan.rows[0].clone();
    plan.version.revision += 1;
    plan.execution.provenance.approved_plan = plan.version;
    plan.rows.clear();
    plan.execution.groups.clear();
    plan.execution.provenance.inventory.entries.clear();
    let mut writer =
        fixture.map(|path| std::io::BufWriter::new(std::fs::File::create(path).unwrap()));
    for i in 0..count {
        let id = serde_json::from_value::<EntryId>(json!(format!(
            "00000000-0000-0000-0000-{:012x}",
            i + 1
        )))
        .unwrap();
        let name = format!("movie-{i:06}.mkv");
        let mut entry = sample_entry.clone();
        entry.id = id;
        entry.absolute_path = media.join(&name);
        entry.display_name = name.clone();
        entry.display_path = entry.absolute_path.to_string_lossy().into_owned();
        entry.relative_path = vidlib_inventory::encode_path(std::path::Path::new(&name));
        entry.stamp.size = Some(9_007_199_254_740_993 + i as u64);
        entry.stamp.modified_ns = Some(1_600_000_000_000_000_000 + i as i128);
        let mut row = sample_row.clone();
        row.entry_id = id;
        row.primary_id = id;
        row.group_id = GroupId::from_uuid(*id.as_uuid());
        row.source_path = entry.absolute_path.clone();
        row.target_path = entry.absolute_path.clone();
        row.old_display_name = name.clone();
        row.new_display_name = name.clone();
        row.old_display_path = entry.display_path.clone();
        row.new_display_path = entry.display_path.clone();
        row.stamp = entry.stamp.clone();
        // Projection-only effects also work while native mutation gates are closed.
        // The synthetic plan is never executed or used as filesystem authority.
        row.status = RowStatus::Changed;
        let group = ExecutionGroup {
            id: row.group_id,
            depends_on: vec![],
            steps: vec![ExecutionStep {
                id: StepId::from_uuid(*id.as_uuid()),
                entry_id: id,
                group_id: row.group_id,
                action: ExecutionAction::SetMetadata {
                    entry: EntryPrestate {
                        path: row.source_path.clone(),
                        identity: ExpectedIdentity::Observed(row.stamp.clone()),
                    },
                    changes: vec![MetadataChange::Timestamp {
                        field: TimestampField::Modified,
                        value_ns: 1_600_000_000_000_000_000,
                    }],
                },
            }],
        };
        if let Some(writer) = &mut writer {
            writeln!(writer, "{}", json!({"index":i.to_string(),"name":name,"sizeBytes":entry.stamp.size.unwrap().to_string(),"modifiedNs":entry.stamp.modified_ns.unwrap().to_string()})).unwrap();
        }
        plan.rows.push(row);
        plan.execution.groups.push(group);
        plan.execution.provenance.inventory.entries.push(entry);
    }
    if let Some(writer) = &mut writer {
        writer.flush().unwrap();
    }
    // Deterministic scrambled input avoids measuring only already-sorted runs.
    plan.rows.sort_by_key(|row| {
        (row.entry_id.as_uuid().as_u128() as u64).wrapping_mul(0x9e3779b97f4a7c15)
    });
    plan.counts = PlanCounts::from_rows(&plan.rows).unwrap();
    let reference = plan.version;
    plans(&state).unwrap().publish(plan, &lease).unwrap();
    drop(lease);
    (dir, state, reference)
}

#[test]
fn scale_projection_preserves_exact_metadata_and_page_bound() {
    let (_dir, state, reference) = seed(257, None);
    let mut query = PageQuery::new(2);
    query.limit = 1000;
    let first = page_owned(
        &state,
        PageRequest {
            plan_ref: reference.into(),
            query: query.clone(),
        },
    )
    .unwrap();
    assert_eq!(first.items.len(), 200);
    assert_eq!(first.items[0]["sizeBytes"], "9007199254740993");
    assert_eq!(
        first.items[0]["metadata"][0]["requested"][0]["valueNs"],
        "1600000000000000000"
    );
    query.cursor = first.next_cursor;
    let last = page_owned(
        &state,
        PageRequest {
            plan_ref: reference.into(),
            query,
        },
    )
    .unwrap();
    assert_eq!(last.items.len(), 57);
    assert_eq!(last.items[56]["oldName"], "movie-000256.mkv");
    assert_eq!(
        last.items[56]["metadata"][0]["original"]["modifiedNs"],
        "1600000000000000256"
    );
}

#[test]
fn cached_natural_keys_preserve_numeric_ties_unicode_and_search() {
    let rows = [
        ("épisode10", "d"),
        ("épisode2", "c"),
        ("épisode02", "b"),
        ("épisode2", "a"),
        ("other1", "e"),
    ];
    let mut query = PageQuery::new(7);
    query.sort = "natural".into();
    query.filter = "ÉPISODE".into();
    let page = page_items(
        rows.iter().collect(),
        &query,
        7,
        |row, _| (row.0.into(), row.1.into()),
        |row| json!({"name":row.0,"id":row.1}),
    )
    .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c", "d"]
    );
    query.expected_revision = 6;
    assert!(page_items(
        rows.iter().collect(),
        &query,
        7,
        |row, _| (row.0.into(), row.1.into()),
        |row| json!(row.0)
    )
    .is_err());
}

#[test]
#[ignore = "explicit metadata-only scale benchmark; run tools/benchmark-library.ps1"]
fn benchmark_metadata_library() {
    let count = std::env::var("VIDLIB_BENCHMARK_ENTRIES")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(100_000usize);
    assert!((1..=100_000).contains(&count));
    let path = std::env::var_os("VIDLIB_BENCHMARK_FIXTURE").map(std::path::PathBuf::from);
    let started = Instant::now();
    let (_dir, state, reference) = seed(count, path.as_deref());
    println!(
        "BENCH fixture entries={count} setup_ms={}",
        started.elapsed().as_millis()
    );
    for sort in ["name", "natural", "path", "size", "status"] {
        for iteration in 0..2 {
            let mut query = PageQuery::new(2);
            query.sort = sort.into();
            if iteration == 1 {
                query.filter = "movie-09".into();
            }
            let started = Instant::now();
            let page = page_owned(
                &state,
                PageRequest {
                    plan_ref: reference.into(),
                    query,
                },
            )
            .unwrap();
            assert!(page.items.len() <= 200);
            if iteration == 0 {
                assert_eq!(page.items.len(), count.min(200));
            }
            if count == 100_000 {
                assert_eq!(page.items.len(), 200);
            }
            println!(
                "BENCH page entries={count} sort={sort} filtered={} rows={} elapsed_us={}",
                iteration == 1,
                page.items.len(),
                started.elapsed().as_micros()
            );
        }
    }
    let lease = state.coordinator.try_acquire(LeaseKind::Read).unwrap();
    let plan = plans(&state).unwrap().snapshot(reference, &lease).unwrap();
    let inventory = &plan.execution.provenance.inventory;
    let ids = inventory.entries.iter().map(|e| e.id).collect::<Vec<_>>();
    let requirements = InputRequirements {
        primary_order: ids.clone(),
        numbering_scope: Default::default(),
        timezone: FrozenTimezone::Utc,
        fields: ids
            .iter()
            .map(|id| RequestedInput {
                scope: FrozenScope::Rule,
                entry_id: *id,
                rule_id: Some("token".into()),
                key: FieldKey::Uuid,
            })
            .collect(),
    };
    let mut cache = FrozenInputCache::default();
    let mut first = None;
    for iteration in 0..2 {
        let started = Instant::now();
        let inputs = cache
            .prepare(&requirements, inventory, &JobControl::default())
            .unwrap();
        println!(
            "BENCH frozen entries={count} iteration={iteration} values={} elapsed_us={}",
            inputs.values.len(),
            started.elapsed().as_micros()
        );
        if let Some(first) = &first {
            assert_eq!(&inputs.values, first);
        }
        first = Some(inputs.values);
    }
    let mut recipe = plan.execution.provenance.recipe.clone();
    recipe.selection = ids.clone();
    recipe.numbering_order = ids.clone();
    recipe.metadata_changes.clear();
    recipe.rules = vec![Rule {
        id: "cpu".into(),
        enabled: true,
        segment: SegmentSelector::Stem,
        parameters: RuleParameters::Regex {
            rule: RegexRule {
                pattern: "movie".into(),
                replacement: "Film".into(),
                scope: MatchScope::All,
                case_sensitive: true,
            },
        },
    }];
    if count == 100_000 {
        let bytes = serde_json::to_vec(&recipe).unwrap().len();
        let error = InputRequirements::from_recipe(&recipe, FrozenTimezone::Utc).unwrap_err();
        assert_eq!(error.code, RenameErrorCode::RecipeTooLarge);
        println!("BENCH full_preview_boundary entries={count} recipe_bytes={bytes} refusal={:?}; projection fixture is not a full preview pass", error.code);
    }
    let inputs = FrozenInputs {
        inventory_id: inventory.id,
        generation: inventory.generation,
        now_ns: 0,
        timezone: FrozenTimezone::Utc,
        primary_order: ids,
        numbering_scope: Default::default(),
        values: vec![],
    };
    let (control, progress) = JobControl::with_progress(128);
    let started = Instant::now();
    let evaluated =
        vidlib_rename::evaluate::evaluate_controlled(inventory, &recipe, &inputs, &control)
            .unwrap();
    let elapsed = started.elapsed();
    let updates = progress.try_iter().count();
    assert_eq!(evaluated.rows.len(), count);
    assert!(evaluated
        .rows
        .iter()
        .all(|row| row.status == RowStatus::Changed));
    assert!(updates <= (elapsed.as_millis() / 100 + 1) as usize);
    println!("BENCH cpu entries={count} elapsed_us={} progress_updates={updates} max_updates_per_second=10", elapsed.as_micros());
}

#[test]
#[ignore = "explicit actual native preview benchmark, 1000 temporary one-byte files"]
fn benchmark_native_preview() {
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("media");
    std::fs::create_dir(&media).unwrap();
    for i in 0..1000 {
        std::fs::write(media.join(format!("movie-{i:06}.mkv")), b"x").unwrap();
    }
    let state = AppState::open(&dir.path().join("app"), dir.path().join("libraries.json")).unwrap();
    crate::selection::install_selection(&state, vec![media]).unwrap();
    fn wait(state: &AppState, job: JobId) -> Value {
        let deadline = Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let status = status_owned(state, JobRequest { job_id: job }).unwrap();
            if status.state != "running" {
                assert_eq!(status.state, "completed", "{status:?}");
                return status.result.unwrap();
            }
            assert!(Instant::now() < deadline, "native preview timeout");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    let started = Instant::now();
    wait(
        &state,
        inventory_owned(&state, InventoryRequestDto { revision: 1 }).unwrap(),
    );
    println!(
        "BENCH native inventory entries=1000 elapsed_us={}",
        started.elapsed().as_micros()
    );
    let ids = lock(&state)
        .unwrap()
        .inventory
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|e| e.id)
        .collect::<Vec<_>>();
    let request =
        serde_json::from_value(json!({"revision":"2","selectedIds":ids,"rule":"keep"})).unwrap();
    let started = Instant::now();
    let result = wait(&state, preview_owned(&state, request).unwrap());
    println!(
        "BENCH native preview entries=1000 elapsed_us={}",
        started.elapsed().as_micros()
    );
    assert_eq!(result["counts"]["unchanged"], "1000");
    let reference = serde_json::from_value(result["planRef"].clone()).unwrap();
    let started = Instant::now();
    let page = page_owned(
        &state,
        PageRequest {
            plan_ref: reference,
            query: PageQuery::new(2),
        },
    )
    .unwrap();
    assert_eq!(page.items.len(), 200);
    println!(
        "BENCH native page entries=1000 rows=200 elapsed_us={}",
        started.elapsed().as_micros()
    );
}
