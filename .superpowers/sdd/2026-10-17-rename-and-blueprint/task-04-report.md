# Task04 implementation report

Status: DONE. Review base: `eb026edfaf31384fa378da4ae7efddb5c8542557`.

Initial implementation commit: `16c5def19586638df38153606e39b4d39d12abaf` — `feat: persist operation intents receipts and outbox events`.

Review round 1 fix: `16fdb4b2fbb35658caa7891a8959346847a93c48` — `perf: bound journal work per execution step`; fix base `3f589ec`. The current schema and hot path below include this correction; original red/green evidence is retained, and covering fix evidence is appended.

## Delivered scope

Implemented a real-file SQLite operation journal and durable outbox, with no media filesystem effects. The coordinated prerequisite correction expands Rename's backend-only execution model to the complete required eight-action family. The legacy duplicate public interface and engine implementation remain unchanged.

New files:

- `crates/vidlib-fileops/src/events.rs`: data-only versioned operation facts.
- `crates/vidlib-fileops/src/journal/mod.rs`: transaction boundaries, state transitions, receipt validation, reads and outbox delivery/acknowledgment.
- `crates/vidlib-fileops/src/journal/schema.rs`: v1 schema, private persistence codecs, inspection snapshots and compiled-reference validation.
- `crates/vidlib-fileops/src/journal/migrations.rs`: checked initial migration and schema identity verification.
- `crates/vidlib-fileops/tests/journal.rs`: 18 integration tests using real temporary SQLite files.

Modified `vidlib-fileops/src/lib.rs`, its manifest and Cargo.lock; added rusqlite 0.38 with `bundled` and `backup`, serde_json, and the fileops -> rename dependency. The only Rename production change is the coordinated model correction in `rename/src/model.rs`.

## Public interfaces and handoff

`vidlib_fileops::journal::Journal` implements:

```rust
open(path: &Path) -> Result<Journal, OpError>
begin(&mut self, plan: &ExecutionPlan) -> Result<OperationId, OpError>
intent(&mut self, op: OperationId, step: &ExecutionStep) -> Result<(), OpError>
complete(&mut self, op: OperationId, receipt: &StepReceipt) -> Result<(), OpError>
pending(&self) -> Result<Vec<PendingOperation>, OpError>
events_after(&self, cursor: u64, limit: usize) -> Result<EventPage, OpError>
ack_events(&mut self, ids: &[EventId]) -> Result<(), OpError>
operation(&self, op: OperationId) -> Result<OperationRecord, OpError>
set_state(&mut self, op: OperationId, state: OperationState) -> Result<(), OpError>
mark_unresolved(&mut self, op: OperationId, step: StepId) -> Result<(), OpError>
```

`Journal` is intentionally !Send/!Sync: create/use it on the owning blocking thread. Read methods open separate read-only connections; multi-query reads use SQLite snapshots. No callback or transaction is exposed that could span filesystem I/O.

`StepReceipt` records the StepId, CompletedPhase, actual before/after native path records and FileStamps, optional ContentDigest, exact i128 observed time, and the existing fileops `Durability`. FileStamp includes native identity and observed metadata. Native backend receipt/operation/step types have no public serde import implementation.

`PendingOperation`/`OperationRecord` contain operation ID/state, ordered `PendingStep`s (compiled `ExecutionStep`, sequence, state, receipts), ordered group/dependency records, and an inspection-only JSON snapshot. Compiled steps are recovery authority. Inspection snapshots preserve approved PlanRef, inventory/recipe/frozen values, grants and exact native supplements for inventory roots, entries and coverage paths; they are not deserialized into executable authority.

The pure Rename model now has:

- `ExecutionStep { id, entry_id, group_id, action }`.
- `ExecutionAction::{RenameEntry, CreateDirectory, StageCopy, PublishCopy, RemoveSource, SetMetadata, RemoveOwnedCopy, RemoveOwnedEmptyDirectory}` with action-specific native paths/prestates. Creation has no fabricated source state.
- `EntryPrestate { path, identity }`, where `ExpectedIdentity` is `Observed(FileStamp)` or `FromReceipt { step_id }`.
- `OwnedEntryPrestate { entry, created_by }` with creation ownership separate from identity.
- `RenameEntry.temporary` is a logical reservation owned by its step/group. All creating/moving action targets must be absent; no variant authorizes replacement.
- StageCopy encompasses create-new staging plus verified content copying. Its receipt must have a digest and native poststate identity. PublishCopy references that verification receipt; its digest must match. RemoveSource references the matching publication, not an unrelated successful copy.

