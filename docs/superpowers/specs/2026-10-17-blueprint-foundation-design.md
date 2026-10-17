# Blueprint: collection map and change-history foundation

Date: 2026-10-17

Status: specification for review; foundation and proof of concept, not a mature flagship feature.

Companion: [design overview](2026-10-17-library-tools-design.md), [Rename](2026-10-17-renaming-design.md).

## 1. Intent and first useful delivery

Blueprint is a remembered map of a collection. When the user opens a known file structure, the app should recognize the collection where evidence permits, load its recorded data, compare it with the current structure, and show what changed. Naming templates and descriptive pattern analysis belong within this workspace, alongside future collection tools.

The user explicitly moved Blueprint out of Rename's hero-feature slot. This release builds a dependable foundation and working proof of concept. Its useful loop is: register a collection, record its structure, reopen or reconnect it, refresh, inspect differences, and reuse attached naming templates. Rename remains a complete independent feature.

The initial map is an expandable, searchable directory tree with summaries and item inspection. It accepts arbitrary directory layouts without forcing Movies/Shows/Season conventions. Recognizing a registered collection and understanding its media semantics are different capabilities; automatic semantic understanding is not promised.

## 2. Confirmed scope, assumptions, and exclusions

Included: explicit registration, app-local persistence, known-collection recognition, manual rebinding, optional portable identity markers, inventory snapshots, coverage-aware refresh comparison, observed change history, attribution of completed app operations, template attachments, and basic pattern summaries.

Assumed default: app-local records with an explicitly opt-in identity file inside the collection. No marker is created just by scanning, adding a library folder, or opening Blueprint. Offline behavior remains the default; the optional earlier online-metadata question received no answer.

Outside the proof of concept: automatic episode/movie identification, automatic convention inference, AI training, semantic clustering, automatic library repair, a full episode/season editor, background services, always-on filesystem watching, USN-journal ingestion, cloud sync, cross-computer database merging, and automatic mutations. A known pattern can be displayed without claiming an authoritative interpretation of a file.

## 3. User workflow

1. Open Blueprint and choose a saved library folder or another explicitly selected root.
2. If it is recognized, load cached structure, history, templates, and observation times immediately. Label the map as recorded data until refreshed. If it is unknown, offer Register collection or inspect without saving.
3. For a registered reachable collection, start a cancellable metadata inventory when opening it, unless another filesystem job holds the coordinator. A visible Refresh action repeats the comparison. Offline collections remain browsable from their recorded state.
4. Inspect an expandable tree, type/size/count summaries, root availability, and the latest refresh status. Each item shows its last observed path and properties, not a claim of continuous live state.
5. Read Changes, attach or edit a naming template, or explore Patterns. Sending a selection/template to Rename is explicit and opens a fresh Rename inventory/preview.

The navigation label is `Blueprint` with an `Experimental` indicator. Main sections are Map, Changes, Templates, and Patterns. Map and Changes deliver the foundation; Templates is functional through Rename's recipe editor; Patterns is a limited descriptive proof of concept. Empty and offline states explain what is known and offer the next valid action. No decorative analytics or disabled controls imply finished capabilities.

## 4. Collection identity and recognition

A collection has an app-generated UUID, display name, explicit root bindings, schema version, and timestamps. A root binding has its own UUID, configured native path, relative path policy, last observed filesystem capabilities, and optional OS volume/directory identity. A collection can contain multiple explicitly attached roots; moving one root does not redefine all others.

Recognition evaluates the following evidence, with no drive-wide search:

1. An explicit collection/root selection in the app establishes the intended binding, but the root must still be verified before a refresh can change its baseline.
2. A valid opt-in marker can identify a candidate collection/root at a selected path. It is an identity hint, not proof of ownership or permission to write. If the path/OS identity is not already a consistent known binding, require explicit reconnect confirmation before comparing against or changing that collection's baseline.
3. A previously recorded filesystem volume and root-directory identity can reconnect a selected root when its drive letter/path changes on the same machine, where those identifiers are available and consistent.
4. A previously bound canonical path is only sufficient when available identity evidence agrees. If identity is unavailable, show the prior collection as a candidate and require explicit reconnect confirmation before adopting the new observation as that collection.
5. Similar names, directory shapes, or a structural fingerprint only suggest candidates. They never silently bind or merge collections.

Windows volume serials and file IDs are useful local evidence, not globally unique content identifiers. They can be absent or reused; copies and cross-volume moves can change them. Match supporting metadata and retain uncertainty. Unsupported or unstable identity on a network share falls back to explicit user confirmation.

