# Rename verification — Task 23

**Release gate CLOSED.** Portable Rename implementation through Task23 is the phase boundary; native Windows qualification and independent final review are separate. Blueprint Tasks24–32 have not been implemented. Do not infer Windows runtime behavior from GNU cross-compilation, Chromium fixtures, Linux subprocess crashes, or an NSIS source harness. See [the resumable handoff](rename-handoff.md) for retained decisions and deferred findings.

## Evidence and reproduction

Worktree `/workspace/VidLibOrganizer/.worktrees/rename-blueprint`, branch `feat/rename-blueprint`; Task23 base `e6f17339e19bd9cd698d73799abc242ead4ffe9c`. Run from the repository root. Cargo.lock and package-lock.json are committed. Verification uses Rust **1.99.1** (58cf8f9c3, 2026-10-13), Node **24.19.0**, Playwright **1.58.2**. CI pins tool versions and action commits; hosted runner image/native SDK packages remain runner-provided and CI has not run in this local-only session.

Standard supported native development environment:

```sh
cargo test --locked --workspace --no-fail-fast
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm ci
npm test
npx playwright test
python3 -m unittest discover -s tools/tests -v
python3 tools/vendor_media_info.py --check
cargo build --locked --release -p vidlib-tauri
```

`npm test` already runs the Node reducer cases and Playwright suite; a separate Playwright invocation is only needed when verifying that suite independently. Syntax-check every `ui/**/*.js` with `node --check`. The Windows CI job performs native workspace tests, Clippy, formatting, release binary build, helper verification and browser development tests without unconditional success or skipped failures. It does not ship or open production capabilities. The Linux job excludes `vidlib-tauri` to keep portable core checks independent of WebKitGTK/Tauri dependencies.

In the current cloud environment, source `/workspace/.tools/desktop-env.sh` and set `CARGO_TARGET_DIR=/workspace/.tools/desktop-target CARGO_INCREMENTAL=0` for Linux desktop/core Cargo commands. A fresh shell sourcing `/workspace/.tools/windows-check-env.sh`, with `CARGO_TARGET_DIR=$PWD/target CARGO_INCREMENTAL=0`, runs `cargo check --locked --workspace --all-targets --target x86_64-pc-windows-gnu`. These local SDK caches are convenience setup, not portable release evidence. Fresh machines need Rust, C/C++ tooling, SQLite/PCRE2 dependency builds, and on Linux WebKitGTK4.1/GTK3 development libraries. Windows needs MSVC C++ Build Tools/Windows SDK. Install Playwright development Chromium with `npx playwright install chromium` (Linux may need `--with-deps`); `CHROMIUM_PATH` can explicitly select an installed browser. The cloud uses `/usr/lib/chromium/chromium` and `env -u NO_COLOR npm test` to avoid an inherited color-variable warning.

Raw local logs: `.superpowers/sdd/2026-10-17-rename-and-blueprint/task23-logs/`. This directory is intentionally ignored; durable conclusions, exact commands and measurements are retained here. Parent/child subprocess harness totals are not added twice.

## R01–R18 matrix

Automated rows refer to the owning Rust integration file under `crates/vidlib-rename/tests/` unless another owner is named. All rows also retain the native installation/storage prerequisites below; portable behavior does not make a Windows acceptance row complete.

