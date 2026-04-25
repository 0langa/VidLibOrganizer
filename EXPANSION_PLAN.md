# VidLibOrganizer Long-Term Product Roadmap

## Executive summary

VidLibOrganizer already has a solid local-first Rust workspace foundation: shared domain contracts in `vidlib-core`, SQLite persistence in `vidlib-db`, bounded-memory scanning in `vidlib-scanner`, safe restructure planning in `vidlib-fileops`, a reusable scan orchestration layer in `vidlib-workflows`, a CLI, and a Tauri desktop shell. It is not starting from zero. It is starting from an MVP platform.

The long-term goal is to evolve that platform into a production-ready, enterprise-grade, premium video library suite for collectors, archivists, media teams, post-production users, and power users who need a single application for:

- large-scale library ingestion and indexing
- intelligent organization and labeling
- fast search and browsing
- duplicate and quality review
- metadata enrichment and media analysis
- safe batch editing and file operations
- integrated preview and playback
- operational reliability, auditability, and recoverability

This roadmap is written as a strategic product and engineering plan, not just a backlog. It is intended to guide the project from MVP to a serious premium desktop program.

---

## 1. Current project understanding

## What exists today

The current repository already demonstrates the correct architectural direction.

### Workspace structure

- `crates/vidlib-core`
	- owns shared domain models, progress, and workspace error contracts
- `crates/vidlib-db`
	- owns SQLite schema, persistence, checkpoints, warnings, audit records, and basic search
- `crates/vidlib-scanner`
	- owns filesystem traversal, extension filtering, hashing, warning generation, and metadata integration
- `crates/vidlib-metadata`
	- owns metadata extraction contracts and `ffprobe` integration
- `crates/vidlib-duplicates`
	- owns duplicate grouping and hashing helpers
- `crates/vidlib-fileops`
	- owns restructure planning, apply, undo, and conflict-safe file operations
- `crates/vidlib-ml`
	- owns local ML abstraction and staged ONNX integration path
- `crates/vidlib-workflows`
	- owns shared orchestration for scan workflows
- `crates/vidlib-cli`
	- exposes CLI workflows
- `src-tauri`
	- exposes desktop app workflows
- `ui/`
	- lightweight frontend shell

### Product capabilities already present

- local-only, offline-first architecture
- SQLite-backed indexed video catalog
- library registration
- recursive video scanning
- warning-tolerant ingestion
- `ffprobe` metadata extraction with fallback behavior
- exact and chunk hashing options
- placeholder perceptual hashing path
- duplicate grouping
- basic search by text, tag, and extension
- restructure planning by extension
- apply + undo manifest behavior
- audit record persistence
- desktop dashboard and basic scan/search/plan flows

### Structural strengths already in place

- crate boundaries are mostly clean and well documented in `docs/crate-contracts.md`
- reusable workflow logic is already moving out of entrypoints
- safety-first file operations already exist
- the app is local-first by design, which is a strong product differentiator

## What is missing today

The current system is still early-stage in the areas that separate an MVP from a premium professional product:

- no real persistent job system
- no true cancellation ownership in the desktop app
- no live enterprise-grade progress model
- basic search only; no FTS, ranking, faceting, or saved intelligence
- no first-class browsing experience
- no embedded playback pipeline
- no timeline-aware review tools
- no batch metadata editing UX
- no real duplicate resolution workflow
- no advanced tagging taxonomy or rule engine
- no production packaging, updater, diagnostics, or supportability model
- no plugin or extension architecture
- no multi-library governance features for advanced users or teams

---

## 2. Product vision

VidLibOrganizer should become a premium all-in-one desktop platform for video library management.

At maturity, the product should feel like a fusion of:

- a media asset organizer
- a desktop DAM-lite system for local video collections
- a safe bulk file operations tool
- a duplicate and quality review system
- a metadata intelligence and tagging engine
- a library browser with rich preview and playback
- a local-first media analysis workbench

## Core vision pillars

### 1. Local-first and privacy-first

- no forced cloud dependency
- no hidden telemetry by default
- local control over metadata, tags, previews, thumbnails, and ML models
- optional connected features later, but never as the foundation

### 2. Safety-first media operations

- preview before destructive action
- reversible workflows where possible
- strong audit trails
- conflict detection and rollback support
- explicit trust boundaries around file operations

### 3. High-performance library scale

- handle hundreds of thousands of media records
- support very large libraries on local, external, and network-attached storage
- remain responsive during scan, search, browse, and review workflows

### 4. Premium workflow depth

- not just scan and search
- also review, compare, label, sort, restructure, inspect, preview, play, analyze, and curate

### 5. Shared business logic across interfaces

- CLI for automation and power workflows
- desktop UI for premium daily usage
- future API/plugin surface without rewriting core logic

---

## 3. Product maturity target

To be considered production-ready and enterprise-grade, VidLibOrganizer should meet the following standards.

## Reliability

- scans survive interruption
- state is resumable and repairable
- large operations are chunked and recoverable
- corruption scenarios are detectable and repairable

## Usability

- polished desktop experience
- clear progress, errors, and recovery guidance
- powerful but understandable workflows
- discoverable advanced features

## Performance

- responsive browsing on large libraries
- indexed search with strong latency targets
- low-memory long-running background operations
- thumbnail and preview caching that scales

## Safety

- all destructive operations previewed
- manifest-backed undo where possible
- audit log for all major changes
- quarantine/recycle workflows before permanent delete

## Extensibility

- clean crate-level boundaries
- versioned data contracts
- plugin-ready service boundaries
- future model and analyzer backends can be added without architectural collapse

## Supportability

- structured logs
- diagnostics bundles
- database integrity checks
- migration discipline
- installer, updater, and version compatibility strategy

---

## 4. Strategic roadmap themes

This roadmap is organized around long-term themes rather than isolated features.

1. Platform and workflow reliability
2. Search, discovery, and browsing
3. Media intelligence and metadata depth
4. Duplicate, quality, and curation workflows
5. Integrated preview, playback, and review tools
6. Editing, organization, and batch operations
7. Premium desktop UX and product polish
8. Enterprise readiness, diagnostics, packaging, and support
9. Extensibility, automation, and ecosystem growth

---

## 5. Roadmap phases

## Phase 1 — Stabilize the platform foundation

### Purpose

Convert the current MVP architecture into a dependable product core that can safely support premium features.

### Why this phase matters

The codebase already has the right crate layout, but the runtime model is still shallow. Premium features will fail if workflows, jobs, persistence, and UI state are not hardened first.

### Key outcomes

- first-class workflow runtime
- persistent job history
- resumable scans
- real cancellation and progress ownership
- stronger schema/versioning discipline
- thin CLI and Tauri entrypoints

### Major workstreams

#### 1. Workflow runtime hardening

