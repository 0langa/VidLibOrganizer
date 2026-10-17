> Current handoff: round1 fix evidence is appended below. The initial implementation sections and 306-test workspace run describe the pre-fix package; the round1 section supersedes its history/recovery/lifecycle and schema statements. This SDD copy is ignored process state. The tracked evidence copy is `docs/verification/task10-rename-interface.md`.

# Task10 — usable Rename vertical slice and bounded IPC

Status: DONE_WITH_CONCERNS. The Task10 vertical slice and available verification are complete. Windows installed/runtime/offline packaging and hardware qualification remain pending; no native mutation/release gate was opened. The command module is substantial (849 formatted lines), called out for independent review rather than split beyond the assigned ownership.

Base: `173adc0` (Task09 approved documentation).
Implementation commit: `48b8984607170ab5027dc919464a5aa8f97455bc` — `feat: add Rename preview execution and recovery interface`.
This report is committed separately after the implementation so it can identify the immutable implementation commit.

Read Task10 brief first, implementer contract, the official SDD implementer prompt, TDD skill and writing-good-tests reference, plus relevant final Task05–09 interfaces/handoffs. No whole canonical-plan reread, new design exercise, delegated work, branch/worktree change, canonical checklist edit, push, merge or publication.

## Delivered behavior and authority

The desktop has a testable `vidlib_tauri` library with `pub fn run()` and a three-line binary entry point. Existing library/finder command argument shapes and behavior were moved into that library without replacing the finder engine. AppState clones Arc-managed state; SQLite connections are opened on their owning blocking worker and never stored in AppState or borrowed across await. Preferences retain the same libraries.json path/implementation; the existing faithful PNG icon is unchanged.

New Tauri commands take one owned `request` argument: `select_sources`, `start_inventory`, `start_rename_preview`, `job_status`, `cancel_job`, `rename_page`, `confirm_rename`, `apply_rename`, `operation_history`, `recovery_preview`, `start_recovery`, `start_undo`. Supporting bounded/native-state commands are `rename_session`, `update_rename_inputs` and `inventory_page`. Public owned handler functions provide the narrow integration-test seam. Requests reject unknown fields. New JSON field names and enum tags use camelCase; revisions, counts, bytes and native-precision timestamps are decimal strings. CommandError carries code/message/phase/itemId/operationId/retryable.

Native selection installs Rust PathBuf/RootBinding grants and returns grant IDs plus display strings. Subsequent inventory pages supply freshly observed item IDs. Selection displays are bounded to 200 sources with an exact sourceCount; this does not truncate the backend selection. No new mutation command accepts a path or reconstructs one from a display string. Native picker results are rejected if newer inputs supersede the picker. A separate backend selection generation prevents an old inventory/preview worker from publishing after a native selection changes even at the same client revision.

Inventory, recipe and selection changes revoke the current plan/confirmation. Each process gets a fresh PlanRegistry session; the UI reads the current exact input revision through rename_session. Workers freeze owned inputs and publish only if their captured revision and native selection generation still match. Apply consumes its backend nonce under the matching ordinary Write lease, then hands the immutable plan and lease to the native executor. Coordinator epochs, native preflight, no-replace semantics, durable journal and pending gates are reused unchanged.

Keep and FixedStem are the only rule controls. The backend evaluates/compiles all selected items before producing a page. An individual file grant captures its original source to observe sibling occupancy, without adding sibling source authority or asking to capture an unauthorized proposed sibling. Directory selection captures original parents of every carried inventory descendant, including unselected descendants; complete directory listings are also retained for empty selected/carried directories so future Undo has historical child evidence. App-local private data is an excluded subtree and uses existing ProtectedPolicy ancestor-carry protection. Selecting its database directly yields no selectable inventory; selecting a directory that would carry it is blocked; unrelated descendants remain usable.

Query projection stays backend-owned and capped at 200. Inventory/plan Arc handles are cloned under short locks, and filtering/sorting/projection run after guards drop. Only the returned page is converted to JSON. Display sorting/filtering never changes frozen recipe numbering order. The PlanStore/PlanRegistry read-only Arc snapshot addition preserves the old current/confirm/consume signatures and validation; retaining a snapshot does not revive registry confirmation or grant execution authority.

