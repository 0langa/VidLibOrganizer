# VidLibOrganizer Expansion Plan

## Goal

Expand VidLibOrganizer from a functional MVP into a reliable, scalable, local-first video library management application with strong workflow orchestration, safe file operations, rich discovery, and practical duplicate review.

## Roadmap principles

- Keep core business logic in reusable Rust crates, not in CLI or Tauri entrypoints.
- Preserve the local-first and offline-first safety model.
- Prioritize safe operations over feature breadth for any file-destructive workflow.
- Ship vertical slices that are usable end-to-end instead of isolated technical pieces.
- Prefer testable service boundaries and shared workflow orchestration.

## Target product outcomes

By the end of this roadmap, VidLibOrganizer should:

- scan and index large video libraries reliably across interrupted sessions
- provide fast, filterable, high-signal search and browsing
- help users review and resolve duplicates safely
- support richer metadata and local ML-assisted tagging
- perform high-confidence restructure operations with rollback support
- expose the same core workflows to CLI and desktop UI through shared crates

---

## Phase 1 — Workflow foundation and scan architecture

### Objective

Turn scanning into a resilient long-running workflow that supports cancellation, checkpointing, and shared orchestration across CLI and Tauri.

### Problems being solved

- scan orchestration is duplicated in entrypoints
- Tauri cancellation is not connected to real runtime state
- progress is not persisted
- library scans are not modeled as first-class jobs

### Deliverables

#### 1. New orchestration crate

Create `crates/vidlib-workflows` to own application workflows such as:

- scan library
- resume scan
- persist warnings and checkpoints
- emit progress events
- enrich entries with metadata and ML tags

#### 2. Shared scan job model

Add shared workflow models for:

- `ScanJobId`
- `ScanJobState`
- `ScanJobSummary`
- `ScanProgressEvent`
- `ScanRequest`
- `ScanOutcome`

These can live in `vidlib-core` if shared broadly, or in `vidlib-workflows` if orchestration-specific.

#### 3. Real cancellation registry

Implement an in-memory job manager for active runs with:

- job id to cancellation token mapping
- job id to current progress snapshot mapping
- job lifecycle transitions: pending, running, completed, failed, cancelled

#### 4. Persistent progress and resumability

Extend DB persistence to track:

- active and historical scan jobs
- per-library latest checkpoint
- last progress message and counters
- summary statistics from completed runs

#### 5. Thin entrypoint integration

Refactor:

- `vidlib-cli` to call workflow APIs instead of manually orchestrating scanning
- `src-tauri` to call the same workflow APIs and emit UI events

### Suggested crate changes

- add `crates/vidlib-workflows`
- keep `vidlib-scanner` focused on discovery and file-level scan mechanics
- keep `vidlib-db` focused on persistence only
- keep entrypoints thin

### Validation

- unit tests for job state transitions
- integration tests for start, cancel, and resume flows
- tests for checkpoint persistence and restore behavior

### Success criteria

- a scan can be started, cancelled, and resumed by library id
- CLI and Tauri both use the same orchestration path
- scan progress can be inspected during execution

### Risks

- over-designing abstractions before workflows stabilize
- race conditions in job registry design

### Mitigation

- begin with a synchronous internal implementation hidden behind a clean API
- add concurrency only where needed

---

## 1. Make scanning a real long-running workflow

### Why

Current scan flow is synchronous and the Tauri cancel path is not truly connected to active work.

### Add

- job registry with real cancellation token ownership
- resumable scan jobs by library id
- persisted progress snapshots in SQLite
- background worker model for Tauri and CLI reuse

### Outcome

Reliable ingestion for large libraries and better recovery for interrupted runs.

## Phase 2 — Search and library discovery

### Objective

Turn search from simple filtering into a fast, high-utility discovery experience for large libraries.

### Problems being solved

- current search performs in-memory filtering over stored entries
- no ranking or full-text support exists
- browsing large libraries will degrade as the dataset grows

### Deliverables

#### 1. FTS-backed search

Add SQLite FTS support for:

- file name
- full path
- tags
- codec and selected textual metadata

#### 2. Search ranking and query interpretation

Support:

- ranked results
- exact phrase vs loose match
- prefix matching where practical
- optional snippet generation for matched fields

#### 3. Rich filter model

Extend `SearchQuery` or create a versioned advanced query type with filters for:

