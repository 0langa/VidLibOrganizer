# VidLibOrganizer implementation plan

This is the historical implementation plan for the duplicate-finder baseline.
The canonical plan for the approved Rename and Blueprint work is
[Rename and Blueprint Foundation](superpowers/plans/2026-10-17-rename-and-blueprint.md).

**Goal:** Deliver an offline Windows app for fast size-based and full-file MD5 duplicate review.
**Architecture:** Pure Rust engine behind Tauri commands; plain ES modules for the interface.
**Spec:** [spec.md](spec.md)

## Progress
- [100%] Inspect legacy workspace and establish the new app contract.
- [100%] Implement and test isolated scan engine, then connect background desktop jobs.
- [100%] Implement library management, scan setup, progress and duplicate review.
- [100%] Run engine/integration tests, build desktop executable, and check interface behavior.

## Work units
- Engine: new `crates/vidlib-engine/{Cargo.toml,src/lib.rs,tests/scanning.rs}`. Tests first with real fixtures for size collisions versus equal MD5, recursion, overlap, unsupported and zero-byte files, cancellation, invalid roots and change detection. `scan(roots, mode, cancel, progress) -> Result<ScanResult,String>`.
- Desktop: replace `src-tauri/src/main.rs`; native `pick_folder`, saved `load_libraries` / `save_libraries`, `start_scan` / `scan_status` / `cancel_scan`, `reveal_file`, `export_report`. Worker status is polled and completed results retrieved once. Only one running scan. Open-folder requests validated against scanned files. JSON report uses native save dialog.
- UI: rewrite `ui/index.html`, `ui/styles.css`, and focused ES modules. Library and duplicates navigation, persistent saved roots, radio scan modes, pagination, filtering/sorting, error and cancellation states. Interface designed with usable empty state; sample data only behind explicit preview controls.
- File actions (user clarification): `crates/vidlib-fileops` validates selection against the current scan and keeps at least one copy, checks freshness, performs permanent deletion or verified cross-drive moves without overwriting. UI requires explicit selection and a detailed final confirmation. `apply_actions({id,paths,action,destination}) -> {completed,issues}` runs off the UI thread. Tests exercise missing/changed keepers, stale copies, unknown paths and destination collisions.
- Retire obsolete crates, docs and tests from the active workspace; document build and launch commands in README.

## Verification
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build -p vidlib-tauri`.
- Browser UI interaction checks and desktop launch smoke check. Report limitations honestly if platform automation is unavailable.
- Exercise Unicode paths, missing drives, nested roots, same-sized different content, a file changed during hashing, and cancellation during a large read.