Expand `vidlib-workflows` into the central orchestration layer for:

- scan jobs
- metadata enrichment jobs
- thumbnail generation jobs
- duplicate analysis jobs
- restructure preview/apply jobs
- background maintenance jobs

Introduce explicit workflow types such as:

- `JobId`
- `JobKind`
- `JobState`
- `JobProgress`
- `JobCheckpoint`
- `JobResult`
- `JobFailure`

#### 2. Real job manager

Replace the Tauri-side `HashMap<String, bool>` placeholder with a true job runtime that owns:

- cancellation tokens
- progress snapshots
- lifecycle transitions
- completion summaries
- last error state
- event fanout to UI subscribers

#### 3. Persistent operational state

Extend `vidlib-db` to persist:

- job table and history
- richer scan checkpoints
- library health status
- ingest sessions
- thumbnail generation queue state
- analysis queue state

#### 4. Data model cleanup

Evolve `vidlib-core` shared types into more production-ready contracts:

- richer `VideoEntry`
- versioned query/filter model
- explicit media stream models
- normalized tag and label types
- review state enums
- operation summary types

#### 5. Migration discipline

Establish a stricter migration policy for SQLite:

- numbered migrations
- migration tests
- upgrade compatibility checks
- downgrade/repair strategy where practical

### Exit criteria

- scans can be started, cancelled, resumed, and inspected reliably
- desktop and CLI both use the same workflow APIs
- job history is queryable
- all runtime-critical flows are covered by integration tests

---

## Phase 2 — Build a serious search and browsing engine

### Purpose

Turn the product from an indexer into a library browser users can live in all day.

### Key outcomes

- fast indexed search
- advanced filters and facets
- saved searches and collections
- browse-first UI patterns
- large library responsiveness

### Major workstreams

#### 1. Full-text and structured search

Upgrade `vidlib-db::search` from in-memory filtering to indexed search using SQLite FTS and structured filtering.

Index candidate fields such as:

- filename
- path
- tags
- codec/container
- user notes
- derived labels
- library name

#### 2. Advanced query model

Replace the minimal `SearchQuery` with a richer query contract supporting:

- text query
- include/exclude tags
- extension/container
- codec
- width/height/resolution classes
- frame rate range
- duration range
- bitrate range
- size range
- date added / modified date
- duplicate state
- quality flags
- review status
- library and collection scope

#### 3. Faceted discovery

Add browse facets and aggregations for:

- format
- resolution
- duration bucket
- codec
- library root
- tag family
- duplicate status
- quality warnings

#### 4. Saved search and smart collections

Support:

- recent searches
- named filters
- dynamic smart collections
- pinned views such as “Needs Review”, “Likely Duplicates”, “No Metadata”, “Large Files”, and “Low Quality Imports”

#### 5. Browser-grade UI

The UI should evolve into a proper library browser with:

- table view
- thumbnail grid view
- details inspector
- quick filter chips
- keyboard navigation
- multi-select workflows
- persistent sort/view preferences

### Exit criteria

- search remains fast at large scale
- the UI supports real browse workflows, not just lookup
- saved views make daily usage practical

---

## Phase 3 — Rich metadata, thumbnails, and media understanding

### Purpose

Expand the system from file indexing to actual media understanding.

### Key outcomes

- richer technical metadata
- thumbnail and preview generation
- stream-level insight
- better foundations for playback, duplicate review, and ML analysis

### Major workstreams

#### 1. Expanded metadata extraction

Extend `vidlib-metadata` to collect and normalize:

- container format
- bitrate
- frame rate
- pixel format
- aspect ratio
- color space / HDR indicators where available
- audio streams, channels, language, codec
- subtitle streams
- creation date and embedded metadata when present
- rotation/orientation

#### 2. Thumbnail and preview asset pipeline

Add a managed preview subsystem for:

- poster thumbnails
- contact sheet generation
- multi-frame preview strips
- cached preview stills
- preview invalidation and regeneration

#### 3. Preview cache management

Implement cache policies for:

- cache location
- size limits
- regeneration rules
- stale cache cleanup
- multi-quality preview variants

#### 4. Media technical diagnostics

Add derived analysis flags such as:

- missing audio
- unusual duration
- corrupted metadata indicators
- very low bitrate for resolution
- likely screen capture vs camera footage
- mismatched container/codec expectations

### Exit criteria

- each indexed record can support richer inspection
- the UI can show meaningful previews and technical detail
- the data model is ready for advanced review workflows

---

## Phase 4 — Duplicate detection, similarity, and curation workflows

### Purpose

Turn duplicate detection into one of the product’s flagship premium workflows.

### Key outcomes

- guided duplicate review
- exact and near-duplicate confidence tiers
- canonical selection assistance
- safe batch resolution

### Major workstreams

#### 1. Duplicate intelligence model

Expand `vidlib-duplicates` and core models to support:

- exact duplicates
- chunk-based near duplicates
- perceptual similarity from sampled frames
- metadata-based likely duplicates
- variant relationships such as transcodes, trims, re-encodes, and renamed copies

#### 2. Review state and decision model

Add support for:

- pending / reviewed / ignored / resolved groups
- canonical keep choice
- user notes
- decision rationale
- confidence scores
- suggested action type

#### 3. Best-version heuristics

Implement ranking heuristics such as:

- prefer higher visual quality
- prefer better codec when practical
- prefer complete duration over truncated variants
- prefer files in trusted roots
- prefer manually tagged or user-pinned versions

#### 4. Duplicate review UI

Add a premium review experience with:

- side-by-side metadata comparison
- thumbnail/contact sheet comparison
- quick playback comparison
- “keep best” recommendations
- batch quarantine/move/delete actions
- reversible action preview

### Exit criteria

- duplicate resolution becomes a real end-user workflow
- users can safely reduce clutter with confidence

---

## Phase 5 — Tagging, taxonomy, and intelligent organization

### Purpose

Move from simple labels to a true organization system.

### Key outcomes

- manual and machine-assisted labeling
- structured tag taxonomy
- rules engine for automated organization
- premium curation workflows

### Major workstreams

#### 1. Tag system redesign

Support multiple label classes:

- freeform tags
- controlled vocabulary tags
- hierarchical categories
- machine-suggested labels
- locked/protected labels
- provenance-aware tags

#### 2. Rules engine

Users should be able to define rules like:

- if codec is `h264` and resolution is `480p`, tag as `legacy`
- if path contains `wedding`, tag as `event/wedding`
- if duration < threshold and resolution low, mark for review
- if duplicate confidence is high, place in duplicate review collection

#### 3. Organization templates

Expand `vidlib-fileops` from extension-based planning to rule/template-driven organization:

- by date
- by event
- by tag
- by library
- by quality tier
- by custom path templates

#### 4. Batch metadata and tag editing

Add bulk edit workflows for:

- tag add/remove/replace
- review status changes
- collection assignment
- naming template preview

### Exit criteria

- the product supports real curation at scale
- users can maintain consistent organization over time

---

## Phase 6 — Integrated browsing, preview, and playback

### Purpose

Make VidLibOrganizer a place where users not only organize media, but also inspect and consume it.

### Key outcomes

- integrated playback
- browser-like media navigation
- review-friendly playback controls
- premium daily-use experience

### Major workstreams

#### 1. Embedded player architecture

Introduce a playback subsystem supporting:

- common desktop playback formats
- reliable seeking
- frame stepping where feasible
- audio track switching where feasible
- subtitle support over time

#### 2. Playback UX

Add:

- inline preview player
- dedicated detail view player
- keyboard shortcuts
- scrub bar with preview markers
- previous/next in filtered result set
- loop and comparison playback modes

#### 3. Browse and inspect flows

Support:

- double-click to play
- hover preview where practical
- quick inspect panel
- collection browsing
- timeline of recently added or recently reviewed content

#### 4. Playback-aware review tools

Premium workflows should eventually support:

- compare two similar clips
- review quality issues while playing
- mark timestamps or notes
- create curation markers for later action

### Exit criteria

- the desktop app becomes a true browsing environment
- preview and playback are good enough for regular use

---

## Phase 7 — Media analysis and premium intelligence features

### Purpose

Introduce advanced offline intelligence without compromising the local-first promise.

### Key outcomes

- usable local ML analysis
- better auto-labeling
- quality and content insights
- premium differentiation

### Major workstreams

#### 1. ONNX-powered local inference

Expand `vidlib-ml` into a real inference subsystem with:

- model registry
- model capability metadata
- threshold tuning
- opt-in model downloads or manual installs
- hardware acceleration support when available
- graceful fallback to heuristic behavior

#### 2. Analysis feature tracks

Potential premium intelligence features:

- scene classification
- object labels
- face clustering
- NSFW/sensitive content flagging with local-only execution
- quality scoring
- low-light / blur / stability indicators
- likely screen recording / webcam / phone footage classification

#### 3. Confidence-aware UX

All ML outputs should be treated as suggestions with:

- confidence display
- provenance display
- user override
- bulk accept/reject review flows

### Exit criteria

- ML features are optional, local, and useful
- analysis enriches workflows without becoming mandatory or opaque

---

## Phase 8 — Editing-adjacent features and operational media tools

### Purpose

Expand beyond organization into light editing-adjacent and review-adjacent capabilities expected from premium tools.

### Key outcomes

- batch operations beyond moving files
- lightweight media transformation workflows
- stronger quality control and review pipelines

### Major workstreams

#### 1. Non-destructive action planning

Broaden planning support for:

- rename preview
- directory restructure preview
- move/copy/quarantine preview
- metadata/tag changes preview

#### 2. Optional transformation workflows

Longer term, consider controlled workflows for:

- clip extraction helpers
- poster frame selection
- thumbnail regeneration
- transcode queue handoff or preset launch
- audio extraction helpers

These should remain clearly separated from full NLE ambitions. The product should be editing-adjacent, not necessarily a full editor.

#### 3. QC and review queues

Add workflows for:

- corrupt or suspicious files
- low-quality imports
- missing metadata
- probable duplicates
- unreviewed clips
- untagged media

### Exit criteria

- VidLibOrganizer becomes a practical operational media workbench
- users can manage cleanup and prep workflows without leaving the app

---

## Phase 9 — Premium desktop UX, settings, and polish

### Purpose

Close the gap between “tool” and “premium product.”

### Key outcomes

- modern desktop UX
- configurable preferences
- cohesive product identity
- reduced friction for long daily sessions

### Major workstreams

#### 1. UI architecture upgrade

The current `ui/` shell should evolve into a more scalable frontend with:

- reusable state model
- view routing
- component library
- virtualization for large result sets
- stronger keyboard accessibility

#### 2. Product settings

Add settings for:

- database location
- cache location and size
- thumbnail quality
- scan defaults
- ignore rules
- model paths
- theme and density preferences
- playback preferences

#### 3. UX polish features

Add:

- onboarding and first-run guidance
- empty-state guidance
- progress center
- notifications and action history
- recent activity feed
- richer error messages and recovery guidance

### Exit criteria

- the app feels cohesive, polished, and premium
- advanced features remain approachable

---

## Phase 10 — Production readiness, packaging, and enterprise supportability

### Purpose

Make the application supportable as a serious shipped product.

### Key outcomes

- production build pipeline
- update strategy
- diagnostics and repair tools
- long-term data safety posture

### Major workstreams

#### 1. Packaging and release engineering

Establish:

- signed installers
- stable release channels
- beta/preview channels
- automated build validation
- artifact verification

#### 2. Upgrades and compatibility

Support:

- safe DB migrations across versions
- preview of breaking schema changes during development
- import/export of configuration and metadata
- backup and restore guidance

#### 3. Diagnostics and support bundle

Add supportability features such as:

- structured logs
- debug log viewer/export
- environment diagnostics
- database integrity check
- cache cleanup tools
- scan health and performance diagnostics

#### 4. Reliability and quality engineering

Raise test maturity to include:

- unit tests for crate internals
- integration tests for full workflows
- synthetic large-library performance tests
- migration tests
- file operation failure injection tests
- UI smoke/regression tests

### Exit criteria

- releases are safer to ship and support
- users can recover from common failures without data loss

---

## Phase 11 — Automation, extensibility, and ecosystem growth

### Purpose

Enable power-user and enterprise-style extension scenarios without weakening the local core.

### Key outcomes

- scriptable workflows
- plugin-ready boundaries
- future ecosystem potential

### Major workstreams

#### 1. Automation surface

Continue growing the CLI as a first-class automation layer for:

- scheduled scans
- smart collection export
- batch organization runs
- reporting and audits

#### 2. Plugin/service boundaries

Define extension points for:

- metadata providers
- analyzers
- duplicate strategies
- import/export adapters
- rule packs

#### 3. Optional future integration surfaces

Longer-term possibilities:

- local API for automation
- plugin SDK
- importers from other media organizers
- team-oriented metadata exchange

### Exit criteria

- the platform can grow without monolithic churn
- advanced users can automate more of their workflows

---

## 6. Capability map for a premium all-in-one program

The mature product should include most or all of the following premium capability areas.

## Library management

- multi-library support
- health status per library
- incremental scanning
- offline/external drive awareness
- library statistics and storage visibility

## Search and discovery

- full-text search
- structured filters
- faceted browse
- smart collections
- saved searches

## Metadata and labeling

- technical metadata
- user metadata
- rule-based labels
- ML-assisted suggestions
- controlled taxonomy

## Duplicate and quality control