If a marker and an existing binding disagree, or two reachable roots present the same marker, enter Identity review. Offer explicitly reconnecting the original, registering the copy with a new identity, or leaving it unbound. Never merge histories or rewrite another marker automatically. A known path now containing a different drive/root does not become the old collection merely because the drive letter matches.

Rebind is an explicit workflow showing old/new roots and evidence. It updates a root binding without inventing a content-move event. Mark the previous observation stale until comparison finishes. A root moved while the app is closed is not assumed to be the same based on file counts alone.

Example: a registered `E:\Archive` appears as `F:\Archive` after reconnecting the same drive. Consistent known root identity loads the recorded map and starts refresh. A different drive containing the same folder names instead presents a candidate/reconnect decision. If one subtree cannot be read, the map retains its last recorded entries with an availability warning while reporting verified changes elsewhere.

## 5. Storage and optional portability

Store collection records in app-local `blueprint.sqlite3`, outside media roots and network shares. Use versioned migrations, local WAL storage, and transactions for snapshot promotion and history projection. On a migration failure preserve the original database and report the failure; never silently reset the collection registry. Export/backup uses a consistent database snapshot or a structured bundle, not a live copy of the main database file alone.

The marker, if enabled for an explicitly chosen root, is `.vidlib-blueprint.json` with only `schema_version: 1`, `collection_id`, and `root_id`. Parse only this schema, limit the file to 4 KiB, and reject links/reparse points. Do not store absolute paths, executable settings, user names, or historical filenames in the marker. Creation/removal/replacement is an explicit action; failure on read-only roots leaves the collection usable through local bindings. Exclude markers and operation staging files from normal Rename selection and collection statistics.

A marker makes an identity portable; it does not transport history. Another installation without matching records must say that the identity is known from a marker but local data are absent, then offer import of a user-selected Blueprint export or a new baseline. Never pretend that past history has been loaded from an ID alone.

Portable exports include a format version, collection/root IDs, relative entry paths, snapshot coverage, change records, and attached template data. Exclude live OS bindings, writer locks, and executable Rename journals. Import validates sizes, schema, relative paths, and reference integrity before registering an inactive candidate; it grants no filesystem access. Limit an import to 256 MiB and 1,000,000 inventory entries. Existing identities require explicit replace-or-import-as-copy review; history merge is not supported in this foundation. Templates reference the documented Rename recipe schema; unknown versions remain inactive with an explanation.

History remains until the user explicitly chooses a retention policy or deletes the local collection record. Offer user-selected pruning by age with the last complete baseline retained. Never prune an operation's authoritative recovery journal through Blueprint. Removing a Blueprint record deletes app-local metadata only; removing a marker is a separate choice. State that a local database is not a backup of the actual videos.

## 6. Inventory, snapshots, and coverage

Use the shared inventory module specified by Rename. It returns folders, regular files of every extension, zero-byte entries, unsupported/link entries, metadata, OS identity where available, and observation coverage. It does not read full video contents by default. This supports a collection map without filtering out material that is not a duplicate or not a video.

Store relative native paths per root alongside display forms, item kind, size, creation/modified times where available, selected attributes, optional volume/file identity, and optional explicitly obtained digests. Record unsupported native paths as visible issues; never match entries by a lossy display name. File IDs identify observed entries, not abstract movies. Hard links remain distinct paths with a shared physical-identity annotation where known.

A refresh attempt records start/end times, selected roots and exclusions, algorithm version, observed entries, and coverage per subtree. Outcomes are Complete, Partial, Cancelled, Offline, or Failed. A scan over a changing filesystem is an observation window, not a simultaneous filesystem snapshot.

Promote observed data and resulting events transactionally, preserving the last reliable state for unobserved subtrees. Additions/changes in successfully covered subtrees can be committed even if another subtree fails. Infer disappearance only when the previous item's parent scope was completely enumerated and the root's identity was verified. Excluded, unreadable, disconnected, or cancelled scopes cannot establish absence. A newly covered subtree gets a baseline rather than a historical flood of supposed additions during an unknown interval.

Cancellation leaves valid prior baselines intact and marks the current attempt incomplete. Opening a cached collection must not mark it fresh. The UI displays last attempted refresh separately from last complete observation for each root/scope. A change in filters/coverage starts new comparison coverage for that scope; it is not evidence of deletion.

## 7. Changelog semantics

The first successful observation produces a Baseline recorded event and summary, not thousands of files falsely described as newly created at that moment. Subsequent comparisons produce:

