# VidLibOrganizer Development Progress

## Purpose

This document tracks current project maturity, what is already implemented, what is partially implemented, and what is still missing. It is intended to help future maintainers quickly understand the real state of the project without reading the entire codebase first.

## Status summary

Current status: **MVP / early platform stage**

The repository already has a strong structural foundation, but it is still early in product maturity. It is best described as a promising local-first Rust workspace with working end-to-end flows, not yet a finished premium application.

## What is implemented today

### Workspace and architecture

- Cargo workspace organization is established
- crate boundaries are documented in `docs/crate-contracts.md`
- reusable workflow layer exists in `crates/vidlib-workflows`
- CLI and Tauri shell are both wired to shared functionality

### Core domain layer

- shared models exist for libraries, entries, duplicates, plans, audit records, and progress
- shared error model exists in `vidlib-core`

### Persistence

- SQLite local database is in place
- database configuration includes WAL mode and foreign keys
- schema bootstrapping exists
- persistence exists for libraries, entries, checkpoints, warnings, and audit records

### Scanning

- recursive scanning works
- common video extensions are filtered
- skip-extension support exists
- scan warnings are captured rather than aborting full runs
- progress callback support exists
- cancellation primitive exists in the scanner crate

### Metadata

- metadata extraction via `ffprobe` exists
- fallback behavior exists when `ffprobe` is unavailable
- basic metadata such as duration, width, height, and codec is supported

### Duplicates

- duplicate grouping support exists
- exact hash support exists
- chunk hash support exists
- placeholder perceptual hashing path exists

### File operations

- restructure planning exists
- apply flow exists
- undo manifest exists
- conflict checks exist
- audit helper generation exists

### ML path

- local heuristic tag generation path exists
- ONNX integration path is planned in the architecture and dependencies

### Desktop shell

- Tauri backend is present
- desktop commands exist for dashboard, add library, run scan, cancel scan, search, and plan generation
- basic event emission for scan progress exists

### CLI

- add library
- list libraries
- scan
- search
- duplicates
- plan move
- apply plan
- undo

### Testing

- unit/integration coverage exists for important current behaviors
- scanner tests exist
- database persistence/search tests exist
- duplicate tests exist
- file operation tests exist

## What is partially implemented

### Workflow orchestration

The project already has a workflow crate, which is a major strength, but the workflow runtime is still early.

Partially done:

- shared scan workflow exists
- scan orchestration is no longer fully duplicated in entrypoints
- scan jobs are now persisted as job records with state, progress, and failure snapshots
- CLI job history visibility is being added so workflow state is not desktop-only

Still missing:

- persistent job model
- proper cancellation ownership across the app
- resumable long-running background jobs beyond the current scan flow
- workflow lifecycle integration tests around cancellation and job inspection

### Search

Partially done:

- text/tag/extension filtering exists

Still missing:

- FTS-backed search
- faceting
- ranking
- saved searches
- collections
- browse-optimized query behavior

### Duplicate workflows

Partially done:

- grouping exists

Still missing:

- canonical resolution
- review states
- best-version heuristics
- side-by-side review workflow
- safe batch duplicate action UX

### Metadata depth

Partially done:

- basic technical extraction exists

Still missing:

- stream-level metadata
- richer audio/subtitle/container analysis
- thumbnail generation
- contact sheets
- preview cache management

### Desktop UX

Partially done:

- MVP shell exists
- dashboard and basic actions work

Still missing:

- scalable frontend architecture
- advanced browser UI
- settings center
- progress center
- polished recovery/error flows
- review and playback experiences

### ML

Partially done:

- abstraction path exists
- heuristic fallback exists

Still missing:

- model registry
- production-grade ONNX inference workflows
- confidence-aware output management
- batch review of suggestions

## What is not implemented yet

The following product areas are still largely future work:

- embedded playback
- hover previews and integrated browsing workflow depth
- rich tag taxonomy and rules engine
- smart collections
- review queues
- production packaging and updater strategy
- diagnostics bundles and repair tools
- plugin/extension model
- team-oriented or enterprise-oriented operational features

## Current technical strengths

- solid crate separation for an early-stage Rust workspace
- shared core models already exist
- reusable scan workflow already exists
- safe file operations were considered early instead of late
- local-first product philosophy is already reflected in the implementation

## Current technical weaknesses

- `vidlib-db` is still a relatively large single implementation file
- search path is not ready for large-scale daily use
- placeholder job/cancellation management in the desktop layer needs redesign
- preview and playback systems do not exist yet
- UI layer is still thin compared to long-term ambitions
- some roadmap ideas exist only in planning, not in code

## Suggested near-term focus

Recommended near-term implementation order:

1. persistent jobs and workflow hardening
2. indexed search and browse improvements
3. richer metadata and thumbnail pipeline
4. duplicate review workflow
5. taxonomy/rules-based organization

For the fuller product sequence, see `EXPANSION_PLAN.md`.

## How to update this document

Update this file when:

- a roadmap area moves from planned to partially implemented
- a partial feature becomes fully operational
- a significant technical debt item is removed
- a new major capability or subsystem appears

Recommended maintenance style:

- keep the document factual
- distinguish clearly between implemented, partial, and planned
- avoid promotional wording that overstates maturity