No Serialize/Deserialize was added to execution authority. The journal uses private serde remote codecs around these exact shared types, including inventory's lossless native path codec.

`vidlib_fileops::events` exposes `CompletedPhase`, `EventKind`, `EventOrigin::App`, `ContentDigest`, `OperationEventV1`, and `EventPage`. Events have stable UUID IDs, operation/group/step IDs, phase/kind, native before/after paths and FileStamps, digest, exact observed time, durable journal sequence and approved PlanRef. Sequences/timestamps/byte counts crossing JSON use exact decimal strings. Event delivery returns unacknowledged records strictly after the supplied cursor, capped at 200; acknowledgments are atomic/idempotent and retain audit events/receipts. Unknown IDs reject the whole acknowledgment batch. There is no event/report import method.

## Persistence and failure invariants

Schema tables: `schema_migrations`, `operations`, `steps`, `receipts`, `operation_events`, `outbox_ack`, `history_visibility`. The current schema is version 2. The v1 SQL/checksum remains immutable. A checked BEGIN IMMEDIATE migration adds the separately persisted approved PlanRef, a compact covering operation-header index and a partial index over unfinished steps; it extracts and validates legacy PlanRefs once, then commits the v2 migration row and user_version together. Initialization from an empty database passes through both versions in the same transaction. Version/checksum/table checks reject unknown or inconsistent schemas before persistent PRAGMA changes. Original unknown/malformed fixture bytes remain preserved; a failed known-v1 migration rolls back the schema, version and evidence rows. The requested backup feature remains enabled.

Intent/completion now read only the compact indexed operation header, next unfinished/current compiled step, and deduplicated prerequisite receipts referenced by that action. The covering header index avoids traversing overflow pages belonging to the inspection snapshot. Intent updates the operation row only for Prepared -> Running, avoiding repeated reconstruction of that large row. State changes/unresolved marking also use targeted reads. Only explicit `pending()`/`operation()` history/recovery reads reconstruct whole operations.

Writer and reader connections set and verify WAL, synchronous=FULL, foreign_keys=ON and busy_timeout=5000 ms. Migration and journal writes use BEGIN IMMEDIATE and explicit commit. A receipt, its completed step state, and its outbox event commit together. A failed statement or COMMIT rolls back the complete transaction. Busy/commit failures poison that writer handle so the next intent is rejected; the previous durable Prepared/Running/IntentDurable evidence remains visible to the startup recovery gate. The commit fixture hook is private, cfg(test), thread-local and one-shot; it is absent from production/IPC.

Operation states are Prepared -> Running -> Completed/Cancelled/Failed/RecoveryRequired. Intent moves Prepared to Running. Terminal states cannot hide unresolved intents, and Completed requires every step to have a durable completion/compensation. Steps persist Planned, IntentDurable, CompletionDurable, Compensated or Unresolved. Applied is exclusively an executor memory state and never a durable SQLite state. `mark_unresolved` preserves a crash gap and sets RecoveryRequired. A failed/unsupported filesystem flush can still produce a truthful durable receipt/event, while keeping RecoveryRequired and blocking the next intent.

References must name earlier compiled steps in the same group or an earlier declared dependency (including transitive ancestors). Forward, missing, wrong-phase, wrong-path and undeclared cross-group references reject the plan. Direct dependency duplicates reject; overlapping transitive ancestors are valid. Actual receipt references resolve only to committed poststates. Unknown physical identity or absent ownership receipts cannot authorize deletion. Compensation ownership is restricted to the same dependency group, preserving completed prior groups. These constraints are tested at the journal consumer boundary; the data-only Rename model has no executor or journal dependency.

## Test-first evidence

Environment for Cargo commands:

```sh
export CARGO_HOME=/workspace/.tools/cargo
export RUSTUP_HOME=/workspace/.tools/rustup
export PATH=/workspace/.tools/cargo/bin:$PATH
```

Test break descriptions were written in comments before their test bodies. Initial tests were written before the model/journal implementation. The first compile run confirmed absent APIs/actions, followed by minimal signatures/no-op scaffolding to reach the intended runtime assertions.