Rename jobs retain at most 64 process-local records and a bounded four-item progress channel; progress is produced through existing JobControl throttling and serialized exactly. Cancel requests reach the real worker control. A cancellation flag cannot discard a factual partial-operation report: the job completes with that result, whose operation state may be Cancelled/Failed/RecoveryRequired and whose completed counts remain visible. The existing close guard includes Rename/Recovery write leases and cancels read jobs when closure is allowed.

History uses the existing stable bounded high-water cursor. A narrow computed HistoryItem.undo_available field accounts for retained original completion, unresolved steps, terminal state and active inverse lineage without loading each full inspection record. Failed/Cancelled records with retained completed work are eligible; an active inverse revokes repeated original Undo. This field is factual eligibility, not mutation authority.

`vidlib_fileops::undo::capture_context` is the narrow cross-interface addition approved by the controller. It returns backend-only transformed historical grants and native namespace-capture candidates, using the same factual forward/compensation transformation already owned by fileops. It does not serialize/import execution authority or manufacture missing historical subtree facts. Desktop start_undo captures fresh namespace/current epochs, calls plan_undo, publishes the resulting preview and uses the same confirmation and executor as Rename. Tests perform actual individual-file and nested-directory forward/Undo moves. An unnecessary capture above a directory grant was removed after it produced a real preflight failure.

Recovery stores the non-Clone reviewed backend snapshot behind an opaque process-local reference, exposes at most 200 display/action rows and derives available choices from unresolved groups and durable Restore context. Both reference and nonce are consumed; application reacquires the matching exclusive Recovery lease and the existing recovery engine rechecks durable/current evidence. Once Restore is durable, only Restore continuation is offered. Legacy interruption is exposed independently of pending operations, remains read-only unresolved and disables recovery/Undo actions in the UI; an empty pending list never clears it. No marker reset API was added.

## Interface and file-list reconciliation

All explicitly requested new files are present: src-tauri library/selection/commands/command contracts; ui/jobs.js, ui/exact.js and Rename main/preview/history; browser spec and external bridge/golden fixtures; localhost static server; package manifest/lockfile and Playwright configuration. AppState, binary main, Jobs, ui/index.html, ui/main.js and scoped styles are modified. ui/recipe/editor.js implements the explicitly required common createRecipeEditor interface (getRecipe/setRecipe/destroy), exposing only Keep/FixedStem and never computing generated names.

Two listed existing files are intentionally untouched:

- `ui/api.js` already gained the required structured invoke-error normalization in Task05. It preserves code/phase/IDs/retryable and the readable message while leaving successful and legacy finder payloads unchanged. Rename reuses call() and renders error context through textContent. Its three existing API tests are retained and run by npm test; rewriting it would add no missing behavior.
- `ui/dom.js` already provides el()/query helpers based on textContent and native DOM nodes. New views reuse those helpers and document fragments, with no HTML string interpolation. No extra DOM helper or duplicate formatting engine was needed, so it was not touched gratuitously.

Rust dependency manifests and Cargo.lock need no changes: all required Rust dependencies already belonged to the approved desktop crates. The new owned development dependency is @playwright/test 1.58.2, recorded in package.json/package-lock.json. Runtime remains static modules in ui; no bundler or script-runtime dependency is introduced into the desktop package. Test bridge/golden files and tooling are outside the packaged frontendDist (`ui`). Browser sample Rename is explicit and read-only.

## Browser interface

Rename navigation provides native folder/file selection, a paged item table, selection, minimal rule editor, backend-generated preview, exact bytes, confirmation, job status/cancellation, history, Undo preview and recovery review/continuation. Generation checks cover delayed job start/completion, delayed page acceptance and delayed confirmation. Navigation aborts the watcher, cancels its backend job, invalidates preview authority and removes recovery/history continuations. watchJob clears timers/listeners on finish, abort and errors. Static application listeners are installed once for the view lifetime; the editor exposes teardown.

Confirmation uses a native dialog, focuses Cancel, handles Escape, and restores trigger focus. Rendering uses textContent/fragments. Exact byte formatting requires a validated decimal string and BigInt; rounded JS numeric input is rejected. Browser sample mode cannot invoke filesystem commands or enable Apply. The UI has no rename transformation engine.

The Node server binds only 127.0.0.1, serves only the ui root, handles malformed/missing requests and sends JavaScript module MIME types. Chromium runs against actual localhost HTTP, without changing managed browser policies. Playwright accepts CHROMIUM_PATH and defaults to the installed `/usr/lib/chromium/chromium` in this environment.