- exact duplicate detection
- near-duplicate detection
- canonical choice support
- quality warnings
- quarantine flows

## Preview and playback

- thumbnails
- preview strips
- integrated player
- compare mode
- playback-aware review

## File operations

- move/copy/rename planning
- preview and conflict handling
- apply with manifests
- undo and audit trails

## Review and curation

- queues
- notes
- status flags
- bulk review actions
- smart worklists

## Product operations

- settings
- logs
- diagnostics
- repair and recovery tools
- installer and updater

---

## 7. Recommended engineering priorities

To maximize product value and reduce rework, the recommended order of execution is:

1. harden workflow runtime and persistent jobs
2. rebuild search and browse around indexed discovery
3. build richer metadata and thumbnail pipeline
4. deliver duplicate review and safe curation workflows
5. add taxonomy, rules, and batch organization depth
6. add integrated playback and media review UX
7. deepen local ML and premium intelligence features
8. harden packaging, diagnostics, and supportability

This order preserves the current architecture and builds on its strengths.

---

## 8. Prioritized milestone plan and implementation order

This section converts the roadmap into a practical milestone sequence. The goal is priority order, not calendar planning.

### Tag legend

- `Complexity: Low | Medium | High | Very High`
- `Coding Size: Small | Medium | Large | Very Large | Massive`

## Milestone 1 — Core workflow runtime and persistent jobs

**Priority:** 1
**Primary roadmap phase:** Phase 1 — Stabilize the platform foundation

### Goal

Establish a production-grade job and workflow backbone before deeper product features are layered on top.

### Implementation steps

1. Define shared job/workflow models in `vidlib-core`
	- add durable job identity, job kind, lifecycle state, richer progress, checkpoints, failure summaries, and operation summaries
	- **Tags:** `Complexity: High`, `Coding Size: Large`

2. Expand `vidlib-workflows` into the central orchestration layer
	- unify scan, metadata enrichment, background maintenance, and future queued workflows under one runtime pattern
	- **Tags:** `Complexity: High`, `Coding Size: Large`

3. Replace the Tauri placeholder job registry in `src-tauri`
	- remove the simple `HashMap<String, bool>` cancellation placeholder and introduce real runtime-owned jobs, progress subscriptions, and cancellation handles
	- **Tags:** `Complexity: High`, `Coding Size: Large`

4. Persist job history and operational state in `vidlib-db`
	- store job records, checkpoints, summaries, and failure state
	- **Tags:** `Complexity: High`, `Coding Size: Large`

5. Refactor `vidlib-cli` and `src-tauri` to consume the same workflow APIs
	- keep entrypoints thin and move orchestration details fully into reusable crates
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

### Crate mapping

- `crates/vidlib-core`
- `crates/vidlib-db`
- `crates/vidlib-workflows`
- `crates/vidlib-scanner`
- `crates/vidlib-cli`
- `src-tauri`

### Schema changes

- new `jobs` table
  - `id`, `kind`, `state`, `created_at`, `started_at`, `finished_at`, `requested_by`, `root_scope`, `summary_json`, `error_json`
- new `job_checkpoints` table
  - `job_id`, `checkpoint_kind`, `payload_json`, `updated_at`
- new `job_events` table
  - append-only progress/event log for audit and resume support
- extend `scan_checkpoints`
  - tie checkpoints to job id and library id more explicitly
- optional `library_status` table
  - health, last scan state, last successful scan, warning counts

### Why first

This milestone reduces rework across search, thumbnails, playback, ML, and duplicate review because all of them benefit from a unified job model.

## Milestone 2 — Indexed search, filters, and browser foundation

**Priority:** 2
**Primary roadmap phase:** Phase 2 — Build a serious search and browsing engine

### Goal

Turn the product into a true browse-and-discover application.

### Implementation steps

1. Replace in-memory search with indexed search in `vidlib-db`
	- adopt SQLite FTS and structured filtering paths
	- **Tags:** `Complexity: High`, `Coding Size: Large`

2. Redesign `SearchQuery` and related result contracts in `vidlib-core`
	- support include/exclude tags, duration, size, codec, library scope, quality flags, and review state
	- **Tags:** `Complexity: High`, `Coding Size: Large`

3. Add saved searches and smart collections workflow support in `vidlib-workflows`
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

4. Upgrade the desktop UI into a real browser surface
	- table view, grid view, inspector, persistent sort/filter state, keyboard navigation
	- **Tags:** `Complexity: High`, `Coding Size: Very Large`

5. Extend `vidlib-cli` search surface for advanced filters and scripted exports
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

### Crate mapping

- `crates/vidlib-core`
- `crates/vidlib-db`
- `crates/vidlib-workflows`
- `crates/vidlib-cli`
- `src-tauri`
- `ui/`

### Schema changes

- new FTS virtual table for searchable text fields
  - filename, path, tags, notes, derived labels, codec/container text
- new `saved_searches` table
  - `id`, `name`, `query_json`, `created_at`, `updated_at`, `pinned`
- new `collections` table
  - support static and smart collections
- new `collection_members` table
  - for static collections
- optional search-related indexes on structured filter columns

### Why second

Search and browsing become the central daily interaction model. Many future premium features depend on users being able to efficiently find and work with subsets of the library.

## Milestone 3 — Rich metadata and thumbnail pipeline

**Priority:** 3
**Primary roadmap phase:** Phase 3 — Rich metadata, thumbnails, and media understanding

### Goal

Deepen media understanding so the product can support inspection, playback, review, and premium labeling workflows.

### Implementation steps

1. Expand media metadata contracts in `vidlib-core`
	- add stream-level models, audio/subtitle descriptors, bitrate/frame-rate/aspect/color fields, and diagnostics flags
	- **Tags:** `Complexity: High`, `Coding Size: Large`

2. Extend `vidlib-metadata` extraction and normalization
	- richer `ffprobe` mapping, fallback behavior, and parse resilience
	- **Tags:** `Complexity: High`, `Coding Size: Large`

3. Build preview asset generation workflows in `vidlib-workflows`
	- thumbnail generation, contact sheets, preview strips, cache refresh jobs
	- **Tags:** `Complexity: High`, `Coding Size: Large`

4. Add preview/cache persistence and lookup in `vidlib-db`
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

5. Surface thumbnails and rich inspection in `src-tauri` and `ui/`
	- **Tags:** `Complexity: High`, `Coding Size: Large`

### Crate mapping

- `crates/vidlib-core`
- `crates/vidlib-metadata`
- `crates/vidlib-workflows`
- `crates/vidlib-db`
- `src-tauri`
- `ui/`

### Schema changes

- extend `video_entries`
  - add container, bitrate, frame_rate, aspect_ratio, pixel_format, color_space, created_media_at, rotation, diagnostics flags
- new `video_streams` table
  - normalized stream-level metadata per video