1. Initial command: `cargo test -p vidlib-fileops --test journal`. `/workspace/scratch/task04-initial-red.log` records unresolved `events`/`journal` and missing RenameEntry/StageCopy/PublishCopy/RemoveOwned* variants. This compile failure was not treated as the final assertion-red evidence.
2. Minimal unsupported-open scaffold: same command, `/workspace/scratch/task04-assertion-red.log`: 9 failed, 2 passed. Nine tests reached the unimplemented open boundary; schema-preservation tests passed vacuously at this stage.
3. Real empty schema plus no-op mutation scaffold: same command, `/workspace/scratch/task04-intended-red.log`: 9 failed, 2 passed. Named intended failures:
   - `intent_survives_reopen`: pending length `left: 0, right: 1`.
   - `receipt_and_event_commit_together`: assertion that complete must fail under the event trigger failed (returned success).
   - `commit_error_prevents_next_step`: expected failed COMMIT returned `Ok(())`.
   - `published_copy_without_source_removal_is_distinct_fact`: no event existed (empty items).
   - `rejects_forged_steps_wrong_phases_and_out_of_order_actions`: second step was incorrectly accepted first.
   - Busy was incorrectly accepted; exact/native and full-action reads were still unimplemented.
4. Core implementation green: same command, `/workspace/scratch/task04-green-attempt.log`: **11 passed, 0 failed**, 5.01s including the real 5000 ms SQLite Busy case.
5. State/error-context red: `cargo test -p vidlib-fileops --test journal -- --skip busy_writer`, `/workspace/scratch/task04-state-red.log`: 12 passed, 2 failed, 1 filtered. `unresolved_step_cannot_be_cancelled_or_retried` observed IntentDurable instead of Unresolved; `mismatched_receipt_has_no_event_and_valid_receipt_can_still_commit` observed missing item_id instead of the known EntryId. Fixed the transition and contextual error propagation. `/workspace/scratch/task04-state-green.log`: **14 passed, 0 failed**, 1 filtered.
6. Dependency self-review red: `cargo test -p vidlib-fileops --test journal group`, `/workspace/scratch/task04-group-red.log`: 1 passed, 2 failed. `transitive_group_dependencies_preserve_declared_order` incorrectly rejected valid overlapping ancestors as duplicate dependencies; `compensation_cannot_claim_an_earlier_dependency_groups_creation` incorrectly accepted cross-group compensation. Fixed both. `/workspace/scratch/task04-group-green.log`: **3 passed, 0 failed**.
7. Private seam/effective PRAGMA verification: `cargo test -p vidlib-fileops journal`, `/workspace/scratch/task04-unit-green.log`: **2 journal unit tests passed**. These supplement the real deferred-FK COMMIT failure test; no mock filesystem was involved.
8. Final focused integration run after tightening copy identity/metadata/ack fixtures and adding owned-cleanup identity coverage: `cargo test -p vidlib-fileops --test journal -- --skip busy_writer`, `/workspace/scratch/task04-final-focused.log`: **17 passed, 0 failed**, 1 filtered. Busy was retained for the single final workspace run.

The actual-COMMIT failure test installs a deferred foreign-key violation triggered by event insertion: receipt/event INSERTs succeed, COMMIT fails, both are absent after rollback, and the next step remains Planned. A separate event INSERT-abort test proves rollback before COMMIT too. SQLITE_BUSY uses a second real connection holding BEGIN IMMEDIATE. Unknown-schema and invalid-checksum fixtures compare original file bytes after rejected open. Native path fixtures include non-UTF8 Unix bytes (and unpaired Windows UTF16 in the Windows build), u64::MAX sizes/revisions and i128::MIN/MAX timestamps.

## Initial implementation validation (before review round 1)

Only one broad portable workspace run was performed, after implementation and self-review corrections:

