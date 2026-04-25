## VidLibOrganizer

VidLibOrganizer is a Windows-first, local-only Rust workspace for organizing, indexing, searching, tagging, and safely restructuring very large video libraries. The repository ships with a Rust CLI, a Tauri desktop shell, resumable SQLite-backed indexing, duplicate analysis, metadata extraction via `ffprobe`, and a staged ML abstraction for local face/object workflows.

## Architecture

Workspace crate boundaries are defined in `docs/crate-contracts.md`.

### Workspace crates

- `crates/vidlib-core` — shared domain models, progress, and structured workspace error types.
- `crates/vidlib-db` — SQLite schema versioning, migrations, transactional persistence, checkpoints, search.
- `crates/vidlib-scanner` — bounded-memory filesystem scanning, skip rules, resilient discovery, chunk/exact hashing integration.
- `crates/vidlib-metadata` — local `ffprobe` metadata provider with offline fallback.
- `crates/vidlib-duplicates` — layered duplicate helpers for exact and chunk hashing.
- `crates/vidlib-fileops` — safe restructure planning, conflict preview, apply, and undo manifests.
- `crates/vidlib-ml` — local ML abstraction with heuristic offline fallback and documented ONNX extension path.
- `crates/vidlib-cli` — `vidlib` command-line interface for power users and automation.
- `src-tauri` — Tauri desktop backend exposing the same core workflows to the GUI.
- `ui/` — lightweight Tauri frontend shell.

### Phase coverage in this MVP

- **Phase 1**: workspace, CLI, SQLite, scanner, metadata extraction.
- **Phase 2**: duplicate grouping, search, tagging hooks, checkpoint persistence.
- **Phase 3**: Tauri shell with dashboard, add library, scan, search, restructure preview.
- **Phase 4**: safe plan generation, conflict detection, apply, undo manifest.
- **Phase 5**: local ML plugin abstraction with working offline heuristic tagging fallback and explicit ONNX roadmap.

## Security and safety model

- Local-only by default. No telemetry, no cloud services, no network dependency for analysis.
- All media paths are treated as untrusted input.
- Restructure operations are preview-first, dry-run by default, require explicit confirmation, and emit undo manifests with audit trails.
- Scanning uses streaming file reads for hashing and avoids loading whole files into memory.
- Scanner tolerates inaccessible files and records warnings instead of aborting full runs.
- Metadata extraction uses `ffprobe` as a subprocess and never executes media-derived content.
- SQLite runs in WAL mode for safer concurrent access.

## Current capabilities

- Add and list library roots.
- Scan folders recursively for common video formats.
- Skip selected extensions during scans and retain scan summaries.
- Extract width, height, codec, and duration with local `ffprobe` when installed.
- Fall back gracefully when `ffprobe` is unavailable.
- Persist indexed entries in SQLite.
- Batch-upsert indexed entries transactionally.
- Compute chunk hashes by default and optional exact hashes.
- Generate local heuristic tags.
- Search by text, tag, and extension.
- Group duplicates by exact, chunk, and perceptual-hash placeholders with grouping metadata.
- Generate folder restructure plans by extension.
- Apply plans only when conflict-free, explicitly confirmed, and save undo manifests with audit logs.
- Use the Tauri GUI for dashboard, scan, search, and preview workflows.

## Planned next work

- Bind checkpoints to persistent library IDs instead of ad-hoc scan IDs.
- Add true incremental scan scheduler and cancellation tokens.
- Add FTS indexes and richer search ranking.
- Implement perceptual video hashing with sampled frame extraction.
- Add ONNX Runtime-backed local face/object recognition.
- Add duplicate review UI, tag editor UI, progress streams, settings, and packaged installers.

## Dependencies

- Rust stable toolchain
- `ffprobe` available on `PATH` for rich metadata extraction
- Windows WebView2 runtime for Tauri on Windows
- Tauri prerequisites from the official Tauri documentation

### Windows prerequisites

- Install Rust stable with the MSVC toolchain.
- Install WebView2 Runtime.
- Install Microsoft Visual Studio Build Tools with the Desktop C++ workload.
- Install `ffmpeg`/`ffprobe` and ensure `ffprobe.exe` is on `PATH`.
- For Tauri builds, ensure the Tauri Windows prerequisites are installed.

## Build and run

### CLI

- `cargo run -p vidlib-cli -- add-library C:\\Videos`
- `cargo run -p vidlib-cli -- scan C:\\Videos --exact-hash --skip-extension tmp`
- `cargo run -p vidlib-cli -- search --text holiday --tag family`
- `cargo run -p vidlib-cli -- duplicates --json`
- `cargo run -p vidlib-cli -- plan-move C:\\Videos\\Organized --output plan.json`
- `cargo run -p vidlib-cli -- apply-plan plan.json --manifest undo.json --confirm`
- `cargo run -p vidlib-cli -- undo undo.json`

### GUI

- `cargo tauri dev`
- `cargo tauri build`

The GUI frontend assets are in `ui/`. The Tauri backend is in `src-tauri/`.

## Validation

- `cargo fmt --all --check`
- `cargo test -p vidlib-core -p vidlib-db -p vidlib-metadata -p vidlib-duplicates -p vidlib-fileops -p vidlib-ml -p vidlib-scanner -p vidlib-cli`
- `cargo clippy -p vidlib-core -p vidlib-db -p vidlib-metadata -p vidlib-duplicates -p vidlib-fileops -p vidlib-ml -p vidlib-scanner -p vidlib-cli --all-targets -- -D warnings`
- `cargo check --workspace`

## Test strategy

Included tests currently cover:

- SQLite persistence and search behavior.
- SQLite update/upsert behavior.
- `ffprobe` JSON parsing.
- Duplicate grouping logic.
- Scanner filtering and skip behavior.
- Safe dry-run/apply/undo file operations.

## Limitations

- Rich metadata requires `ffprobe` installed locally.
- GUI is still an MVP shell and currently does not stream live scan progress.
- ML tagging is currently heuristic fallback until ONNX-backed models are integrated.
- Perceptual video hashing is reserved for a later phase.

## ONNX / local ML roadmap

- Add a model registry under `crates/vidlib-ml`.
- Integrate ONNX Runtime behind the existing trait abstraction.
- Support sampled-frame extraction via local `ffmpeg`.
- Add local face clustering and object label confidence thresholds.
- Keep all inference offline and opt-in.

## Open-source posture

All dependencies are open-source crates or local tools. The design is suitable for an open-source desktop application and keeps analysis on the local machine.
