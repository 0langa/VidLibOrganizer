# VidLibOrganizer Code Map

## Purpose

This document is a maintainer-oriented map of the repository. It explains where major logic lives, how to navigate the workspace, and where future contributors should add new functionality.

## Repository top level

### `Cargo.toml`

Workspace manifest.

Defines:

- workspace members
- workspace-wide package metadata
- shared dependency versions

### `README.MD`

Primary repository entrypoint for users and contributors.

Should provide:

- project overview
- current capabilities
- setup and run instructions
- pointers into `docs/`

### `EXPANSION_PLAN.md`

Long-term product and engineering roadmap.

Use this for:

- strategic direction
- milestone priority review
- future architecture and schema planning

### `docs/`

Persistent project documentation.

Current and intended contents:

- `crate-contracts.md`
- `architecture.md`
- `codemap.md`
- `development_progress.md`
- `full_project_documentation.md`

### `tests/`

Workspace-level integration tests.

Use this area for:

- public behavior tests
- end-to-end workflow tests
- cross-crate integration coverage

### `ui/`

Frontend assets for the Tauri desktop app.

### `src-tauri/`

Desktop backend crate and Tauri project configuration.

## Workspace crates

## `crates/vidlib-core`

### Purpose

Shared domain contracts and reusable error/progress models.

### Key files

#### `crates/vidlib-core/src/lib.rs`

Public export surface for the core crate.

#### `crates/vidlib-core/src/models.rs`

Primary domain model definitions.

Contains models such as:

- `LibraryFolder`
- `VideoEntry`
- `ScanCheckpoint`
- `DuplicateGroup`
- `DuplicateStrategy`
- `JobState`
- `RestructureAction`
- `RestructurePlan`
- `UndoManifest`
- `SearchQuery`
- `ScanSummary`
- `ScanWarningRecord`
- `AuditRecord`

This is the first place to look when shared state needs to change.

#### `crates/vidlib-core/src/error.rs`

Shared `VidLibError` and result alias.

Use this when:

- adding new cross-crate error categories
- standardizing user-facing error formatting

#### `crates/vidlib-core/src/progress.rs`

Shared `ProgressSnapshot` model.

Use this when:

- evolving progress reporting
- coordinating workflow/UI feedback

## `crates/vidlib-db`

### Purpose

SQLite persistence and queries.

### Key file

#### `crates/vidlib-db/src/lib.rs`

Currently the main persistence implementation.

Contains:

- database open/configuration
- migration bootstrap
- library CRUD-style operations
- entry upsert operations
- search behavior
- checkpoint persistence
- scan warning persistence
- audit record persistence
- workflow job persistence

When adding new persisted concepts, this file is likely the first implementation point until the crate is split into smaller modules.

## `crates/vidlib-scanner`

### Purpose

Filesystem discovery and scan result assembly.

### Key file

#### `crates/vidlib-scanner/src/lib.rs`

Contains:

- `ScanOptions`
- `ScanResult`
- `CancellationToken`
- `scan_path`
- video file filtering
- progress callback invocation

This is the main file to update when scan behavior or traversal policy changes.

## `crates/vidlib-metadata`

### Purpose

Media metadata extraction.

### Main concern areas

- metadata provider trait(s)
- `ffprobe` integration
- fallback behavior
- parsing normalization

Use this crate for any change to how technical media metadata is produced.

## `crates/vidlib-duplicates`

### Purpose

Duplicate analysis and helper hashing behavior.

### Main concern areas

- exact hash grouping
- chunk hash grouping
- future perceptual similarity
- future best-version heuristics

## `crates/vidlib-fileops`

### Purpose

Safe filesystem planning and apply/undo behavior.

### Key file

#### `crates/vidlib-fileops/src/lib.rs`

Contains:

- `plan_by_extension`
- `apply_plan`
- `undo_from_manifest`
- `audit_records_for_manifest`

This is the most sensitive filesystem mutation area in the workspace.

## `crates/vidlib-ml`

### Purpose

Local ML abstractions and engines.

### Main concern areas

- heuristic label generation
- ONNX-backed inference evolution
- model integration boundary

## `crates/vidlib-workflows`

### Purpose

Cross-crate orchestration.

### Key files

#### `crates/vidlib-workflows/src/lib.rs`

Public export surface.

#### `crates/vidlib-workflows/src/scan.rs`

Current shared scan workflow.

Contains:

- `ScanWorkflowConfig`
- `ScanWorkflowOutcome`
- `run_scan_workflow`
- persisted scan job record lifecycle updates

Use this crate whenever business workflows start getting duplicated in CLI and Tauri.

## `crates/vidlib-cli`

### Purpose

Command-line interface.

### Key file

#### `crates/vidlib-cli/src/main.rs`

Contains:

- CLI argument definitions
- command dispatch
- workflow invocation
- workflow job history inspection
- output formatting

Keep this file thin. If command logic grows, move it into helper modules or shared workflow code.

## `src-tauri`

### Purpose

Desktop shell backend.

### Key files

#### `src-tauri/src/main.rs`

Contains:

- `AppState`
- Tauri commands
- dashboard/search/scan wiring
- progress event emission

#### `src-tauri/tauri.conf.json`

Desktop application configuration.

#### `src-tauri/build.rs`

Tauri/Rust build support.

## `ui/`

### Purpose

Frontend shell.

### Key files

#### `ui/index.html`

HTML shell entrypoint.

#### `ui/main.js`

Frontend behavior.

#### `ui/styles.css`

Styling.

## Where to make common changes

## Add a new shared domain field

Start in:

- `crates/vidlib-core/src/models.rs`

Then review:

- `crates/vidlib-db/src/lib.rs`
- workflow crates that build or consume that model
- UI/CLI surfaces that serialize or display it

## Add a new persisted concept

Start in:

- `crates/vidlib-core/src/models.rs` if it is shared
- `crates/vidlib-db/src/lib.rs` for schema and persistence

Then update:

- workflows
- tests
- docs in `docs/`

## Add a new workflow

Start in:

- `crates/vidlib-workflows`

Then integrate into:

- `crates/vidlib-cli`
- `src-tauri`

## Change scanning behavior

Start in:

- `crates/vidlib-scanner/src/lib.rs`

Then review:

- checkpoint persistence in `vidlib-db`
- orchestration in `vidlib-workflows`
- user-facing progress in CLI/Tauri

## Change restructure/apply/undo behavior

Start in:

- `crates/vidlib-fileops/src/lib.rs`

Then review:

- audit persistence in `vidlib-db`
- plan/apply commands in CLI
- UI preview flows in Tauri/frontend

## Change search behavior

Start in:

- `crates/vidlib-core/src/models.rs` for query contracts
- `crates/vidlib-db/src/lib.rs` for query execution

Then review:

- CLI args
- Tauri commands
- UI filter views

## Navigation tips for maintainers

- start with `docs/crate-contracts.md` for ownership rules
- use `docs/architecture.md` for system-level understanding
- use this file for repo navigation
- use `docs/development_progress.md` for current maturity and known gaps
- use `EXPANSION_PLAN.md` for long-term direction

## Editing guidance for maintainers

Update this file when:

- crates are added, removed, or split
- important files move
- new major documents are added to `docs/`
- the recommended implementation locations for common changes shift