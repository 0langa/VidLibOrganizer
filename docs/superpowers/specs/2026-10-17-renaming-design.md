# Rename: complete bulk-renaming feature

Date: 2026-10-17

Status: specification for review; no implementation approved by this document alone.

Companion: [design overview](2026-10-17-library-tools-design.md), [Blueprint foundation](2026-10-17-blueprint-foundation-design.md).

## 1. Purpose and release scope

Let a user rename and organize a large video collection with precise, composable rules, an exact preview, and recoverable filesystem operations. Rename is a standalone navigation destination alongside Library folders and Duplicate finder. Every capability marked included below belongs to the completed feature; implementation may be staged, but the first few stages are not the whole requested feature.

The release works offline on Windows without a user-installed script runtime. It supports local disks, removable drives, and user-selected Windows network shares when the relevant filesystem operations are supported. Unsupported capabilities are reported before mutation. Portable domain tests run on Linux; Linux desktop release support is not added implicitly.

Blueprint is optional. Rename can start from saved library folders, a newly chosen folder, or explicit file selection. No collection registration, automatic media identification, online provider, or hero feature is required.

## 2. Context and design choices

The current `vidlib-engine::scan` returns duplicate groups and discards unique-size candidates. It is not a complete rename inventory. Existing `vidlib-fileops::apply_actions` requires duplicate membership and a retained copy, so Rename must not reuse that command as its execution contract. Existing background jobs, native dialogs, and UI conventions are useful patterns.

Introduce a read-only inventory module shared by Rename and Blueprint. Keep deterministic name planning in a separate module and filesystem execution behind an interface in `vidlib-fileops`. The operation journal belongs to execution; Blueprint's observed history is a separate projection. This avoids making a proof-of-concept database responsible for file safety.

## 3. Included capability matrix

This is VidLibOrganizer's acceptance scope, not a claim of binary compatibility with BRU presets, its interface, or every undocumented setting. BRU's site mixes free and licensed features; JavaScript renaming/filtering is explicitly licensed. The capabilities below are intentionally included irrespective of an unresolved entitlement for a particular BRU version.