| Requirement | Automated behavior evidence | Expected result / native pending acceptance |
| --- | --- | --- |
| R01 text insertion/replacement/removal/crop | `text.rs::text_rule_goldens`, `invalid_operations_never_succeed_as_noops`; UI `rename-rules.spec.mjs` | Ordered literal operations preserve the untargeted extension; invalid positions block the row. |
| R02 copy/move/reverse/translation | `text_rule_goldens`, `unicode_graphemes_are_not_split`; new `repeated_current_segment_copy_refuses_resource_growth` | Explicit grapheme operations only; transliteration opt-in; resource refusal before excessive growth. |
| R03 Unicode case/exceptions/Roman | `case_exceptions_and_roman_are_explicit`, `sentence_case_consumes_titlecase_letters_and_preserves_context` | Exceptions/Roman choice respected. Deferred dotted-I spelling item07 remains separately tracked. |
| R04 interpreted PCRE2 | `regex.rs` complete suite; new `regex_replacement_refuses_oversized_native_output_buffer` | Captures/lookarounds/pattern backreferences; first/all; no JIT; match1,000,000/depth1,000/heap8MiB/pattern16KiB. Typed exhaustion, no silent fallback. |
| R05 current-input selectors/order | `segment_resolves_each_current_input`, `delimiters_are_flat_literal_grapheme_aligned_and_current`, `evaluator_preserves_source_target_after_a_later_rule_fails` | Repeated/reordered enabled rules use current text; failure retains original target and prior bounded explanations. |
| R06 numbering | `numbers_dates.rs`; `selection_mappings.rs::mixed_kinds_have_explicit_counter_scope_and_cache_recomputes_scope_switch`, `number_rule_skips_other_kind_and_companions_do_not_consume_ordinals` | Frozen explicit primary vector; file/directory scope separate; companions inherit without consuming counters; display sort does not reorder assignment. |
| R07 dates | `numbers_dates.rs`; `templates_hashes.rs::date_sources_are_distinct_and_unknown_recording_zone_is_not_optional_missing`; `date_formatter_stops_before_oversized_expansion` | No ambiguous-format/timezone guessing; now frozen; exact timestamps. Deferred significant >9-digit RFC3339 fractions item08 remains. |
| R08 templates | `templates_hashes.rs` required/optional/fallback, original/current, scope and typed-value tests; new repeated CurrentStem growth test | Missing required field blocks; optional missing group disappears; integer/decimal formatting exact; bounded output. |
| R09 hashes/generated tokens | `full_file_hashes_include_bytes_after_first_mebibyte_and_share_one_read`, `all_requested_algorithms_share_one_full_read_and_cached_hashes_recheck_source`, scoped hash tests and cache retention tests | Full requested file hashes only; generated values frozen; cache hits revalidate source binding, never grant native authority. |
| R10 stem/extension/whole name | `extension_requires_explicit_mode`, `whole_name_is_validated_before_splitting_and_noops_preserve_spelling` | Extension changes explicit and are names only, never conversion. |
| R11 recursive/filter selection | inventory tests; `selection_mappings.rs::every_filter_dimension`, `filters_deduplicate_overlapping_roots_and_preserve_order`, `invalid_and_exhausted_filter_expressions_report_errors` | Depth/kind/pattern/size/date/attribute filters; overlaps deduplicated; selection edits re-evaluate all selected targets. |
| R12 mappings/overrides/export | `selection_mappings.rs` import boundary/ambiguity/native path/hidden collision/export cases; desktop `mapping_import_export_and_display_queries_use_current_native_plan` |64MiB/100,000-row cap; exact mappings stay within native grants; spreadsheet-safe export does not change authority. |
| R13 presets/recipe edit history | `presets.rs`; UI `recipe-editor.spec.mjs` | Versioned strict1MiB/100-rule recipe import; immutable preset revisions; precommit failure preserves current, postcommit flush uncertainty requires reload. |
| R14 companions | `companions.rs`; fileops companion compensation cases; UI selected ownership/group exclusion | None/manual Explicit/opt-in MatchExact remain distinct; ambiguity warns unchanged; selected-only authority, primary counters and dependency-group compensation. |
| R15 folders/copy/move/layout | `graph.rs` parent/child/cycle/carry tests; fileops `copy_move.rs`51 cases; Task22 `entrypoint.rs` native root/overlap/execution/Undo/crash cases | Explicit native destination grants; durable created staging identity then write/hash/flush/publish/remove; ordered one-to-many inverse lineage and complete namespace proofs. Windows volume/sharing/runtime PENDING. |
| R16 attributes/timestamps | fileops `metadata_actions.rs`, rename `metadata.rs`, strict desktop controls and UI metadata journeys | Keep default; exact Set/Offset/Increment; original vs factual pre-set vs requested vs observed state distinct; controlled fields survive destructive Undo/recovery. Windows4flags/3times/precision/ACL/read-only native PENDING. |
| R17 review/history/recovery/Undo | desktop `command_contracts.rs`; UI `rename-workflows.spec.mjs`, `rename-basic.spec.mjs`; fileops `journal.rs`, `recovery.rs`, `undo.rs`; new `rename-verification.spec.mjs` |200-row cap; exact quantities; hidden conflicts block; stale pages/nonces rejected; review/export/history; refusal visible and Apply disabled; recovery evidence never synthesized by UI. |
| R18 native entry/Explorer/keyboard | desktop `entrypoint.rs`; UI `native-selection.spec.mjs`; NSIS hook compilation harness | Picker/drop/argv remain Rust-owned IDs and inactive review; cancellation preserves prior plan; current library policy protects roots. Real Windows dialogs/WebView2/Explorer/multi-instance/opt-in/uninstall PENDING. |