- `cargo test --workspace --exclude vidlib-tauri`: **126 passed, 0 failed**, all portable test binaries/doc tests successful. Breakdown: engine scan 14; fileops unit 10 (2 journal + 8 existing); duplicate actions 21; journal integration 18; platform 18; inventory 22; path codec 5; Rename contracts 18. Log `/workspace/scratch/task04-workspace-final.log`.
- `cargo clippy -p vidlib-fileops -p vidlib-rename --all-targets -- -D warnings`: **passed**, no warnings. Log `/workspace/scratch/task04-clippy-final.log`.
- `cargo fmt -p vidlib-fileops -p vidlib-rename -- --check`: **passed**, no output. Log `/workspace/scratch/task04-fmt-final.log`.
- `source /workspace/.tools/windows-check-env.sh` then `cargo check -p vidlib-fileops -p vidlib-rename --all-targets --target x86_64-pc-windows-gnu`: **passed**, including bundled SQLite C compilation, 11.71s. Log `/workspace/scratch/task04-windows-check.log`.
- `cargo metadata --no-deps --format-version 1` plus direct dependency assertions: inventory has no rename/fileops dependency; rename has no fileops dependency; fileops consumes rename: **passed**. Snapshot `/workspace/scratch/task04-metadata.json`.
- `git diff --check` and staged diff check: **passed**.
- After restoring the unrelated Cargo-resolver `cssparser-macros` syn2->syn3 drift to the base lockfile choice, `cargo check --locked -p vidlib-fileops`: **passed**; log `/workspace/scratch/task04-lock-check.log`. No task dependency was changed and no extra broad test run was performed.

## Self-review and limitations

Read the complete implementation/model diff, receipt/state boundaries, migration code, tests and lockfile. Corrected transitive group dependency deduplication, restricted compensation ownership to its group, supplied known EntryId error context, and added the durable Unresolved transition with named red/green evidence. Strengthened fixtures to distinguish copy/source native identities, persist metadata change parameters, and prove a mixed known/unknown acknowledgment batch rolls back without losing events. The full action family and persistence semantics are covered through the real journal consumer; existing Rename contract tests remain green without trivial model-construction tests.

The implementation adds no media effects, executable imports, UI recovery authority, coordinator or epoch-store mutation. Task05 still owns absolute app-local placement/local-filesystem validation, cross-instance coordination, startup pending gate and persisted namespace/policy epoch. Task07 owns actual effects and must stop when intent/receipt fails. Task08 owns filesystem reconciliation and the controller-defined versioned non-application audit/reset with fresh Recovery-lease proof and no fabricated completion. Task19 owns splitting staging creation/streaming into individually journaled phases/steps with durable creation identity before streaming. The present combined StageCopy is not claimed to be partial-copy recovery ready. This journal retains missing action/receipt evidence and refuses blind retries rather than implementing those later protocols. No full Rename/Blueprint release claim is made.

Windows evidence is compile-only. Windows runtime, MSVC packaging, NTFS/exFAT/share behavior, Explorer/property/helper execution remain pending. The portable workspace run excludes `vidlib-tauri` exactly as instructed because the known missing PNG baseline is owned by Task05. No new unexplained test failure remains.


## Review round 1 — bounded per-effect journal work

Finding addressed: **“Every effect performs whole-operation reconstruction inside both write transactions, making execution quadratic in batch size.”** Review source: `/workspace/scratch/task04-review.md`. Fix base: `3f589ec`; implementation commit: `16fdb4b2fbb35658caa7891a8959346847a93c48`.

Changes:

- Removed `load_operation` from all mutation paths. Intent gets the indexed next unfinished step and compares it to the immutable supplied step. Completion gets only its exact `(operation_id, step_id)` row. The existing phase, identity, ownership, receipt-state, ordering and transaction checks still run.
- Loads the action's bounded set of unique receipt references using compound keys scoped to the same operation, including receipt-producing step state checks. It does not load unrelated receipts or inspection inventory.
- Stores approved PlanRef independently in v2 and reads it through `operation_effect_header`, a covering index containing only ID/state/PlanRef. This prevents a large inspection column from affecting header reads even through SQLite overflow pages.
- `unfinished_steps` is a partial ordered index over Planned/IntentDurable/Unresolved steps, so finding the next step does not scan the completed prefix.
- A Running operation is no longer redundantly updated on each intent. Prepared -> Running is the one normal execution transition; terminal/failure transitions remain explicit.
- The v1 -> v2 migration preserves the original v1 SQL/checksum, copies and validates legacy approved references once, and commits schema/metadata changes atomically. Failure retains original evidence, schema and version. Full reconstruction remains available for explicit history/recovery reads.
- Added test-only rusqlite hooks as a dev dependency feature and private cfg(test) JSON decode accounting. No production counter, public testing switch or IPC option was added. No Cargo.lock package changes were needed.