| ID | Included behavior | Acceptance example or rule |
| --- | --- | --- |
| R01 | Add prefixes/suffixes, insert text, literal replacement lists, remove characters/ranges, trim/collapse whitespace, crop around delimiters. | Strip a known release suffix and replace separators without changing the extension. |
| R02 | Move or copy a substring within a name; reverse a chosen segment; character translation and optional transliteration. | Move an existing year to the end; preserve non-Latin names unless a translation rule is enabled. |
| R03 | Upper/lower/title/sentence case, user-defined case exceptions, Roman-numeral case handling. | Preserve `BBC`, `III`, and a user exception while title-casing other words. |
| R04 | Multiple regex replacements with captures, lookarounds, backreferences in patterns, first/all match controls, and case sensitivity. | Extract/reorder an existing episode identifier; show unsupported or exhausted expressions as errors. |
| R05 | Restrict a rule to a segment by positions, positions from the end, or delimiter occurrences; reorder/repeat/disable rules. | Replace the second bracketed segment only; preview the effect of each rule. |
| R06 | Sequential numbering: start, step, padding, position, prefix/suffix, decimal/hex/alphabetic representation, repeat-each count, reset by folder. Pad existing numbers independently. | Natural order puts `2` before `10`; applying uses the numbering order frozen in the preview. |
| R07 | Date parsing/reformatting; date insertion from filename, filesystem dates, recording metadata, or a frozen current time. | Reformat a user-confirmed `DD-MM-YYYY` date; do not guess an ambiguous numeric date. |
| R08 | Name templates using original/current stem, extension, parent directories, relative directory, counters, dates, file size, available media properties, and explicit fallback/optional groups. | Missing required title blocks the row; an optional resolution group disappears cleanly if unknown. |
| R09 | Optional full-file CRC32, MD5, SHA-1, SHA-256, and SHA-512 name tokens; optional generated UUID/random tokens. | Hashing reads only when requested; generated values are frozen in the reviewed plan. |
| R10 | Keep/replace/remove a stem; extension case/change/add/remove controls and explicit whole-name mode. | Default rules affect the stem only; changing `.avi` to `.mp4` is labelled a name change, not conversion. |
| R11 | Recursive selection with depth limit; include/exclude wildcards and regex; filters on kind, extension, name/path length, size, dates, and attributes. | Root overlaps count a path once; selection changes regenerate counters and conflict checks. |
| R12 | Import explicit old-to-new mappings from CSV or delimited text; paste name lists; single-row override; export reviewed mappings/reports. | Ambiguous basename mapping is rejected; exact path mappings remain within authorized roots. |
| R13 | Saved, versioned presets; import/export of app recipes; undo/redo of recipe edits. | A preset reproduces the same names for the same frozen input and clock values. |
| R14 | Associated-file linking for subtitles, metadata, artwork, and paired assets. | Rename `old.en.forced.srt` with `old.mkv` while retaining `.en.forced`; keep `.idx`/`.sub` pairs together. |
| R15 | Subfolder renaming; new folder layouts; rename in place or copy/move to an explicit destination; preserve or transform relative hierarchy. | File and parent-folder renames have one consistent final-path preview. |
| R16 | Opt-in Windows attributes and filesystem timestamp changes. | Set/clear hidden/read-only/archive/system; set or offset created/modified/accessed time, including a per-item increment. Defaults leave them unchanged. |
| R17 | Searchable/sortable preview with highlighted differences, manual order, error details, export, execution history, recovery, validated undo. | A target conflict is visible before confirmation and cannot silently overwrite another file. |
| R18 | Native file/folder selection, drag/drop, keyboard access; optional Windows Explorer entry point passing selected paths. | Shell arguments select files for review; they never trigger execution or evaluate a command string. |

Photo-only EXIF modification, PDF metadata tooling, music-specific tagging, arbitrary JavaScript execution, transcoding, online identification, filesystem watchers that automatically rename, scheduled jobs, and BRU/BRC file-format compatibility are outside this release. Existing duplicate detection remains separate.

## 4. Workflow and interface

1. Choose files or roots. The initial filter selects videos; the user may explicitly include collection assets, other files, and folders. Inventory includes zero-byte entries with their status; the duplicate engine's zero-byte exclusion does not govern Rename.
2. Load a preset or build a recipe with ordered rule cards. Templates have a field picker and visible missing-field behavior. Advanced options are disclosed without requiring scripting.
3. Review a table of current name/path, proposed name/path, kind, size, and status. Filters are All, Changed, Unchanged, Needs review, and Blocked. Show exact counts for the approved selection, not just the displayed page.
4. Resolve or explicitly exclude blocked rows/dependency groups. Excluding a row or editing a rule creates a new plan revision and recomputes numbering, associated files, and collisions.
5. Review a summary of the current plan: file/folder counts, destinations, companion changes, metadata changes, and conflicts. Only this plan revision can be confirmed.
6. Execute in the background; display progress and partial outcomes. Completed operations appear in history with recovery/undo availability.

Rename has independent selection state. Importing a Blueprint selection or template is an explicit action. Browser preview uses fixture data and cannot perform operations. Screen-reader announcements describe progress and errors; focus returns to the initiating control after dialogs. Names/metadata are rendered as text, never HTML.

The backend is authoritative for generated names. The UI does not implement a second transformation engine. It requests paged rows, per-rule explanations, and aggregate counts from a versioned plan. A table sort changes display only; a separate visible control adopts that order for numbering. Manual reordering creates a new plan.

## 5. Inventory and recipe contract

An inventory has an opaque ID, immutable generation, root bindings, observation coverage, and stable item IDs within that generation. Each item records its native path, display path, relative path, kind, size, relevant timestamps/attributes, and OS file identity when available. Folders and all file extensions can be inventoried; metadata extraction is limited to explicitly requested fields. Link/reparse entries are reported and not followed or mutated.