## Safety and fault matrix

| Contract / boundary | Automated evidence | Required outcome |
| --- | --- | --- |
| Native no-replace, identities, links and invalid paths | inventory `path_codec.rs`/`inventory.rs`; fileops `platform.rs`, rename `graph.rs` native-policy cases | Source/foreign target bytes preserved on collision/replacement; no lossy display-string reconstruction, sibling grant or reparse traversal. |
| Global conflicts/stale plans | `graph.rs::hidden_page_conflict_blocks_entire_approval`, `target_created_after_preview_fails_pure_preflight`; desktop revision/confirmation cases | Complete selected set validated; changed inventory/recipe/order/selection/epochs/restart revokes authority. |
| WAL/FULL journal before mutation | `journal.rs::intent_survives_reopen`, `receipt_and_event_commit_together`, `commit_error_prevents_next_step`, `failed_filesystem_durability_records_fact_and_stops_next_intent` | Durable intent first; immutable receipt/outbox together; failure prevents next effect, applied-but-unflushed facts remain recoverable. Real power-loss durability still PENDING. |
| Prepared / every rename intent-effect-receipt gap | `recovery.rs::prepared_process_crash_resumes_without_invented_effects`, `crash_before_and_after_each_phase_reconciles`, `temporary_names_are_reported` | Actual child exit86; repeat read-only inspection stable; temporary reservations visible; Resume exactly once. |
| Every inverse rename boundary / pinned Restore | `crashes_during_restore_do_not_repeat_an_inverse`, `restore_scope_survives_final_gap_receipt_before_inverse_intent`, completed-group Restore cases | No repeated inverse, immutable historical facts, current partial group only, completed earlier groups preserved. |
| CreateStaging / WriteStaging / publish / remove source / cleanup | `copy_move.rs` actual subprocess phase/gap/Resume/Restore tests; `recovery.rs::every_persisted_phase_reports_evidence_or_explicit_unknown_ownership` | Created identity durably captured before bulk writes; intent digest differs from verified digest; full-content verification and owned quarantine cleanup; immutable legacy StageCopy unknown ownership remains unresolved. |
| Folder creation, child effects, source-directory quarantine/removal and inverse cleanup | `copy_move.rs::folder_directory_quarantine_receipt_gap_restores`, `folder_inverse_directory_removal_receipt_gap_restores`, folder Resume tests; `directory_after_effects_rename_gap_requires_complete_surviving_namespace` | Parent creation before child copy-back; original-directory metadata after all children; all required inverse effects finish before compensation; unknown/nonempty foreign entries survive refusal. |
| Metadata prestate+intent / setter / receipt / inverse/Undo | `metadata_actions.rs::metadata_process_crashes_resume_and_restore_each_field_boundary`, `metadata_undo_process_crashes_resume_or_restore_immutable_history`, directory native crash tests, controlled quarantine crash cases | Factual first prestate immutable and bound before setter, distinct original restoration target; no recovery from invented post-effect before-state. Legacy dynamic gaps remain unresolved. |
| Concurrent instances / journal writer contention | `coordinator.rs`; `journal.rs::busy_writer_preserves_pending_operation`; Task22 actual producer subprocesses; new `real_crash_gap_keeps_concurrent_instances_read_only_until_reviewed_resume` | Recovery owns exclusive gate; a second instance cannot write during pending recovery or take its lease; after successful reviewed Resume writer access returns. |
| Unknown target/child, disconnected drive, stale/replaced object | `unknown_target_prevents_blind_resume`, `compensation_stops_without_deleting_unowned_entries`, `unowned_child_blocks_directory_material_refresh`, `lost_drive_is_unresolved_and_reconnect_requires_fresh_review` | Refusal preserves foreign bytes, existing receipts and pending recovery, never blind retry/delete. |
| Companion group failure/cancel | `companions.rs`, fileops rename group/cancellation cases and copy/move faults | Whole partial dependency group restored only with fresh evidence; completed earlier groups retained; uncertainty remains RecoveryRequired. |
| Markers and current library protected roots | `graph.rs::root_and_marker_protected`, `root_guards_do_not_exclude_normal_library_contents`; Task22 protection/config epoch/forged receipt/unknown sibling tests | No marker created by scan; guarded root or carrying ancestor mutation refused; ordinary descendant work and read-only diagnostics remain permitted. |
| Helper failure/hang/cancel | desktop `metadata_helper.rs`; `metadata.rs::helper_failure_is_clear_and_explicit_fallbacks_still_work`, transient retry/cache cases | Cancellable child helper,10s per-file deadline, one active probe per selected drive; bounded typed failures, no invented fields or runtime network download. Native property/MediaInfo execution PENDING. |
| CPU cancellation/progress | desktop `desktop_cpu_preview_reports_between_items_and_can_cancel`, `desktop_cpu_selection_reports_between_items_and_can_cancel`; regex once-per-batch tests; scale benchmark | Real1200-file/100-rule CPU work reports midbatch then cancels between items; throttle<=10updates/s, not just post-batch cancellation or helper I/O. |
| Generated output resource budget | new text/regex/template/number/date tests; existing transformation goldens |65,536 UTF-8 bytes per generated intermediate filename/segment; checked before oversized append/repeat/format/native resize; structured OutputLimit and original target retained. No truncation. |
| Duplicate finder compatibility | engine and fileops legacy action suites; UI finder sample | Existing grouping and validated duplicate actions preserved. |