- new `preview_assets` table
  - poster, contact sheet, strip, cache path, generation status, checksum, updated_at
- new `preview_cache_settings` or app settings persistence entry

### Why third

This unlocks higher-value UX: thumbnails, inspection, playback readiness, duplicate review fidelity, and better ML input quality.

## Milestone 4 — Duplicate review and canonical resolution system

**Priority:** 4
**Primary roadmap phase:** Phase 4 — Duplicate detection, similarity, and curation workflows

### Goal

Make duplicate review one of the first truly premium workflows in the application.

### Implementation steps

1. Redesign duplicate domain models in `vidlib-core`
	- group state, canonical selection, confidence, rationale, suggested action, variant relationship types
	- **Tags:** `Complexity: High`, `Coding Size: Large`

2. Extend `vidlib-duplicates` to support richer similarity strategies
	- exact, chunk, perceptual, metadata similarity, and best-version heuristics
	- **Tags:** `Complexity: High`, `Coding Size: Large`

3. Persist duplicate groups and review decisions in `vidlib-db`
	- **Tags:** `Complexity: High`, `Coding Size: Large`

4. Add duplicate review orchestration in `vidlib-workflows`
	- queue generation, review summaries, safe resolution actions
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

5. Build review UI with compare, recommend, quarantine, and batch resolve flows
	- **Tags:** `Complexity: High`, `Coding Size: Very Large`

### Crate mapping

- `crates/vidlib-core`
- `crates/vidlib-duplicates`
- `crates/vidlib-db`
- `crates/vidlib-workflows`
- `crates/vidlib-fileops`
- `src-tauri`
- `ui/`

### Schema changes

- new `duplicate_groups` table
  - `id`, `strategy`, `fingerprint`, `confidence`, `status`, `canonical_video_id`, `summary_json`, `updated_at`
- new `duplicate_group_members` table
  - `group_id`, `video_id`, `rank`, `relationship_type`
- new `duplicate_review_actions` table
  - review decisions, notes, rationale, selected action, acted_at

### Why fourth

After browse, metadata, and thumbnails exist, duplicate review becomes much more valuable and much easier to trust.

## Milestone 5 — Tag taxonomy, rules engine, and premium organization

**Priority:** 5
**Primary roadmap phase:** Phase 5 — Tagging, taxonomy, and intelligent organization

### Goal

Move beyond loose tags into a full organization system that can drive curation and automated placement.

### Implementation steps

1. Redesign tag and label contracts in `vidlib-core`
	- controlled tags, hierarchical categories, provenance, locked tags, suggestion state
	- **Tags:** `Complexity: High`, `Coding Size: Large`

2. Add taxonomy persistence and tag assignment data in `vidlib-db`
	- **Tags:** `Complexity: High`, `Coding Size: Large`

3. Build a rules engine in `vidlib-workflows`
	- derive tags, route content into collections, assign review queues, and propose organization changes
	- **Tags:** `Complexity: Very High`, `Coding Size: Very Large`

4. Expand `vidlib-fileops` to support rule/template-driven placement
	- rename/move logic by metadata, tags, date, and custom templates
	- **Tags:** `Complexity: High`, `Coding Size: Large`

5. Add batch metadata/tag editing UI and workflow surfaces
	- **Tags:** `Complexity: High`, `Coding Size: Large`

### Crate mapping

- `crates/vidlib-core`
- `crates/vidlib-db`
- `crates/vidlib-workflows`
- `crates/vidlib-fileops`
- `crates/vidlib-cli`
- `src-tauri`
- `ui/`

### Schema changes

- new `tags` table
- new `tag_assignments` table
- new `tag_taxonomy` or hierarchical parent linkage
- new `rules` table
  - rule definitions and serialized conditions/actions
- new `rule_execution_log` table
- optional `collections` reuse for rule-driven queues

### Why fifth

By this point the app can identify, search, preview, and compare. The next major value jump is organized curation at scale.

## Milestone 6 — Integrated playback and review-grade browsing

**Priority:** 6
**Primary roadmap phase:** Phase 6 — Integrated browsing, preview, and playback

### Goal

Make the desktop application a true media browsing and review environment.

### Implementation steps

1. Introduce playback abstractions and review models in `vidlib-core`
	- playback session, markers, notes, compare session contracts
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

2. Add playback orchestration in `vidlib-workflows`
	- review sessions, compare flows, marker persistence coordination
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

3. Implement desktop playback integration in `src-tauri` and `ui/`
	- embedded player, timeline controls, keyboard shortcuts, compare mode
	- **Tags:** `Complexity: Very High`, `Coding Size: Very Large`

4. Persist review markers and notes in `vidlib-db`
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

### Crate mapping

- `crates/vidlib-core`
- `crates/vidlib-workflows`
- `crates/vidlib-db`
- `src-tauri`
- `ui/`

### Schema changes

- new `playback_notes` table
- new `review_markers` table
- optional `review_sessions` table

### Why sixth

Playback is premium-value UX, but it is more effective after search, metadata, thumbnails, duplicates, and organization are already strong.

## Milestone 7 — Local ML and advanced media analysis

**Priority:** 7
**Primary roadmap phase:** Phase 7 — Media analysis and premium intelligence features

### Goal

Introduce optional offline intelligence that improves labeling, triage, and quality review.

### Implementation steps

1. Expand model and inference contracts in `vidlib-ml` and `vidlib-core`
	- model registry, capabilities, confidence, provenance, output classes
	- **Tags:** `Complexity: High`, `Coding Size: Large`

2. Build analysis job orchestration in `vidlib-workflows`
	- batch inference, retry, partial success handling, background scheduling
	- **Tags:** `Complexity: High`, `Coding Size: Large`

3. Persist analysis outputs and reviewable suggestions in `vidlib-db`
	- **Tags:** `Complexity: High`, `Coding Size: Large`

4. Add confidence-aware suggestion UX in `src-tauri` and `ui/`
	- accept/reject, provenance display, override flows
	- **Tags:** `Complexity: High`, `Coding Size: Large`

### Crate mapping

- `crates/vidlib-ml`
- `crates/vidlib-core`
- `crates/vidlib-workflows`
- `crates/vidlib-db`
- `src-tauri`
- `ui/`

### Schema changes

- new `ml_models` table
- new `analysis_results` table
- new `analysis_suggestions` table
- optional `face_clusters` / `object_detections` specialized tables later

### Why seventh

ML should enrich a mature product, not compensate for weak foundations.

## Milestone 8 — Advanced operational file workflows and editing-adjacent tooling

**Priority:** 8
**Primary roadmap phase:** Phase 8 — Editing-adjacent features and operational media tools

### Goal

Turn the app into a stronger operational workstation for cleanup, prep, and controlled media actions.

### Implementation steps