Native paths remain owned by Rust. UI actions refer to item/plan IDs, not reconstructed display strings. A path that cannot round-trip through supported Windows path handling is visibly unsupported; lossy conversion must never select a different file. New byte counts and timestamps crossing JSON use exact encodings, not rounded JavaScript numbers.

A recipe is versioned data: ordered rules, selection filters, explicit numbering order, template formatting, companion policy, destination mode, collision policy, and metadata options. Unknown versions or rule kinds are rejected rather than silently dropped. Presets are stored in app data and may be exported. Clipboard content is read only on an explicit paste action and becomes frozen recipe input.

Counter assignment uses the frozen selected primary-item vector; companions inherit their primary's result and do not consume counters. Directory numbering is a separate explicitly selected scope. Ties in natural sorting use original relative path and item ID for determinism. Unchanged rows remain part of that vector unless explicitly excluded.

Date parsing requires a specified input format where ambiguous. Formatting declares local/UTC timezone; an absent recording timezone remains unknown unless the user selects an assumption. Filesystem-created time is never presented as recording time. The plan freezes current time, random values, hash results, and requested metadata, so applying cannot produce a different name from the preview.

Use a PCRE2 adapter for R04, not a silently reduced Rust-regex dialect. User expressions run without JIT initially, with match limit 1,000,000, depth limit 1,000, heap limit 8 MiB, and pattern limit 16 KiB. Exhaustion is a row/rule error. Compile once per rule and check cancellation between items. These limits are implementation defaults to verify against the chosen PCRE2 binding, not a claim of a universal wall-clock timeout.

## 6. Media properties and associated files

Bundle a local MediaInfo-based metadata helper with the Windows distribution; ordinary text operations do not require running it. Obtain normalized duration, dimensions, video/audio codecs, frame rate, channels, language, embedded title, and recording-date fields when present. A Windows Property System adapter also exposes available typed canonical property keys, including author/title/date fields. Keep the source and absence of each value visible.

Run metadata probes off the UI thread in a cancellable helper process with a 10-second per-file deadline, one active probe per selected drive by default. Pass native paths as arguments without a shell. Malformed media, missing helpers, or unsupported properties produce actionable missing-field results. Probe only fields actually used by a recipe or explicitly requested in the inspector; cache results against identity/size/mtime and adapter version. Do not edit embedded metadata.

Companion matching initially uses exact original basename plus recognized language/role suffixes in the same folder. Supported assets include `.srt`, `.ass`, `.ssa`, `.vtt`, `.sup`, `.idx`, `.sub`, `.nfo`, `.jpg`, `.jpeg`, `.png`, and `.webp`. Recognized roles include `forced`, `sdh`, `cc`, `poster`, `fanart`, and `thumb`; retained language/role suffixes are shown in the preview. Shared `poster.jpg`, `tvshow.nfo`, and similar folder-wide files are not assigned to one video automatically.

If multiple primary files could own an asset, require a manual association or leave it unchanged with a warning. A companion belongs to at most one primary in a plan. Companion groups and `.idx`/`.sub` pairs form execution dependencies. Cross-folder pairing requires explicit manual association within selected roots. Choosing to rename only the video is allowed but visibly warns that associations may break. No hidden propagation changes unselected assets.

## 7. Plan validation and execution

The planner produces an immutable plan ID/revision, input generation, recipe revision, source snapshots, exact final paths, dependency groups, issues, counts, and proposed metadata changes. Apply accepts that ID/revision and confirmation; it does not accept arbitrary replacement paths from the UI. Plans expire on process restart and after inventory/recipe/selection changes.

Required preflight rules:

- Reject missing/stale sources, repeated mappings, ambiguous imports, unsupported names, target conflicts, and unavailable drives for the approved selection before any mutation. Report all detected problems, including ones on non-displayed rows.
- Use actual filesystem name semantics, including case-insensitive comparison where applicable. Validate reserved Windows names, trailing dots/spaces, illegal characters, component length, and supported long-path handling. Preserve Unicode; warn about suspiciously similar names without rewriting them automatically.
- Protect configured library roots and active Blueprint markers from ordinary rename selection. Subfolders are supported; root relocation is a separate explicit rebind workflow. Never move a directory into its own descendant or create a traversal path outside an approved root/destination.
- Recheck source identity/size/mtime and all path components immediately before each mutating step. Reparse points in source or destination ancestry are rejected. An OS handle/identity-aware adapter reduces path races; the release does not claim immunity to a hostile privileged process.
- Never overwrite an existing entry. A platform adapter must provide no-replace semantics for the individual step or report it unsupported. An existence check followed by an overwriting rename is insufficient. A user may request deterministic conflict suffixes, but every suffix must appear in the approved preview; runtime suffix invention is forbidden.

Folder renames, case-only changes, swaps, and longer cycles are planned as a path-dependency graph. Use reserved, unpredictable temporary names on the same volume to break cycles. Track entries by their plan identity as ancestors move; never apply stale child paths. The preview expands the affected descendant count and warns about unselected descendants carried by a selected directory. Excluding a descendant from selection does not pretend it can stay at its original path when its parent moves. Conflicting descendant requests block that dependency group.

Before mutation, write and durably flush an execution journal in app-local `operations.sqlite3`. Use SQLite WAL with `synchronous=FULL` on a local filesystem. Store intent before every filesystem step and observed completion afterward; if journal persistence fails, stop before the next step. A crash between a step and its completion record must be reconcilable from the recorded identity, paths, and phase.

The batch is recoverable, not an atomic filesystem transaction. After complete preflight, a runtime error stops further groups and attempts safe compensation of the current dependency group. Completed groups remain completed. If compensation is blocked, retain recovery records and show the exact paths requiring attention. Cancellation has the same safe-group behavior. Never leave temporary names unreported or erase an unfinished journal. A library drive disconnected mid-operation remains pending until reconnected. If the volume or adapter cannot provide the required ordering/durability for a recovery claim, block that operation with a specific explanation; do not advertise recovery based only on a local journal while ignoring unflushed remote filesystem state.

Same-volume operations use no-replace renames. Cross-volume moves and all copy modes stream to a private destination staging file, hash source/copy with SHA-256, recheck source freshness, preserve the intended timestamps, and publish without replacement. A move removes its source only after verified publication and a final freshness/identity check. Disk-full, locked-file, and permissions failures leave the source or a recoverable, explicitly reported state. Copying a folder uses the same per-entry execution/journal rules; special filesystem entries are not recreated silently.

An app-wide coordinator permits one filesystem-writing job and prevents duplicate scans or Blueprint refreshes from racing app mutations. Existing duplicate actions participate in it. Path-changing operations invalidate duplicate results, rename inventories/plans, and affected Blueprint freshness; the UI must not continue to offer operations on stale paths. Cross-process app instances must share the writer exclusion and operation recovery gate.

## 8. History, recovery, and undo

History records approved inputs, every step/result, source/destination identities, original and requested metadata, created directories, operation time, and completion status. Recipe edit undo/redo is separate from filesystem undo. History is local; imported reports are data and cannot become executable journals.

On startup, detect unfinished operations before enabling new writes. Reconcile intent/completion windows without blindly retrying a move or deleting an unexpected file. Offer Resume or Restore only for paths whose recorded state can be verified; show unresolved groups and allow read-only inspection. Recovery may require reconnecting a drive.

Filesystem Undo creates a new inverse plan with the same preview, no-overwrite checks, journal, and confirmation. Changes to a retained target, a recreated original path, or a missing volume block the affected inverse dependency group. Restore renamed/moved files and explicitly changed attributes/times only after validating the current expected state. Cross-volume undo may require copying back and sufficient space. Undo of Copy removes only a created copy whose identity and full digest still match its completion record; that deletion is explicitly disclosed. Created directories are removed only if still empty and owned by that operation.