The Task22 policy limitation remains: out-of-band configuration can newly protect a root during pending recovery. Receipt-only reconciliation/descendant Resume can remain available, but an actual protected root Restore relocation refuses and retains truthful pinned pending intent. Normal library saves cannot cross the pending-recovery writer barrier. There is no reset/bypass exception.

## Scale and resource evidence

`tools/benchmark-library.ps1` generates a deterministic100,000-line JSONL metadata fixture outside the repository, exercises actual `page_owned` authorization/projection/filter/sort and the real frozen-input cache, then runs a1000-real-file desktop inventory/preview benchmark. Linux equivalent:

```sh
VIDLIB_BENCHMARK_ENTRIES=100000 VIDLIB_BENCHMARK_FIXTURE=/tmp/rename-100000.jsonl cargo test --locked -p vidlib-tauri --lib benchmark_metadata_library -- --ignored --nocapture
cargo test --locked -p vidlib-tauri --lib benchmark_native_preview -- --ignored --nocapture
VIDLIB_INVERSE_ENTRIES=1000 cargo test --locked -p vidlib-fileops --test undo benchmark_large_inverse_journal_begin -- --ignored --nocapture
```

The JSONL contains deterministic index/name/exact size/exact timestamp, no real content or granted native path. SHA256 `37d26e0db41d947b5504ac7c171ef0d8781aeb66e873bb2841f89d5ee5b63ad2` for100,000 rows. Test plans are explicitly seeded from one real Keep plan with synthetic metadata effects for projection only and never applied. IDs, output order and immutable facts are preserved by temporary indexes. Projection still scans/indexes and sorts the snapshot per page; this is not a persistent query cache. Runtime generated UUID values deliberately differ between fresh benchmark processes, but each repeated preparation preserves its exact first values.

