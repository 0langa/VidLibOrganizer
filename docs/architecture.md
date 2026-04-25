# VidLibOrganizer Architecture

## Purpose

This document describes the current and intended architecture of `VidLibOrganizer`. It is written as a long-term reference for maintainers and should be updated whenever crate responsibilities, runtime flow, or persistent data boundaries materially change.

## Architectural goals

- keep core business logic in reusable Rust crates
- keep application entrypoints thin
- preserve a local-first and offline-first product model
- isolate filesystem, database, subprocess, and UI concerns
- support future evolution into a premium desktop application without rewriting the entire workspace

## High-level system model

`VidLibOrganizer` is a Cargo workspace built around a layered architecture:

1. **Domain layer**
   - shared types and error contracts
   - owned by `crates/vidlib-core`

2. **Feature/service layer**
   - persistence, scanning, metadata extraction, duplicate detection, file operations, and ML
   - owned by feature crates such as `vidlib-db`, `vidlib-scanner`, `vidlib-metadata`, `vidlib-duplicates`, `vidlib-fileops`, and `vidlib-ml`

3. **Workflow/orchestration layer**
   - composes feature crates into user-facing operations
   - owned by `crates/vidlib-workflows`

4. **Application layer**
   - CLI and desktop entrypoints
   - owned by `crates/vidlib-cli` and `src-tauri`

5. **Presentation layer**
   - lightweight desktop frontend assets
   - owned by `ui/`

## Dependency direction

The intended dependency flow is:

`vidlib-core` <- feature crates <- `vidlib-workflows` <- application entrypoints

Expanded form:

`vidlib-core` <- (`vidlib-db`, `vidlib-scanner`, `vidlib-metadata`, `vidlib-duplicates`, `vidlib-fileops`, `vidlib-ml`) <- `vidlib-workflows` <- (`vidlib-cli`, `src-tauri`)

### Key rule

`vidlib-core` must not depend on any other workspace crate.

## Workspace architecture by crate

### `crates/vidlib-core`

Owns the shared domain contracts of the workspace.

Current examples:

- `LibraryFolder`
- `VideoEntry`
- `ScanCheckpoint`
- `DuplicateGroup`
- `SearchQuery`
- `ScanSummary`
- `ScanWarningRecord`
- `RestructurePlan`
- `UndoManifest`
- `AuditRecord`
- `ProgressSnapshot`
- `VidLibError`

This crate should remain free of:

- direct database access
- filesystem traversal logic
- CLI parsing
- Tauri state and commands
- subprocess execution details

### `crates/vidlib-db`

Owns SQLite storage and persistence concerns.

Current responsibilities:

- database open/configuration
- schema bootstrapping and migration entrypoints
- persistence for libraries, entries, checkpoints, warnings, and audit records
- persistence for workflow job records
- query APIs such as search and list operations

Architectural note:

Today, `vidlib-db` contains both persistence and some behavior that will likely need to become more structured over time, especially around migrations, advanced indexes, FTS, saved searches, collections, and future review state.

### `crates/vidlib-scanner`

Owns filesystem traversal and scan result assembly.

Current responsibilities:

- recursive walking
- extension filtering
- skip rule application
- file metadata gathering from the OS
- exact/chunk/perceptual hash hooks
- warning collection
- progress emission
- cancellation token primitive

Architectural note:

The scanner should remain focused on discovery and per-file analysis assembly, not end-user workflow orchestration.

### `crates/vidlib-metadata`

Owns media metadata extraction contracts and provider implementations.

Current responsibilities:

- `ffprobe` integration
- graceful fallback behavior when metadata tooling is unavailable

Intended future scope:

- richer stream-level metadata
- safer normalization of technical media details
- multiple metadata provider strategies if needed

### `crates/vidlib-duplicates`

Owns duplicate grouping and related hashing helpers.

Current responsibilities:

- exact hash grouping
- chunk hash grouping
- placeholder perceptual paths

Intended future scope:

- similarity scoring
- best-version heuristics
- canonical recommendation support
- richer duplicate relationship modeling

### `crates/vidlib-fileops`

Owns destructive and semi-destructive filesystem operations behind safe planning layers.

Current responsibilities:

- restructure planning by extension
- conflict detection
- apply plan
- undo via manifest
- audit record generation helpers

Architectural note:

This crate is part of the product’s trust boundary. It should remain conservative, preview-first, and heavily tested.

### `crates/vidlib-ml`

Owns local ML abstractions and inference engines.

Current responsibilities:

- heuristic local fallback behavior
- ONNX integration path

Intended future scope:

- model registry
- confidence-aware outputs
- optional hardware-accelerated local inference

### `crates/vidlib-workflows`

Owns orchestration across feature crates.

Current responsibilities:

- scan workflow composition
- database coordination
- metadata enrichment coordination
- ML tag enrichment in scan workflow
- persisted scan job creation and job progress updates

Intended future scope:

- persistent job runtime
- background maintenance workflows
- preview generation workflows
- duplicate review workflows
- rules engine orchestration

### `crates/vidlib-cli`

Owns terminal-facing workflow composition.

Current responsibilities:

- command parsing with `clap`
- wiring CLI commands to shared workspace workflows
- JSON/plain output formatting

Architectural rule:

The CLI should remain an interface layer, not a business-logic layer.

### `src-tauri`

Owns desktop backend composition and Tauri command exposure.

Current responsibilities:

- desktop command handlers
- application state container
- workflow invocation
- event emission to the frontend
- job history listing via persisted workflow state

Architectural note:

The current Tauri state still contains placeholder job management and should evolve toward a real workflow runtime integration.

### `ui/`

Owns the frontend shell.

Current responsibilities:

- dashboard and lightweight desktop interactions

Intended future scope:

- rich browser UI
- advanced search/filter UX
- duplicate review views
- playback and inspection UX
- settings and operations center

## Current runtime flow

## Scan flow

Current scan behavior roughly follows this path:

1. user invokes scan through CLI or Tauri
2. entrypoint calls `vidlib_workflows::run_scan_workflow`
3. workflow resolves or creates a library record
4. workflow invokes `vidlib_scanner::scan_path`
5. scanner walks filesystem and emits `ProgressSnapshot`
6. metadata is collected via `vidlib-metadata`
7. hashing is collected via duplicate helpers
8. workflow enriches tags via ML abstraction
9. workflow persists entries, warnings, checkpoints, and audit records through `vidlib-db`
10. result is returned to the entrypoint

## Search flow

Current search behavior roughly follows this path:

1. user submits query from CLI or desktop UI
2. entrypoint builds a `SearchQuery`
3. `vidlib-db` loads candidate rows
4. filtering is currently performed in Rust rather than by an indexed FTS-driven query engine

This is functional but will need redesign for scale.

## File operation flow

Current restructure behavior roughly follows this path:

1. entries are loaded from the database
2. `vidlib-fileops` builds a `RestructurePlan`
3. plan is reviewed or serialized
4. apply requires explicit confirmation
5. operations execute filesystem renames
6. undo manifest is written
7. audit records can be persisted

## Architectural strengths

- clear workspace split between shared models and feature crates
- existing workflow layer already prevents logic duplication from getting worse
- local-first and safe-by-default product philosophy is visible in the codebase
- Rust is a strong fit for reliability, filesystem-heavy operations, and long-term desktop tooling

## Architectural weaknesses and current debt

- Tauri job cancellation is still placeholder-level even though scan job history is now persisted
- search is not yet database-native enough for large libraries
- current perceptual hash path is not yet production-grade for video similarity
- preview and playback subsystems do not yet exist
- schema lifecycle is still early and will need stronger migration discipline
- UI architecture is still a lightweight shell rather than a scalable frontend system

## Trust boundaries

The following inputs should be treated as untrusted:

- all filesystem paths
- file names and directory names
- metadata returned by external tools
- media-derived content
- user-provided plan/config JSON

Special caution areas:

- file moves and deletes
- subprocess integration with `ffprobe`/future `ffmpeg`
- database migrations
- future ML model loading

## Persistence architecture

The system currently uses SQLite as the local system of record.

Current persisted concepts include:

- library folders
- video entries
- scan checkpoints
- scan warnings
- audit records

SQLite is configured with:

- WAL mode
- foreign keys on
- `NORMAL` synchronous mode

This is a good fit for a local-first desktop application, but future growth will require:

- structured migrations
- richer indexing
- FTS
- history tables for workflows and reviews
- settings/config persistence

## Architectural evolution strategy

Near-term architecture should prioritize:

1. persistent jobs and workflow runtime
2. indexed search and browse model
3. richer metadata and preview pipeline
4. duplicate review and taxonomy systems
5. playback and premium desktop UX

These match the roadmap in `EXPANSION_PLAN.md`.

## Editing guidance for maintainers

When updating this document:

- update crate responsibilities when ownership changes
- update runtime flows when orchestration changes
- update trust boundaries when new subprocesses, plugins, or network features are added
- update persistence notes when schema expands significantly
- prefer stable architectural statements over short-lived implementation trivia