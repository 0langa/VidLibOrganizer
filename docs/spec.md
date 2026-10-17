# VidLibOrganizer — first release

This document describes the implemented duplicate-finder baseline. The proposed
Rename feature and separate Blueprint foundation are specified in the
[2026-10-17 library-tools design](superpowers/specs/2026-10-17-library-tools-design.md).
Those specifications are approved designs. Their implementation is tracked in the
[canonical plan](superpowers/plans/2026-10-17-rename-and-blueprint.md); they do not
describe shipped functionality.

## Purpose
Replace the deprecated prototype with a Windows desktop organizer for roughly 1–3 TB of videos on local and external drives. The first useful workflow is finding duplicate video files across folders and their subfolders. Library management and duplicate review are separate navigation surfaces so future organization tools can be added without rebuilding the shell.

## Product
- Local, offline Tauri desktop app, Rust scan engine, dependency-free HTML/CSS/JavaScript interface.
- Save multiple library folder paths. Choose included folders for each scan. Native folder picker and manual path entry.
- Fast mode groups nonempty video files by exact byte length. Results explicitly say “Size match” and remain candidates, not verified duplicates.
- MD5 mode first groups by size, then streams complete contents of candidate files through MD5. Unique sizes cannot have identical contents and are not hashed. No sampling or perceptual matching.
- Results show each full path, exact bytes, duplicate group count and potential redundant bytes (keeping one file per group), search, sort, pagination, and opening the containing folder.
- Review actions: Keep all (no file changes), move selected copies to a chosen folder, or permanently delete selected copies. No preselected files; each group must retain a copy. Deletion requires an explicit confirmation listing paths, file count and bytes; size-only warnings remain visible.
- Responsive charcoal interface with restrained green accent, persistent sidebar, clear empty states, accessible controls and a real progress view.

## Engine and safety
- Recursive traversal includes hidden video files, recognizes common extensions case-insensitively, skips empty files and symbolic links/junctions, and avoids counting overlapping roots twice.
- Filesystem metadata only in fast mode; one bounded read buffer and sequential hashing in MD5 mode to avoid thrashing external drives.
- Background scan leaves UI responsive; cancellation is checked during enumeration and between hash chunks. Progress is throttled and contains discovery counts, hashed bytes, totals and current path.
- Unreadable/offline files and directories are reported as issues; one failure does not discard the remainder. Missing roots never appear as a successful empty scan without warnings.
- Detect changed size or modification time while hashing and exclude that file with an issue. No stale hash cache in this version.
- Partial scans are visibly marked cancelled. Results are based on files actually examined. No automatic mutations of media. Actions revalidate selected files and a retained copy against scan metadata, and MD5 when available. Moves never overwrite and support cross-drive bounded copying with verification before removing the source.
- Saved paths are local preferences; scan results are in memory and can be exported to JSON. No network service, metadata provider or ML dependencies.

## Structure and checks
`crates/vidlib-engine` owns traversal, grouping, hashing, cancellation, and serializable records. `src-tauri` owns job state, native dialogs, persistence and shell integration. `ui` owns navigation, forms, progress and results. Tests use disposable real directories with identical contents, size collisions, nesting, overlaps, invalid roots, empty files, changing files and cancellation. Build and lint the Rust workspace; exercise UI states and actual desktop launch.

## Deferred
Metadata enrichment, transcoding, renaming, moving, perceptual similarity, thumbnails and persistent file catalogues. MD5 is intended for accidental duplicates, not adversarial content authentication. Hard links require physical-file identity handling before treating their bytes as reclaimable disk space.