Covering test files:

- `crates/vidlib-fileops/src/journal/mod.rs` private unit fixtures: deterministic SQLite VM instruction counts, JSON bytes decoded, operation-row update counts, migration rollback/evidence retention, existing PRAGMA/commit-failure tests. All use real temporary SQLite files. The performance fixture starts with actual v1 schema/evidence, opens/migrates it, executes public journal effects, then confirms explicit history still reconstructs the inspection snapshot.
- `crates/vidlib-fileops/tests/journal.rs`: adds `many_steps_keep_exact_provenance_and_order_after_reopen`, with a large inspection snapshot, 24 effects, duplicate step IDs in a different operation, rejection of out-of-order requests, and exact approved PlanRef preservation. The full existing journal integration binary covers every pre-existing state/identity/reference/commit invariant.

RED, before changing the hot path:

```sh
cargo test -p vidlib-fileops --lib journal::tests::effect_ -- --nocapture
```

`/workspace/scratch/task04-fix1-red.log`: **0 passed, 2 failed**. `effect_work_growth_is_bounded_as_batch_size_rises` observed 16-step work `(15739 VM instructions, 293113 decoded bytes)` versus 64-step work `(188923, 4745149)`, exceeding the bounded-growth assertion. `effect_reads_do_not_decode_inspection_snapshots` observed `(15739, 294537)` for empty padding versus `(15739, 8682571)` for 256 KiB inspection padding, exceeding the decode bound. These are measured work/byte counts, not wall-clock thresholds.

The first indexed implementation passed both tests (`/workspace/scratch/task04-fix1-green-attempt.log`). Self-review then identified a remaining redundant Running -> Running operation-row update, which would still rebuild the row containing the large inspection snapshot. Added a specific regression before fixing it:

```sh
cargo test -p vidlib-fileops --lib journal::tests::effect_writes_operation_header_only_on_state_change
```

`/workspace/scratch/task04-fix1-write-red.log`: **0 passed, 1 failed**, expected one operation-row update but observed **16**. Fixed by updating only the Prepared state; added the covering header index so scalar reads do not access inspection overflow pages.

Final GREEN/verification for this fix, using the same Cargo environment as the initial report:

```sh
cargo test -p vidlib-fileops --lib journal::tests -- --nocapture
cargo test -p vidlib-fileops --test journal
cargo clippy -p vidlib-fileops --all-targets -- -D warnings
cargo fmt -p vidlib-fileops -- --check
git diff --check
```

Results:

- `/workspace/scratch/task04-fix1-unit-final.log`: **6 passed, 0 failed**, 8 unrelated unit tests filtered. Actual output: 16 steps `(5849 VM instructions, 14021 decoded bytes, 1 operation update)`; 64 steps `(23225, 56133, 1)`. Four times the batch now produces approximately four times the work. The regression permits only `4 * small + 1024` VM instructions and `4 * small + 4096` decoded bytes. Empty versus 256 KiB padded inspection measured `(5849, 13957, 1)` and `(5849, 14085, 1)`; small native/UUID encoding differences are covered by a 1024-byte allowance, while the snapshot is absent from the effect decode path.
- `/workspace/scratch/task04-fix1-integration-final.log`: **19 passed, 0 failed**, 5.02s, including the real SQLITE_BUSY/deferred-COMMIT/atomic-outbox fixtures.
- `/workspace/scratch/task04-fix1-clippy-final.log`: **passed**, no warnings.
- `/workspace/scratch/task04-fix1-fmt-final.log`: **passed**, no output.
- `git diff --check`: **passed**.

Self-review confirmed `load_operation` is now called only by explicit `pending()`/`operation()` reads; state and receipt checks remain inside BEGIN IMMEDIATE; no transaction boundary or public authority was broadened. The new test hooks are dev/cfg(test) only. The immutable v1 checksum is preserved, successful legacy migration is exercised by the performance fixtures, and the failure fixture proves no partial v2 migration is committed.

No broad portable suite or Windows-target compile was repeated for this focused review correction. The earlier 126-test/Windows compile evidence remains historical to the initial implementation; the current correction is covered by the 25 journal tests and scoped checks above. Windows runtime, Task05 placement/coordination/epochs, Task08 non-application reconciliation and Task19 staging phase work remain as stated limitations, with no new release claim.
