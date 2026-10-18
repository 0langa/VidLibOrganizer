# Rename phase review — interrupted continuation checkpoint

Range: `711b422fc7c060e51b188bae419b0b0204f56317..f7595687788376343c1c8067e6b03167dde0b200`, branch `feat/rename-blueprint`. Read-only checkout review; only this report was written. No helpers, source changes, tests, builds or suite confirmation reruns. User changed scope to wrap up and push for local continuation; further exploration stopped immediately. This is a durable findings checkpoint, **not completed whole-phase approval**.

## Coverage and evidence provenance

This seat continues the previous review stopped solely for the user's model preference. The prior reviewer's preserved checkpoints report inventory/native-selection/desktop pipeline, model/recipe, graph/composition, executor/journal/copy/Undo/recovery/evidence, helper and UI passes. Its actual written partial notes cover inventory package lines25990–28316 and deferred01/12 concerns. These are **inherited reported passes**, not source independently re-reviewed by this continuation. The preserved checkpoints and notes are in `.superpowers/sdd/2026-10-17-rename-and-blueprint/rename-phase-review-handoff.md` and `rename-phase-review-astra-partial.md`.

I independently inspected the six interface joins below in current source: collision loops/recipe validation; executor/copy/desktop job controls; UI reducer/navigation/job/history; mapping authorization versus selected-root inventory/Ruling19; full journal loading. I also inspected focused current text casing, date parsing, frozen-cache indexes, metadata equal-Set/cancellation test, page projection, native retry notice, overlapping-root coverage and offline vendor guard. Read the official reviewer contract, phase brief, current16deferred index, approved Rename spec sections/capability matrix, current ruling snapshot (including19/20), Task23 report and tracked Task23 review. Some large combined document output was truncated; I do not claim complete fresh inspection of all canonical01–23 source/spec/ruling text or the entire2.67MB package. Task-specific prior reviews provide useful evidence but do not replace an independent broad pass.

## Strengths

- Native paths/opaque IDs remain separate from display strings; held source/ancestor and no-replace authority, immutable plans, durable intent/receipts and conservative recovery are coherent design strengths supported by inherited passes and focused joins.
- Current journal truth survives cancellation; the UI ownership finding below concerns presenting that truth, not loss of durable receipts. Existing backend history removal protects unfinished work and keeps execution evidence separate from visibility.
- Task23 uses narrow disposable lookup indexes and checked generated-output limits, preserving frozen order and exact quantities. Reviewed frozen-cache code revalidates content binding even for cached hashes and checks cancellation before publishing prepared state.
- Verification documentation distinguishes portable/native evidence and exposes actual scale limits rather than presenting seeded100k data as a full native preview.

## Critical

None established by the inspected code. This is not a claim that unfinished broad coverage excludes every Critical defect.

## Important — verified findings for one combined fix wave

### I1. Collision suffix search accepts billions of uncancellable attempts

**References:** `crates/vidlib-rename/src/graph.rs:267`, `:388`; `crates/vidlib-rename/src/recipe.rs:254`, `:464`.

`max_attempts` is `u32`; validation rejects zero and path separators/NUL in the separator but sets no practical upper bound. Both independent-row and companion loops enumerate every attempt, with no `JobControl`/cancellation checkpoint. A colliding target already at its component-length limit makes every appended suffix invalid; an imported valid recipe with `max_attempts=4294967295` then performs billions of guaranteed failures. This needs neither billions of occupied files nor an oversized import. Cancellation cannot interrupt this planning phase, retaining the job/coordinator resources and leaving the user waiting.

**Correction:** impose a documented practical attempt/work bound, stop structurally impossible suffix candidates early, and thread existing cooperative control through collision planning and companion loops. Include bounded exhaustion and deterministic cancellation checks; do not benchmark billions of attempts or invent runtime suffixes after approval.

### I2. Filesystem mutation jobs never publish execution progress

**References:** `crates/vidlib-fileops/src/rename.rs:113`, `:120`, `:258`, `:300`; `crates/vidlib-fileops/src/copy.rs:31`, `:54`; `src-tauri/src/commands/rename.rs:1192`, `:1476`; `ui/rename/main.js:266`.

The forward executor checks cancellation and persists receipts, but emits no progress. The copy/hash loops likewise consume control only for cancellation. Desktop apply/recovery pass this control straight into fileops; the UI waits for `watchJob` progress that mutation work does not produce. Ordinary long Rename/Copy/Move/Undo apply therefore offers no completed-step/group or byte progress until the terminal result. The recorded1000-file forward run lasted476.397s, making this material user feedback rather than polish. Approved spec section4 step6 explicitly requires background progress and partial outcomes.