| Event | Meaning and confidence |
| --- | --- |
| Added | A path newly observed within a comparable, successfully enumerated scope. Its real creation time may precede the refresh. |
| Missing | A formerly observed path absent from a completed comparable scope. External deletion is one possible cause; do not claim it was permanently deleted. |
| Properties changed | Observed size, timestamps, kind, or recorded attributes changed. It is not proof that video content changed or stayed identical. |
| Location changed | Same observed entry identity appears at another path with consistent supporting evidence and unambiguous pairing. Describe it as detected between observations. |
| Possible relocation | A weak path/size/metadata resemblance suggests a link. Keep original Added/Missing facts and show a non-authoritative candidate relation. |
| App operation | A completed Rename, move, delete, copy, or undo event from this installation's durable operation record. This has known operation time and origin. |
| Availability changed | A root became unreachable/reachable, or coverage was lost/restored. It does not imply changes to its files. |

External change records contain their observation interval and `origin: external_or_unknown`. Do not invent actor, exact event time, or intermediate actions while the app was closed. Matching metadata does not prove byte equality; hashing is an explicit optional verification workflow. File IDs are considered with volume/root context, supporting metadata, and possible reuse, not as eternal identity.

For a renamed directory, show one expandable group with affected descendant count while retaining queryable underlying path changes. Do not make a library-wide folder rename look like an unexplained delete/add storm. When identity evidence is insufficient, show observed facts and uncertainty instead of forcing that grouping.

Filter history by root/path, observation interval, event type, origin, and operation ID. Export includes those qualifications. Acknowledging a change changes review state only and never edits files or rewrites the recorded event.

## 8. Templates and Patterns proof of concept

Naming templates use Rename's versioned recipe schema. Rename owns evaluation and validation; Blueprint stores attachments by preset ID and revision plus an immutable recipe snapshot. A new preset revision does not silently alter an attached revision. The user can explicitly update the attachment. Editing from Blueprint opens the same recipe editor and creates a revision; it does not introduce a second rule language.

A template attachment can target a collection or explicit relative subtree. If multiple scopes match, show candidate templates and require a choice; no template is automatically applied. Manual template editing, attaching, and opening a preview in Rename are real initial functionality. Deriving a complete naming convention from examples is deferred.

Patterns initially describes observed filenames: extension/case distribution, common delimiter shapes, recognized explicit episode markers such as `S02E03`/`2x03`, and counts of unmatched examples. Attach recognizer/version and observation coverage to each summary. Plain numbers and years are not automatically classified as episode IDs. Counts exclude unobserved scopes and identify their denominator. Pattern data never overwrite recorded filenames or confirmed user fields.

Clicking a pattern filters the map to its examples and can send that explicit selection to Rename. No confidence score pretends to be statistical certainty; list which rule matched. Suggestions, semantic season mapping, automatic template inference, and library-wide reconciliation remain future work and are labelled as such in documentation, not exposed as working controls.

## 9. Modules and shared integration contract

Proposed module placement, to be refined into exact files in implementation plans:

- `crates/vidlib-inventory`: read-only discovery, native-path/entry snapshots, root evidence, coverage, and cancellation. Rename and Blueprint use this directly; duplicate scanning may adopt it only through a separately verified adaptation.
- `crates/vidlib-rename`: deterministic recipes and immutable plans; has no Blueprint dependency.
- `crates/vidlib-fileops`: existing duplicate operations plus a separate Rename execution/journal interface. It publishes durable completed-operation events.
- `crates/vidlib-blueprint`: collection registry, persistence, recognition decisions, refresh diffs/history, template attachments, and pattern summaries. It may consume Rename's recipe data types, never its filesystem executor implicitly.
- `src-tauri`: native OS adapters, app-local paths, shared job/writer coordination, command adapters, and event delivery.
- `ui`: independent Rename and Blueprint screens using the existing HTML/CSS/ES-module approach; shared rendering helpers and a single recipe editor.

Use small interfaces for obtaining an inventory, generating a plan, executing an approved plan, recognizing a collection, refreshing it, and querying paged history/map data. Keep SQL and OS-specific identity handling behind those interfaces. The shared contract includes opaque IDs, immutable generations, exact source snapshots, coverage, cancellation/progress, and typed operation outcomes. It does not expose private SQL tables to UI callers.

Every completed mutation event has a stable event ID, operation ID, item-level before/after paths and identities, result phase, observation metadata, and known app origin. The executor persists pending events with its journal. Blueprint consumes them idempotently by event ID and records its own projection cursor. Delivery retries survive restarts; no distributed transaction across the two databases is assumed.