- extension
- codec
- resolution range
- duration range
- size range
- duplicate state
- library root
- tags include/exclude

#### 4. Saved search support

Persist:

- recent queries
- named saved searches
- UI-friendly filter presets

#### 5. Search UX improvements

Desktop UI should gain:

- filter panels
- sortable result tables
- quick-open or preview actions
- result count and latency display

### Suggested implementation order

1. DB schema and FTS setup
2. core query model expansion
3. repository/search API updates
4. CLI support for advanced filters
5. Tauri/UI support

### Validation

- tests for FTS indexing and query correctness
- performance checks on synthetic large libraries
- regression tests for combined filters

### Success criteria

- search remains responsive on large libraries
- users can narrow results precisely with combined filters

---

## 2. Upgrade search from basic filtering to fast discovery

### Why

Search works, but it is still shallow and does not yet support fast exploratory workflows.

### Add

- SQLite FTS for filename, path, and tag search
- ranking and optional snippets
- saved searches and recent queries
- filters for codec, resolution, duration, size, and duplicate status

### Outcome

The app becomes a practical daily-use browser for large libraries.

## Phase 3 — Duplicate review and resolution workflow

### Objective

Transform duplicate detection from raw grouping output into a guided decision workflow.

### Problems being solved

- users can detect duplicates but cannot resolve them safely in-app
- duplicate groups are not prioritized by confidence or actionability

### Deliverables

#### 1. Duplicate review domain model

Add support for:

- canonical item selection
- reviewed / ignored / pending statuses
- per-group notes or rationale
- confidence class for exact, chunk, perceptual, and metadata similarity

#### 2. Heuristics for best candidate selection

Implement rules such as:

- prefer higher resolution
- prefer larger bitrate or size when metadata supports it
- prefer newer container/codec only when clearly superior
- prefer user-pinned library roots

#### 3. Safe duplicate actions

Support:

- mark canonical and hide alternates
- move duplicates to review folder
- send duplicates to recycle bin or quarantine area
- batch action preview before commit

#### 4. Duplicate review UI

Desktop UI should show:

- grouped duplicate sets
- side-by-side metadata comparison
- recommended keep candidate
- action preview and confirmation

### Validation

- tests for grouping stability
- tests for heuristic ranking
- tests for batch action previews and rollback paths

### Success criteria

- duplicate groups become reviewable and actionable
- destructive decisions remain preview-first and reversible

---

## 3. Turn duplicate detection into a review workflow

### Why

Duplicate grouping exists, but users cannot yet review and act on results safely.

### Add

- duplicate review UI
- keep-best heuristics
- bulk actions for canonical selection, move, and recycle-bin delete
- confidence tiers for exact, near, and perceptual matches

### Outcome

Users can safely reduce clutter instead of only seeing raw duplicate groups.

## Phase 4 — Rich metadata, thumbnails, and local ML tagging

### Objective

Increase the quality of organization and search by expanding metadata extraction and making local ML features practical.

### Problems being solved

- metadata is limited to a small `ffprobe` subset
- there are no thumbnails or sampled frames for UI review
- ML tagging is placeholder-level today

### Deliverables

#### 1. Richer media metadata extraction

Expand support for:

- bitrate
- frame rate
- aspect ratio
- audio streams and codecs
- container format
- creation timestamps where available

#### 2. Thumbnail and frame sampling pipeline

Add sampled-frame extraction for:

- preview thumbnails in UI
- future perceptual video hashing improvements
- downstream ML inference input

#### 3. ONNX model integration path

Extend `vidlib-ml` with:

- model registry/config
- pluggable model descriptors
- threshold tuning support
- fallback logic when models are unavailable

#### 4. Tag management model

Support:

- manual tags
- machine-suggested tags
- locked tags
- tag provenance
- user rules for auto-tagging

### Validation

- tests for metadata parsing edge cases
- tests for thumbnail generation pipeline
- deterministic tests for rule-based tagging behavior

### Success criteria

- results display richer media detail
- users can organize libraries based on more than filename and extension
- ML tagging remains optional and local-only

---

## 4. Build a real metadata and tagging pipeline

### Why

Metadata extraction is still minimal and ML tagging is currently heuristic.

### Add

- richer `ffprobe` parsing
- thumbnail and sampled-frame generation
- ONNX-backed local tagging
- user-editable tags and tag rules
- folder naming templates driven by metadata and tags