**Correction:** emit throttled factual execution phase/count/current-item updates at safe step/group boundaries and useful copy-byte boundaries through the existing bounded control channel; cover recovery/inverse mutation and waiting states without claiming blocked OS I/O is cancellable. Preserve durable receipt semantics and truthful terminal reports.

### I3. Leaving Rename silently cancels active mutation and abandons its UI outcome owner

**References:** `ui/main.js:36`; `ui/rename/main.js:208`, `:231`, `:257`, `:384`, `:387`, `:445`, `:473`, `:486`; `ui/rename/history.js:66`.

Switch views while Apply or recovery is active. `deactivate()` calls `edited()`, whose invalidation clears `active`, aborts polling and sends `cancel_job`, including for `task.mutation`. The generation/sequence checks then reject the eventual factual report, and history is destroyed. `activate()` is empty, so returning neither reconnects to the job nor refreshes history. An ordinary navigation click thus acts as an undisclosed cancellation; completed groups may remain while the view reports changed inputs and has lost the job's final outcome/history ownership. Journal evidence remains durable, but the user must discover the partial outcome manually.

**Correction:** retain mutation job ownership across view switches and reconnect/present terminal results and refreshed history; reserve cancellation for the explicit Cancel action. Read-only preview work may keep its invalidation behavior. Test leaving/returning during forward mutation and recovery, including a partial/cancelled terminal result.

### I4. Completed history has neither detail inspection nor explicit removal in the product

**References:** `ui/rename/history.js:49`; `src-tauri/src/commands/rename.rs:1204`, `:1580`; `crates/vidlib-fileops/src/journal/mod.rs:223`.

A completed/failed/cancelled history item renders only state, completed-step count, operation ID and optionally Preview Undo. There is no completed-operation detail view or removal action; the backend's protected `remove_history` API is not exposed through the inspected desktop command/UI surface. A user cannot inspect retained actual old/new paths, approved inputs, timestamps or per-step outcomes for an ordinary completed operation, nor explicitly remove completed history/Undo support as spec section8 and Task09 require. Undo preview is not a history inspector and is unavailable for some entries.

**Correction:** expose bounded read-only paged operation details from durable facts and a separately confirmed completed-history removal command/action that clearly revokes Undo while preserving recovery/audit/outbox evidence. Keep unfinished removal blocked and honor history revision invalidation.

### I5. Imported mappings reject the exact selected-directory sibling rename authorized by Ruling19

**References:** `crates/vidlib-rename/src/mappings.rs:348`, `:359`, `:401`; `crates/vidlib-inventory/src/scan.rs:195`; `crates/vidlib-inventory/src/model.rs:82`; `crates/vidlib-rename/src/validate.rs:243`; Ruling19 in `rename-rulings-source.md`.

Fresh-select directory `/parent/old` with `NativeSelectedEntry`, then import its exact mapping to `/parent/new`. Inventory correctly includes the selected root as a directory item. Mapping authorization only accepts target paths under an inventory root/grant; its sibling-target exception is restricted to `EntryKind::File`. The valid directory sibling target is rejected as OutsideGrant before the regular planner can apply the selected-entry boundary. This breaks R12 composition with selected-folder Rename/R15/R18 despite Ruling19 expressly authorizing that exact fresh selected-root sibling rename.

**Correction:** share the canonical exact selected-entry authorization predicate for mappings, respecting NativeSelectedEntry source, exact source-root identity/path and validated same-parent sibling target. Do not grant the parent, arbitrary siblings, parent metadata or an ungranted destination. Add exact directory mapping success plus wrong grant/descendant/sibling-access/protection refusals.

### I6. Each execution step repeatedly reconstructs the whole journal operation

**References:** `crates/vidlib-fileops/src/rename.rs:127`, `:135`, `:152`, `:187`, `:190`, `:192`; `crates/vidlib-fileops/src/journal/mod.rs:830`, `:844`, `:860`, `:890`.

Every ordinary RenameEntry invokes `journal.operation(op)` separately for material, controlled fields and parents; inverse and non-rename paths add further full loads. `load_operation` loads/deserializes every compiled step, queries/deserializes receipts per step, loads recovery effects and inspection. Thus an N-step operation repeatedly performs whole-operation work before each step, producing quadratic-or-worse practical work. Existing targeted journal prerequisite indexing does not remove these executor reads. Retained real measurements are consistent with this defect:100→1000 files, forward4198→476397ms, Undo-plan40→4744ms, Journal::begin79→5579ms. Timings alone do not isolate every cost, but the repeated full reconstruction is directly verified.