Observed cloud: Linux6.18.55 x86_64, Intel Xeon Platinum8573C,5 exposed CPUs,17GiB RAM, no swap; temporary media/journals tmpfs, repository overlayfs. Rust debug profile, shared CI-like environment, no Windows reference hardware. Baseline frozen preparation1,000 entries:25.3ms cold/15.4ms repeated;5,000:599.5/360.8ms (quadratic). After temporary key indexes100,000:1,256.2/988.4ms. Initial actual page API100,000:444.7/387.2/334.7ms for200 rows. These initial page timings used an already ordered fixture. The final scrambled fixture and actual filter/sort measurements are below.

**Known boundary:** runtime selection and numbering IDs are serialized as part of the1MiB recipe preparation budget. A full100,000-selected-item preview currently refuses `RecipeTooLarge`; the seeded100,000-entry page/cache evidence is not an end-to-end preview success. The approved1MiB import bound is unchanged. Smaller actual desktop preview, CPU cancellation and all-selected conflict behavior are separate evidence. A future change separating runtime bindings from recipe-import size must retain import safety and be reviewed explicitly.

The64KiB generated intermediate budget permits later shrinking only while every earlier intermediate remains within budget; a recipe intentionally exceeding it before shrinking is unsupported. It is a resource policy, not the native final-name limit. Native per-volume validation remains unchanged. It does **not** bound aggregate memory: retained inventory, plan clones, frozen values, rule explanations and steps grow with entries/rules/output. A full adversarial multirow/multirule memory qualification has not been run; no OOM stress was attempted. Metadata helper probing/hashing bytes retain their independent limits. The100,000-entry cache benchmark uses frozen generated values, not100,000 actual external metadata probes.

`Journal::begin` canonically reconstructs historical inverse plans inside the SQLite write transaction, with no filesystem I/O there. The inverse benchmark executes real files first, times pure Undo planning and durable inverse begin separately, verifies historical receipts unchanged, and does not execute the measured inverse. Its begin duration includes work outside/inside the transaction, an upper bound on write-lock hold rather than isolated lock instrumentation. It is not a100,000-effect inverse qualification, a retention threshold, or hardware durability proof.

### Final measured scale outcomes (Linux debug; Windows target pending)

The deterministic fixture scrambles plan row order before each query. The owning desktop API applies the requested filter, exact existing sort semantics, revision authority and metadata projection. Each page returned 200 rows. These are measurements, not a Windows reference-machine pass.

| Sort | Before key caching, all rows / filtered (ms) | Final warm build, all rows / filtered (ms) |
| --- | --- | --- |
| Name | 532.803 / 310.111 | 514.074 / 336.721 |
| Natural | 3477.588 / 558.945 | 2597.740 / 720.818 |
| Path | 453.670 / 307.962 | 524.756 / 313.346 |
| Size | 419.474 / 311.789 | 438.260 / 314.841 |
| Status | 478.890 / 321.869 | 490.929 / 338.600 |

Natural sorting still exceeds one second on this debug cloud fixture. The approved <=1-second target must be measured on documented Windows reference hardware in the intended build profile; it remains pending. Minor14's repeated per-row projection scans are addressed, but each query still indexes/scans/sorts its snapshot. The optimization caches exact comparator keys per call, with numeric tie, Unicode search, ID tie ordering and stale-revision regression coverage; no new collation or persistent cache was introduced.