## Test-first evidence

All logs below are raw files under `/workspace/scratch/`. Desktop commands source `/workspace/.tools/desktop-env.sh` and set `CARGO_TARGET_DIR=/workspace/.tools/desktop-target`; that environment supplies the instructed Cargo/Rustup paths. No compilation failure or already-passing stub is counted as a behavioral red.

1. `npx playwright test tests/ui/rename-basic.spec.mjs --timeout=7000` (with FORCE_COLOR unset): `task10-browser-intended-red.log`, six Rename journeys failed the user-visible navigation assertion (expected one Rename button, actual zero); the existing finder sample interaction passed. The preceding `task10-browser-red.log` exposed a test-server headers error/connection refusals and is **tooling failure, not an intended red**. The server was corrected before the intended rerun.
2. `cargo test --locked -p vidlib-tauri --test command_contracts`: `task10-desktop-intended-red.log`, **1 failed/4 passed**. stale_nonce_cannot_apply failed with `editing recipe retained apply authority` against an existing-core-backed adapter whose input-change invalidation was not yet wired. old_session_plan_is_invalid, page_never_returns_native_paths, real Apply/history and exact owned DTO cases already passed from existing backend behavior; they are not claimed independent reds. `task10-desktop-compile.log` contains the earlier FileStamp field-name compile correction, also not a behavioral red.
3. `npx playwright test tests/ui/rename-basic.spec.mjs -g preview_response_after_edit_is_ignored --timeout=10000` (NO_COLOR unset): `task10-browser-race-red.log`, **1 intended failure**. The test explicitly awaited consumption of the delayed job's terminal status before asserting: Apply was enabled, expected disabled. A first weaker run (`task10-browser-behavior-red.log`, seven passing) checked too early and is not claimed proof of the race. The strengthened test failed before generation-aware result acceptance and passes afterward.
4. `cargo test --locked -p vidlib-tauri --test command_contracts`: `task10-selection-undo-red.log`, **3 failed/6 passed**. Stale recipe confirmation still applied; old inventory survived native selection replacement; retained file effects did not yet offer an Undo preview. After wiring revocation and native Undo, `task10-undo-history-red.log` reduced this to **1 failed/8 passed**: actual individual-file Undo succeeded, but the original still incorrectly reported undoAvailable=true despite an active inverse. The computed history eligibility correction makes this false.
5. Expanded same desktop command: `task10-policy-red.log`, **2 failed/10 passed**. A private app-data source yielded selectable managed databases; nested-directory Undo hit `InvalidNamespace: namespace or protected policy changed since preview` because a directory grant's root itself unnecessarily captured its parent outside the affected namespace. Explicit private-data policy and removal of that extra capture resolved these. The added carrying-private-namespace test proves selected ancestor blocking, direct database exclusion and successful unrelated-descendant Apply.
6. `cargo test --locked -p vidlib-tauri commands::rename`: `task10-cancel-report-red.log`, **1 failed/1 passed**. cancel_signal_does_not_hide_factual_operation_report expected job state completed with the factual retained-work result, received cancelled. RenameJob now distinguishes completion facts from cancellation requests. The shared-browser-golden exact row/error test passed already and is not a red.
7. `cargo test --locked -p vidlib-tauri --test command_contracts empty_directory`: `task10-empty-directory-red.log`, **1 failed**. Empty-directory Undo preview had blocked count `1`, expected `0`; the current empty directory could not replace an absent historical child listing. Forward capture now retains that listing without weakening Undo's historical-evidence checks.
8. `node --test tests/ui/exact.test.mjs`: `task10-exact-red.log`, **1 failed**, `Missing expected exception` when a rounded JS numeric input was accepted. The helper now requires a string. `task10-exact-green.log`: **1 passed** and exact `9,007,199,254,740,993 bytes` output.

Focused intermediate greens are retained as task10-desktop-green1 through green5 logs. Final focused `cargo test --locked -p vidlib-tauri commands::rename`: `task10-commands-final.log`, **2 unit tests passed**. The filter deliberately selects those tests, not the integration binary. Final `cargo test --locked -p vidlib-tauri --test command_contracts`: `task10-contracts-final.log`, **15 passed**.

