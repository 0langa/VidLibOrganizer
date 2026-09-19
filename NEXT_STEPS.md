# VidLibOrganizer — Status & Roadmap
_Portfolio audit: 2026-07-11_

## What this is
A Windows-first, local-first Rust workspace for organizing large video libraries:
scanning/indexing into SQLite, ffprobe metadata extraction, duplicate grouping,
search, and safe restructure plans with undo manifests. Nine crates (`vidlib-core`,
`-db`, `-scanner`, `-metadata`, `-duplicates`, `-fileops`, `-ml`, `-workflows`,
`-cli`) plus a Tauri desktop shell (`src-tauri`) with a small vanilla-JS UI. The
stated long-term ambition (README, `EXPANSION_PLAN.md` — 2,182 lines) is a premium
all-in-one video library management product.

## Current state
Five commits, all on a single day (2026-04-25), no tags, clean tree, untouched for
~2.5 months. The architecture documentation is extensive and honest — the docs
themselves label the project "MVP / early platform stage". The code is real but thin:
most crates are a single `lib.rs`, the CLI is 253 lines, the Tauri main is 147 lines.
Roughly 8 unit tests exist across `vidlib-db`, `-duplicates`, `-fileops`, `-metadata`.

Findings:

- `tests/workflow_integration.rs` at the workspace root is **orphaned**: the root
  `Cargo.toml` is a virtual workspace with no `[package]`, and no crate declares it,
  so `cargo test` never compiles or runs it. It should be moved into a crate (e.g.
  `vidlib-workflows/tests/`).
- No CI, no tags, no releases.
- Docs-to-code ratio is inverted for this stage: the expansion plan describes an
  enterprise product while the UI is a single `index.html`/`main.js`.
- No TODO/FIXME markers; what exists is clean.

## Definition of "finished"
The premium-product vision is not a realistic near-term finish line for a solo side
project. A defensible "finished v0.1" is: **the CLI as a genuinely usable tool** —
scan, search, duplicates, plan/apply/undo verified against a real library, the
orphaned integration test wired in and passing, CI running `cargo test` + `clippy`,
and a tagged `v0.1.0`. The Tauri shell and EXPANSION_PLAN phases are explicitly
"later"; alternatively, if motivation has moved on, park it with a status note in the
README — the architecture docs already make it a respectable reference project.

## Roadmap

### Phase 1 — Now (next 1-2 weeks)
1. Decide: continue as CLI-first tool, or park. (The rest assumes continue.)
2. Move `tests/workflow_integration.rs` into `crates/vidlib-workflows/tests/` and
   confirm `cargo test --workspace` passes.
3. Add a minimal GitHub Actions workflow: `cargo build`, `cargo test`, `cargo clippy`.
4. Run the CLI end-to-end against a real video folder; record rough edges.

### Phase 2 — Next (2-6 weeks)
1. Fix the rough edges from the dogfood run; make `scan` → `duplicates` → `plan` →
   `apply` → `undo` trustworthy on thousands of files.
2. Trim the README "not finished yet" list to match reality and tag `v0.1.0`.
3. Only then return to the Tauri shell: job progress, cancellation, and search UX.

### Phase 3 — Later (optional/stretch)
- FTS-backed search, thumbnail pipeline, duplicate review UI (EXPANSION_PLAN Phase 1
  items).
- Perceptual hashing to replace the placeholder path in `vidlib-duplicates`.
- ONNX-based analysis in `vidlib-ml` (currently an abstraction stub).

## Effort to "finished"
**M** — the crate skeleton and persistence exist, but reaching a trustworthy CLI v0.1
needs real dogfooding, test wiring, CI, and hardening; the full desktop vision would
be L+ and should not gate "finished".