1. Expand `vidlib-fileops` planning and action model
	- rename, copy, quarantine, metadata-change preview, richer preflight validation
	- **Tags:** `Complexity: High`, `Coding Size: Large`

2. Add QC queue orchestration in `vidlib-workflows`
	- suspicious files, low quality, unreviewed, missing metadata, probable duplicates
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

3. Add queue-centric UI workflows for cleanup and prep operations
	- **Tags:** `Complexity: High`, `Coding Size: Large`

4. Optionally add controlled helper transforms later
	- clip extraction, poster frame selection, transcode queue handoff
	- **Tags:** `Complexity: Very High`, `Coding Size: Very Large`

### Crate mapping

- `crates/vidlib-fileops`
- `crates/vidlib-workflows`
- `crates/vidlib-core`
- `crates/vidlib-db`
- `src-tauri`
- `ui/`

### Schema changes

- new `review_queues` table
- new `review_queue_items` table
- extend audit tables for richer operation typing
- optional `transform_jobs` table for helper media actions

### Why eighth

These workflows are valuable, but they become much more coherent after taxonomy, playback, and ML-assisted triage exist.

## Milestone 9 — Premium UX, settings, and desktop polish

**Priority:** 9
**Primary roadmap phase:** Phase 9 — Premium desktop UX, settings, and polish

### Goal

Raise the product from powerful to polished.

### Implementation steps

1. Restructure the frontend architecture for scale
	- reusable state, routing, virtualization, shared components
	- **Tags:** `Complexity: High`, `Coding Size: Very Large`

2. Add comprehensive settings management across `src-tauri`, `ui/`, and persistence
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

3. Add progress center, notifications, onboarding, and improved recovery UX
	- **Tags:** `Complexity: Medium`, `Coding Size: Large`

### Crate mapping

- `src-tauri`
- `ui/`
- `crates/vidlib-core`
- `crates/vidlib-db`

### Schema changes

- new `app_settings` table or config store
- new `user_preferences` table if separated
- optional `notification_log` table

### Why ninth

Polish should follow depth. Once the core workflows are strong, polish compounds value significantly.

## Milestone 10 — Production readiness, packaging, diagnostics, and quality hardening

**Priority:** 10
**Primary roadmap phase:** Phase 10 — Production readiness, packaging, and enterprise supportability

### Goal

Make the application shippable and supportable as a serious product.

### Implementation steps

1. Add structured diagnostics and support bundle generation
	- logs, environment info, health summaries, repair helpers
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

2. Formalize migration testing and recovery tools in `vidlib-db`
	- **Tags:** `Complexity: High`, `Coding Size: Large`

3. Add installer/update/release pipeline work
	- **Tags:** `Complexity: High`, `Coding Size: Large`

4. Raise automated test maturity across the workspace
	- large-library tests, failure injection, UI smoke/regression, workflow integration coverage
	- **Tags:** `Complexity: Very High`, `Coding Size: Massive`

### Crate mapping

- `crates/vidlib-db`
- `crates/vidlib-core`
- `crates/vidlib-workflows`
- `crates/vidlib-fileops`
- `crates/vidlib-cli`
- `src-tauri`
- `tests/`

### Schema changes

- minimal direct schema change likely
- possible `diagnostic_reports` table if in-app history is desired
- possible migration/version audit table expansion

### Why tenth

This milestone is essential before broad release, but most of its value compounds after the product workflows are already substantial.

## Milestone 11 — Automation and extension surfaces

**Priority:** 11
**Primary roadmap phase:** Phase 11 — Automation, extensibility, and ecosystem growth

### Goal

Open the platform for power-user automation and future ecosystem growth without destabilizing the core product.

### Implementation steps

1. Expand CLI automation coverage in `vidlib-cli`
	- reporting, batch exports, scheduled workflow support, smart collection output
	- **Tags:** `Complexity: Medium`, `Coding Size: Medium`

2. Formalize extension boundaries in `vidlib-core` and `vidlib-workflows`
	- metadata providers, analyzers, duplicate strategies, import/export adapters, rules packs
	- **Tags:** `Complexity: High`, `Coding Size: Large`

3. Optionally add local API/plugin surface later
	- **Tags:** `Complexity: Very High`, `Coding Size: Very Large`

### Crate mapping

- `crates/vidlib-core`
- `crates/vidlib-workflows`
- `crates/vidlib-cli`
- `crates/vidlib-metadata`
- `crates/vidlib-duplicates`
- `crates/vidlib-ml`

### Schema changes

- likely minimal initially
- optional `import_export_profiles` table
- optional plugin/adapter registration metadata if in-app management is introduced

### Why eleventh

Extension points are strongest when the internal product workflows are already mature and stable.

---

## 9. Roadmap phase to crate and schema mapping summary

## Phase 1 — Stabilize the platform foundation

- **Primary crates:** `vidlib-core`, `vidlib-db`, `vidlib-workflows`, `vidlib-scanner`, `vidlib-cli`, `src-tauri`
- **Primary schema changes:** `jobs`, `job_checkpoints`, `job_events`, enhanced `scan_checkpoints`, optional `library_status`

## Phase 2 — Search and browsing engine

- **Primary crates:** `vidlib-core`, `vidlib-db`, `vidlib-workflows`, `vidlib-cli`, `src-tauri`, `ui/`
- **Primary schema changes:** FTS virtual tables, `saved_searches`, `collections`, `collection_members`, structured filter indexes

## Phase 3 — Metadata, thumbnails, and media understanding

- **Primary crates:** `vidlib-core`, `vidlib-metadata`, `vidlib-db`, `vidlib-workflows`, `src-tauri`, `ui/`
- **Primary schema changes:** extended `video_entries`, `video_streams`, `preview_assets`

## Phase 4 — Duplicate detection and curation

- **Primary crates:** `vidlib-core`, `vidlib-duplicates`, `vidlib-db`, `vidlib-workflows`, `vidlib-fileops`, `src-tauri`, `ui/`
- **Primary schema changes:** `duplicate_groups`, `duplicate_group_members`, `duplicate_review_actions`

## Phase 5 — Tagging, taxonomy, and intelligent organization

- **Primary crates:** `vidlib-core`, `vidlib-db`, `vidlib-workflows`, `vidlib-fileops`, `vidlib-cli`, `src-tauri`, `ui/`
- **Primary schema changes:** `tags`, `tag_assignments`, taxonomy hierarchy, `rules`, `rule_execution_log`

## Phase 6 — Integrated playback and review tools

- **Primary crates:** `vidlib-core`, `vidlib-workflows`, `vidlib-db`, `src-tauri`, `ui/`
- **Primary schema changes:** `playback_notes`, `review_markers`, optional `review_sessions`

## Phase 7 — Media analysis and premium intelligence

