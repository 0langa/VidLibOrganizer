# Task04 implementation report

Status: DONE. Review base: `eb026edfaf31384fa378da4ae7efddb5c8542557`.

Implementation commit: `16c5def19586638df38153606e39b4d39d12abaf` — `feat: persist operation intents receipts and outbox events`.

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

Schema tables: `schema_migrations`, `operations`, `steps`, `receipts`, `operation_events`, `outbox_ack`, `history_visibility`. The current schema is version 1; initialization from an empty version-0 database is transactional. Version/checksum/table checks reject unknown or inconsistent schemas before persistent PRAGMA changes. Original unknown/malformed fixture bytes are preserved. No existing supported schema upgrade is needed at v1; the requested backup feature is enabled for future checked upgrades.

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

## Final validation

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

The implementation adds no media effects, executable imports, UI recovery authority, coordinator or epoch-store mutation. Task05 still owns absolute app-local placement/local-filesystem validation, cross-instance coordination, startup pending gate and persisted namespace/policy epoch. Task07 owns actual effects and must stop when intent/receipt fails. Task08 owns filesystem reconciliation; this journal retains missing action/receipt evidence and refuses blind retries rather than attempting that reconciliation. No full Rename/Blueprint release claim is made.

Windows evidence is compile-only. Windows runtime, MSVC packaging, NTFS/exFAT/share behavior, Explorer/property/helper execution remain pending. The portable workspace run excludes `vidlib-tauri` exactly as instructed because the known missing PNG baseline is owned by Task05. No new unexplained test failure remains.