**Correction:** load immutable compiled/inspection data once per operation and maintain validated execution/receipt state or use narrowly keyed durable prerequisite queries. Update cached state only after successful intent/completion persistence; preserve recovery/metadata lineage, crash gaps and truthful receipt authority. Measure end-to-end forward/inverse at representative scales after the fix. Journal::begin includes pretransaction work and is not an isolated SQL lock timer; the inverse benchmark does not execute inverse effects.

## Known Minor findings and all16 deferred dispositions

This table distinguishes current source observations from prior evidence and incomplete adjudication. Open items have **not** been silently accepted or fixed by this review.

| ID | Checkpoint disposition / evidence | Required follow-up |
|---|---|---|
|01|Open Minor, source confirmed: `crates/vidlib-inventory/src/scan.rs:180` clones overlapping-root coverage, replaces root/directory binding but leaves issue relative paths in the old binding.|Rebase diagnostic paths only; preserve native authority/per-binding coverage.|
|02|Implementation addressed by reviewed Task19 lifecycle; current `copy.rs:99` drops staging writer. Portable lifecycle and Windows fixture compilation are inherited evidence; native sharing runtime still pending.|Retain Windows sharing qualification; no native pass inferred.|
|03|Resolved by Task06 checked plan/provenance grants per prior review and inherited broad checkpoint. This continuation did not freshly verify the full publication boundary.|Retain existing resolution/evidence; avoid claiming fresh independent closure.|
|04|Original kind casing defect addressed: current desktop `rename.rs:715` and `:1092` use camel tags. Native regression/review is prior evidence; shared literal golden breadth not freshly checked.|Controller reconcile canonical/golden closure; no current casing defect found.|
|05|Canonical filename-list mismatch, not a demonstrated runtime defect. Existing behavior was previously independently verified; this continuation did not reconcile all task ownership hunks.|Reconcile plan wording against actual changes, without gratuitous code.|
|06|Open maintainability Minor: current `ui/rename/main.js:126` and `:139` onward retain dense handlers, with later Task21 extension recorded.|Targeted formatting/extraction when touching these handlers; no broad refactor required.|
|07|Open Minor, source confirmed `text.rs:682`: lowercase the whole word, then title first character. Initial `İ` becomes `i`+combining dot then `I`+combining dot rather than preserving composed `İ`.|Narrow Unicode byte golden/preservation decision; no global normalization.|
|08|Open Minor, current `dates.rs:157` still sends arbitrary RFC3339 to Chrono before exact conversion. Prior Task13 reviewer inspected Chrono's excess-digit truncation; this continuation did not repeat dependency inspection or run a fixture.|Reject significant excess fractional digits; allow harmless trailing zero precision or explicitly document policy.|
|09|Original retained/output/primary-position scans addressed in inspected Task23 `hashes.rs:187`, `:193`, `:306` indexes. Full frozen/template/provider scale closure remains incomplete; report admits residual unmeasured caches.|Close only the named loops; retain resource qualification and I6 separately.|
|10|Resolved by necessary Task15 clean UI run per retained evidence; historical warning remains historical.|No code change; use `env -u NO_COLOR` for necessary future UI runs.|
|11|Addressed Task23: source confirmed explicit `--fetch` guard at `tools/vendor_media_info.py:56`; prior four-Python run includes downloader-not-called regression.|Retain resolved code disposition; offline installer runtime remains distinct/pending.|
|12|Open presentation Minor, source confirmed `rename.rs:1102`: arbitrary user `Rule.id=companion_warning` is projected as generated companion warning.|Use typed/separate warnings or a reserved validated namespace with DTO regression.|
|13|Open Minor, source confirmed `metadata.rs:475`: every directory preserves an equal timestamp Set, including an empty unchanged directory with no child/path effects, yielding false Changed/effects/summary.|Retain equal-Set only when relevant effects require finalization; preserve Ruling17 direct/nested-child guarantees.|
|14|Original per-row full projection scans addressed in inspected `rename.rs:1066` per-query indexes. Actual100k seeded page measured, but natural debug2.598s and native reference gate remain pending.|Close the original scans, retain remaining per-query/sort/platform scale limits.|
|15|Open Minor, source confirmed `ui/main.js:229`: successful automatic retry hides discard but never clears its prior waiting/error notice.|Track notice ownership and clear/replace on success/null pending, retaining unrelated notices; busy→success browser regression.|
|16|Open test-reliability Minor, source confirmed `metadata.rs:738` vs `control.rs:63`: waits for positive partial progress despite100ms throttle; fast completion or delayed reader can fail correct code.|Deterministic partial-row handshake/test clock, with throttle covered separately. No production cancellation defect/failing run claimed.|