- **Primary crates:** `vidlib-ml`, `vidlib-core`, `vidlib-workflows`, `vidlib-db`, `src-tauri`, `ui/`
- **Primary schema changes:** `ml_models`, `analysis_results`, `analysis_suggestions`, optional specialized analysis tables

## Phase 8 — Editing-adjacent and operational media tools

- **Primary crates:** `vidlib-fileops`, `vidlib-workflows`, `vidlib-core`, `vidlib-db`, `src-tauri`, `ui/`
- **Primary schema changes:** `review_queues`, `review_queue_items`, extended audit records, optional `transform_jobs`

## Phase 9 — Premium desktop UX and polish

- **Primary crates:** `src-tauri`, `ui/`, `vidlib-core`, `vidlib-db`
- **Primary schema changes:** `app_settings`, optional `user_preferences`, optional `notification_log`

## Phase 10 — Production readiness and supportability

- **Primary crates:** workspace-wide, especially `vidlib-db`, `vidlib-workflows`, `src-tauri`, `tests/`
- **Primary schema changes:** likely minimal; possible diagnostics/audit expansion

## Phase 11 — Automation and ecosystem growth

- **Primary crates:** `vidlib-core`, `vidlib-workflows`, `vidlib-cli`, `vidlib-metadata`, `vidlib-duplicates`, `vidlib-ml`
- **Primary schema changes:** likely minimal initially; optional import/export or plugin-management metadata later

---

## 10. Non-negotiable product principles

These should remain true throughout the roadmap.

- keep core logic in reusable crates, not entrypoints
- keep `vidlib-core` free of runtime-specific concerns
- preserve preview-first and rollback-first file safety
- treat media files and metadata as untrusted inputs
- prefer local-first features over cloud-first shortcuts
- avoid over-coupling UI concerns into feature crates
- validate on the workspace root with formatting, linting, and tests

---

## 11. Definition of success

VidLibOrganizer succeeds when it is no longer just a useful Rust project, but a dependable premium desktop program that a serious user can trust with a large and valuable video collection.

That means:

- it scales well
- it is safe to operate
- it is pleasant to use daily
- it provides elite browsing and organization workflows
- it offers meaningful preview, playback, review, and analysis features
- it remains local-first and privacy-respecting
- it is engineered like a product, not just a prototype

In short: the destination is a professional-grade, all-in-one video library platform built on the strong Rust workspace foundation already present in this repository.
- tests for naming template determinism

### Success criteria

- users trust the tool for large-scale reorganization
- partial failures do not leave the library in an unclear state

---

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

## Phase 6 — Architecture hardening and release readiness

### Objective

Prepare the workspace for sustained feature growth and public release quality.

### Problems being solved

- orchestration logic still risks drifting between entrypoints
- some modules will grow quickly as features land
- release engineering and observability are still thin

### Deliverables

#### 1. Shared workflow boundary

Complete migration of workflow orchestration into `vidlib-workflows`.

#### 2. Service-oriented module structure

Introduce clear service layers for:

- search
- scan orchestration
- duplicate review
- restructure planning
- metadata enrichment

#### 3. Workspace quality gates

Add or tighten:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- targeted integration tests for key workflows

#### 4. Packaging and release readiness

Plan for:

- installer generation
- app settings persistence
- user data migration rules
- release notes and changelog process

### Success criteria

- entrypoints are thin and stable
- the codebase has clear ownership and test boundaries
- releases are predictable and low-risk

---

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

## Cross-cutting workstreams

These should progress alongside the main phases.

### A. Testing and validation

- add regression tests for all bug fixes
- maintain workspace-level validation commands
- add synthetic-library fixtures for large-scale tests

### B. Performance

- benchmark scan throughput
- benchmark search latency with realistic datasets
- reduce duplicate unnecessary file reads

### C. UX and product polish

- clarify terminology in UI and CLI
- standardize progress and error messaging
- improve dashboard usefulness over time

### D. Documentation

- keep `README.MD` aligned with product capabilities
- document crate contracts as architecture evolves
- add operator docs for destructive workflows and recovery

## Quarter-style roadmap view

### Milestone A

- create `vidlib-workflows`
- implement real scan job model
- wire shared scan orchestration into CLI and Tauri

### Milestone B

- add FTS search
- implement richer filters
- improve dashboard and search UX

### Milestone C

- add duplicate review model and UI
- ship safe duplicate actions and review states

### Milestone D

- expand metadata
- add thumbnails and sampled frames
- integrate first ONNX-assisted tagging path

### Milestone E

- harden restructure policies
- improve rollback and preview flows
- prepare packaging and release readiness

## Recommended immediate next tasks

1. Create `crates/vidlib-workflows`
2. Move current scan orchestration out of `vidlib-cli` into the new crate
3. Refactor `src-tauri` scan command to call the same workflow
4. Add workflow tests for start, cancel, and resume
5. Extend `EXPANSION_PLAN.md` later with issue links and status tracking

---

## Tracked execution plan

This section translates the roadmap into a practical implementation backlog.

### Priority model

- **P0** — foundational, blocks multiple future features
- **P1** — high-value core capability
- **P2** — important product improvement
- **P3** — polish, scale, or release hardening

### Effort guide

- **S** — small, likely 0.5 to 1 day
- **M** — medium, likely 2 to 4 days
- **L** — large, likely 1 to 2 weeks
- **XL** — multi-milestone effort

## Backlog by milestone

### Milestone A — Shared workflow orchestration

#### A1. Create workflow crate
- Priority: **P0**
- Effort: **M**
- Status: Not started
- Description: Create `crates/vidlib-workflows` and establish shared orchestration APIs.
- Target files/crates:
	- `Cargo.toml`
	- `crates/vidlib-workflows/Cargo.toml`
	- `crates/vidlib-workflows/src/lib.rs`
	- `docs/crate-contracts.md`

#### A2. Add scan workflow service
- Priority: **P0**
- Effort: **L**
- Status: Not started
- Description: Move scan/index orchestration out of entrypoints into a reusable workflow service.
- Target files/crates:
	- `crates/vidlib-workflows/src/scan.rs`
	- `crates/vidlib-cli/src/main.rs`
	- `src-tauri/src/main.rs`
	- `crates/vidlib-db/src/lib.rs`

#### A3. Introduce scan job state model
- Priority: **P0**
- Effort: **M**
- Status: Not started
- Description: Define job ids, lifecycle state, summary, and progress event types.
- Target files/crates:
	- `crates/vidlib-core/src/models.rs`
	- or `crates/vidlib-workflows/src/models.rs`

#### A4. Implement cancellation registry
- Priority: **P0**
- Effort: **M**
- Status: Not started
- Description: Track active jobs and wire real cancellation to running scan work.
- Target files/crates:
	- `crates/vidlib-workflows/src/jobs.rs`
	- `src-tauri/src/main.rs`

