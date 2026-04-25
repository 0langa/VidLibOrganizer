# VidLibOrganizer crate contracts

This document is the source of truth for workspace crate boundaries and shared type ownership.

## Dependency direction

- `vidlib-core` must not depend on other workspace crates.
- Feature crates may depend on `vidlib-core`.
- Application entrypoints (`vidlib-cli`, `src-tauri`) may compose feature crates.
- Feature crates should not redefine shared domain types that already belong in `vidlib-core`.

Allowed high-level flow:

`vidlib-core` <- (`vidlib-db`, `vidlib-scanner`, `vidlib-metadata`, `vidlib-duplicates`, `vidlib-fileops`, `vidlib-ml`) <- (`vidlib-cli`, `src-tauri`)

## Crate responsibilities

### `vidlib-core`

Owns shared domain contracts used across the workspace:

- library definitions
- indexed video records
- duplicate grouping models
- search/query models
- restructure plan/audit models
- scan progress and warning records
- shared error types

Must stay free of IO, database, CLI, Tauri, and subprocess orchestration logic.

### `vidlib-db`

Owns SQLite access, schema lifecycle, indexes, migrations, and persistence/query implementations for `vidlib-core` models.

### `vidlib-scanner`

Owns filesystem traversal, filtering, hashing orchestration, cancellation primitives, and scan result assembly.

### `vidlib-metadata`

Owns media metadata extraction contracts and provider implementations such as `ffprobe` integration.

### `vidlib-duplicates`

Owns hashing helpers and duplicate grouping logic over `vidlib-core::VideoEntry` values.

### `vidlib-fileops`

Owns safe file move planning, apply/undo behavior, and filesystem-side conflict handling.

### `vidlib-ml`

Owns local ML tagging abstractions and inference engines.

### `vidlib-cli`

Owns command parsing, terminal UX, and composition of feature crates into user-facing workflows.

### `src-tauri`

Owns desktop application wiring, Tauri commands, and GUI-facing orchestration.

## Shared type ownership

The following types belong in `vidlib-core` and should be reused instead of redefined elsewhere:

- `LibraryFolder`
- `VideoEntry`
- `ScanCheckpoint`
- `ScanSummary`
- `ScanWarningRecord`
- `DuplicateGroup`
- `DuplicateStrategy`
- `SearchQuery`
- `RestructurePlan`
- `RestructureAction`
- `UndoManifest`
- `AuditRecord`
- `ProgressSnapshot`
- `VidLibError`

The following kinds of types should stay out of `vidlib-core` unless they become true shared contracts:

- scanner runtime control tokens
- database connection wrappers
- metadata subprocess details
- CLI argument structs
- Tauri state containers

## Enforcement rules

- When a new feature needs a shared model, add it to `vidlib-core` first.
- When logic is runtime-specific, keep it in the owning crate instead of `vidlib-core`.
- Prefer converting lower-level errors into `vidlib_core::VidLibError` at crate boundaries exposed to other workspace crates.
- Keep `main.rs` files thin and move reusable workflow logic into library crates when it grows.