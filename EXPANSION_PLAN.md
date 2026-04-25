# VidLibOrganizer Expansion Plan

## Goal

Expand VidLibOrganizer from a functional MVP into a reliable, scalable, local-first video library management application with strong workflow orchestration, safe file operations, rich discovery, and practical duplicate review.

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

## Recommended implementation order

1. Real scan job and cancellation architecture
2. FTS search and richer filters
3. Duplicate review workflow
4. Safer restructure engine and UI
5. Rich metadata and ONNX tagging
6. `vidlib-workflows` orchestration refactor

## Suggested next milestone

Create a new `vidlib-workflows` crate and move scan orchestration out of `vidlib-cli` and `src-tauri` so both entrypoints share one workflow implementation.