Retain operation journals/history until explicit user removal; block removal of unfinished operations. Removing completed history clearly removes undo support. Blueprint history retention is independent. If optional Blueprint event delivery fails after files change, Rename still reports the filesystem truth and retains an outbox event for replay.

## 9. Security and resource properties

Only user-selected roots and destinations are authoritative. Presets, filenames, clipboard text, CSV mappings, media properties, local metadata, and Blueprint imports are untrusted data. Reject path escape, arbitrary device namespaces, alternate data-stream targets, embedded NULs, and shell interpretation. Validated extended-length Windows path prefixes are an internal path representation for approved filesystem paths, not a way to bypass these checks. Do not import executable code or relax the existing local-content CSP for a recipe.

Validate configuration schemas and bound imports: 64 MiB/100,000 rows for mapping files, 1 MiB/100 rules for recipes, and the regex limits above. Treat spreadsheet formulas as literal names on import; default spreadsheet-oriented report export neutralizes formula-leading cells without changing the underlying recorded path. Use paged backends rather than unbounded IPC responses. Error messages must preserve the failing item and phase without reading unrelated files.

Content reads are explicit for hashes, metadata extraction, and verified copy/move. Text-only in-place rename does not hash or decode videos. Progress is throttled to at most 10 updates/second. Table pages are 200 rows; target validation includes all selected rows. Target a 100,000-entry synthetic inventory with first-page cached filtering/sorting within one second on a documented Windows reference machine; record actual hardware and timings. Blocked OS I/O is reported as waiting and is not subject to an invented cancellation guarantee.

## 10. Release acceptance and delivery slices

Every R01-R18 capability must have representative behavior tests or Windows interaction evidence. The implementation plan should group them into these independently reviewable outcomes:

1. Full inventory, deterministic plan previews, no-replace execution, persistent recovery/history/undo, operation coordination, and a usable minimal Rename screen.
2. All text, regex, segment, number, date, extension, filtering, mapping, and preset workflows with consistent backend-generated previews.
3. Video property tokens, hashes, companion associations, folder/destination operations, advanced metadata controls, and Windows entry points.
4. Complete user-journey, recovery, performance, accessibility, and Windows packaging verification. No advertised included control remains a placeholder.

Mandatory failure fixtures include case-only renames, file and folder cycles, nested ancestor/descendant changes, existing and concurrently created destinations, stale selection on a hidden table page, filename/device/path edge cases, hard-link aliases, changed source during copying, disconnected/network drives, partial failure inside a companion group, disk-full/journal-write failure, crash injection before/after each durable step, interrupted undo, and two app instances attempting writes. Hard-linked paths are separate entries; content identity alone never authorizes deleting an alias.

Run the existing workspace tests, Clippy, formatting checks, and UI module checks plus the new suites. Verify real Windows Explorer entry, WebView2 UI, NTFS operations, exFAT/removable behavior, and a supported network share. A filesystem operation that lacks required semantics must be visibly blocked there; successful Linux tests alone are not Windows release evidence. Fixtures use disposable files, never a user's library.

## 11. Sources and research limits

- [BRU feature overview](https://www.bulkrenameutility.co.uk/#features), [current manual](https://www.bulkrenameutility.co.uk/Downloads/BRU_Manual.pdf), and [download/licensing notes](https://www.bulkrenameutility.co.uk/Download.php): capability benchmark; not a definitive per-feature free-version entitlement matrix.
- [Microsoft MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw): replacement/cross-volume flags; execution must choose no-replace behavior explicitly.
- [SQLite WAL](https://www.sqlite.org/wal.html): keep the operation database on the local machine, even when media are on a network share.
- [PCRE2 interface](https://www.pcre.org/current/doc/html/pcre2api.html): matching resource limits must be configured and tested in the selected binding.

Public-document research does not replace running the chosen adapters on Windows or prove undocumented BRU compatibility. The reference app has not been installed or exercised in this workspace.