#### A5. Persist scan jobs and progress snapshots
- Priority: **P1**
- Effort: **L**
- Status: Not started
- Description: Extend DB for scan job history and progress tracking.
- Target files/crates:
	- `crates/vidlib-db/src/lib.rs`
	- `crates/vidlib-core/src/models.rs`

#### A6. Add workflow tests
- Priority: **P0**
- Effort: **M**
- Status: Not started
- Description: Cover start, cancel, resume, and checkpoint restore flows.
- Target files/crates:
	- `crates/vidlib-workflows/src/*.rs`
	- `tests/workflow_integration.rs`

### Milestone B — Search and discovery

#### B1. Add FTS schema and indexing
- Priority: **P1**
- Effort: **L**
- Status: Not started
- Description: Add SQLite FTS tables and indexing strategy for search-heavy fields.
- Target files/crates:
	- `crates/vidlib-db/src/lib.rs`
	- migration helpers inside `vidlib-db`

#### B2. Expand search query model
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Support richer filters for metadata, duplicates, and library scope.
- Target files/crates:
	- `crates/vidlib-core/src/models.rs`
	- `crates/vidlib-db/src/lib.rs`

#### B3. Add ranked search API
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Return ranked search results and optional snippets or scores.
- Target files/crates:
	- `crates/vidlib-db/src/lib.rs`
	- `crates/vidlib-cli/src/main.rs`
	- `src-tauri/src/main.rs`

#### B4. Add saved searches
- Priority: **P2**
- Effort: **M**
- Status: Not started
- Description: Persist named and recent searches for reuse.
- Target files/crates:
	- `crates/vidlib-db/src/lib.rs`
	- `crates/vidlib-core/src/models.rs`
	- UI files under `ui/`

### Milestone C — Duplicate review workflow

#### C1. Add duplicate review state model
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Model canonical picks, ignored groups, and review status.
- Target files/crates:
	- `crates/vidlib-core/src/models.rs`
	- `crates/vidlib-db/src/lib.rs`

#### C2. Implement keep-best heuristics
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Rank duplicate candidates using metadata and quality rules.
- Target files/crates:
	- `crates/vidlib-duplicates/src/lib.rs`

#### C3. Add safe duplicate actions
- Priority: **P1**
- Effort: **L**
- Status: Not started
- Description: Preview and execute move/quarantine/delete flows safely.
- Target files/crates:
	- `crates/vidlib-fileops/src/lib.rs`
	- `crates/vidlib-workflows/src/duplicates.rs`

#### C4. Build duplicate review UI
- Priority: **P2**
- Effort: **L**
- Status: Not started
- Description: Add group comparison and review actions to desktop UI.
- Target files/crates:
	- `ui/index.html`
	- `ui/main.js`
	- `ui/styles.css`
	- `src-tauri/src/main.rs`

### Milestone D — Metadata and ML enrichment

#### D1. Expand metadata extraction schema
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Capture additional audio/video/container attributes.
- Target files/crates:
	- `crates/vidlib-metadata/src/lib.rs`
	- `crates/vidlib-core/src/models.rs`

#### D2. Add thumbnail generation pipeline
- Priority: **P2**
- Effort: **L**
- Status: Not started
- Description: Generate sampled frames and thumbnails for previews.
- Target files/crates:
	- `crates/vidlib-metadata/src/lib.rs`
	- new thumbnail module or crate if needed

#### D3. Build ONNX model configuration path
- Priority: **P2**
- Effort: **L**
- Status: Not started
- Description: Add model configuration, thresholds, and runtime selection.
- Target files/crates:
	- `crates/vidlib-ml/src/lib.rs`
	- config handling in CLI/Tauri

#### D4. Add tag provenance and rule system
- Priority: **P2**
- Effort: **L**
- Status: Not started
- Description: Distinguish manual, inferred, and locked tags with rules.
- Target files/crates:
	- `crates/vidlib-core/src/models.rs`
	- `crates/vidlib-db/src/lib.rs`
	- `crates/vidlib-ml/src/lib.rs`

### Milestone E — Restructure hardening and release readiness

#### E1. Add naming policy engine
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Support template-driven and rule-driven destination planning.
- Target files/crates:
	- `crates/vidlib-fileops/src/lib.rs`

#### E2. Add conflict resolution policies
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Support rename, skip, quarantine, and confirm policies.
- Target files/crates:
	- `crates/vidlib-fileops/src/lib.rs`
	- `crates/vidlib-core/src/models.rs`

#### E3. Strengthen rollback verification
- Priority: **P1**
- Effort: **M**
- Status: Not started
- Description: Validate apply/undo integrity and log consistency.
- Target files/crates:
	- `crates/vidlib-fileops/src/lib.rs`
	- `crates/vidlib-db/src/lib.rs`

#### E4. Prepare release pipeline
- Priority: **P3**
- Effort: **L**
- Status: Not started
- Description: Add packaging, validation gates, and release documentation.
- Target files/crates:
	- workspace root CI/config files
	- `README.MD`
	- Tauri packaging config under `src-tauri/`

## Suggested issue templates

Use issue titles in this form:

- `workflow: extract scan orchestration into vidlib-workflows`
- `db: add persisted scan job records`
- `search: add SQLite FTS index for video discovery`
- `duplicates: add canonical selection heuristics`
- `fileops: add collision resolution policies`
- `ml: add ONNX model configuration support`

Each issue should include:

- problem statement
- target crate(s)
- public API changes
- migration or schema impact
- tests required
- acceptance criteria

## Suggested sprint order

### Sprint 1
- A1 create workflow crate
- A2 extract scan workflow
- A3 add scan job state model

### Sprint 2
- A4 cancellation registry
- A5 persisted scan jobs
- A6 workflow tests

### Sprint 3
- B1 FTS schema
- B2 query model expansion
- B3 ranked search API

### Sprint 4
- C1 duplicate review state
- C2 keep-best heuristics
- C3 safe duplicate actions

### Sprint 5
- D1 richer metadata
- D2 thumbnail pipeline
- E1 naming policy engine

### Sprint 6
- D3 ONNX config path
- D4 tag provenance and rules
- E2 conflict resolution policies
- E3 rollback verification

## Recommended next implementation target

If work starts immediately, begin with **A1 + A2**:

- create `vidlib-workflows`
- move scan orchestration into it
- make both CLI and Tauri consume the same scan workflow API

That unlocks the cleanest path for all later roadmap items.

## Recommended implementation order

1. Real scan job and cancellation architecture
2. FTS search and richer filters
3. Duplicate review workflow
4. Safer restructure engine and UI
5. Rich metadata and ONNX tagging
6. `vidlib-workflows` orchestration refactor

## Suggested next milestone

Create a new `vidlib-workflows` crate and move scan orchestration out of `vidlib-cli` and `src-tauri` so both entrypoints share one workflow implementation.