## Cannot verify and controller ownership

Final recorded baseline, not rerun here:633 parent Rust tests,0failed/4ignored (raw634 includes regex child);4Node/57browser/4Python; Clippy/fmt/19JS/Linux release/GNU cfg/helper8/NSIS hook source compilation pass. I inspected reported evidence/docs and the tracked Task23 review, not all raw logs in this continuation. Earlier review inspected raw logs. No available portable result proves native qualification.

Native Windows/MSVC, NTFS/exFAT/network/storage/sharing/ACL/reparse runtime, property/MediaInfo runtime, installed dialogs/Explorer/registry, clean disconnected offline installer, hardware durability/power loss and documented Windows reference performance remain **PENDING; production mutation/release gates CLOSED**. Full installer artifact/version/hash/provenance and runtime must be recorded before qualification.

100k seeded metadata/page/core evaluation is not end-to-end desktop preview. Full100k selection serializes7800424bytes and refuses unchanged1MiB recipe cap; retain explicit product-scale limitation. Actual1000-file native-bound Linux preview is a separate measurement. Natural debug2597.740ms does not establish native release target pass/fail.1000 forward476397ms/Undo-plan4744ms/Journal::begin5579ms retain their exact distinctions; inverse execution not benchmarked. Per-intermediate65536-byte output budget does not bound total inventory/plan/clone/explanation/frozen/step memory; aggregate memory remains unqualified.

Root owns findings consolidation, any ONE combined fix wave and ONE scoped rereview, explicit remaining16item adjudication, tracked durable handoff/backup/push as now requested, and pause before Blueprint24. No fix or full-phase approval occurred during this seat. Local continuation must use this report and inherited limits rather than treat earlier task approvals as whole-branch completion.

## Behaviors considered but declined to judge before verdict

- Blueprint24–32 implementation/completion: explicitly later scope and user pause boundary; current Rename protections remain in scope.
- Actual native Windows/platform/storage/installer/hardware/reference performance: unavailable on this Linux review host; retain required qualification and closed gates, not inferred passes.
- General hostile privileged-process race immunity: explicitly disclaimed by approved spec; ordinary held-object/no-follow/current-authority behavior remains in scope.
- Universal cancellation deadline for blocked OS I/O: spec expressly declines that guarantee; I1 concerns cooperative CPU planning, not blocked I/O.
- PCRE2 universal wall-clock timeout: approved match/depth/heap/pattern budgets are not a universal deadline; no new regex defect established by this continuation.
- Full100k-selected runtime preview as a promised numeric acceptance threshold: spec's100k target concerns synthetic inventory cached page filtering/sorting; the actual selection-binding cap remains an explicit unresolved product limitation, not silently qualified.
- Arbitrary aggregate-memory/OOM safety and unmeasured template/provider scaling: not fully assessed before user requested immediate wrap-up; qualification remains open, no safety/performance pass.
- R01–R18 exhaustive whole-source reproving, every canonical task/ruling/legacy fixture and complete duplicate/preferences regression review: broad earlier passes/task reviews are inherited; this continuation's targeted independent checks are listed above. User stopped further exploration before full completion.
- Optional Blueprint event projection behavior beyond retained outbox authority: later implementation; current persistence/independence contract remains relevant, but no new projection judged here.
- Out-of-release arbitrary scripts, online identification, embedded media editing, watchers/transcoding/BRU binary compatibility: approved spec explicitly excludes them.

## Assessment

**Spec/coverage:** incomplete whole-phase assessment; six demonstrated Important gaps affect R12/R15/R17/R18 composition, required mutation progress/history, ordinary navigation behavior and practical large-operation execution. Earlier per-task approvals do not close these interface joins.

**Quality:** sound safety architecture and substantial retained portable evidence, with concrete usability/resource joins needing correction and several residual Minors. **Ready to merge/release: No approval; with fixes and remaining qualification/coverage work.** This report is a resumable review checkpoint produced under the user's immediate wrap-up instruction, not a completed broad review or release gate.