Final frozen preparation of 100,000 generated UUID values: **1147.133 ms cold / 898.283 ms repeated**, with identical retained values. Minor09's repeated retained/output scans are addressed; other template/probe-cache lookup costs and aggregate retained data remain unqualified. The actual 100,000-row core regex evaluation (typed inputs, separate from refused desktop preparation) completed in **2384.403 ms**, with 10 delivered progress updates and the existing <=10/s sink. The earlier metadata-tail baseline was intentionally terminated after **131.80 s total benchmark wall time**, during CPU evaluation, with **624,912 KiB** peak RSS; it was not a completed run. Empty metadata intents now skip the tail scan, and nonempty intents use per-call directory/first-primary-ordinal indexes. A focused 30,000-row test cancels during this actual tail after partial work and preserves explicit directory Set, full primary order and companion inheritance.

Final warm-build combined synthetic benchmark: **14.30 s test / 14.78 s command**, **625,036 KiB peak RSS** (GNU time over Cargo plus test child, no compilation). A preceding post-fix run included compilation and reported 1,423,224 KiB; that is not benchmark-only memory. This RSS covers the seeded plan/cache/evaluation fixture, not a global adversarial bound. No release RSS or native Windows reference measurements were obtained.

Actual desktop pipeline on **1,000 temporary one-byte files**: inventory **11.025 ms**, native-bound Keep preview **2509.951 ms**, 200-row page **9.864 ms**; command peak RSS **99,444 KiB**. This runs real scan/preparation/compile/page through desktop jobs within the approved preparation cap. In contrast, the 100,000 selected-ID recipe is **7,800,424 bytes** and correctly refuses `RecipeTooLarge` before preparation. The synthetic page/cache benchmark cannot waive that limitation.

| Real inverse fixture | Forward prepare (ms) | Forward execute (ms) | Undo plan (ms) | Inverse Journal::begin (ms) |
| --- | --- | --- | --- | --- |
| 100 files | 59 | 4198 | 40 | 79 |
| 1,000 files | 3559 | 476397 | 4744 | 5579 |

The 1,000-file test completed in 490.80 s and verified original journal facts/target files. It measures inverse planning/begin, not inverse execution or a separately instrumented SQL lock interval. The expensive forward execution and 5.579-second begin remain material scale concerns. No 100,000-file mutation stress, retention threshold or release-performance pass is claimed.

## Offline packaging and native pending procedures

MediaInfo25.10 x86_64 and all8 redistribution resources are pinned by `tools/media-info-manifest.json`; `python3 tools/vendor_media_info.py --check` verifies local hashes/PE architecture/resource mappings with no network. `--archive` hydrates a supplied pinned archive but now refuses a missing supplementary notice unless `--fetch` is explicit. `--fetch` is a development/build action only. The running app never downloads a script runtime or helper. Property probing uses the packaged application helper path; actual Windows property/MediaInfo subprocess runtime remains pending.

`src-tauri/tauri.conf.json` now selects Tauri `webviewInstallMode: offlineInstaller` with silent runtime installation. A Windows NSIS bundle must embed the WebView2 offline installer rather than depend on a network bootstrapper. Build-time Tauri tooling may download that redistributable; record the resulting runtime version, hash, provenance and installer artifact hash during the real bundle build. `cargo build --release` only produces the binary, not a qualified NSIS bundle. With the recorded CLI version, run `npx @tauri-apps/cli@2.12.3 build --bundles nsis` on the provisioned Windows build host after the verification commands. This packaging command is documented, not executed here.

NSIS3.11-1 source harness compilation succeeded for the actual install/uninstall hooks (`tools/tests/installer-hooks.nsi`,102,675-byte fixture executable). Cloud reproduction:

```sh
NSISDIR=/workspace/.tools/nsis-3.11/usr/share/nsis /workspace/.tools/nsis-3.11/usr/bin/makensis -DOUT_FILE=/tmp/vidlib-hooks-verification.exe tools/tests/installer-hooks.nsi
```