After a completed app operation, invalidate affected views immediately and mark relevant collection scopes for refresh. Apply exact known path changes when possible, but do not claim the entire root is fresh. On the next refresh, associate matching observations with the operation rather than duplicating the event. If external changes intervened or the current state disagrees, preserve the app event and record the distinct observed difference. Blueprint storage failure never changes a successful Rename outcome into a supposed failure or causes the mutation to be repeated.

Existing duplicate actions must emit operation outcomes through this shared contract before their Blueprint attribution is called complete. Duplicate findings are invalidated after path-changing operations. A previously unregistered folder used by Rename remains usable and receives Rename history without automatic Blueprint registration.

## 10. Safety, performance, and failure behavior

Blueprint refreshes are read-only filesystem jobs coordinated against app writes. The app can browse cached data during other jobs, but a refresh waits rather than comparing against an app mutation in progress. Registration and refresh do not execute templates. Stored/exported history contains private paths; keep it local and user-controlled.

Treat markers, exports, presets, metadata, path names, and database contents as data. Reject unknown/oversized marker or export-envelope schemas, traversal and absolute paths in portable entry records, reparse-point markers, and invalid IDs. A recognized export envelope may preserve a newer embedded recipe as opaque inactive data; it must never evaluate or silently downgrade it. Imported IDs never confer a native root binding, write permission, or an executable operation. Render text safely and preserve the existing CSP. Protect active-marker paths from Rename and from Blueprint's own statistics to avoid self-generated change noise.

The local database is advisory for current filesystem state. A stale snapshot is never authority for deletion, rename, or undo. All mutation workflows go through Rename/current file-operation validation. A failed refresh, a full app-data disk, or a corrupt Blueprint database must not silently erase prior history or block independent Rename use. Show a repair/export/reset choice preserving the original data.

Use indexed relative paths and identity keys, paged tree/history queries of 200 rows, and incremental subtree summaries. Target 100,000 entries with cached collection open/first page within one second on a documented Windows reference machine. Store metadata proportional to entries and history; no thumbnail library or mandatory media hashing is introduced. Show scan progress at most 10 updates/second and record actual benchmark results before release claims.

## 11. Proof-of-concept acceptance

The foundation is ready only when all of these are demonstrated with disposable fixtures:

1. Register an arbitrary nested layout, restart, and reopen the saved map, templates, and history.
2. Recognize a known selected root using available identity; handle a drive-letter change and explicit rebind without silently merging an unrelated lookalike.
3. Show an offline collection from cache; reconnection refreshes it without generating false mass deletions.
4. Add/modify/remove/rename fixture entries externally, then refresh and show accurately qualified events. A first baseline is clearly distinguished from additions.
5. Simulate unreadable/excluded subtrees and cancelled scans; preserve prior observations and never infer missing entries from incomplete coverage.
6. Use a non-identifying or unstable filesystem; present a candidate/manual choice and truthful Added/Missing facts when relocation cannot be proven.
7. Copy a marker to a second root, present identity review, and preserve separate histories. A fresh installation with only the marker does not claim to have its history.
8. Deliver/replay a Rename or duplicate-action event twice and across restart; project it once and avoid duplicate refresh events. Rename works if Blueprint delivery/storage is unavailable.
9. Attach/revise a real template, open an explicit selection in Rename, and get a fresh validated preview. A pattern summary filters examples without performing changes.
10. Exercise marker refusal on a read-only root, database migration failure, consistent export/import, retention, unsupported paths, large-inventory paging, and accessibility.

The complete Rename release and this Blueprint foundation have separate completion reports. The foundation's domain tests and map/history behaviors must work; its experimental label is not permission to ship placeholders for the loop above.

## 12. Sources and future extension points

- [Microsoft FILE_ID_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info) and [filesystem identity behavior](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/d4bc551b-7aaf-4b4f-ba0e-3a75e7c528f0): file/volume identity is local evidence, with reuse and cross-volume limitations.
- [Microsoft directory change notifications](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesw): future watchers need rescan/reconciliation when notifications are lost. This foundation deliberately uses refresh comparisons.
- [SQLite WAL](https://www.sqlite.org/wal.html): local database placement and consistent persistence/backup requirements.

Later work can add efficient change notifications, richer verified metadata, visual collection tools, example-driven convention inference, and optional online providers behind the collection model. None is necessary to satisfy this foundation, and none is implicitly approved by this spec.