The 15 desktop integration cases use actual temporary files, native scans/capture/renames and SQLite journals. They cover stale nonce/session, serialization/native-path projection, actual Apply/history, narrow transformed individual-file Undo, opaque consumed recovery, independent legacy uncertainty, native selection invalidation/supersession, 205 selected conflicts despite a 200-row page, carried unselected nested descendants/Undo, empty-directory historical listings, private subtree/ancestor protection and exact process revision restoration. The new JobResult unit case checks the completion boundary rather than mocking fileops compensation; fileops' existing real native partial/recovery tests remain in the full suite.

## Final verification

One full workspace run after focused development:

```sh
source /workspace/.tools/desktop-env.sh
CARGO_TARGET_DIR=/workspace/.tools/desktop-target cargo test --locked --workspace
```

`/workspace/scratch/task10-workspace-final.log`: **306 passed, 0 failed**, all doc tests successful, no warnings. This includes all existing engine/finder, inventory, planner, fileops/journal/coordinator/recovery/Undo and desktop tests. Desktop comprises **17 library tests** (15 pre-existing plus two new) and **15 command integration tests**. Windows-only test binaries with zero active Linux tests are not counted as Windows runtime evidence.

```sh
env -u NO_COLOR npm test
```

`/workspace/scratch/task10-npm-final2.log`: **4 Node tests passed** (all three retained API tests plus exact-byte rejection/formatting), **10 real Chromium tests passed**, no warnings. Product browser cases: rename_basic_preview_confirm_cancel; preview_response_after_edit_is_ignored; browser_preview_cannot_apply; exact_bytes_above_js_safe_integer; keyboard_dialog_focus_and_escape; navigation_cancels_preview_and_tears_down_polling (no further polls across more than two polling periods); finder_sample_interaction_is_preserved; only_one_200_row_page_is_rendered; confirmation_response_after_edit_is_ignored; recovery_continuation_and_unknown_legacy_are_factual. These execute actual UI modules; only the external desktop boundary is mocked by test-owned fixtures. Bridge requests exercise distinct input and plan revisions and only opaque IDs.

```sh
source /workspace/.tools/desktop-env.sh
CARGO_TARGET_DIR=/workspace/.tools/desktop-target cargo clippy --locked -p vidlib-tauri -p vidlib-fileops -p vidlib-rename --all-targets -- -D warnings
```

`/workspace/scratch/task10-clippy-final3.log`: **passed, no warnings**. Earlier task10-clippy-final/final2 logs identify test-token drop_non_drop and items_after_test_module issues, both corrected before final validation; no lint was suppressed.

`/workspace/scratch/task10-static-final.log`: **18 JS/MJS syntax checks passed**, including all UI/test/tool modules and Playwright config; `git diff --check` passed. `cargo fmt --all -- --check` returned success with no output. Staged diff check also passed before commit.

Fresh separate shell:

```sh
source /workspace/.tools/windows-check-env.sh
cargo check --locked -p vidlib-fileops -p vidlib-rename --all-targets --target x86_64-pc-windows-gnu
```

`/workspace/scratch/task10-windows-cross-check.log`: **passed, no warnings**. This checks the narrow shared helper/query/snapshot additions for that target, not the installed desktop app or native Windows mutation behavior.

Visual inspection used real Chromium **154.0.8037.57**, viewport 1360×900, localhost UI and the external bridge. Inspected `/workspace/scratch/task10-rename-preview.png` and `/workspace/scratch/task10-rename-confirmation.png`: controls, complete small preview table, exact byte count and confirmation are visible/readable; the stem input received a scoped size/style correction after the first inspection. Final captures were inspected again. `/workspace/scratch/task10-visual-check.log` records `dialogVisible:true`, `cancelFocused:true`, `pageErrors:[]`. This is visual/product-browser evidence; it is not an installed Tauri WebView or Windows picker runtime test.

## Self-review and limits

Reviewed staged source, cross-interface additions, complete task file/interface list, IPC field/decimal/tag fixtures, locking/lease lifetimes, stale publication/nonce consumption, native file-grant occupancy, carried/empty directory evidence, history inverse eligibility, durable Restore choices, independent unknown legacy uncertainty, exact number handling, packaged UI boundary, finder compatibility and scoped styles. No unrelated engine, platform mutation, journal schema/migration, icon or canonical checklist changes are included. The original data-only dependency direction is preserved. There is no second executor or frontend rename engine.