Compiler packages: official Debian `nsis-common_3.11-1_all.deb` SHA256 `103a3284c1a5356efa0aba90fdfc391c3cde8e7df8726377cef0f98471584047`; `nsis_3.11-1_amd64.deb` SHA256 `e6eb6ffb70f0df09e95dedb2d2cf55ad3c2679f7c2ae357d0c00cc4c83178690`. Compilation validates macro syntax/labels in an installer and uninstaller. It does not run registry code, package the real app, validate opt-in/uninstall behavior, or establish offline installation.

| Native gate — all PENDING | Reproducible acceptance procedure and expected evidence |
| --- | --- |
| Windows MSVC workspace/release + native primitives | Run exact workspace test/Clippy/fmt/build commands on x64 Windows; record OS/SDK/compiler/hardware. Run private Windows native primitive qualification cases and public closed-gate refusal tests; do not enable gates from compile success. |
| NTFS, exFAT/removable, SMB/share | For each actual volume record filesystem/API capabilities, identities/case policy, no-replace collisions, locked/read-only/ACL paths, reparse points, source/parent races, large paths and invalid UTF16. Verify source/foreign bytes preserved on refusal. Unsupported results remain closed, not silently downgraded. |
| Durability/storage faults | Run all journal and phase fault suites on actual local app-data storage, removable source/destination and shares; interrupt/disconnect/reconnect and separately test power-loss persistence. Never store operations.sqlite3 on media/share. Record actual flush outcomes and unresolved gaps. |
| Installed offline app | On clean disconnected Windows with no WebView2/script runtime/developer tools, install the actual reviewed/provenanced NSIS artifact and record signature status if present; verify bundled WebView2 installs offline, app opens, helper probes work, no outbound network required. Repeat with compatible preinstalled WebView2. Artifact/runtime hashes and network-capture evidence required. |
| Explorer/installer/registry | Interactive default-No leaves no verb; explicit opt-in registers direct quoted argv for files/folders; silent install does not opt in; foreign owner/command/values survive install+uninstall; only exactly owned values/empty keys removed. Test Unicode/spaces/quotes/UNC/multiple items and native UTF16 handling. Selection opens review only. |
| Dialog/drop/keyboard/concurrency | Real native file/folder picker multiselect/cancel, WebView2 drag/drop/focus/keyboard, concurrent installed windows, pending native selection retry/discard and writer exclusion. Browser bridges are not substitute evidence. |
| Reference performance | Document Windows CPU/RAM/storage, release profile and dataset. Run100,000 metadata fixture with cached page/filter/sort target<=1s,<=200rows/page,<=10progress updates/s; record cold/warm timing/RSS/CPU/helper-cache costs and bounded actual selected preview separately. |

## Final outcomes

- Final portable/full Linux desktop workspace: **633 parent/top-level Rust tests passed, 0 failed**, with four ignored declarations (three scale benchmarks run explicitly above; one subprocess helper run by its parent). Raw nested summaries include one additional isolated regex child, which is not counted twice.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: **PASS**. No warnings in the final workspace or Clippy output.
- `npm ci` and `env -u NO_COLOR npm test`: **PASS**, four Node tests and 57 Playwright tests. UI code did not change after this run.
- Python vendor tests: **4 passed**; local MediaInfo resource check: **8 resources verified**. NSIS source harness compiled as described above.
- Formatting, 19 UI JavaScript syntax checks and Git whitespace checks: **PASS**.
- `cargo build --locked --release -p vidlib-tauri`: **PASS**, optimized Linux build in 7m15s. `cargo check --locked --workspace --all-targets --target x86_64-pc-windows-gnu`: **PASS**, separate Windows target in 2m16s, no warnings. This is compilation only. Native Windows/MSVC/runtime/storage/offline installation and the Windows reference-machine performance target remain **PENDING**; hosted CI and independent controller review are not claimed as run.

This record does not open any release gate. The controller owns independent Task23 review, broad Rename phase review/fix disposition, the pause before Task24 and the final verified local bundle.