### Outcome

Organization becomes meaningfully smarter while staying local-first.

## Phase 5 — Production-safe restructure engine

### Objective

Make restructure and move operations safe enough for large real-world libraries.

### Problems being solved

- file move operations are inherently high risk
- policy decisions for naming and conflicts are still basic
- recovery guarantees need to be stronger

### Deliverables

#### 1. Naming and placement policy engine

Support configurable strategies such as:

- by extension
- by codec
- by year
- by tags
- by custom templates

#### 2. Conflict policy model

Support:

- rename on collision
- skip on collision
- quarantine on collision
- compare-and-confirm behavior

#### 3. Stronger rollback model

Improve undo manifests with:

- validation before apply
- verification after apply
- post-action audit trail
- rollback consistency checks

#### 4. Safety-oriented UI

Add:

- preflight summary
- diff preview
- confirmation checkpoints
- dry-run as default in GUI and CLI

### Validation

- integration tests for conflict-heavy plans
- rollback tests on partial failure scenarios
- tests for naming template determinism

### Success criteria

- users trust the tool for large-scale reorganization
- partial failures do not leave the library in an unclear state

---

## 5. Make restructure operations production-safe

### Why

File moves are one of the highest-risk features and need stronger safeguards.

### Add

- preview diff UI
- destination naming policies
- rollback verification
- recycle-bin or quarantine mode
- collision and normalization policy engine

### Outcome

The app becomes trustworthy for large-scale reorganization.

## Phase 6 — Architecture hardening and release readiness

### Objective

Prepare the workspace for sustained feature growth and public release quality.

### Problems being solved

- orchestration logic still risks drifting between entrypoints
- some modules will grow quickly as features land
- release engineering and observability are still thin

### Deliverables

#### 1. Shared workflow boundary

Complete migration of workflow orchestration into `vidlib-workflows`.

#### 2. Service-oriented module structure

Introduce clear service layers for:

- search
- scan orchestration
- duplicate review
- restructure planning
- metadata enrichment

#### 3. Workspace quality gates

Add or tighten:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- targeted integration tests for key workflows

#### 4. Packaging and release readiness

Plan for:

- installer generation
- app settings persistence
- user data migration rules
- release notes and changelog process

### Success criteria

- entrypoints are thin and stable
- the codebase has clear ownership and test boundaries
- releases are predictable and low-risk

---

## 6. Improve architecture for growth

### Why

Too much orchestration still lives in entrypoints.

### Add

- a new `vidlib-workflows` crate for scan, index, and review orchestration
- thinner `main.rs` files in CLI and Tauri
- service traits for database, scanner, metadata, ML, and file operations
- shared event model for CLI and GUI progress/reporting

### Outcome

The codebase becomes easier to test, extend, and maintain.

## Cross-cutting workstreams

These should progress alongside the main phases.

### A. Testing and validation

- add regression tests for all bug fixes
- maintain workspace-level validation commands
- add synthetic-library fixtures for large-scale tests

### B. Performance

- benchmark scan throughput
- benchmark search latency with realistic datasets
- reduce duplicate unnecessary file reads

### C. UX and product polish

- clarify terminology in UI and CLI
- standardize progress and error messaging
- improve dashboard usefulness over time

### D. Documentation

- keep `README.MD` aligned with product capabilities
- document crate contracts as architecture evolves
- add operator docs for destructive workflows and recovery

## Quarter-style roadmap view

### Milestone A

- create `vidlib-workflows`
- implement real scan job model
- wire shared scan orchestration into CLI and Tauri

### Milestone B

- add FTS search
- implement richer filters
- improve dashboard and search UX

### Milestone C

- add duplicate review model and UI
- ship safe duplicate actions and review states

### Milestone D

- expand metadata
- add thumbnails and sampled frames
- integrate first ONNX-assisted tagging path

### Milestone E

- harden restructure policies
- improve rollback and preview flows
- prepare packaging and release readiness

## Recommended immediate next tasks

1. Create `crates/vidlib-workflows`
2. Move current scan orchestration out of `vidlib-cli` into the new crate
3. Refactor `src-tauri` scan command to call the same workflow
4. Add workflow tests for start, cancel, and resume
5. Extend `EXPANSION_PLAN.md` later with issue links and status tracking

---

## Tracked execution plan

This section translates the roadmap into a practical implementation backlog.