Outstanding qualification is unchanged: actual Windows runtime/native sharing/handle/timestamp behavior, installed/offline Windows packaging, multiple installed instances, and hardware power-loss tests remain pending. Linux native filesystem/command tests, real Chromium tests, cross-compilation and screenshots establish their stated scopes only. No Windows gate is opened. Keep/FixedStem is the Task10 executable slice; later approved Rename/Blueprint owners retain their scope and this report does not claim complete Rename release or Blueprint foundation completion.

No current-scope blocker remains. Independent controller review is still required, especially the substantial command orchestration module and the three narrow shared additions: read-only Arc snapshots, native Undo capture context and computed history eligibility.


## Round1 review fixes — I1–I4

Status: DONE_WITH_CONCERNS. Fix base `fce2ae2a08e1087b9e61483e1c3e0e6186c174e0`; implementation commit `571475fcd83566c93ae235603f47679332cfd833` (`fix: order Rename requests and complete reviewed query paging`). Independent controller review remains required. This section records the final fix package, not a release gate approval.

Read the full `/workspace/scratch/task10-review.md` and receiving-code-review instructions. Followed the existing approved brief/implementer/TDD contract; did not restart design, reread the canonical plan, delegate, edit canonical checklists, change branches, push, merge or publish. Fixed I1–I4 together. The controller expressly approved the narrow live-history persisted revision extension and required preservation of historical schema evidence.

### Final changes and interfaces

**I1 — ordered pending requests.** `ui/rename/main.js` reserves an active request ticket synchronously before awaiting input updates or command dispatch. The ticket combines monotonic sequence, edit generation and exact input revision, and survives start replies, terminal job results, initial/subsequent pages and confirmation. Duplicate events while pending cannot launch another Preview. Edits/navigation invalidate the ticket, abort polling, cancel known or subsequently returned jobs, and revoke plan/recovery UI state. Each Preview intent also advances the backend input revision, including unchanged recipe inputs; an older dispatch/worker cannot republish over the newer backend plan. Plan Apply authority is accepted only after the matching first page returns. A focused native command test rejects an older delayed preview dispatch while the latest confirmation remains valid. The UI lifecycle rewrite also restores Apply focus after cancelling/Escaping its confirmation once pending controls are re-enabled.

**I2 — recovery input binding.** Recovery capture, publication, paging and consumption bind both exact input revision and hidden native selection generation. `prepare_recovery_owned` captures a nonserializable `RecoveryInspection` with the existing exclusive lease, real journal record and backend policy; `publish_recovery_owned` performs real native reconciliation and checks the captured binding under the publication lock. These are the normal handler's two phases, allowing deterministic integration interleavings without test hooks or sleeps. Edit/native replacement during reconciliation rejects publication. `recovery_owned` independently checks the retained binding before nonce/choice consumption, even if a stale cached entry is retained. Native authorities remain private Rust values; no new IPC native-path authority is introduced.

**I3 — live bounded history query.** `HistoryRequest { query: PageQuery }` now implements cursor, capped limit, display sort/filter and expected data revision. `HistoryResponse` includes exact decimal revision and completed-step counts, lightweight operation rows, opaque next cursor and existing independent pending/legacy facts. Supported display sorts are name/operationId, state, oldest and newest; unknown sorts are rejected. The new narrow `history_revision({})` command obtains the current exact data revision for an initial query. Shared golden request/response payloads are consumed by Rust DTO tests and the external browser bridge. The UI exposes sort/filter controls and gracefully retries one stale query after reading a fresh revision and resetting to the first page.

History has two distinct concepts:

- **Persisted data revision:** schema v8 adds a checked canonical decimal-text `u64` counter. Each successful journal write advances it in the same durable transaction before commit; failures and overflow roll back facts and revision together. The implementation conservatively includes coordination/outbox-only writes, so some refreshes occur without a changed visible row. No SQLite REAL conversion, unchecked integer arithmetic, native filesystem I/O inside SQLite transactions, or metadata snapshot engine is added. Operation state, completion/eligibility, visibility and lineage mutations all pass through that checked write boundary.
- **Stable keyset/range cursor:** retains the established row high-water bound, last row ID and display sort key/filter. Inserts cannot shift a retained range. The counter is checked in the same read transaction as metadata projection; a changed revision produces staleState. The cursor is not treated as a data revision or immutable fact snapshot. The normal UI resets its cursor on refresh; an explicitly retained range queried with a freshly acknowledged revision remains a live view within that range. The older `Journal::history` API and its Task09 stable-page tests remain intact.

