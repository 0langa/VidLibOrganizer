# VidLibOrganizer Full Project Documentation

## Purpose

This document is a broad project reference intended to help future maintainers, contributors, and planners understand what the project is, how it is organized, how it works today, and how it is expected to evolve.

It is intentionally more comprehensive than the `README.MD`, but less strategic than `EXPANSION_PLAN.md`.

## Project overview

`VidLibOrganizer` is a Windows-first, local-first Rust workspace for managing large video libraries. The long-term product direction is a premium desktop application for video organization, labeling, analysis, browsing, review, playback, and safe batch operations.

Today, the repository already contains:

- a Rust CLI
- a Tauri desktop backend
- a lightweight frontend shell
- SQLite-backed local persistence
- filesystem scanning
- metadata extraction via `ffprobe`
- duplicate grouping
- safe restructure planning/apply/undo behavior
- a staged local ML abstraction

## Product philosophy

The project is built around several durable principles.

### Local-first

- no required cloud dependency
- no need to upload media for analysis
- local ownership of data, metadata, and previews

### Safety-first

- file operations should be preview-first
- destructive behavior should require explicit confirmation
- rollback and audit support should exist where practical

### Shared logic first

- business logic belongs in reusable crates
- application entrypoints should stay thin
- the same workflows should be reusable across CLI and desktop app

### Long-term maintainability

- crate boundaries matter
- documentation should outlive short implementation details
- new features should strengthen the architecture rather than bypass it

## Current architecture summary

The workspace is divided into:

- `vidlib-core` for shared models and errors
- `vidlib-db` for SQLite persistence
- `vidlib-scanner` for discovery and scan assembly
- `vidlib-metadata` for media metadata extraction
- `vidlib-duplicates` for duplicate analysis
- `vidlib-fileops` for safe filesystem operations
- `vidlib-ml` for local ML abstractions
- `vidlib-workflows` for shared orchestration
- `vidlib-cli` for command-line usage
- `src-tauri` + `ui/` for the desktop application

For the authoritative architecture rules, see:

- `docs/crate-contracts.md`
- `docs/architecture.md`

## Core domain concepts

The current codebase already models several important concepts.

### Library

A library is a registered root folder that can be scanned and indexed.

### Video entry

A video entry is the indexed representation of a file, currently including path, name, extension, size, modification time, some technical metadata, hashes, and tags.

### Scan checkpoint

A checkpoint records partial scan state and provides the basis for future resumable workflows.

### Duplicate group

A duplicate group represents a set of entries that appear to match by some strategy, such as exact hash or chunk hash.

### Restructure plan

A restructure plan models safe filesystem changes before they are applied.

### Undo manifest

An undo manifest stores the reverse actions needed to restore previously applied changes.

### Audit record

An audit record captures important operational events for later inspection.

### Progress snapshot

A progress snapshot is the current shared representation for workflow progress updates.

## Current user-facing workflows

## CLI workflows

Current CLI capabilities include:

- add library
- list libraries
- scan
- search
- duplicates
- plan move
- apply plan
- undo

The CLI is suitable for power users, scripting, and future automation.

## Desktop workflows

Current desktop shell capabilities include:

- dashboard summary
- add library
- run scan
- cancel scan placeholder path
- search
- generate plan preview

The desktop shell is functional but still early compared to the roadmap.

## Database and persistence

SQLite is the local persistence layer.

Current persisted areas:

- library folders
- video entries
- scan checkpoints
- scan warnings
- audit records
- workflow job records

Current database configuration includes:

- WAL journal mode
- foreign key enforcement
- normal synchronous mode

This is appropriate for an offline desktop application and gives a good foundation for future indexed search and job history.

## Scanning and ingestion behavior

Current scan flow:

1. choose a root path
2. resolve or create a library record
3. walk the filesystem for known video extensions
4. collect OS file metadata
5. attempt media metadata extraction through `ffprobe`
6. compute hashes based on configured options
7. create `VideoEntry` values
8. optionally enrich tags through ML abstraction
9. persist entries and scan artifacts in SQLite

Current scan characteristics:

- warning-tolerant
- local-only
- bounded around filesystem iteration rather than loading all files into memory at once
- now records persisted scan job state and progress snapshots
- not yet a full cancellable persistent background job runtime

## Search behavior

Current search is functional but limited.

What it does now:

- query by text
- query by extension
- query by tag

What it does not yet do well:

- FTS-backed search
- ranking
- faceting
- very large library browse optimization
- saved searches and collections

## Duplicate behavior

Current duplicate support exists primarily as grouping.

What it does now:

- exact hash grouping
- chunk hash grouping
- placeholder perceptual path support

What it does not yet do:

- canonical keep recommendation
- duplicate review state tracking
- compare workflow
- integrated safe duplicate resolution UI

## File operation behavior

Current restructure support is one of the more important trust-sensitive areas.

What it does now:

- generate a plan by extension
- detect obvious destination conflicts
- require explicit confirmation before apply
- write undo manifest
- support undo by manifest

What it still needs:

- richer naming and placement rules
- stronger preflight checks
- more advanced collision policies
- richer recovery verification

## ML and analysis behavior

Current ML support is intentionally early.

What it does now:

- local heuristic labeling path
- abstraction boundary for future ONNX-backed inference

What it still needs:

- model registry
- confidence-aware outputs
- reviewable suggestion workflows
- local batch inference pipelines

## Testing and validation

The repository already contains meaningful tests, especially for current core behavior.

Current tested areas include:

- scanner behavior
- persistence behavior
- duplicate grouping
- file plan/apply/undo behavior
- metadata parsing

Recommended validation commands at the workspace root:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Known limitations

Current limitations include:

- rich metadata depends on `ffprobe`
- GUI is still MVP-level
- live job runtime is not fully implemented
- search is not yet optimized for very large libraries
- playback does not yet exist
- ML remains early and optional

## Documentation guide

Use the documents in `docs/` as follows:

- `architecture.md`
  - system structure and design boundaries
- `codemap.md`
  - repo navigation and where changes belong
- `development_progress.md`
  - current maturity and reality check
- `crate-contracts.md`
  - source of truth for crate ownership rules
- `full_project_documentation.md`
  - broad project reference

## Future direction

The long-term direction is documented in `EXPANSION_PLAN.md`.

In short, the project is expected to grow toward:

- persistent workflow runtime
- richer search and browse UX
- deeper metadata and thumbnail pipeline
- duplicate review and curation
- tag taxonomy and rules engine
- integrated playback
- local ML analysis
- premium product polish and supportability

## Maintenance guidance

When updating this document:

- keep it accurate to the current codebase
- avoid duplicating volatile implementation details better kept in source comments
- update cross-links when docs are added or renamed
- treat this file as the broad reference document, not the strategic roadmap