### Priority model

- **P0** — foundational, blocks multiple future features
- **P1** — high-value core capability
- **P2** — important product improvement
- **P3** — polish, scale, or release hardening

### Effort guide

- **S** — small, likely 0.5 to 1 day
- **M** — medium, likely 2 to 4 days
- **L** — large, likely 1 to 2 weeks
- **XL** — multi-milestone effort

## Backlog by milestone

### Milestone A — Shared workflow orchestration

#### A1. Create workflow crate
- Priority: **P0**
- Effort: **M**
- Status: Not started
- Description: Create `crates/vidlib-workflows` and establish shared orchestration APIs.
- Target files/crates:
	- `Cargo.toml`
	- `crates/vidlib-workflows/Cargo.toml`
	- `crates/vidlib-workflows/src/lib.rs`
	- `docs/crate-contracts.md`

#### A2. Add scan workflow service
- Priority: **P0**
- Effort: **L**
- Status: Not started
- Description: Move scan/index orchestration out of entrypoints into a reusable workflow service.
- Target files/crates:
	- `crates/vidlib-workflows/src/scan.rs`
	- `crates/vidlib-cli/src/main.rs`
	- `src-tauri/src/main.rs`
	- `crates/vidlib-db/src/lib.rs`

#### A3. Introduce scan job state model
- Priority: **P0**
- Effort: **M**
- Status: Not started
- Description: Define job ids, lifecycle state, summary, and progress event types.
- Target files/crates:
	- `crates/vidlib-core/src/models.rs`
	- or `crates/vidlib-workflows/src/models.rs`

#### A4. Implement cancellation registry
- Priority: **P0**
- Effort: **M**
- Status: Not started
- Description: Track active jobs and wire real cancellation to running scan work.
- Target files/crates:
	- `crates/vidlib-workflows/src/jobs.rs`
	- `src-tauri/src/main.rs`

#### A5. Persist scan jobs and progress snapshots
- Priority: **P1**
- Effort: **L**
- Status: Not started
- Description: Extend DB for scan job history and progress tracking.
- Target files/crates:
	- `crates/vidlib-db/src/lib.rs`
	- `crates/vidlib-core/src/models.rs`

#### A6. Add workflow tests
- Priority: **P0**
- Effort: **M**
- Status: Not started
- Description: Cover start, cancel, resume, and checkpoint restore flows.
- Target files/crates:
	- `crates/vidlib-workflows/src/*.rs`
	- `tests/workflow_integration.rs`

### Milestone B — Search and discovery

#### B1. Add FTS schema and indexing
- Priority: **P1**
- Effort: **L**
- Status: Not started
- Description: Add SQLite FTS tables and indexing strategy for search-heavy fields.
- Target files/crates:
	- `crates/vidlib-db/src/lib.rs`
	- migration helpers inside `vidlib-db`

#### B2. Expand search query model
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Support richer filters for metadata, duplicates, and library scope.
- Target files/crates:
	- `crates/vidlib-core/src/models.rs`
	- `crates/vidlib-db/src/lib.rs`

#### B3. Add ranked search API
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Return ranked search results and optional snippets or scores.
- Target files/crates:
	- `crates/vidlib-db/src/lib.rs`
	- `crates/vidlib-cli/src/main.rs`
	- `src-tauri/src/main.rs`

#### B4. Add saved searches
- Priority: **P2**
- Effort: **M**
- Status: Not started
- Description: Persist named and recent searches for reuse.
- Target files/crates:
	- `crates/vidlib-db/src/lib.rs`
	- `crates/vidlib-core/src/models.rs`
	- UI files under `ui/`

### Milestone C — Duplicate review workflow

#### C1. Add duplicate review state model
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Model canonical picks, ignored groups, and review status.
- Target files/crates:
	- `crates/vidlib-core/src/models.rs`
	- `crates/vidlib-db/src/lib.rs`

#### C2. Implement keep-best heuristics
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Rank duplicate candidates using metadata and quality rules.
- Target files/crates:
	- `crates/vidlib-duplicates/src/lib.rs`

#### C3. Add safe duplicate actions
- Priority: **P1**
- Effort: **L**
- Status: Not started
- Description: Preview and execute move/quarantine/delete flows safely.
- Target files/crates:
	- `crates/vidlib-fileops/src/lib.rs`
	- `crates/vidlib-workflows/src/duplicates.rs`