The new `journal/history.rs` queries only lightweight stored metadata and factual eligibility, applies filtering/sorting/keyset bounds in SQLite, and fetches at most limit+1 rows (limit clamped 1–200). Migration adds only V8_SQL and VERSION=8. V1–V7 SQL bytes and their checksum sources are unchanged. Older synthetic migration fixtures now remove v8 table/index/checksum when constructing actual older schemas; existing rollback assertions remain. Older readers reject v8 rather than silently ignoring its freshness protocol. No Rust dependency or lockfile change is necessary.

**I4 — complete bounded recovery review.** The same opaque recovery reference retains the same reviewed preview/nonce and an Arc of its real actions. Initial recovery metadata contains `page`, exact `actionCount` and exact binding revision. The new owned `recovery_page({ recoveryRef, query })` returns bounded display rows from that retained snapshot, with expected-revision and native-generation checks; it does not reconcile again, renew a nonce or add mutation authority. UI First/Next recovery controls reach the last action and ignore delayed pages after edits. Real native 205-action integration coverage reaches all 205 distinct step IDs across 200+5 rows with the same reference, checks wrong revision and native replacement revocation, and checks that encoded native path records are absent. Display-only path strings still render via textContent and never become targets.

Changed covering tests: `src-tauri/tests/command_contracts.rs`, the command module's focused unit test, `tests/ui/rename-basic.spec.mjs`, external bridge/shared JSON goldens, `crates/vidlib-fileops/tests/journal.rs`, and directly affected old-schema fixtures in recovery/rename/undo tests. Existing finder command signatures and native engine authority remain unchanged. `ui/api.js` and `ui/dom.js` remain intentionally untouched for the concrete already-satisfied reasons recorded above (M2). Inventory kind spelling (M1) remains deferred by controller instruction. Lifecycle/bridge formatting is incidental to these fixes; broader compressed-test cleanup (M3) is deferred.

### Meaningful red evidence

All paths below are raw `/workspace/scratch/` logs. Linux Cargo commands use the desktop environment and target directory shown in final commands. Compile/tooling/fixture setup failures are distinguished from behavioral reds.

- `cargo test --locked -p vidlib-tauri --test command_contracts` → `task10-fix1-desktop-red.log`: **15 passed, 4 failed**. `history_accepts_the_owned_page_query_contract` rejected the required query field (`unknown field query, expected cursor`). Both `recovery_reconciliation_cannot_republish_after_edit` and `recovery_reconciliation_cannot_republish_after_native_replacement` returned a real executable review after deterministic revocation. `recovery_all_actions_are_reachable_through_one_review` failed `review permanently truncated actions beyond the first 200`. Capture/publication were first separated without changing the old behavior, so these tests exercised the actual journal/native reconciliation rather than mock success responses.
- Playwright selected `delayed_start` and `recovery_navigation` tests in `tests/ui/rename-basic.spec.mjs` → `task10-fix1-browser-red.log`: **2 failed, 1 passed**. `delayed_start_reserves_pending_before_reply` expected Preview disabled while its first actual start reply was held, received enabled. `recovery_navigation_reaches_last_action_without_new_review` found zero Next recovery page controls, expected one. The edited-generation case already passed on the baseline and is not claimed as a red. The test injects only desktop-boundary delays and runs actual UI modules in real Chromium at localhost.
- Focused journal history tests (`cargo test --locked -p vidlib-fileops --test journal history_`) → `task10-fix1-history-core-red.log`: **0 passed, 3 failed, 19 filtered**. `history_revision_tracks_durable_facts_and_visibility` observed no advancing revision; `history_page_limit_filter_sort_and_range_are_stable` returned 3 rows instead of limit 1; `v7_history_revision_upgrade_preserves_evidence_and_rolls_back_failure` retained version 7 instead of 8. The compilable query adapter still delegated the old core behavior at this point; compilation was not the failure being asserted.

Intermediate failures are retained transparently: `task10-fix1-history-core-green.log` exposed a synthetic v7 fixture that had left the new index behind (`index history_by_state already exists`); corrected fixture then passes in green2/green3. `task10-fix1-desktop-green2.log` contains a Rust Option<String> inference compile correction, not an intended red. `task10-fix1-browser-green1.log` had 12 passes and an existing keyboard focus regression caused by the new pending state; focus restoration was fixed before final browser green. No warnings/lints were suppressed.

### Final commands and output

