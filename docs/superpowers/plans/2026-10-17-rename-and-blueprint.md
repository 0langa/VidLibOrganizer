# Rename and Blueprint Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the complete standalone Rename feature and a separate working Blueprint foundation for remembered collection maps, observed changes, templates, and descriptive patterns.

**Architecture:** A shared read-only inventory preserves native paths and observation coverage. Rust produces immutable rename plans; a separate filesystem executor owns no-overwrite operations, durable journals, recovery, and events. Blueprint stores an advisory projection of inventories/events and never becomes a prerequisite for Rename.

**Tech Stack:** Existing Rust 2021 workspace, Tauri 2, dependency-free HTML/CSS/JavaScript ES modules; add rusqlite with bundled SQLite/backup, a bounded PCRE2 native adapter, Windows filesystem/property adapters, and a bundled MediaInfo CLI helper. Node/Playwright are development tools only.

**Spec:** [approved overview](../specs/2026-10-17-library-tools-design.md), [Rename](../specs/2026-10-17-renaming-design.md), [Blueprint foundation](../specs/2026-10-17-blueprint-foundation-design.md). The user approved these written specifications on 2026-10-17 and requested this one canonical implementation-plan file. This supersedes the overview's earlier suggestion of separate plan files.

## Global Constraints

The following requirements are copied from the approved specs and apply to every task:

- "The release works offline on Windows without a user-installed script runtime."
- "Blueprint is optional."
- "Native paths remain owned by Rust. UI actions refer to item/plan IDs, not reconstructed display strings."
- "New byte counts and timestamps crossing JSON use exact encodings, not rounded JavaScript numbers."
- "The backend is authoritative for generated names. The UI does not implement a second transformation engine."
- "Never overwrite an existing entry."
- "Before mutation, write and durably flush an execution journal in app-local `operations.sqlite3`. Use SQLite WAL with `synchronous=FULL` on a local filesystem."
- "The batch is recoverable, not an atomic filesystem transaction."
- "Plans expire on process restart and after inventory/recipe/selection changes."
- "User expressions run without JIT initially, with match limit 1,000,000, depth limit 1,000, heap limit 8 MiB, and pattern limit 16 KiB."
- "Run metadata probes off the UI thread in a cancellable helper process with a 10-second per-file deadline, one active probe per selected drive by default."
- "Validate configuration schemas and bound imports: 64 MiB/100,000 rows for mapping files, 1 MiB/100 rules for recipes, and the regex limits above."
- "Table pages are 200 rows; target validation includes all selected rows."
- "Store collection records in app-local `blueprint.sqlite3`, outside media roots and network shares."
- "No marker is created just by scanning, adding a library folder, or opening Blueprint."
- "The marker, if enabled for an explicitly chosen root, is `.vidlib-blueprint.json` with only `schema_version: 1`, `collection_id`, and `root_id`. Parse only this schema, limit the file to 4 KiB, and reject links/reparse points."
- "Limit an import to 256 MiB and 1,000,000 inventory entries."
- "A stale snapshot is never authority for deletion, rename, or undo."
- "The complete Rename release and this Blueprint foundation have separate completion reports."

The rest of both specs remains normative. Implement R01–R18 completely; intermediate slices are not the finished Rename feature. Retain the current duplicate-finder behavior and fixtures. No arbitrary scripting, online identification, automatic renaming, cloud sync, continuous watcher service, semantic season mapping, or convention-inference hero feature is added.

## Review Focus

These five implied failure classes receive explicit owning tests below:

1. Metadata lookup fails after a directory name was enumerated: the cached child must not become Missing (Task 26).
2. A filesystem has case-sensitive child directories inside an otherwise case-insensitive volume: collision checks must use each target parent (Tasks 02/06).
3. Copy publication succeeds but source removal or its journal acknowledgement fails: retain the verified copy and report/reconcile the real state, never repeat the copy blindly (Tasks 04/08/19).
4. A new template/selection edit races an older preview response: ignore obsolete results and prevent confirmation of the old revision (Tasks 10/21).
5. History pruning races outbox delivery or a multi-root collection refresh: retain unfinished recovery data, preserve event identity, and keep each root's valid baseline (Tasks 27/30).

---

## 1. Fresh-session entry and working rules

Repository: `VidLibOrganizer`; source baseline `d804e40`, approved-spec commit `c72ba3f`. Locate the actual checkout before using commands; paths below are repository-relative. Do not reset or replace another session's edits. No product code is implemented by this planning commit.

A fresh session needs this plan and the linked approved specs in its checkout. Use the existing workspace or transfer its documentation commits; a new GitHub clone only contains commits that have actually been pushed. Do not reconstruct missing specifications from memory or start implementation from the historical duplicate-finder plan.

Recommended execution method is **subagent-driven**, because the filesystem journal/graph interfaces and the collection projection have independently reviewable failure cases. **Native** execution with `superpowers:executing-plans` is also supported. The user intends to start implementation in a fresh session; choose the method there. The current session does not start implementation.

Fresh-session prompt:

```text
Use Superpowers to implement docs/superpowers/plans/2026-10-17-rename-and-blueprint.md.
The written specs were approved. Read this canonical plan and all three linked
specs first. Use subagent-driven development, create an isolated worktree, and
begin with the first incomplete task whose dependencies are satisfied. Follow
the test-first steps, preserve existing behavior, and update the checkboxes and
execution ledger in this file. Continue across the full Rename scope and then
the Blueprint foundation. Record unavailable Windows checks as pending and
continue independent portable work; do not claim a Windows release is verified
until the required Windows evidence exists. Do not restart brainstorming or
split the plan into competing plan files.
```

Before implementation: load `superpowers:using-git-worktrees`, inspect applicable `AGENTS.md`, identify current environment/network policy, and establish an isolated worktree. A fresh session should not assume the previous environment's tool installation survived. In this Linux workspace a local toolchain was installed under `/workspace/.tools/{cargo,rustup}`; if present, select it with `CARGO_HOME=/workspace/.tools/cargo`, `RUSTUP_HOME=/workspace/.tools/rustup`, and its `bin` directory in PATH. Do not redefine HOME. On Windows use the installed MSVC toolchain and WebView2.