#### C4. Build duplicate review UI
- Priority: **P2**
- Effort: **L**
- Status: Not started
- Description: Add group comparison and review actions to desktop UI.
- Target files/crates:
	- `ui/index.html`
	- `ui/main.js`
	- `ui/styles.css`
	- `src-tauri/src/main.rs`

### Milestone D — Metadata and ML enrichment

#### D1. Expand metadata extraction schema
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Capture additional audio/video/container attributes.
- Target files/crates:
	- `crates/vidlib-metadata/src/lib.rs`
	- `crates/vidlib-core/src/models.rs`

#### D2. Add thumbnail generation pipeline
- Priority: **P2**
- Effort: **L**
- Status: Not started
- Description: Generate sampled frames and thumbnails for previews.
- Target files/crates:
	- `crates/vidlib-metadata/src/lib.rs`
	- new thumbnail module or crate if needed

#### D3. Build ONNX model configuration path
- Priority: **P2**
- Effort: **L**
- Status: Not started
- Description: Add model configuration, thresholds, and runtime selection.
- Target files/crates:
	- `crates/vidlib-ml/src/lib.rs`
	- config handling in CLI/Tauri

#### D4. Add tag provenance and rule system
- Priority: **P2**
- Effort: **L**
- Status: Not started
- Description: Distinguish manual, inferred, and locked tags with rules.
- Target files/crates:
	- `crates/vidlib-core/src/models.rs`
	- `crates/vidlib-db/src/lib.rs`
	- `crates/vidlib-ml/src/lib.rs`

### Milestone E — Restructure hardening and release readiness

#### E1. Add naming policy engine
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Support template-driven and rule-driven destination planning.
- Target files/crates:
	- `crates/vidlib-fileops/src/lib.rs`

#### E2. Add conflict resolution policies
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Support rename, skip, quarantine, and confirm policies.
- Target files/crates:
	- `crates/vidlib-fileops/src/lib.rs`
	- `crates/vidlib-core/src/models.rs`

#### E3. Strengthen rollback verification
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Validate apply/undo integrity and log consistency.
- Target files/crates:
	- `crates/vidlib-fileops/src/lib.rs`
	- `crates/vidlib-db/src/lib.rs`

#### E4. Prepare release pipeline
- Priority: **P3**
- Effort: **L**
- Status: Not started
- Description: Add packaging, validation gates, and release documentation.
- Target files/crates:
	- workspace root CI/config files
	- `README.MD`
	- Tauri packaging config under `src-tauri/`

## Suggested issue templates

Use issue titles in this form:

- `workflow: extract scan orchestration into vidlib-workflows`
- `db: add persisted scan job records`
- `search: add SQLite FTS index for video discovery`
- `duplicates: add canonical selection heuristics`
- `fileops: add collision resolution policies`
- `ml: add ONNX model configuration support`

Each issue should include:

- problem statement
- target crate(s)
- public API changes
- migration or schema impact
- tests required
- acceptance criteria

## Suggested sprint order

### Sprint 1
- A1 create workflow crate
- A2 extract scan workflow
- A3 add scan job state model

### Sprint 2
- A4 cancellation registry
- A5 persisted scan jobs
- A6 workflow tests

### Sprint 3
- B1 FTS schema
- B2 query model expansion
- B3 ranked search API

### Sprint 4
- C1 duplicate review state
- C2 keep-best heuristics
- C3 safe duplicate actions

### Sprint 5
- D1 richer metadata
- D2 thumbnail pipeline
- E1 naming policy engine

### Sprint 6
- D3 ONNX config path
- D4 tag provenance and rules
- E2 conflict resolution policies
- E3 rollback verification

## Recommended next implementation target

If work starts immediately, begin with **A1 + A2**:

- create `vidlib-workflows`
- move scan orchestration into it
- make both CLI and Tauri consume the same scan workflow API

That unlocks the cleanest path for all later roadmap items.

## Recommended implementation order

1. Real scan job and cancellation architecture
2. FTS search and richer filters
3. Duplicate review workflow
4. Safer restructure engine and UI
5. Rich metadata and ONNX tagging
6. `vidlib-workflows` orchestration refactor

## Suggested next milestone

Create a new `vidlib-workflows` crate and move scan orchestration out of `vidlib-cli` and `src-tauri` so both entrypoints share one workflow implementation.