```sh
source /workspace/.tools/desktop-env.sh
CARGO_TARGET_DIR=/workspace/.tools/desktop-target cargo test --locked -p vidlib-fileops -p vidlib-tauri > /workspace/scratch/task10-fix1-affected-suites.log 2>&1
```

**218 passed, 0 failed**, doc tests successful. Includes 23 journal tests, all existing Task09 stable history and migration rollback tests, native forward/recovery/Undo suites, 18 desktop library tests and 21 desktop command contracts. Broadened these two crates because the common durable write boundary and schema changed. The prior **306-test full workspace run remains separate pre-fix evidence**; it was not rerun or represented as validating this fix.

`/workspace/scratch/task10-fix1-browser-green2.log`: `npx playwright test tests/ui/rename-basic.spec.mjs` (NO_COLOR unset), **15 real Chromium tests passed**. Retains all original ten including finder sample, Escape/focus, stale terminal/confirmation, cancellation/navigation and 200-row bounding; adds delayed pending start, delayed old start after edit/newer preview, 205-action navigation, live history revision/filter refresh, and stale recovery page after edit. External bridge remains outside packaged ui and uses the shared golden contract. `/workspace/scratch/task10-fix1-node.log`: `node --test tests/ui/*.test.mjs`, **4 passed, 0 failed**, including existing structured-error compatibility and exact BigInt byte handling.

```sh
source /workspace/.tools/desktop-env.sh
CARGO_TARGET_DIR=/workspace/.tools/desktop-target cargo clippy --locked -p vidlib-fileops -p vidlib-tauri --all-targets -- -D warnings > /workspace/scratch/task10-fix1-clippy.log 2>&1
CARGO_TARGET_DIR=/workspace/.tools/desktop-target cargo fmt --all -- --check
```

**Passed, no warnings.** `task10-fix1-static.log`: formatting success, all **18 JS/MJS node --check checks passed**, git diff --check passed. Staged diff check also passed before implementation commit.

Fresh separate shell:

```sh
source /workspace/.tools/windows-check-env.sh
cargo check --locked -p vidlib-fileops --all-targets --target x86_64-pc-windows-gnu > /workspace/scratch/task10-fix1-windows-cross-check.log 2>&1
```

**Passed, no warnings.** This covers the changed shared journal API/schema code only; no Windows runtime evidence is claimed.

`task10-fix1-schema-preservation.log`: compared the actual raw SQL constant bytes against FIX_BASE using Python regex extraction and asserted byte equality for SQL/V2_SQL through V7_SQL; all seven passed and logged the identical MD5 checksum inputs. The focused journal tests prove v7 upgrade preserves historical operation evidence, injected migration conflicts roll back to v7 without deleting original evidence, injected revision-update failure rolls back state plus counter, and exact u64 exhaustion prevents a fact write. Existing v3/v4/v5/v6 migration/rollback tests also pass in the 218-test run.

Visual follow-up: inspected real Chromium 154.0.8037.57 screenshot `/workspace/scratch/task10-fix1-recovery-last-page.png` at localhost after reaching action 205. History sort/filter, last five actions, First recovery page and Restore controls are visible and readable; no unrelated redesign. `/workspace/scratch/task10-fix1-visual-check.log`: `visibleActions:5, firstPageControl:true, restoreControl:true, pageErrors:[]`. Original preview/confirmation screenshots remain the initial visual evidence; final functional keyboard/dialog focus checks pass.

### Closing self-review and limits

Reviewed staged lifecycle ownership, revision ordering, backend worker publication, recovery binding on all phases, query caps/unknown fields/decimal transport, immutable reviewed action reuse, live revision versus stable range semantics, readonly query transactions, old-schema preservation/rollback, existing native leases/gates and finder signatures. No current-scope blocker found. The v8 revision is deliberately conservative (extra refreshes possible) and does not claim an immutable history snapshot.

Implementation commit untracks the accidentally force-tracked ignored SDD report while retaining this required on-disk handoff copy. This complete updated report is tracked normally under `docs/verification/task10-rename-interface.md`; no SDD workspace artifact is force-added. No manifests or lockfiles changed in round1.

Actual Windows native runtime/hardware, installed/offline desktop packaging, WebView/native-picker behavior, multiple installed instances and power-loss qualification remain pending. No native release/mutation gate is opened by Linux tests, Chromium, screenshots or cross-compilation. Deferred Minors remain as stated above, for final controller triage.