Baseline commands: `cargo test --locked -p vidlib-engine -p vidlib-fileops`, `cargo clippy --locked -p vidlib-engine -p vidlib-fileops --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `node --check` for existing UI modules. Run `cargo test --workspace` and the desktop build on an environment with the required native dependencies. Earlier `docs/verification.md` results are historical evidence, not evidence for new changes.

For every task: write its named behavior tests, run them red for the intended missing behavior, implement the specified interface, run the scoped checks green, and commit only that task's files. Minimal crate/workspace declarations or browser harness setup belong to the test-writing step when needed; a missing package, browser, or test configuration is not the intended red behavior. The exact test body can use idiomatic fixture setup, but its named assertions/golden values below are required. Tests cross public interfaces; do not mirror private implementation. The final plan self-review was performed by the author; implementation reviews are separate.

If hardware/platform checks are unavailable, leave those checkboxes unchecked and record the limitation in the ledger. Once the public contracts are implemented, continue independent pure-domain/UI work. Unverified Windows filesystem capabilities may not be enabled by assumption; portable mocks do not satisfy a platform gate. Never replace a pending check with an empty passing test or silently reduce the accepted feature scope.

## 2. Documentation checked before planning

Context7 was queried on 2026-10-17 after resolving these IDs. Its current examples are guidance, not a reason to upgrade this repository or assume an upstream private function is stable:

| Context7 ID and source | Finding used in this plan |
| --- | --- |
| `/tauri-apps/tauri-docs`: [calling Rust](https://v2.tauri.app/develop/calling-rust/) and [managed state](https://v2.tauri.app/develop/state-management/) | Owned inputs and serializable results/errors across the Rust/JS interface; short managed-state locks; blocking work stays off the UI/async executor. Use job IDs plus small progress/status messages and paged results. |
| `/rusqlite/rusqlite`: [transactions](https://docs.rs/rusqlite/latest/rusqlite/struct.Transaction.html), [Connection](https://docs.rs/rusqlite/latest/rusqlite/struct.Connection.html) | `Connection` is Send, not Sync. Own connections on blocking worker threads; use explicit `TransactionBehavior::Immediate` commits and propagate commit/busy errors. Never share an unguarded connection across async tasks. |
| `/rust-lang/rust`: [rename](https://doc.rust-lang.org/std/fs/fn.rename.html), [Path](https://doc.rust-lang.org/std/path/struct.Path.html), [OsStr](https://doc.rust-lang.org/std/ffi/struct.OsStr.html) | Ordinary `fs::rename` can replace targets. Native paths need not be UTF-8. Preserve native paths and use an explicitly tested no-replace adapter; upstream source examples/private helpers are not a supported public API contract. |

Three Context7 library queries were used for the Rust/SQLite/Tauri decisions above. JavaScript guidance is grounded in Tauri's JavaScript interface examples and directly checked MDN references: [ES modules](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Guide/Modules), [textContent](https://developer.mozilla.org/en-US/docs/Web/API/Node/textContent), and [BigInt](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/BigInt). Use ES modules, text nodes, exact decimal-string transport/BigInt formatting, and explicit listener cleanup. Do not serialize BigInt directly to JSON.

Additional normative references already attached to the specs: [SQLite WAL](https://www.sqlite.org/wal.html), [PCRE2 limits](https://www.pcre.org/current/doc/html/pcre2api.html), [Windows file identity](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info), and [MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw). Recheck the selected stable crate/API versions during their owning tasks. Keep Rust edition 2021 and existing Tauri `2.12`/tauri-build `2.7` requirements; commit exact dependency resolution in Cargo.lock rather than copying an unverified latest version from documentation.

## 3. File ownership and dependency direction

| Owner | Files/responsibility |
| --- | --- |
| Shared read-only inventory | New `crates/vidlib-inventory/Cargo.toml`; `src/lib.rs`, `ids.rs`, `model.rs`, `path.rs`, `scan.rs`, `platform/mod.rs`, `platform/windows.rs`, `platform/unix.rs`; `tests/inventory.rs`, `tests/path_codec.rs`. |
| Deterministic rename planning | New `crates/vidlib-rename/Cargo.toml`; `src/lib.rs`, `model.rs`, `recipe.rs`, `evaluate.rs`, `graph.rs`, `validate.rs`, `text.rs`, `regex.rs`, `numbers.rs`, `dates.rs`, `template.rs`, `hashes.rs`, `selection.rs`, `mappings.rs`, `presets.rs`, `metadata.rs`, `companions.rs`, `destination.rs`; owning integration tests named in tasks. |
| Filesystem execution | Existing `crates/vidlib-fileops`; keep old public duplicate interface in `src/lib.rs`. Add `src/error.rs`, `platform/`, `journal/`, `coordinator.rs`, `events.rs`, `rename.rs`, `recovery.rs`, `undo.rs`, `copy.rs`, `metadata.rs`, `duplicate_events.rs`; new test binaries listed below. |
| Blueprint | New `crates/vidlib-blueprint/Cargo.toml`; `src/lib.rs`, `model.rs`, `store/`, `identity.rs`, `markers.rs`, `refresh.rs`, `diff.rs`, `operations.rs`, `query.rs`, `templates.rs`, `patterns.rs`, `portability.rs`, `retention.rs`. |
| Desktop adapters | Split modified `src-tauri/src/main.rs` into registration/setup plus `app_state.rs`, `jobs.rs`, `selection.rs`, `commands/rename.rs`, `commands/blueprint.rs`, `metadata.rs`, `entrypoint.rs`. Keep `preferences.rs`/libraries.json compatibility. |
| Interface | Modify `ui/index.html`, `main.js`, `api.js`, `dom.js`, `styles.css`; add `ui/jobs.js`, `ui/exact.js`, focused `ui/rename/`, `ui/recipe/`, and `ui/blueprint/` modules. New CSS stays scoped to these views. |
| Verification/build | New `tests/ui/`, `package.json`, `package-lock.json`, `playwright.config.mjs`, `tools/vendor_media_info.py`, `tools/media-info-manifest.json`, `tools/benchmark-library.ps1`, `.github/workflows/check.yml`, and release evidence under `docs/verification/`. Tests/tools are outside the packaged `ui` directory. |

Dependency direction: `vidlib-inventory <- vidlib-rename <- vidlib-fileops`; Blueprint consumes inventory, recipe data, and the **data-only** `vidlib-fileops::events` interface, never the executor. Tauri owns orchestration. `vidlib-engine` remains unchanged unless a narrow tested coordinator/receipt adaptation requires it; do not replace its scan implementation in this project. Add dependencies in their owning task, with explicit platform cfgs and committed lockfile; no speculative framework, ORM, task scheduler, or frontend bundler.

Task file lists use repository-relative paths. The owner/file map above defines each module's responsibility; shared manifest/lockfile and desktop navigation edits remain serialized by the lead. Integration tests stay with their owning crate, and browser fixtures stay outside the packaged `ui` directory.

## 4. Shared contracts to implement once

### 4.1 Native paths, IDs, observations

Task 01 owns opaque UUID newtypes in `vidlib_inventory::ids`: `InventoryId`, `RootId`, `EntryId`, `GrantId`, `SessionId`, `JobId`, `PlanId`, `GroupId`, `StepId`, `OperationId`, `EventId`, `PresetId`, `CollectionId`, `AttemptId`, `AttachmentId`. IDs are not permissions. Revisions are Rust `u64`; schema versions are small `u32`.

`NativePathRecord { platform: WindowsUtf16Le | UnixBytes, bytes: Vec<u8> }` round-trips exact OS strings. Persist path keys as bytes, with a separate display string; only root-relative validated records enter portable inventories. Never use display strings or physical file IDs as unique path keys. Unsupported/unroundtrippable rename names remain visible issues. All new JSON u64/i128 counts/times/revisions are validated decimal strings; UUIDs are strings. SQLite stores exact quantities as decimal text, with fixed-width big-endian numeric sort keys where numeric indexing is required. Do not cast them to SQLite REAL or JS Number.

`FileStamp` contains optional physical identity, size for regular files, native-precision created/modified/accessed epoch-nanoseconds (`Option<i128>`), and Windows attribute bits. `PhysicalId` includes platform, volume identity, and file identity. `EntrySnapshot` contains EntryId, RootId, exact relative path, Rust-owned absolute PathBuf, display_name/display_path, kind, FileStamp, observation time, and issue state. File hard-link aliases remain separate entries. `RootBinding` contains RootId, backend-owned source grant, PathBuf, evidence, and capability facts; a binding records evidence absence separately from mismatch.

`Inventory { id, generation, roots, entries, coverage, window, issues }` is immutable. `ScopeCoverage` records exact directory, **listed child names even if stat fails**, `children_complete`, `subtree_complete`, outcome, filter fingerprint, observation interval, and issues. `JobControl` owns cancellation (`Arc<AtomicBool>`) and a bounded progress sink; `Progress { phase, done, total, current_entry }` is throttled to 10/s.

Public inventory interface: `scan(request: &InventoryRequest, control: &JobControl) -> Result<Inventory, InventoryError>`. Request grants come from backend native selection/validated manual paths; inventory sources can be roots or explicit files. Child traversal never follows a link/reparse point.

Task 01 also defines read-only `DirectoryFacts` and `NamespaceSnapshot` in inventory/model.rs: exact observed parent paths/entries, per-parent case/name/length facts, filesystem capabilities, source grants, protected-root/marker policy, persisted namespace/policy epoch and capture interval. `PlatformFs::capture_namespace(grants: &GrantSet, candidates: &[PathBuf])` in Task 02 populates these data types. Their definition never imports Rename/fileops. A test adapter can supply this same data to the pure planner.

### 4.2 Recipe semantics and immutable plans

Task 03 owns `RecipeV1 { schema_version: 1, revision, selection, numbering_order, rules, companions, destination, collision_policy, metadata_changes }`. Every rule is tagged data with its own ID, enabled flag, segment selector, and parameters; reject unknown tags/versions. No eval or embedded JavaScript.

Pin these semantics before implementing rules:

- Text positions are 1-based Unicode extended grapheme indices; negative positions count from the end (`-1` is last), endpoints are inclusive, zero/out-of-range is a row issue. Delimiter occurrence is 1-based, literal, left-to-right/non-overlapping; delimiters are excluded unless explicitly requested. Resolve each rule's segment against that rule's **current** input; do not alter outside text.
- Case conversion uses Unicode word boundaries/full mappings; exceptions are whole-word case-insensitive matches restored to the configured spelling. Roman handling is an explicit Keep/Upper/Lower option, disabled unless selected. Transliteration is explicit. Preserve original spelling/normalization otherwise.
- PCRE2 operates on supported Unicode names with UTF/UCP enabled, JIT off. Replacements support `$1`, `${name}`, and `$$`; invalid capture references are recipe errors, an unmatched valid optional capture yields empty text. First/all and case-sensitive/insensitive are explicit.
- Counter: `start + floor(ordinal/repeat_each) * step`, checked arithmetic; ordinal starts at 0. Alphabetic is spreadsheet-style `1=A, 26=Z, 27=AA` and rejects nonpositive results. Decimal/hex accept signed values; padding applies to magnitude after the sign. Per-folder resets use the **original** source parent. Companions do not consume ordinals. Display sorting never edits numbering order.
- Date formats use the documented chrono/strftime subset, not locale guessing. Require an input format for ambiguous numeric dates. Timezone is UTC, a confirmed IANA zone, or fixed offset; resolve local zone explicitly and freeze it. Reject DST ambiguity/nonexistence unless the recipe explicitly chooses a resolution. Absent recording timezone stays unknown.
- Templates use an AST, not a second executable language: `TemplatePart::Literal(String)`, `Field { key: FieldKey, format: ValueFormat, fallback: Option<String> }`, and `Optional(Vec<TemplatePart>)`. Missing required fields block; Optional renders nothing if an inner required field is missing. Token chips in the common editor edit this AST. Fields cover original/current stem, ext, parent level, relative directory, counter, dates, exact size, canonical media properties, hashes, UUID/random values. Explicit extension mode alone changes extension unless WholeName is selected.
- Random/UUID, now/timezone, hashes, and requested metadata are frozen in `FrozenInputs` for a preview. Explicit missing-field/fallback decisions are preserved. Do not silently regenerate inputs during execution.

`evaluate(inventory: &Inventory, recipe: &RecipeV1, inputs: &FrozenInputs) -> Result<EvaluatedBatch, RenameError>` returns candidate names/paths, group ownership, and per-rule explanations. `PlatformFs::capture_namespace(grants, candidates) -> Result<NamespaceSnapshot, FsError>` supplies observed target entries, per-parent case/name limits, and capabilities. `plan(inventory, evaluated, namespace, version: PlanRef) -> Result<RenamePlan, RenameError>` is deterministic; it does not perform writes.

`PlanRef { session_id, plan_id, revision }`; `RenamePlan` holds input generation, recipe revision, immutable rows/counts/issues, execution graph, frozen inputs and grants. Task 03 owns the pure `ExecutionPlan`/`ExecutionStep` data types in rename/model.rs; Task 06 builds them. ExecutionPlan includes ordered dependency groups, exact steps and inspection provenance (approved PlanRef, inventory/recipe snapshots, frozen values). These data types never depend on fileops. `PlanRow` exposes old/new display names/paths, exact quantities, source EntryId, group, status (`Changed | Unchanged | NeedsReview | Blocked`), and explanations. Native paths remain in backend-only fields. `PlanStore` is process/session scoped; invalidation revokes confirmation nonces and old PlanRefs. Apply cannot accept caller-supplied target paths.

Preview orchestration order is fixed: validate grants/inventory/recipe -> select and resolve explicit associations -> freeze the primary numbering vector -> determine requested input fields and prepare their frozen values -> evaluate ordered primary rules and propagate companion names -> resolve destination layouts -> capture target namespace/protected-root policy -> build/validate the graph for the complete selection -> publish the current PlanRef/summary. Do not assign counters before companion ownership is known or derive companions from partially transformed primary names. Preserve per-entry/rule generated values within the current inventory's FrozenInputs until explicit regeneration or inventory refresh.

### 4.3 Execution, journal, events, and coordination

Task 04 owns Journal, StepReceipt, event/state persistence and `Journal::open(path: &Path) -> Result<Journal, OpError>`, `begin(&mut self, plan: &ExecutionPlan) -> Result<OperationId, OpError>`, `intent(&mut self, op, step: &ExecutionStep) -> Result<(), OpError>`, `complete(&mut self, op, receipt: &StepReceipt) -> Result<(), OpError>`, `pending(&self) -> Result<Vec<PendingOperation>, OpError>`, `events_after(cursor, limit) -> Result<EventPage, OpError>`, and `ack_events(&mut self, ids: &[EventId]) -> Result<(), OpError>`. ExecutionPlan/ExecutionStep are imported from Rename's pure model, not redefined. A connection is opened/used on its owning blocking thread; read queries use separate connections. Set/verify WAL, FULL, foreign_keys=ON, busy_timeout=5000 ms; migrations/intent/receipt transactions use BEGIN IMMEDIATE and explicit commit. No SQLite transaction spans filesystem I/O.

Own schema: `schema_migrations`, `operations`, `steps`, `receipts`, `operation_events`, `outbox_ack`, `history_visibility`; Task 25 adds protected-root policy tables. Each step stores full native path codec, expected prestate, object/temporary ownership, group/sequence, and observed poststate; plan/recipe inputs are stored for inspection but recovery uses the compiled steps. State machine: Prepared -> Running -> Completed / Cancelled / Failed / RecoveryRequired; steps Planned -> IntentDurable -> Applied -> CompletionDurable, or Compensated/Unresolved. Applied is the observed in-memory action state until its receipt transaction commits; a restart without that receipt follows reconciliation, not a fabricated durable flag. SQLite commit failure stops the next filesystem step.

`ExecutionStep` variants cover RenameEntry, CreateDirectory, StageCopy, PublishCopy, RemoveSource, SetMetadata, RemoveOwnedCopy, and RemoveOwnedEmptyDirectory, with exact expected states. `StepReceipt` records actual paths/identity, digest when applicable, metadata, completed phase, and durability result. Store intent before action and receipt after action; reconcile the action/receipt gap by identity, not blind retries. One dependency group is the compensation unit; completed prior groups remain completed.

Rename-cycle temporary names are logical reservations recorded in the graph/journal; the first no-replace move materializes the path atomically. Do not create an empty placeholder and then overwrite it. Copy staging uses create-new ownership with a recorded native identity, and publication still uses no-replace semantics.

`OperationEventV1` contains stable EventId, OperationId, GroupId, StepId, kind, completed phase, native before/after paths, before/after FileStamp, digest if available, observed time, `origin: App`, and journal sequence/provenance. Emit facts for each completed phase, including a verified published copy whose later source removal fails. Events and receipts commit together. Imported reports never become executable journals.

Task 05 owns `Coordinator::open(app_data: &Path) -> Result<Coordinator, OpError>` and `try_acquire(kind: LeaseKind) -> Result<Lease, OpError>` with RAII Read/Write/Recovery leases. Use Tauri's `app.path().app_local_data_dir()` for the two databases and lock file, validate local storage, and retain the existing app_config_dir libraries.json location. Use a per-app-data OS advisory lock file (shared readers, exclusive writer), plus process-local job registry and startup pending-journal gate. A recovery lease is exclusive but may enter RecoveryRequired state. No destructive job bypasses it, including legacy duplicate actions. A second instance cannot write to this app data concurrently. Denied acquisition returns Busy/RecoveryRequired, not a blocking UI wait.

The operation store also owns a monotonic namespace/policy epoch. Read/capture it under a read lease, store it with the plan, and compare again inside the write lease before preflight. Increment it after any app path mutation or protected-root policy update, including partial success, before releasing the lease. Read queries/registries use it to revoke stale results across app instances. An intent/receipt failure leaves the recovery gate active rather than authorizing a stale plan. Blueprint root registration cannot retroactively permit an older plan to rename a newly protected root.

Task 07 owns `execute(plan: &RenamePlan, lease: &Lease, journal: &mut Journal, fs: &mut dyn Filesystem, control: &JobControl) -> Result<OperationReport, OpError>`. The real Windows and Linux adapters and fault-injection test adapter implement `Filesystem`: capture/verify state, no-replace rename/publish, owned directory/staging creation, verified content I/O, metadata changes, and flush results. `OpError` has stable code, phase, operation/step/item IDs when known, message, and retryability; UI receives a serializable DTO, not a panic/debug dump.

Task 08 owns `reconcile(op: &PendingOperation, fs: &mut dyn Filesystem) -> Result<RecoveryPreview, OpError>` and `apply_recovery(preview, choice: Resume | Restore, lease, journal, fs, control) -> Result<OperationReport, OpError>`. Task 09 owns `plan_undo(completed: &OperationRecord, namespace: &NamespaceSnapshot, version: PlanRef) -> Result<RenamePlan, OpError>`; inverse operations use the same executor/journal/confirmation flow.

### 4.4 Tauri/JavaScript and Blueprint contracts

Task 10 exposes owned command request structs and `Result<_, CommandError { code, message, phase, item_id, operation_id, retryable }>`; serialize using camelCase consistently. Commands clone Arc-managed handles under short locks, release guards, then start blocking workers. Job types carry owned results; query responses page at 200 rows. Never keep a std MutexGuard or rusqlite Connection borrow across await.

Core commands: `select_sources(PickerKind) -> SelectionDto`, `start_inventory(InventoryRequestDto) -> JobId`, `start_rename_preview(RenameRequestDto) -> JobId`, `job_status(JobId) -> JobStatusDto`, `cancel_job(JobId)`, `rename_page(PlanRef, PageQuery) -> Page<PlanRowDto>`, `confirm_rename(PlanRef) -> ConfirmationDto`, `apply_rename(PlanRef, confirmation_nonce) -> JobId`, `operation_history(PageQuery) -> Page<OperationDto>`, `recovery_preview(OperationId)`, `start_recovery(RecoveryRef, choice, nonce)`, `start_undo(OperationId) -> JobId`. Nonces bind the current session/revision/summary and are revoked on edits. Queries do not reauthorize native paths.

All **new** commands use one owned argument named `request` (wrap multi-field cases in the corresponding request DTO) plus managed state; JS invokes them with `{ request: dto }`. Existing commands retain their prior argument shape. Task 10 adds `src-tauri/tests/command_contracts.rs` and `tests/ui/fixtures/command-contracts.json` to verify camelCase/tag/decimal-string serialization against the same golden payloads used by browser mocks. PageQuery contains cursor, limit capped at 200, display sort/filter, and expected data revision; it never changes recipe numbering order.

Common JS interfaces: `watchJob(jobId, {signal, onProgress}) -> Promise<JobResult>` cleans up timers on finish/navigation/error; `createRecipeEditor({recipe, onChange}) -> {getRecipe(), setRecipe(recipe), destroy()}` never computes filenames; `formatExactBytes(decimalString)` uses BigInt and string-safe formatting. `call()` in `ui/api.js` remains compatible with existing commands while new errors are formatted centrally.

Blueprint Task 24 owns `BlueprintStore::open(path: &Path) -> Result<Self, BlueprintError>`, collection/RootBinding revision records, and paged queries. Task 25 owns `recognize(evidence: &RecognitionInput) -> RecognitionDecision` (`Known`, `CandidateNeedsConfirmation`, `IdentityConflict`, `MarkerOnlyNoLocalData`, `Unknown`), `register`, and `rebind` with expected binding revision. Task 26 owns `begin_refresh(collection_id, request) -> AttemptId`, `stage_inventory(attempt, inventory)`, and `promote_refresh(attempt) -> RefreshReport`; promotion commits baseline scopes and observed events together.

Task 27 owns `project_events(events: &[OperationEventV1]) -> ProjectionReceipt` with EventId uniqueness and transactional cursor updates. Task 28 owns `attach_template(collection, scope, preset_id, revision, recipe_snapshot) -> AttachmentId` and `summarize_patterns(selection, baseline_revision) -> PatternSummary`. Task 30 owns `export_bundle`, `validate_import`, `import_inactive`, `prune_history`, and `remove_collection`; none grants filesystem access or accepts executable journals.

Blueprint queries return `Page<MapEntryDto>`, `CollectionSummaryDto`, `Page<ChangeDto>` with observation interval/origin/confidence and separate last-attempt/last-complete times. Its commands mirror the store operations through owned DTOs, not SQL rows. Sending cached items to Rename resolves an explicit fresh native selection and scans again; stale EntryIds are never reused.

## 5. Dependency map and completion gates

Implement task order unless the following dependencies allow safe parallel work. Shared-file edits are serialized by the lead; no two agents rewrite Cargo.lock, main.rs, or index.html concurrently.

| Track | Tasks | Gate/dependencies |
| --- | --- | --- |
| Safe foundation | 01–10 | 01 -> 02/03; 03 -> 04/06; 02+04 -> 05; 02–06 -> 07 -> 08 -> 09; 05–09 -> 10. |
| Naming breadth | 11–18 | 11–16 need 03; 12 needs the PCRE2 adapter proof; 13 -> 14; 14+17 -> metadata templates; 18 needs 06 and recipe/group data. Integrate all through Task 21. |
| Full execution/UI | 19–23 | 19 needs 07–09+18; 20 needs 02/04/09; 21 needs 10–20; 22 needs 10/21; 23 verifies 01–22. **Complete Rename gate.** |
| Blueprint foundation | 24–31 | 24 needs 01/04 contracts; 25 -> 26; 27 needs 04/05 and 26; 28 needs 16/26; 29 needs 24–28/10; 30 needs 24–29; 31 verifies 24–30. It may start after shared contracts stabilize. |
| Final handoff | 32 | Separate truthful completion records for Rename and Blueprint; Blueprint polish must not delay a verified Rename release. |

## 6. Tasks

### Task 01: Native-path inventory and coverage

**Files:** Create the `crates/vidlib-inventory` files listed in section 3, `crates/vidlib-inventory/tests/support/mod.rs`, `crates/vidlib-inventory/tests/inventory.rs`, `crates/vidlib-inventory/tests/path_codec.rs`; modify root `Cargo.toml`/`Cargo.lock`.

**Interfaces:** Produces IDs, native path codec, FileStamp/RootBinding/Inventory/ScopeCoverage/JobControl and `scan` from 4.1; no dependency on Rename or Blueprint. Tests use actual temporary directories and a read-only OS adapter seam for injected stat/listing failures.

- [ ] **Step 1 — Red:** Add `scan_keeps_unique_empty_and_nonvideo_entries`, `overlap_deduplicates_paths_not_hardlinks`, `failed_stat_retains_listed_child`, `path_codec_round_trips_native_units`, and `cancellation_preserves_coverage`. Fixture has `a.mkv`, zero-byte `b.mp4`, `notes.txt`, a nested directory, overlap, a link, and two hard-link paths. Required assertions:
  ```rust
  assert!(inventory.entries.iter().any(|e| e.display_name == "b.mp4"));
  assert_eq!(hardlink_entries.len(), 2);
  assert!(!failed_scope.subtree_complete);
  assert!(failed_scope.listed_names.contains(&failed_name));
  assert_eq!(decode_path(&encode_path(&native_path))?, native_path);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-inventory --test inventory --test path_codec`; it must fail because the public inventory contract is missing, not because a test touched a user directory.
- [ ] **Step 3 — Implement:** Add workspace member, serde/uuid/walkdir dependencies and the 4.1 interface; native timestamp precision, no-follow traversal, per-directory coverage, read-only volume/file identity adapters, protected marker/staging exclusions. Supply deterministic injected cancellation/failure fixture support. JSON exact-number/path tests belong here.
- [ ] **Step 4 — Verify green:** Same command, then `cargo clippy -p vidlib-inventory --all-targets -- -D warnings`. Exercise non-UTF8 Unix bytes and Windows UTF-16 fixture paths; report unsupported transformations without lossy lookup.
- [ ] **Step 5 — Commit:** `feat: add native-path collection inventory`. Include only this crate and required workspace/lock changes.

### Task 02: Prove the no-replace filesystem adapters

**Files:** Create `crates/vidlib-fileops/src/platform/mod.rs`, `crates/vidlib-fileops/src/platform/windows.rs`, `crates/vidlib-fileops/src/platform/linux.rs`, `crates/vidlib-fileops/src/error.rs`, `crates/vidlib-fileops/tests/platform.rs`, `crates/vidlib-fileops/tests/windows_platform.rs`; modify `crates/vidlib-fileops/src/lib.rs`, crate/root manifests and Cargo.lock; add `docs/verification/filesystem-capabilities.md` during execution.

**Interfaces:** Consumes FileStamp/native paths; produces Filesystem, PlatformFs populated NamespaceSnapshot, per-parent DirectoryFacts, capability evidence and OpError. NamespaceSnapshot/DirectoryFacts are defined by Task 01. The Filesystem seam exposes native primitives and does not depend on future Rename recipe/step types. Use the Windows API binding for handle-aware inspection/rename and Linux `renameat2(RENAME_NOREPLACE)` through rustix; unsupported targets fail closed. Do not call private Rust std internals or use an existence-check/overwriting rename fallback.

- [ ] **Step 1 — Red:** Add `existing_target_never_replaced`, `target_created_after_inspection_is_not_overwritten`, `directory_case_policy_is_per_parent`, `reparse_ancestor_rejected`, and Windows locked/case-only/directory tests. Assert both source/destination contents survive the conflict and sibling directories retain their distinct case policy.
  ```rust
  assert_eq!(err.code, ErrorCode::DestinationExists);
  assert_eq!(std::fs::read(&target)?, b"existing");
  assert!(source.exists());
  assert_ne!(insensitive.case_sensitive, sensitive.case_sensitive);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-fileops --test platform`; Windows: `cargo test -p vidlib-fileops --test windows_platform -- --nocapture`.
- [ ] **Step 3 — Implement:** Pin the public Filesystem seam and no-replace primitives using the `windows` crate under cfg(windows) and rustix under cfg(target_os="linux"), native handle identity/freshness/reparse checks, per-directory case/length/reserved-name facts, create-new staging ownership, flush/durability outcomes, and typed unsupported results. Add capability fixtures and record tested API/OS/filesystem combinations. Distinguish deterministic process-termination recovery from unsupported hardware/power-loss guarantees; a journal does not make a whole batch atomic. Prove metadata setters now, but expose them through recipes only in Task 20.
- [ ] **Step 4 — Verify green/platform gate:** Run both tests on their platforms; record NTFS file+folder behavior, exFAT/removable and SMB capabilities or explicit unsupported outcomes. A Linux-only session leaves Windows rows pending and continues Tasks 03/06/11–16 without assuming Windows support.
- [ ] **Step 5 — Commit:** `feat: add verified no-replace filesystem adapters` with the actual capability evidence.

### Task 03: Versioned recipes and deterministic plan models

**Files:** Create `crates/vidlib-rename/Cargo.toml`, `crates/vidlib-rename/src/lib.rs`, `crates/vidlib-rename/src/model.rs`, `crates/vidlib-rename/src/recipe.rs`, `crates/vidlib-rename/src/evaluate.rs`, `crates/vidlib-rename/tests/contracts.rs`; modify root Cargo.toml/Cargo.lock.

**Interfaces:** Consumes Inventory; produces every 4.2 model, `evaluate`, backend-only RenamePlan/PlanRef, serializable DTO projections, and stable RenameError codes. Rule evaluation initially supports Keep/FixedStem; later tasks supply each rule implementation through the same pipeline.

- [ ] **Step 1 — Red:** Add `unknown_recipe_version_or_rule_rejected`, `preview_freezes_clock_and_random`, `unselected_or_outside_grant_id_rejected`, `json_preserves_large_sizes`, and `explanations_follow_rule_order`.
  ```rust
  assert_eq!(first.rows, repeated.rows);
  assert_eq!(json_row.size_bytes.as_deref(), Some("9007199254740993"));
  assert_eq!(err.code, RenameErrorCode::UnknownRecipeVersion);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test contracts`.
- [ ] **Step 3 — Implement:** Implement 4.2's tagged RecipeV1, typed template AST, FrozenInputs, EvaluatedBatch and owned plan models with backend-only native fields. Pin field/format enums and reject invalid combinations before rows are generated. Implement schema round-trip tests; use checked arithmetic and exact serialization. Do not expose unimplemented rule controls yet.
- [ ] **Step 4 — Verify green:** Contracts test plus `cargo clippy -p vidlib-rename --all-targets -- -D warnings`; repeated identical frozen inputs yield byte-identical DTO plans except explicitly supplied PlanRef.
- [ ] **Step 5 — Commit:** `feat: define deterministic rename recipes and plans`.

### Task 04: Durable operation journal and event outbox

**Files:** Create `crates/vidlib-fileops/src/journal/mod.rs`, `crates/vidlib-fileops/src/journal/schema.rs`, `crates/vidlib-fileops/src/journal/migrations.rs`, `crates/vidlib-fileops/src/events.rs`, `crates/vidlib-fileops/tests/journal.rs`; modify src/lib.rs and manifests/Cargo.lock. Enable rusqlite bundled/backup; add dependency on vidlib-rename.

**Interfaces:** Produces Journal, StepReceipt, OperationEventV1 and state machine from 4.3. Consumes ExecutionPlan/ExecutionStep from Task 03 and native codec; no filesystem mutation occurs in SQLite methods.

- [ ] **Step 1 — Red:** Add `intent_survives_reopen`, `receipt_and_event_commit_together`, `commit_error_prevents_next_step`, `published_copy_without_source_removal_is_distinct_fact`, `unknown_schema_preserves_original`, and exact numeric/native-path persistence tests.
  ```rust
  assert_eq!(reopened.pending()?.len(), 1);
  assert_eq!(events.items[0].phase, CompletedPhase::CopyPublished);
  assert!(!events.items.iter().any(|e| e.phase == CompletedPhase::SourceRemoved));
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-fileops --test journal`.
- [ ] **Step 3 — Implement:** Implement the specified local schema/PRAGMAs, checked migration transactions, explicit commit/error propagation, per-phase receipts, stable outbox IDs and pending-operation queries. Create a small private failpoint seam for commit failures; failpoints are fixture/development code, not a production IPC switch. Keep journal import impossible.
- [ ] **Step 4 — Verify green:** Journal tests against real temporary database files (not in-memory WAL substitutes); inspect effective PRAGMA values after reopen and force SQLITE_BUSY/failed commit. Confirm the original migration fixture is retained.
- [ ] **Step 5 — Commit:** `feat: persist operation intents receipts and outbox events`.

### Task 05: App-wide and cross-process coordination

**Files:** Create `crates/vidlib-fileops/src/coordinator.rs`, `crates/vidlib-fileops/tests/coordinator.rs`, `src-tauri/src/app_state.rs`; modify fileops src/lib.rs, `src-tauri/src/jobs.rs`, `src-tauri/src/main.rs`, manifests/Cargo.lock.

**Interfaces:** Produces Coordinator/Lease and recovery gate from 4.3. Extend desktop state with Arc-managed plan/job registries and app-data paths, while retaining libraries.json preferences. Use fs4's OS shared/exclusive lock adapter, owned by RAII guards; each lease owns a separately opened lock-file handle and is not Clone, so one reader's Drop cannot release another reader's lock.

- [ ] **Step 1 — Red:** Add `readers_exclude_writer`, `writer_excludes_second_process`, `panic_releases_lease`, `startup_pending_journal_blocks_writes`, `other_process_epoch_revokes_old_plan`, and desktop tests that duplicate scans/actions use the same coordinator. A spawned fixture process must actually contend on the same lock file.
  ```rust
  assert_eq!(writer_attempt.unwrap_err().code, ErrorCode::Busy);
  assert_eq!(new_write.unwrap_err().code, ErrorCode::RecoveryRequired);
  assert!(coordinator.try_acquire(LeaseKind::Recovery).is_ok());
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-fileops --test coordinator`; run desktop job tests where Tauri dependencies exist.
- [ ] **Step 3 — Implement:** Short process-local locks plus shared read/exclusive write OS lock file; check pending journals while holding the relevant lock. Integrate existing duplicate scan/action entry points and close-window handling. Cache browsing is allowed; new physical scans wait/refuse while a write/recovery is active. Return typed busy status and release all guards on error/panic.
- [ ] **Step 4 — Verify green:** Coordinator subprocess tests and existing job tests. Verify two installed app processes cannot mutate concurrently, not just two threads of one Jobs struct.
- [ ] **Step 5 — Commit:** `feat: coordinate collection reads writes and recovery`.

### Task 06: Validate final paths and build dependency graphs

**Files:** Create `crates/vidlib-rename/src/validate.rs`, `crates/vidlib-rename/src/graph.rs`, `crates/vidlib-rename/tests/graph.rs`; extend model/evaluate; consume NamespaceSnapshot/DirectoryFacts from `vidlib-inventory::model` without creating a fileops dependency cycle.

**Interfaces:** Produces `plan` from 4.2 and an identity-based graph of ExecutionSteps/data. Native Filesystem effects stay in fileops; graph data belongs to rename. Namespace capture from Task 02 returns this shared data contract.

- [ ] **Step 1 — Red:** Add `swap_and_three_cycle_plan_without_overwrite`, `case_only_rename_uses_reserved_temp`, `parent_and_child_keep_one_location_model`, `target_parent_case_policy_controls_collisions`, `root_and_marker_protected`, `outside_destination_and_ancestor_cycle_blocked`, and `hidden_page_conflict_blocks_entire_approval`.
  ```rust
  assert!(plan.rows.iter().any(|r| r.status == RowStatus::Blocked));
  assert_eq!(plan.summary.blocked_count, 1);
  assert_eq!(final_child_path, expected_renamed_parent.join("new.mkv"));
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test graph`.
- [ ] **Step 3 — Implement:** Resolve final paths globally, use per-parent name policy, derive strongly connected rename dependencies and recorded temporary reservations, update current locations when ancestors move, and expose carried-descendant counts. Validate source/destination ancestry, Windows names/lengths/ADS/device targets, grants, collision suffixes frozen in preview, and protected roots. Conflicting descendant exclusions become issues rather than impossible promises.
- [ ] **Step 4 — Verify green:** Graph tests include invalid Unicode display aliases, long/UNC paths, case-sensitive siblings, hard-link paths and target-created-after-preview preflight fixtures. Domain graph tests never write source fixtures.
- [ ] **Step 5 — Commit:** `feat: plan collision-safe hierarchical renames`.

### Task 07: Journaled in-place rename execution

**Files:** Create `crates/vidlib-fileops/src/rename.rs`, `crates/vidlib-fileops/tests/rename.rs`; extend platform/journal/events; export execute from src/lib.rs.

**Interfaces:** Implements execute from 4.3 for in-place rename graph groups, consuming the exact approved PlanRef/source snapshots, Write lease, Journal and Filesystem. It emits receipts/events, never generates target names at runtime.

- [ ] **Step 1 — Red:** Add `executes_exact_reviewed_plan`, `late_stale_item_changes_nothing_in_preflight`, `no_runtime_collision_suffix`, `all_graph_steps_have_prior_durable_intent`, and `failed_group_stops_following_groups`.
  ```rust
  assert_eq!(std::fs::read(&expected_target)?, b"fixture");
  assert!(!unexpected_suffix.exists());
  assert_eq!(recorded_intents_before_actions, executed_step_ids);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-fileops --test rename`.
- [ ] **Step 3 — Implement:** Whole-selection preflight, then per-step identity/ancestor/freshness checks and no-replace execution under the writer lease. Flush intent before action and completion after; stop on journal or filesystem error. Retain unresolved steps and invalidate path-dependent results before releasing the writer. Avoid broad refactors of old duplicate actions.
- [ ] **Step 4 — Verify green:** Rename tests on real temporary files and fault-injection adapter; Windows tests for sharing violations and timestamps. Cancel at deterministic group boundaries; assert completed groups are reported honestly.
- [ ] **Step 5 — Commit:** `feat: execute reviewed rename plans with durable receipts`.

### Task 08: Crash reconciliation and group compensation

**Files:** Create `crates/vidlib-fileops/src/recovery.rs`, `crates/vidlib-fileops/tests/recovery.rs`, `crates/vidlib-fileops/tests/support/crash_child.rs`; extend journal schema/state transitions as versioned migrations.

**Interfaces:** Produces reconcile/apply_recovery/RecoveryPreview from 4.3. Recovery uses compiled journal steps and verified identities, not rerun recipe evaluation. Destructive recovery requires an exclusive Recovery lease and reviewed nonce.

- [ ] **Step 1 — Red:** Add `crash_before_and_after_each_phase_reconciles`, `temporary_names_are_reported`, `unknown_target_prevents_blind_resume`, `compensation_stops_without_deleting_unowned_entries`, and `published_phase_without_receipt_is_reconciled_once`.
  ```rust
  assert_eq!(second_reconcile.actions, first_reconcile.actions);
  assert!(unowned_target.exists());
  assert_eq!(preview.unresolved_groups.len(), 1);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-fileops --test recovery`; crash fixtures terminate a child process, not just return a mocked Result.
- [ ] **Step 3 — Implement:** Reconcile expected pre/post/current paths and identities at every intent/action/receipt gap. Generate reviewable Resume/Restore plans, stop further groups after a runtime failure, compensate only verified current-group entries, and retain explicit unresolved ownership. Reconnection and retries cannot silently repeat a completed filesystem step.
- [ ] **Step 4 — Verify green:** Run recovery tests for file/folder cycles and a lost drive fixture. Assert complete preservation/reporting after every failpoint and no writes before journal durability. Extend these same tests in Tasks 19/20 for new step kinds.
- [ ] **Step 5 — Commit:** `feat: reconcile interrupted operations and compensate groups`.

### Task 09: Validated undo and operation history

**Files:** Create `crates/vidlib-fileops/src/undo.rs`, `crates/vidlib-fileops/tests/undo.rs`; extend journal read queries and model DTOs.

**Interfaces:** Produces plan_undo, paged OperationRecord/history DTOs, and explicit history-removal policy. Undo is a new RenamePlan with inverse_of OperationId and the same confirmation/executor path; edit undo/redo comes later in Task 16.

- [ ] **Step 1 — Red:** Add `undo_requires_current_poststate`, `recreated_original_blocks_inverse`, `interrupted_undo_is_recoverable`, `unfinished_history_cannot_be_removed`, and `history_pages_are_stable`.
  ```rust
  assert_eq!(inverse.rows[0].status, RowStatus::Blocked);
  assert_eq!(delete_pending.unwrap_err().code, ErrorCode::RecoveryRequired);
  assert_eq!(page.items.len(), 200);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-fileops --test undo`.
- [ ] **Step 3 — Implement:** Inverse graph from receipts/expected poststates, full conflict/freshness checks, paged read-only history and explicit deletion of completed undo support. Keep pending recovery and undelivered outbox facts independently protected. Later tasks extend typed inverse variants, not a second undo engine.
- [ ] **Step 4 — Verify green:** Undo/history tests plus existing duplicate actions tests. Confirm an undo failure never overwrites a newly created original or removes unrelated content.
- [ ] **Step 5 — Commit:** `feat: add reviewed inverse plans and operation history`.

### Task 10: Usable Rename vertical slice and bounded IPC

**Files:** Create `src-tauri/src/lib.rs`, `src-tauri/src/selection.rs`, `src-tauri/src/commands/mod.rs`, `src-tauri/src/commands/rename.rs`, `src-tauri/tests/command_contracts.rs`, `ui/jobs.js`, `ui/exact.js`, `ui/rename/main.js`, `ui/rename/preview.js`, `ui/rename/history.js`, `tests/ui/rename-basic.spec.mjs`, `tests/ui/fixtures/desktop-bridge.js`, `tests/ui/fixtures/command-contracts.json`, `tools/serve-ui.mjs`, `package.json`, `package-lock.json`, `playwright.config.mjs`; modify app_state/main/jobs, ui/index.html/main.js/api.js/styles.css/dom.js.

**Interfaces:** Implements the core commands and JS interfaces in 4.4 for Keep/FixedStem and current executable plans. Native selection returns grant/item IDs plus display strings; plans/history stay backend-paged. Existing library/finder flows keep their command signatures.

- [ ] **Step 1 — Red:** Add command tests `stale_nonce_cannot_apply`, `old_session_plan_is_invalid`, `page_never_returns_native_paths`; browser tests `rename_basic_preview_confirm_cancel`, `preview_response_after_edit_is_ignored`, `browser_preview_cannot_apply`, `exact_bytes_above_js_safe_integer`, and keyboard/dialog focus behavior.
  ```js
  await expect(page.getByTestId('rename-apply')).toBeDisabled();
  await expect(page.getByTestId('exact-bytes')).toContainText('9,007,199,254,740,993');
  expect(bridge.appliedTargets).toEqual([]);
  ```
- [ ] **Step 2 — Verify red:** `npx playwright test tests/ui/rename-basic.spec.mjs`; desktop command tests with `cargo test -p vidlib-tauri commands::rename` where native dependencies exist.
- [ ] **Step 3 — Implement:** Move desktop setup/command modules into a testable library with `pub fn run()` and keep main.rs as the small binary entry point; expose the narrow owned handler/adapter interfaces required by integration tests. Add Rename navigation/table/minimal rule editor exposing only implemented controls, confirmation summary, status polling, cancel/history/recovery, structured error formatting, generation-aware response acceptance and listener teardown. Use textContent/document fragments and BigInt formatting; no HTML string interpolation or JS rename engine. Scope mocked bridge to tests outside packaged UI; browser sample mode remains read-only. Add Playwright dev scripts and a localhost-only Node static server with correct module MIME types, without a build/bundler/runtime dependency.
- [ ] **Step 4 — Verify green:** Same tests, JS syntax checks, existing finder sample interaction tests, desktop owned-state serialization tests. UI renders at most one 200-row page and cannot enable a stale plan through out-of-order job completion.
- [ ] **Step 5 — Commit:** `feat: add Rename preview execution and recovery interface`.

### Task 11: Text, case, extension, and segment rules

**Files:** Create `crates/vidlib-rename/src/text.rs`, `crates/vidlib-rename/tests/text.rs`; extend recipe/evaluate; add unicode-segmentation and explicit transliteration dependency.

**Interfaces:** Implements R01–R03/R10 through `apply_text(input: &NameParts, rule: &TextRule) -> Result<NameParts, RuleIssue>` and `resolve_segment(current: &str, selector: &SegmentSelector) -> Result<Range<usize>, RuleIssue>` using 4.2 semantics. Public positions are logical grapheme positions; the returned Rust Range is a checked zero-based, half-open UTF-8 byte range for safe slicing. NameParts separates stem/ext; rules produce intermediate explanations for evaluate.

- [ ] **Step 1 — Red:** Add table-driven `text_rule_goldens`, `segment_resolves_each_current_input`, `unicode_graphemes_are_not_split`, `case_exceptions_and_roman_are_explicit`, and `extension_requires_explicit_mode`. Include prefix/suffix/insert, ordered replacement, removal/range/crop, move/copy/reverse, trim/collapse, translation/transliteration, every case mode, Keep/Fixed/Remove stem, extension add/remove/replace/case, and WholeName.
  ```rust
  assert_eq!(actual.filename(), "BBC Show III.mkv");
  assert_eq!(reversed.stem, "e\u{301}faC");
  assert_eq!(ordinary.ext, "MKV"); // stem-only lowercase leaves ext unchanged
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test text`.
- [ ] **Step 3 — Implement:** Resolve grapheme/delimiter ranges, apply only within the segment, reassemble NameParts, and validate without implicit sanitization. Add transliteration only when the recipe enables it; preserve original/current fields separately.
- [ ] **Step 4 — Verify green:** Text goldens, contract tests and Clippy; test negative/zero/out-of-range positions and expanded Unicode case mappings.
- [ ] **Step 5 — Commit:** `feat: add composable Unicode text and extension rules`.

### Task 12: Bounded PCRE2 rule pipeline

**Files:** Create `crates/vidlib-rename/src/regex.rs`, `crates/vidlib-rename/tests/regex.rs`; update recipe/evaluate/Cargo.lock. Use a small safe wrapper over pcre2-sys to expose every approved limit explicitly.

**Interfaces:** `CompiledRegexRule::compile(rule: &RegexRule) -> Result<Self, RenameError>`; `apply(&self, input: &str) -> Result<String, RuleIssue>`. It integrates the existing segment/explanation pipeline and does not add a competing regex dialect for filters.

- [ ] **Step 1 — Red:** Add `lookarounds_pattern_backrefs_and_capture_replacement`, `literal_dollar_and_unmatched_optional_capture`, `first_all_and_case_modes`, `disabled_and_repeated_rules`, and `resource_limits_are_real` in a subprocess fixture.
  ```rust
  assert_eq!(actual, "Show - S02E03");
  assert_eq!(limits.match_limit, 1_000_000);
  assert_eq!(limits.heap_limit_kib, 8192); // PCRE2 uses KiB, not bytes
  assert_eq!(exhausted.code, RuleIssueCode::RegexLimit);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test regex`; the exhaustion fixture has a harness timeout solely to detect a hung test, not a promised regex timeout.
- [ ] **Step 3 — Implement:** UTF/UCP, no JIT, 16 KiB pattern cap, match/depth/heap configuration, named/numbered replacement validation and literal escaping. Compile once per enabled rule; use RAII for native allocations and document each unsafe FFI invariant in this one adapter. Limits/failures are structured row/rule issues, not silent nonmatches.
- [ ] **Step 4 — Verify green:** Regex tests plus text/graph regressions; inspect the effective binding calls for all limits and run the real native adapter on Windows CI.
- [ ] **Step 5 — Commit:** `feat: add resource-bounded PCRE2 renaming rules`.

### Task 13: Deterministic numbering and explicit dates

**Files:** Create `crates/vidlib-rename/src/numbers.rs`, `crates/vidlib-rename/src/dates.rs`, `crates/vidlib-rename/tests/numbers_dates.rs`; update recipe/evaluate and lockfile. Use chrono/chrono-tz with an explicit local-zone resolver; no locale date guessing.

**Interfaces:** `assign_numbers(entries, order: &NumberingOrder, rule: &NumberRule) -> Result<NumberAssignments, RenameError>`; `format_date(value: &DateValue, rule: &DateRule, env: &FrozenEnvironment) -> Result<String, RuleIssue>`. FrozenEnvironment records now, zone/offset, and selected ambiguity policy.

- [ ] **Step 1 — Red:** Add `natural_display_sort_does_not_change_number_order`, `counter_repeat_step_reset_and_formats`, `pad_existing_number_in_segment`, `ambiguous_date_requires_format`, `missing_recording_timezone_stays_unknown`, and `dst_ambiguity_is_explicit`.
  ```rust
  assert_eq!(alphabetic_values, ["Y", "Z", "AA"]);
  assert_eq!(repeated_values, ["01", "01", "03", "03"]);
  assert_eq!(reformatted, "2026-10-17"); // %d-%m-%Y input, %Y-%m-%d output
  assert_eq!(ambiguous.code, RuleIssueCode::DateNeedsFormat);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test numbers_dates`.
- [ ] **Step 3 — Implement:** All R06 settings and source-parent resets; checked decimal/hex/alphabetic arithmetic; original vector tie-breaks; date parsing, insert/reformat and fixed/UTC/IANA modes. Distinguish filesystem-created/modified/accessed from recording time. Resolve DST only through recipe policy; freeze now/zone once.
- [ ] **Step 4 — Verify green:** Counter/date tests, including overflow, negative step, repeat_each=0, absent file dates, and a Europe/Berlin repeated/nonexistent local time fixture.
- [ ] **Step 5 — Commit:** `feat: add stable numbering and explicit date formatting`.

### Task 14: Typed templates and lazy frozen hash/random inputs

**Files:** Create `crates/vidlib-rename/src/template.rs`, `crates/vidlib-rename/src/hashes.rs`, `crates/vidlib-rename/tests/templates_hashes.rs`; extend metadata field model/evaluate. Add explicit hash/UUID/random dependencies without changing the old engine's MD5 interface.

**Interfaces:** `render_template(parts: &[TemplatePart], context: &TemplateContext) -> Result<String, RuleIssue>`; `freeze_inputs(request: &InputRequirements, inventory: &Inventory, control: &JobControl) -> Result<FrozenInputs, RenameError>`. File I/O is a separate input-preparation stage, not inside pure evaluation; source freshness is checked before/after content reads.

- [ ] **Step 1 — Red:** Add `all_template_field_families`, `missing_required_optional_and_fallback`, `original_and_current_stem_differ`, `hashes_match_independent_full_file_vectors`, `text_recipe_reads_no_contents`, `generated_tokens_survive_repeated_preview`, and `hash_cancel_and_stale_source`.
  ```rust
  assert_eq!(optional_result, "Movie (2020)"); // no dangling resolution brackets
  assert_eq!(abc_sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  assert_eq!(content_read_count_for_text_only, 0);
  assert_eq!(first.uuid_tokens, second.uuid_tokens);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test templates_hashes`.
- [ ] **Step 3 — Implement:** Template AST fields/formatters, fallback/optional behavior, exact numeric/date formatting; prepare each requested full-file CRC32/MD5/SHA-1/SHA-256/SHA-512 digest once using a bounded 1 MiB buffer and chunk cancellation. Store clock/random results in FrozenInputs. Verify all five algorithms against fixed independent known vectors, not another call to the production hash function.
- [ ] **Step 4 — Verify green:** Template/hash tests plus date contracts; compare files differing only beyond the first buffer and assert a required missing metadata property blocks the row.
- [ ] **Step 5 — Commit:** `feat: render typed templates with frozen lazy inputs`.

### Task 15: Selection filters, mappings, overrides, and exports

**Files:** Create `crates/vidlib-rename/src/selection.rs`, `crates/vidlib-rename/src/mappings.rs`, `crates/vidlib-rename/tests/selection_mappings.rs`; extend DTOs/evaluate; use a bounded CSV parser rather than split-on-comma.

**Interfaces:** `select(inventory: &Inventory, filter: &SelectionFilter) -> Result<Vec<EntryId>, RenameError>`; `import_mappings(reader, format, inventory, grants) -> Result<MappingSet, RenameError>`; `export_report(plan, SpreadsheetSafe | ExactMapping) -> Result<Vec<u8>, RenameError>`. Explicit mapping format v1 has `old_path,new_path` with quoted CSV or a user-selected delimiter; exact path lookup precedes unique-basename fallback. Pasted new-name lists map the frozen explicit order and require exact cardinality.

- [ ] **Step 1 — Red:** Add `every_filter_dimension`, `exclude_rebuilds_numbers_and_collisions`, `ambiguous_basename_and_outside_root_rejected`, `quoted_csv_and_paste_cardinality`, `mapping_limits_before_allocation`, and `spreadsheet_safe_export_does_not_change_recorded_path`.
  ```rust
  assert_eq!(err.code, RenameErrorCode::AmbiguousMapping);
  assert_eq!(limit_error.code, RenameErrorCode::ImportLimit);
  assert_eq!(stored_original_name, "=SUM(1,2).mkv");
  assert!(spreadsheet_cell.starts_with('\''));
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test selection_mappings`.
- [ ] **Step 3 — Implement:** Wildcard/PCRE2 filters across every R11 field, depth/kind/extension/date/size/attribute selection, exact path mapping, explicit row override, 64 MiB/100,000-row streaming limits and safe reports. Reject traversal/duplicate mappings and unknown IDs. Distinguish spreadsheet-safe reports from lossless mapping exports so protection prefixes never accidentally become renamed filenames.
- [ ] **Step 4 — Verify green:** Boundary tests at/above byte/row limits and quoted Unicode/multiline CSV; all-row preflight after selection changes. Formula-like input is literal data and never executed.
- [ ] **Step 5 — Commit:** `feat: add bounded selection mappings and report workflows`.

### Task 16: Presets and the single recipe editor

**Files:** Create `crates/vidlib-rename/src/presets.rs`, `crates/vidlib-rename/tests/presets.rs`, `src-tauri/src/rename_preferences.rs`, `ui/recipe/editor.js`, `ui/recipe/history.js`, `ui/recipe/fields.js`, `tests/ui/recipe-editor.spec.mjs`; extend commands/app state.

**Interfaces:** `PresetStore::load/save_revision/import/export`; PresetId with immutable revision/RecipeV1 snapshot, app-local durable save. Implements createRecipeEditor from 4.4 plus edit undo/redo. Blueprint later embeds this same editor without evaluating names in JS.

- [ ] **Step 1 — Red:** Add `recipe_round_trip_and_version_rejection`, `failed_save_preserves_previous`, `import_limits_and_rule_count`, `edit_undo_redo_is_not_filesystem_undo`; browser test reorder/repeat/disable rules and restore the same serialized recipe.
  ```rust
  assert_eq!(reloaded.recipe, original.recipe);
  assert_eq!(too_many_rules.code, RenameErrorCode::ImportLimit);
  assert_eq!(active_revision, old_revision); // failed save cannot replace it
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test presets`; `npx playwright test tests/ui/recipe-editor.spec.mjs`.
- [ ] **Step 3 — Implement:** 1 MiB/100-rule validation, revisioned presets, atomic persistence through create-new temp+flush+safe replacement of the **app-owned** preset record (never a media target), exact recipe import/export, common editor lifecycle and undo/redo state. Exported recipe data grants no access and carries no executable operations.
- [ ] **Step 4 — Verify green:** Preset/browser tests and reproduced preview goldens from saved recipes; edited presets create new revisions rather than mutate attached snapshots.
- [ ] **Step 5 — Commit:** `feat: add versioned presets and shared recipe editing`.

### Task 17: Bundled metadata helper and Windows properties

**Files:** Create `src-tauri/src/metadata.rs`, `src-tauri/tests/metadata_helper.rs`, `crates/vidlib-rename/src/metadata.rs`, `crates/vidlib-rename/tests/metadata.rs`, `tools/vendor_media_info.py`, `tools/media-info-manifest.json`, `tests/fixtures/media/README.md`; update Tauri bundle resources and command adapters.

**Interfaces:** `MetadataProvider::probe(path: &Path, fields: &[PropertyKey], control: &JobControl) -> Result<MetadataRecord, MetadataIssue>`; each MetadataValue contains typed value, source key/provider/version, timezone status, or explicit Missing/Unsupported/Error. A cache key is physical identity + exact path + size/mtime + provider version + requested fields; absence of identity never merges unrelated paths.

- [ ] **Step 1 — Red:** Add `no_probe_for_text_recipe`, `requested_fields_only_and_cache_invalidates`, `helper_timeout_kills_and_reaps`, `stdout_stderr_and_json_are_bounded`, `windows_property_types_are_normalized`, and fixture-video duration/resolution/codec/title/date tests.
  ```rust
  assert_eq!(timeout.code, MetadataIssueCode::Timeout);
  assert!(child_was_reaped);
  assert_eq!(text_recipe_probe_count, 0);
  assert_eq!(record.recorded_date.timezone, TimezoneEvidence::Unknown);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test metadata`; Windows: `cargo test -p vidlib-tauri --test metadata_helper`.
- [ ] **Step 3 — Implement:** Resolve a stable official MediaInfo Windows CLI artifact per supported architecture, verify version/URL/SHA-256 and redistribution notices into the committed manifest, and package it as a local resource. Do not use floating `latest`, startup downloads, or a user PATH executable. Spawn directly without shell; 10-second monotonic deadline, 4 MiB stdout/64 KiB stderr caps, cancellation kill/reap and one probe per volume/drive. Parse only requested normalized fields. Add COM-initialized worker-owned Windows Property System extraction by canonical typed key. Fail clearly if a helper/field is unavailable; never edit embedded metadata.
- [ ] **Step 4 — Verify green:** Real fixture integration, fake hanging/noisy process fixtures, cache mutation tests, installer resource smoke check and hashes/notice validation. Fixture media is tiny, redistributable, and documented; no user's media are copied into the repository.
- [ ] **Step 5 — Commit:** `feat: bundle bounded offline media metadata probing`.

### Task 18: Explicit companion association groups

**Files:** Create `crates/vidlib-rename/src/companions.rs`, `crates/vidlib-rename/tests/companions.rs`; extend recipe/model/graph and UI association DTOs.

**Interfaces:** `associate(inventory: &Inventory, selection: &[EntryId], policy: &CompanionPolicy) -> Result<AssociationSet, RenameError>` and `propagate_names(evaluated, associations)`. One asset has at most one primary; associations enter GroupIds before graph validation and cannot silently extend selection.

- [ ] **Step 1 — Red:** Add `language_and_role_suffixes_preserved`, `idx_sub_pair_is_one_dependency_group`, `two_possible_owners_require_choice`, `folder_artwork_not_assigned_automatically`, `cross_folder_requires_explicit_pair`, and `primary_only_warns_without_mutating_asset`.
  ```rust
  assert_eq!(subtitle_target, "Harbor Watch - S02E02 [1080p].en.forced.srt");
  assert_eq!(idx.group_id, sub.group_id);
  assert!(ambiguous.owner.is_none());
  assert!(!plan.selected_ids.contains(&shared_poster_id));
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-rename --test companions`.
- [ ] **Step 3 — Implement:** Exact original-stem matching with explicit supported extension list from the spec; recognize retained ISO language/role suffixes without guessing a shared poster/tvshow file's owner. Manual cross-folder owner binding stays within selected grants. Derive all companions from the primary's **same** frozen name/counter, then rerun global collisions and group validation.
- [ ] **Step 4 — Verify green:** Association tests and executor fault injection failing after the first companion step; assert compensation/recovery names every affected asset and never drops unrelated files.
- [ ] **Step 5 — Commit:** `feat: preserve explicit video companion associations`.

### Task 19: Folder layouts and verified copy/cross-volume moves

**Files:** Create `crates/vidlib-rename/src/destination.rs`, `crates/vidlib-fileops/src/copy.rs`, `crates/vidlib-fileops/tests/copy_move.rs`; extend graph/rename/recovery/undo and destination recipe controls.

**Interfaces:** `resolve_destinations(evaluated, policy: &DestinationPolicy, grants) -> Result<EvaluatedBatch, RenameError>`; extend execute/reconcile/plan_undo for CreateDirectory/StageCopy/PublishCopy/RemoveSource and owned cleanup. DestinationPolicy is InPlace, CopyTo, or MoveTo with explicit root grant and PreserveHierarchy/typed relative folder template.

- [ ] **Step 1 — Red:** Add `folder_layout_preview_matches_all_descendants`, `cross_volume_source_removed_only_after_verified_publish`, `copy_digest_and_metadata_verified`, `publication_then_remove_failure_preserves_fact`, `disk_full_cleans_only_owned_staging`, `copy_undo_checks_full_digest`, and `created_directory_undo_requires_empty`.
  ```rust
  assert_eq!(source_delete_calls_before_verified_publication, 0);
  assert_eq!(published.sha256, expected_sha256);
  assert!(source.exists()); // injected remove failure
  assert_eq!(receipt.phase, CompletedPhase::CopyPublished);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-fileops --test copy_move`; extend the real-process crash matrix from Task 08.
- [ ] **Step 3 — Implement:** Same-volume graph rename when supported; otherwise explicit bounded 1 MiB staged copying, source streaming SHA-256, independent reread destination digest, freshness recheck, metadata/data flush, no-replace publication and only then checked source removal for Move. Journal every phase and owned directory. Cross-volume folder operations expand into dependency groups with descendant paths; never flatten unintentionally or recreate special entries. Resume/restore and Copy undo use the recorded phase/identity/digest; no invented suffixes or deletion of unknown staging files.
- [ ] **Step 4 — Verify green:** Copy/move/undo/recovery fixtures, a real second volume where available, publication-race/permissions/space faults, and cancellation between safe group barriers. Record unsupported volume semantics rather than substituting an unverified fallback.
- [ ] **Step 5 — Commit:** `feat: add verified destination layouts and copy move workflows`.

### Task 20: Opt-in filesystem attributes and timestamps

**Files:** Create `crates/vidlib-fileops/src/metadata.rs`, `crates/vidlib-fileops/tests/metadata_actions.rs`; extend rename recipe/evaluate/journal/undo and desktop metadata DTOs.

**Interfaces:** `MetadataChangeSet` carries per-field Keep/Set/Clear or timestamp Set/Offset/Increment; original and requested states are part of the plan and receipt. Extend Filesystem::set_metadata and inverse planning; metadata-only rows are Changed even if their path stays identical.

- [ ] **Step 1 — Red:** Add `all_defaults_leave_metadata_unchanged`, `each_windows_attribute_and_timestamp_can_be_explicit`, `offset_increment_uses_frozen_item_order`, `partial_set_failure_is_reported_and_recoverable`, and `undo_refuses_changed_poststate`.
  ```rust
  assert_eq!(default_receipt.after.modified_ns, before.modified_ns);
  assert_eq!(metadata_only_row.status, RowStatus::Changed);
  assert_eq!(restored.attribute_bits, original.attribute_bits);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-fileops --test metadata_actions`; run Windows-created/accessed/modified and hidden/read-only/archive/system fixtures there.
- [ ] **Step 3 — Implement:** Native supported setters, typed unsupported results elsewhere, frozen date/time policies from Task 13 and checked per-item increments. Journal expected/current/original states per field, not only filenames. Do not silently clear read-only or write embedded recording metadata to make a rename succeed. Restore only the fields actually changed after verifying expected poststate.
- [ ] **Step 4 — Verify green:** Metadata/undo/recovery tests including failure after a subset of fields; no-op controls stay invisible in the operation summary. Document OS-managed access-time limitations truthfully.
- [ ] **Step 5 — Commit:** `feat: add explicit recoverable file metadata changes`.

### Task 21: Complete the Rename controls and user journeys

**Files:** Create `ui/rename/rules.js`, `ui/rename/selection.js`, `ui/rename/associations.js`, `ui/rename/destinations.js`, `ui/rename/metadata.js`, `tests/ui/rename-rules.spec.mjs`, `tests/ui/rename-workflows.spec.mjs`; extend the common recipe editor, preview/history and desktop request validation.

**Interfaces:** R01–R17 use only the implemented RecipeV1/typed command DTOs. All controls share createRecipeEditor and start_rename_preview; a single state reducer tracks inventory/recipe/plan revisions and current job. UI grouping does not create alternate filename behavior.

- [ ] **Step 1 — Red:** Browser golden scenarios cover every R01–R17 control family, per-rule explanations, manual numbering order, search/200-row paging/highlighted differences, imports/paste/export/presets/edit undo, companion choice, folder/destination/metadata summaries, recovery and filesystem undo. Add `older_job_finishes_after_newer_recipe` and malicious filename/metadata text fixtures.
  ```js
  expect(bridge.lastAppliedPlanRef).toEqual(latestReviewedPlanRef);
  await expect(page.locator('#rename-results script')).toHaveCount(0);
  await expect(page.getByTestId('rename-row')).toHaveCount(200);
  ```
- [ ] **Step 2 — Verify red:** `npx playwright test tests/ui/rename-rules.spec.mjs tests/ui/rename-workflows.spec.mjs`.
- [ ] **Step 3 — Implement:** Complete the actual controls and accessible labels for the full matrix; lazy advanced panels for metadata/regex, explicit missing-field actions, fixed revision acceptance, abortable watchers and focus restoration. Display-only sorts cannot edit numbering. Review summaries include non-displayed selection rows and carried descendants. Use JSDoc data interfaces and text nodes; no placeholders or disabled unfinished options count as implemented scope.
- [ ] **Step 4 — Verify green:** Browser journeys at normal/narrow viewport, keyboard-only use, screen-reader live status, zero app console errors, and current finder/libraries regressions. Syntax-check all modules and verify the packaged browser sample cannot call file actions.
- [ ] **Step 5 — Commit:** `feat: complete bulk rename editing and review workflows`.

### Task 22: Native multi-selection, drop, and Explorer entry

**Files:** Create `src-tauri/src/entrypoint.rs`, `src-tauri/installer/hooks.nsh`, `src-tauri/tests/entrypoint.rs`, `tests/ui/native-selection.spec.mjs`; extend selection/commands, tauri.conf.json/capabilities and UI source selection.

**Interfaces:** select_sources accepts Files/Folders/LibraryRoots; entrypoint parses a path argument list into inactive backend grants and opens review. Optional installer-controlled Explorer integration uses arguments directly, not a shell script; uninstall removes only app-owned registrations.

- [ ] **Step 1 — Red:** Add `native_picker_preserves_paths`, `explorer_arguments_select_without_applying`, `multiple_instances_forward_selection_safely`, `malicious_quoted_path_is_data`, `cancelled_picker_changes_nothing`, and keyboard/drop fixtures.
  ```rust
  assert_eq!(selected_native_paths, expected_os_paths);
  assert_eq!(mutation_calls, 0);
  assert!(!selection.applied);
  ```
- [ ] **Step 2 — Verify red:** Windows desktop entrypoint tests and `npx playwright test tests/ui/native-selection.spec.mjs`.
- [ ] **Step 3 — Implement:** Native rfd multi-selection with no to_string_lossy round-trip, Tauri native drop paths resolved on Rust side, explicit argument framing/validation and optional opt-in Explorer registration. A second instance forwards only selection to the active app or presents it after acquiring the coordinator; never accepts an apply command from process arguments. Keep capabilities narrowly scoped and local CSP intact.
- [ ] **Step 4 — Verify green:** Windows Explorer right-click/multi-file/Unicode/UNC/cancel smoke tests and uninstall registry cleanup; record actual evidence, not just a parsed argument unit test.
- [ ] **Step 5 — Commit:** `feat: add native collection selection and Explorer entry`.

### Task 23: Complete Rename release verification

**Files:** Create `.github/workflows/check.yml`, `tools/benchmark-library.ps1`, `docs/verification/rename.md`; extend Task 08 crash fixtures and UI regression suites. Modify README.MD/development instructions only for actually completed behavior.

**Interfaces:** No new feature interface. This task proves R01–R18 and the safety contracts, validates packaged helpers, and records the independent Rename completion gate.

- [ ] **Step 1 — Define expected evidence:** Add a verification matrix covering all R IDs, real Windows NTFS/exFAT/share outcomes, every journal boundary and inverse step, concurrent instances, stale-page conflicts, invalid/native paths, helper failure, disk/journal failure, companion compensation and protected markers. Each row names an automated test or an actual native smoke procedure and its expected outcome; missing evidence stays pending.
- [ ] **Step 2 — Run targeted fault suites:** All new Rust test binaries, real-process crash injection and existing duplicate tests; negative fixtures must demonstrate refusal/preservation, not merely a generic error.
- [ ] **Step 3 — Implement verification infrastructure:** Linux portable Rust tests excluding the Tauri desktop dependency where unavailable; Windows workspace tests/Clippy/format/build plus browser development tests. Pin toolchain/dependency lock inputs and do not skip failures with unconditional success. Install only development browser/native build dependencies. Add a reproducible 100,000-entry metadata fixture generator/benchmark, not a huge committed library.
- [ ] **Step 4 — Verify release gate:** `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `npm ci`, `npx playwright test`, JS module checks and `cargo build --locked --release -p vidlib-tauri` on supported environments. Record reference Windows hardware, <=1-second cached first-page target, <=200 rows per page, <=10 progress updates/s, native dialogs/Explorer and no network dependency. If a required capability/test is pending, do not mark Rename complete or ship a partial matrix as complete.
- [ ] **Step 5 — Commit:** `test: verify complete Rename behavior and recovery` with measured evidence. Rename may release at this gate independently of Tasks 24–31.

### Task 24: Blueprint registry and local persistence

**Files:** Create `crates/vidlib-blueprint/Cargo.toml`, `crates/vidlib-blueprint/src/lib.rs`, `crates/vidlib-blueprint/src/model.rs`, `crates/vidlib-blueprint/src/store/mod.rs`, `crates/vidlib-blueprint/src/store/schema.rs`, `crates/vidlib-blueprint/src/store/migrations.rs`, `crates/vidlib-blueprint/src/query.rs`, `crates/vidlib-blueprint/tests/store.rs`; update workspace manifests/Cargo.lock.

**Interfaces:** Implements BlueprintStore and DTO models from 4.4; consumes inventory/path/recipe data and event types only. Local blueprint.sqlite3 failures remain independent of operations.sqlite3 and Rename.

- [ ] **Step 1 — Red:** Add `registry_and_map_survive_restart`, `migration_failure_keeps_original`, `exact_path_and_number_storage`, `cached_queries_page_200`, and `blueprint_db_error_does_not_open_or_mutate_operation_store`.
  ```rust
  assert_eq!(reopened.collection(id)?.display_name, "Archive");
  assert_eq!(page.items.len(), 200);
  assert_eq!(operations_db_writes, 0);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-blueprint --test store`.
- [ ] **Step 3 — Implement:** Local WAL/versioned migrations with failure preservation and thread-owned connections, following Task 04's settings. Own tables: collections, roots (binding_revision/evidence/availability), refresh_attempts, coverage, observations, current_entries, changes, template_attachments, consumed_operation_events, projection_cursors, pattern_cache. Current_entries references **per-entry/scope observation attempts**, not one root-wide snapshot pointer. Index `(root_id,parent_path_key,path_key)`, `(root_id,physical_identity)`, history time/type/origin/operation and exact path. IDs/path codecs/numeric sort keys use section 4 contracts; SQL rows stay private.
- [ ] **Step 4 — Verify green:** Store tests against real local DB files, query-plan/index checks for paging and current old/mixed-age rows, corrupt/migration fixture retention. Schema errors return actionable BlueprintError without changing media or Rename state.
- [ ] **Step 5 — Commit:** `feat: persist Blueprint collections and observations locally`.

### Task 25: Recognition, markers, rebinding, and root guards

**Files:** Create `crates/vidlib-blueprint/src/identity.rs`, `crates/vidlib-blueprint/src/markers.rs`, `crates/vidlib-blueprint/tests/identity_markers.rs`; extend roots/registry queries, fileops journal policy tables and Rename protected-root namespace capture.

**Interfaces:** Implements recognize/register/rebind from 4.4. Marker actions take an explicit selected-root grant and confirmed intent. Add `Journal::upsert_root_guard(root_id, revision, native_root)` / `remove_root_guard(root_id, revision)` for an independent protected-root policy mirror; protected guards do not authorize file access.

- [ ] **Step 1 — Red:** Add `known_identity_reconnects_drive_letter`, `same_path_replacement_requires_review`, `lookalike_structure_is_only_candidate`, `copied_marker_conflicts`, `marker_only_install_has_no_history`, `no_marker_on_register_or_scan`, `readonly_marker_failure_keeps_registration`, `blueprint_db_offline_keeps_root_guards`, and `registration_revokes_plan_built_before_new_guard`.
  ```rust
  assert_eq!(decision, RecognitionDecision::CandidateNeedsConfirmation);
  assert_eq!(marker_only, RecognitionDecision::MarkerOnlyNoLocalData);
  assert!(!root.join(".vidlib-blueprint.json").exists());
  assert!(independent_rename_namespace.protected_roots.contains(&root_id));
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-blueprint --test identity_markers`; run native ID/readonly/reparse cases on Windows.
- [ ] **Step 3 — Implement:** Evidence hierarchy and explicit binding revisions; marker strict schema/UUIDs/4 KiB limit, no links or implicit creation; distinct conflict/no-record/unknown decisions, no drive-wide search. Mirror protected root changes in operations.sqlite3 **before** committing a new Blueprint binding; retire old guards only after the binding commits. Crash/Blueprint DB failure may retain extra conservative guards, never remove required protection or globally block unrelated Rename. Reconcile pending policy updates separately from filesystem recovery. Marker filenames remain protected independently of Blueprint DB availability.
- [ ] **Step 4 — Verify green:** All identity decisions including unstable/missing share identity, file-ID reuse evidence, portable clone conflict and explicit rebind. Rebind updates evidence/freshness without inventing a media-move event; unknown markers never create active root grants.
- [ ] **Step 5 — Commit:** `feat: recognize and explicitly reconnect Blueprint collections`.

### Task 26: Coverage-aware refresh and truthful changes

**Files:** Create `crates/vidlib-blueprint/src/refresh.rs`, `crates/vidlib-blueprint/src/diff.rs`, `crates/vidlib-blueprint/tests/refresh.rs`, `crates/vidlib-blueprint/tests/diff.rs`; extend store/query.

**Interfaces:** Implements begin_refresh/stage_inventory/promote_refresh from 4.4 and ChangeDto. RefreshOutcome is Complete/Partial/Cancelled/Offline/Failed; origins are App or ExternalOrUnknown. A comparison uses matching root/binding/filter scope and observation intervals, not wall-clock claims about external actions.

- [ ] **Step 1 — Red:** Add `first_and_newly_covered_scope_are_baselines`, `partial_refresh_keeps_unobserved_entries`, `stat_failure_after_listing_is_not_missing`, `offline_root_is_not_mass_deletion`, `hardlinks_do_not_force_relocation`, `directory_move_is_expandable_only_with_evidence`, and `multi_root_times_are_independent`.
  ```rust
  assert_eq!(first.added_count, 0);
  assert_eq!(first.baseline_count, 1);
  assert_eq!(offline.missing_count, 0);
  assert!(current.contains_path(&listed_but_unstatted_child));
  assert_ne!(summary.last_attempt, summary.last_complete_observation);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-blueprint --test refresh --test diff`.
- [ ] **Step 3 — Implement:** Stage attempts separately; transactionally promote successful scopes and history while retaining old observations for unreadable/excluded/cancelled scopes. Determine Missing from complete parent enumeration **and listed child names**, not missing metadata rows. First coverage/filter-policy changes establish baselines. LocationChanged requires unambiguous consistent physical evidence; weak relocation links preserve Added/Missing facts. Group directory changes while retaining queryable descendants. Store observed intervals and separate last-attempt/per-scope last-complete times.
- [ ] **Step 4 — Verify green:** Real fixture refreshes before/after external add/remove/rename/metadata changes; injected failed stat/listing/cancel, duplicate physical IDs, root replacement, exclusion changes, and DB promotion rollback. Pattern denominators cannot include retained-but-unobserved stale entries as freshly examined.
- [ ] **Step 5 — Commit:** `feat: compare Blueprint observations with explicit coverage`.

### Task 27: Idempotent operation projection and duplicate-action bridge

**Files:** Create `crates/vidlib-blueprint/src/operations.rs`, `crates/vidlib-blueprint/tests/operation_projection.rs`, `crates/vidlib-fileops/src/duplicate_events.rs`, `crates/vidlib-fileops/tests/duplicate_events.rs`; extend old fileops helpers narrowly, journal/outbox and Tauri coordinator delivery.

**Interfaces:** Implements project_events/ProjectionReceipt; an EventId unique insert and projection cursor commit in blueprint.sqlite3 precede outbox acknowledgement. Add `apply_duplicate_actions_with_journal(result, paths, action, destination, lease, journal) -> ActionReport`, preserving the existing outward report/keeper/hash behavior. Internal receipts carry actual destinations and completed phases.

- [ ] **Step 1 — Red:** Add `event_replay_and_crash_before_ack_project_once`, `refresh_matches_app_event_without_duplicate`, `intervening_external_change_is_preserved`, `projection_failure_never_repeats_mutation`, `duplicate_move_records_actual_destination`, `duplicate_delete_keeps_existing_guards`, and `pruning_never_removes_unacknowledged_event_identity`.
  ```rust
  assert_eq!(store.changes_for_operation(op)?.len(), expected_fact_count);
  assert_eq!(filesystem_mutation_calls, 1);
  assert_eq!(receipt.actual_destination, real_destination);
  assert_eq!(retained_keeper_bytes, b"fixture");
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-blueprint --test operation_projection`; `cargo test -p vidlib-fileops --test duplicate_events --test actions`.
- [ ] **Step 3 — Implement:** Commit receipt/event projections and consumption cursor together, then ack the source outbox; replay by EventId after restart. Record a local root-activation event cursor/registration epoch so delayed events from a removed or earlier binding cannot resurrect its history. Exact known updates invalidate freshness and correlate with later observations without suppressing genuinely different external changes. Add pre-intent/post-receipt hooks around legacy duplicate mutations to capture actual destination/final phase durably; preserve current duplicate safety and avoid changing its collision policy as an unrelated refactor. Permanent duplicate deletion remains non-undoable. Unknown/unregistered roots do not auto-register; acknowledge irrelevant events explicitly without inventing collection history.
- [ ] **Step 4 — Verify green:** Replay before/after each DB commit/ack, Blueprint disk-full/corruption, multi-root event fan-out and existing 36 portable tests plus Windows-specific duplicate tests. A successful source operation remains successful even while Blueprint is unavailable.
- [ ] **Step 5 — Commit:** `feat: project durable file-operation facts into Blueprint`.

### Task 28: Template attachments and descriptive Patterns

**Files:** Create `crates/vidlib-blueprint/src/templates.rs`, `crates/vidlib-blueprint/src/patterns.rs`, `crates/vidlib-blueprint/tests/templates_patterns.rs`; extend schema/query and shared editor DTOs.

**Interfaces:** Implements attach_template/summarize_patterns from 4.4. Attachment records AttachmentId, collection/optional relative subtree, preset ID/revision, immutable RecipeV1 snapshot and timestamps. PatternSummary records recognizer version, baseline/coverage revision, denominator, examples and matched rule ID.

- [ ] **Step 1 — Red:** Add `new_preset_revision_does_not_mutate_attachment`, `multiple_matching_attachments_require_choice`, `unknown_embedded_recipe_stays_inactive`, `patterns_are_coverage_qualified`, and `plain_year_or_digits_are_not_episode_identity`.
  ```rust
  assert_eq!(attachment.recipe_revision, original_revision);
  assert_eq!(recognized_episode_examples, ["Show.S02E03.mkv", "Show.2x03.mkv"]);
  assert!(!recognized_episode_examples.contains(&"Movie.2020.mkv"));
  assert_eq!(pattern.observed_count, freshly_observed_count);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-blueprint --test templates_patterns`.
- [ ] **Step 3 — Implement:** Explicit collection/subtree attachments, revision updates by user choice and the same Rename schema/editor. Derive extension/case/delimiter distributions and explicit SxxExx/NxNN recognizers with examples; do not infer canonical names, media identity, episode completeness or a statistical confidence score. Cache only against the exact observation/filter revision; clicking a pattern later filters its explicit examples.
- [ ] **Step 4 — Verify green:** Attachment/denominator tests across partial refresh and removed presets, suffix/Unicode filenames, non-media layouts and unsupported recipe versions. Opening a pattern/template cannot perform any filesystem mutation.
- [ ] **Step 5 — Commit:** `feat: add Blueprint template attachments and pattern summaries`.

### Task 29: Blueprint map, history, and desktop workflows

**Files:** Create `src-tauri/src/commands/blueprint.rs`, `ui/blueprint/main.js`, `ui/blueprint/map.js`, `ui/blueprint/changes.js`, `ui/blueprint/templates.js`, `ui/blueprint/patterns.js`, `tests/ui/blueprint.spec.mjs`; extend desktop app state, navigation/index/styles and shared jobs/editor.

**Interfaces:** Exposes Blueprint store/query commands via 4.4 owned DTOs; map_children/search/history are cursor-paged at 200 rows and stable baseline revision. Register/rebind/marker changes require explicit selected-root grants. `open_in_rename` creates fresh source selection and inventory; never reuses cached EntryIds.

- [ ] **Step 1 — Red:** Add browser scenarios `known_collection_load_then_refresh`, `offline_cached_map_is_labelled`, `identity_conflict_needs_choice`, `partial_refresh_shows_coverage`, `template_uses_shared_editor`, `pattern_filters_examples_only`, `cached_selection_requests_fresh_rename`, and keyboard/tree/dialog focus tests.
  ```js
  await expect(page.getByTestId('blueprint-label')).toContainText('Experimental');
  await expect(page.getByTestId('map-freshness')).toContainText('Recorded');
  expect(bridge.createdMarkerCount).toBe(0);
  expect(bridge.renameRequests[0].reuseCachedEntryIds).toBe(false);
  ```
- [ ] **Step 2 — Verify red:** `npx playwright test tests/ui/blueprint.spec.mjs`; `cargo test -p vidlib-tauri commands::blueprint` on supported desktop build environments.
- [ ] **Step 3 — Implement:** Functional Map/Changes/Templates/Patterns sections, registration/inspect-without-saving, cached-first view, automatic reachable refresh on open plus manual Refresh, truthful availability and dual observation times, explicit identity decisions, attachment editing and example selection. Central coordinator delays physical refresh during mutations while allowing cached browsing. Prevent stale query/job responses after navigation; render all path/metadata data as text.
- [ ] **Step 4 — Verify green:** Browser and desktop adapter tests, page limits, empty/arbitrary layout examples, offline reconnect, template selection conflicts, screen-reader state/keyboard navigation and existing app regression journeys. No unimplemented hero controls are presented as working.
- [ ] **Step 5 — Commit:** `feat: add the Blueprint collection workspace`.

### Task 30: Portable records, retention, and local repair

**Files:** Create `crates/vidlib-blueprint/src/portability.rs`, `crates/vidlib-blueprint/src/retention.rs`, `crates/vidlib-blueprint/tests/portability_retention.rs`; extend commands and Blueprint settings/history UI with `tests/ui/blueprint-records.spec.mjs`.

**Interfaces:** Implements 4.4 export/import/prune/remove functions and structured import-review DTOs. Export v1 contains IDs, relative native paths/observations/coverage, change records and template snapshots; no OS grants, writer locks, native absolute bindings or executable operation journals. SQLite backup/export uses a consistent read snapshot/backup API.

- [ ] **Step 1 — Red:** Add `export_while_refreshing_is_consistent`, `import_rejects_escape_unknown_envelope_and_limits`, `import_never_activates_root`, `copy_import_remaps_ids_and_references`, `prune_keeps_each_root_baseline_and_dedup_tombstones`, `remove_collection_does_not_remove_marker_or_media`, and `repair_preserves_original_db`.
  ```rust
  assert!(imported.active_root_grants.is_empty());
  assert_eq!(oversize.code, BlueprintErrorCode::ImportLimit);
  assert!(store.last_complete_scope(root_a)?.is_some());
  assert!(store.last_complete_scope(root_b)?.is_some());
  assert!(media_file.exists() && marker.exists());
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-blueprint --test portability_retention`; `npx playwright test tests/ui/blueprint-records.spec.mjs`.
- [ ] **Step 3 — Implement:** Bounded streaming imports (256 MiB/1,000,000 entries), strict envelope/path/reference validation, inactive candidates, explicit replace/import-as-copy review and complete ID/reference remapping. Unknown embedded recipes remain opaque inactive within a recognized envelope. Age-based pruning preserves mixed-age root/scope baselines and non-content dedup EventId tombstones needed to prevent delayed replay from resurrecting pruned history. Do not prune pending source outbox/recovery records. User-triggered local record removal and separate marker removal are distinct actions. Offer consistent backup/export and preserve corrupt/original DB before explicit repair/reset.
- [ ] **Step 4 — Verify green:** Export/import during refresh, concurrent event delivery/pruning, interrupted export, known-ID collision, invalid path codecs/limits, removal/re-registration and all baseline scopes. A deleted collection stays deleted when late events arrive; markers and media are unchanged unless their separate explicit action is confirmed.
- [ ] **Step 5 — Commit:** `feat: manage portable Blueprint records and retention`.

### Task 31: Blueprint foundation acceptance and performance

**Files:** Create `docs/verification/blueprint-foundation.md`, `crates/vidlib-blueprint/tests/foundation.rs`, `tests/ui/blueprint-foundation.spec.mjs`; extend the fixture benchmark tool and CI's portable/Windows matrix.

**Interfaces:** No new feature interface. Verifies all ten acceptance scenarios in Blueprint spec section 11 independently of Rename's release gate.

- [ ] **Step 1 — Red:** Add a multi-root disposable library walkthrough: register, baseline, restart, rebind one drive, external edits, failed subtree, app rename + replay, template preview, export/import to a new local store, prune/remove. Assert retained coverage, qualified events and no identity merge throughout.
  ```rust
  assert_eq!(offline_refresh.missing_count, 0);
  assert_eq!(replayed_event_occurrences, 1);
  assert!(!imported_collection.has_active_binding());
  assert_eq!(unexpected_media_mutation_count, 0);
  ```
- [ ] **Step 2 — Verify red:** `cargo test -p vidlib-blueprint --test foundation`; `npx playwright test tests/ui/blueprint-foundation.spec.mjs`. This should expose any missing integration before release claims.
- [ ] **Step 3 — Implement:** Fix only integration gaps against approved scope; add reproducible 100,000-entry cached map/first-page/history benchmark with mixed-age roots and pattern denominators. Record explicit observed schema/algorithm versions and platform details. Do not upgrade this task into convention inference or a semantic media organizer.
- [ ] **Step 4 — Verify green:** Full Blueprint suite, UI accessibility/console checks, real Windows removable/share identity/coverage evidence, cached first page <=1 second target on recorded hardware, 200-row paging and 10/s progress. Mark achieved/failed/pending benchmarks truthfully; a proof-of-concept label does not replace the acceptance loop.
- [ ] **Step 5 — Commit:** `test: verify the Blueprint collection-map foundation`.

### Task 32: Whole-project regression and execution handoff

**Files:** Update README.MD, docs/verification/rename.md, docs/verification/blueprint-foundation.md and this canonical plan's execution ledger; update older design status references only to reflect actual completion. No competing implementation-plan file.

**Interfaces:** Preserve existing duplicate commands/preferences and both features' independent completion statuses.

- [ ] **Step 1 — Check coverage:** Walk the traceability table below and both approved specs. Link each item to a committed automated test or recorded Windows/native evidence. Open checkboxes or unavailable platform evidence remain visible; do not infer completion from code presence.
- [ ] **Step 2 — Run final regression:** After final code fixes, run the platform-appropriate Rust workspace/core checks, JS checks, browser suites and release build from Task 23. Do not repeat broad suites after a passing unchanged state merely to accumulate output.
- [ ] **Step 3 — Document actual behavior:** Describe offline runtime requirements, bundled helper/resources, new app-data/marker behavior, recovery/undo limitations, native entry points and the experimental Blueprint scope. Keep the earlier duplicate-only verification record as history. Document remaining unsupported filesystem cases and unresolved platform checks precisely.
- [ ] **Step 4 — Review readiness:** Use Superpowers verification-before-completion and requesting-code-review; review the whole implementation branch for plan/spec coverage and regressions after task-level reviews. Integrate fixes with their owning regression tests. No production library benchmark or destructive user-file test is authorized by this plan.
- [ ] **Step 5 — Commit/handoff:** `docs: record Rename and Blueprint implementation evidence`. Report completed gates, exact checks, remaining limitations, worktree/branch, and next incomplete task. Follow finishing-a-development-branch for integration; pushing/publishing/releasing is a separate action, not implicit in this planning request.

## 7. Requirement-to-task traceability

| Rename requirement | Implementation owner | Verification |
| --- | --- | --- |
| R01 text editing | 11, 21 | text goldens + complete UI rule scenarios |
| R02 substring/reverse/translations | 11, 21 | grapheme/translation/move/copy tests |
| R03 case/exceptions/Roman | 11, 21 | explicit case goldens and Unicode cases |
| R04 regex | 12, 21 | native PCRE2 captures/backrefs/lookarounds/limits |
| R05 segment/ordered repeated rules | 03, 11, 12, 16, 21 | intermediate explanations + editor reorder/undo |
| R06 numbering | 03, 13, 21 | frozen vector, repeat/reset/formats/display sort |
| R07 dates | 13, 17, 20, 21 | format/timezone/DST/source distinctions |
| R08 templates/properties | 14, 17, 21 | every field family, optional/fallback/missing |
| R09 hashes/generated values | 14, 21 | five independent vectors, full read/lazy/cancel/frozen |
| R10 stem/extensions | 11, 14, 21 | explicit stem/whole-name/extension cases |
| R11 recursive selection/filters | 01, 15, 21 | all dimensions, coverage, rebuilt approval |
| R12 mappings/paste/export/override | 15, 21 | bounds, ambiguous paths, spreadsheet/lossless separation |
| R13 presets/edit undo | 16, 21 | revisions, round-trip, failed save, editor history |
| R14 companions | 06–09, 18, 21 | suffixes/owners/pairs/group failure/no hidden selection |
| R15 folders/copy/move/layouts | 02, 06–09, 19, 21 | final graph, verified publication, inverse ownership |
| R16 attributes/timestamps | 02, 04, 09, 20, 21 | explicit supported fields/defaults/restoration |
| R17 preview/history/recovery/undo | 03–10, 15, 21, 23 | all-row validation, crash matrix, stale responses, UX |
| R18 native/Explorer/keyboard | 10, 21–23 | native path selection, opt-in shell entry, focus/drop |

| Blueprint acceptance (spec section 11) | Tasks/evidence |
| --- | --- |
| 1 registration/reopen/map/history | 24, 29, 31 |
| 2 identity, drive-letter/rebind/lookalike | 25, 31 |
| 3 offline cache/reconnection/no false deletion | 26, 29, 31 |
| 4 qualified external changes/first baseline | 26, 31 |
| 5 incomplete coverage/cancel preservation | 01, 26, 31 |
| 6 unstable identity/truthful relocation | 25, 26, 31 |
| 7 marker clone/marker-only installation | 25, 30, 31 |
| 8 replay/duplicate attribution/storage isolation | 04, 05, 27, 31 |
| 9 real templates/fresh Rename/pattern examples | 16, 28, 29, 31 |
| 10 read-only marker/migrations/portability/pruning/paths/performance/accessibility | 24–26, 29–31 |

Non-matrix safety requirements are owned explicitly: native-path and exact transport (01/03/10); no-clobber/grants (02/06/07); graph/folder identity (06/19); durable local journal and recovery (04/07/08/19/20); inverse-state verification (09/19/20); cross-process exclusion/invalidation (05/10/27); metadata helper isolation (17); independent protected roots (25); observational truth/coverage (26); durable exactly-once projection (27); trusted local binding versus portable data (25/30); complete Windows evidence and benchmark limits (23/31/32).

## 8. Execution ledger

Planning status: user-approved specs; this canonical plan is ready for review and fresh-session execution. No implementation checkbox has been completed by writing this document. The storage assumption accepted with the specs is app-local Blueprint data plus opt-in portable identity markers; no new preference question is required before following it.

| Date | Task / commit | Evidence / next action |
| --- | --- | --- |
| 2026-10-17 | Plan authored; Context7 and repository checked | Start Task 01 in an isolated worktree after reviewing the plan and selecting execution method. |

During execution append task commits, commands/results, pending Windows capability rows and the next eligible incomplete task here. Keep this file canonical. Scope changes require an explicit spec/plan amendment, not an implementation agent silently dropping a required capability or expanding Blueprint into a mature hero feature.

| 2026-10-17 | Execution setup; base `711b422` | Cloned `0langa/VidLibOrganizer`; isolated worktree `.worktrees/rename-blueprint`, branch `feat/rename-blueprint`; all three specs read; 36 portable baseline tests and UI syntax passed. Rust 1.99.1 installed. Task 01 is first incomplete. Windows checks pending; continue portable work. |
