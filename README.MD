# VidLibOrganizer

VidLibOrganizer is a Windows-first, local-first Rust workspace for organizing, indexing, searching, analyzing, and safely restructuring large video libraries.

The repository already includes:

- a Rust CLI for power users and automation
- a Tauri desktop shell
- SQLite-backed local persistence
- filesystem scanning and indexing
- `ffprobe`-based metadata extraction
- duplicate grouping support
- safe restructure planning, apply, and undo flows
- an early local ML abstraction for future premium analysis features

The long-term goal is a premium all-in-one desktop application for video library organization, labeling, review, analysis, browsing, playback, and safe operational workflows.

## Project status

Current maturity: **MVP / early platform stage**

This project already has a strong architecture foundation, but it is still early in product maturity. Core workflows exist, yet many advanced product features described in `EXPANSION_PLAN.md` are still planned rather than finished.

## Core characteristics

- local-first and offline-first
- Windows-first desktop focus
- safe-by-default file operations
- shared Rust crate architecture
- designed to grow into a premium video library management platform

## Current capabilities

### Library management

- add and list library roots
- persist library information in SQLite

### Scanning and indexing

- recursively scan common video formats
- skip selected extensions
- collect warnings without aborting the entire scan
- persist scan checkpoints and summaries

### Metadata extraction

- extract width, height, codec, and duration via local `ffprobe`
- fall back gracefully when `ffprobe` is unavailable

### Duplicate analysis

- exact-hash grouping
- chunk-hash grouping
- placeholder perceptual-hash path for future evolution

### Search

- search by text
- search by tag
- search by extension

### File operations

- generate restructure plans
- preview conflict state
- apply plans with explicit confirmation
- create undo manifests
- undo from manifests

### Desktop shell

- dashboard summary
- add library
- run scan
- persisted scan job history
- search
- plan generation preview

### CLI

- add library
- list libraries
- list jobs
- scan
- search
- duplicates
- plan move
- apply plan
- undo

## What is not finished yet

Important planned but not yet complete areas include:

- true cancellation ownership in the desktop app
- full persistent multi-job runtime beyond the current scan job record foundation
- FTS-backed search and browse UX
- thumbnails and preview pipeline
- duplicate review UI and canonical resolution workflow
- structured tag taxonomy and rules engine
- integrated playback
- richer local ML analysis
- premium UX polish, diagnostics, packaging, and updater flows

## Workspace structure

### Main crates and folders

- `crates/vidlib-core`
	- shared domain models, error types, progress types
- `crates/vidlib-db`
	- SQLite persistence, schema bootstrapping, queries
- `crates/vidlib-scanner`
	- filesystem scanning and scan result assembly
- `crates/vidlib-metadata`
	- local media metadata extraction
- `crates/vidlib-duplicates`
	- duplicate analysis helpers
- `crates/vidlib-fileops`
	- safe filesystem planning, apply, and undo
- `crates/vidlib-ml`
	- local ML abstraction and future inference path
- `crates/vidlib-workflows`
	- shared business workflow orchestration
- `crates/vidlib-cli`
	- command-line interface
- `src-tauri`
	- Tauri backend and desktop app wiring
- `ui/`
	- frontend shell assets
- `docs/`
	- long-term project documentation

For authoritative crate ownership rules, see `docs/crate-contracts.md`.

## Documentation index

The `docs/` folder is intended to be the long-term reference set for the repository.

- `docs/architecture.md`
	- system architecture, layering, trust boundaries, and runtime flow
- `docs/codemap.md`
	- where code lives and where changes should be made
- `docs/development_progress.md`
	- current maturity, implemented areas, partial areas, and gaps
- `docs/full_project_documentation.md`
	- broad project reference document
- `docs/crate-contracts.md`
	- crate ownership and dependency rules

For long-term roadmap planning, see `EXPANSION_PLAN.md`.

## Architecture summary

The workspace follows a layered model:

1. `vidlib-core`
	 - shared domain contracts
2. feature crates
	 - database, scanner, metadata, duplicates, fileops, ML
3. `vidlib-workflows`
	 - shared orchestration layer
4. application entrypoints
	 - CLI and Tauri backend
5. frontend shell
	 - `ui/`

Intended dependency direction:

`vidlib-core` <- feature crates <- `vidlib-workflows` <- application entrypoints

## Security and safety model

- local-only by default
- no required cloud services
- no hidden telemetry in the current design
- media paths are treated as untrusted input
- metadata extraction is subprocess-based and should remain defensive
- restructure operations are preview-first and confirmation-gated
- undo manifests and audit records are part of the product safety model
- SQLite runs in WAL mode for safer local access patterns

## Requirements and prerequisites

### Runtime and tooling

- Rust stable toolchain
- Windows environment
- `ffprobe` on `PATH` for rich metadata extraction
- WebView2 runtime for Tauri on Windows
- Tauri prerequisites for desktop builds

### Windows prerequisites

- install Rust stable with the MSVC toolchain
- install WebView2 Runtime
- install Visual Studio Build Tools with Desktop C++ workload
- install `ffmpeg` / `ffprobe` and ensure `ffprobe.exe` is on `PATH`
- install Tauri prerequisites required for Windows builds

## Build and run

## CLI examples

- `cargo run -p vidlib-cli -- add-library C:\\Videos`
- `cargo run -p vidlib-cli -- list-libraries`
- `cargo run -p vidlib-cli -- list-jobs`
- `cargo run -p vidlib-cli -- scan C:\\Videos --exact-hash --skip-extension tmp`
- `cargo run -p vidlib-cli -- search --text holiday --tag family`
- `cargo run -p vidlib-cli -- duplicates --json`
- `cargo run -p vidlib-cli -- plan-move C:\\Videos\\Organized --output plan.json`
- `cargo run -p vidlib-cli -- apply-plan plan.json --manifest undo.json --confirm`
- `cargo run -p vidlib-cli -- undo undo.json`

## Desktop app

- `cargo tauri dev`
- `cargo tauri build`

Frontend assets live in `ui/`. The desktop backend lives in `src-tauri/`.

## Validation

Recommended workspace-level validation:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Testing coverage today

The repository already includes tests for important current behavior, including:

- SQLite persistence and search behavior
- batch update/upsert behavior
- `ffprobe` JSON parsing
- duplicate grouping logic
- scanner filtering and skip behavior
- safe plan/apply/undo file workflows

## Development priorities

The recommended near-term priorities are:

1. persistent workflow runtime and job model
2. indexed search and browse improvements
3. richer metadata and thumbnail pipeline
4. duplicate review workflow
5. taxonomy and rules-based organization

For the full long-range sequencing, see `EXPANSION_PLAN.md`.

## Contribution guidance

When making changes:

- keep business logic in reusable crates
- keep `vidlib-core` free of runtime-specific concerns
- keep CLI and Tauri entrypoints thin
- prefer updating shared models before duplicating types in feature crates
- preserve preview-first and rollback-aware file operation behavior
- update `docs/` when architecture, ownership, or project maturity materially changes

## Known limitations

- rich metadata currently depends on `ffprobe`
- the GUI is still an MVP shell
- desktop cancellation is not yet a full persistent job system
- search is still basic for large-library use cases
- playback is not implemented yet
- ML is still early and mostly future-facing

## License and open-source posture

The workspace uses open-source crates and local tools. The project is suitable for open-source development and is intentionally designed to keep user media analysis on the local machine.
