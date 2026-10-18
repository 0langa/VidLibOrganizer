# VidLibOrganizer

Cloud checkpoint paused for local continuation: [current handoff](docs/verification/rename-local-handoff.md). Six Rename phase findings remain open; Windows release qualification is pending.


A local Windows desktop app for organizing large video libraries. Add folders from your computer or external drives, scan their subfolders, review duplicate groups, and decide which copies to keep, move or delete.

## Run

Double-click `Launch.cmd`, or run:

```powershell
cargo run --release -p vidlib-tauri
```

The first launch compiles the application. Rust with the MSVC toolchain, Visual Studio C++ Build Tools, and Microsoft Edge WebView2 are required. Once built, `target\release\vidlib-tauri.exe` runs directly without Cargo. No Node.js, ffmpeg, accounts or cloud service is required. The release build optimizes MD5 throughput; for development use `cargo run -p vidlib-tauri`.

## Find duplicates

1. Add one or more folders in **Library folders**, using Browse or a full path. Folders are remembered locally, including paths on disconnected drives.
2. Open **Duplicate finder**, select folders and choose a mode.
3. **Fast / file size** compares exact byte lengths without reading video contents. It finds candidates: different videos can have the same size.
4. **Deep / MD5** reads the complete contents of same-sized candidates. Files with unique sizes are skipped because they cannot be exact duplicates. Memory use is bounded; files are read sequentially to suit external drives.
5. Review groups, search by filename/path and open files in their containing folder. Reports can be exported as JSON.
6. Select unwanted copies explicitly, then **Move selected** or **Delete selected**. The final confirmation lists the affected files and bytes. **Keep all** leaves a group untouched.

A group always retains at least one copy. The app checks that selected files and a retained copy still match the scan before changing anything. MD5 scans recheck hashes before actions. Move verifies the destination copy before removing the original; existing destination files are never overwritten. Permanent deletion bypasses the Recycle Bin.

Scans show discovery counts and hashing progress and can be cancelled. Unavailable drives, unreadable files and files that change during hashing are reported. Partial results are labelled. Symbolic links, Windows reparse points/junctions and zero-byte files are skipped. Overlapping folders are scanned once. Supported video extensions are defined in `crates/vidlib-engine/src/lib.rs`.

## Scope and limits

This release finds exact copies, not visually similar videos, alternate encodes or different resolutions. MD5 is for ordinary accidental duplicates, not authentication of adversarial files. Redundant byte totals are estimates: hard links, sparse files and filesystem compression can mean less disk space is reclaimed. Source files should remain unchanged during scans and file actions; metadata and hash checks reduce stale-result risks but are not filesystem snapshots.

Scan results and Keep-all decisions last for the current session. Saved folders live in the app configuration directory (`%APPDATA%\com.vidliborganizer.app\libraries.json` on Windows). Files stay on your computer. No telemetry or network requests are used by the running application.

## Rename (implemented; Windows release qualification pending)

The Rename view supports ordered text/regex/number/date/template rules, explicit selection and numbering scopes, mappings, versioned presets, optional companion ownership, folder layouts and explicit Copy/Move destinations. Preview and confirmation use backend-owned plans and exact native grants; edits require a new preview. Table pages contain at most 200 rows and conflicts include all selected items. History, recovery and validated Undo use the durable app-local journal; a batch is recoverable, not an atomic filesystem transaction.

Native picker/drop/Explorer arguments only select items for review. Explorer integration is optional at interactive installation and silent installs do not opt in. Metadata flags/timestamps default to Keep. Renaming an extension does not convert media. Blueprint remains a later optional feature and is not implemented at this checkpoint.

**Windows mutation gates remain closed pending native qualification.** This branch is a reviewable Rename implementation, not a qualified Windows release. See [the evidence and pending gates](docs/verification/rename.md) and [resumable phase handoff](docs/verification/rename-handoff.md). Generated intermediate names have a 64 KiB UTF-8 resource budget; recipes that exceed it before a later shrink are refused. Recipe imports are bounded at 1 MiB / 100 rules; large runtime selection bindings currently also count toward that preparation cap. Mapping imports are bounded at 64 MiB / 100,000 rows.

## Development

Use Rust 1.99.1, Node 24.19.0 and committed lockfiles. Windows builds need MSVC C++ Build Tools and the Windows SDK. Node/Playwright are development tools, not installed-app runtime requirements.

```powershell
cargo test --locked --workspace --no-fail-fast
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm ci
npx playwright install chromium
npm test
python tools/vendor_media_info.py --check
cargo build --locked --release -p vidlib-tauri
./tools/benchmark-library.ps1
```

MediaInfo resources and notices are shipped and hash-verified offline. `tools/vendor_media_info.py --fetch` is an explicit build-time hydration action; a local `--archive` never implicitly fetches a missing notice. The NSIS configuration uses the WebView2 offline installer; a real Windows bundle build must embed and verify it. `cargo build` alone does not create the installer. The documented Windows packaging command is `npx @tauri-apps/cli@2.12.3 build --bundles nsis`; actual clean-machine offline installation remains a pending release gate.

- `crates/vidlib-engine`: existing duplicate traversal/grouping/cancellation.
- `crates/vidlib-inventory`: native observations, paths, grants and bounded progress.
- `crates/vidlib-rename`: authoritative pure recipe/planning logic and read-only input preparation.
- `crates/vidlib-fileops`: no-replace execution, journal, recovery, Undo and legacy duplicate actions.
- `src-tauri`: native selection/helpers, app-local state, jobs and desktop commands.
- `ui`: dependency-free HTML/CSS/JavaScript interface; browser samples cannot mutate files.

Run `npm run dev` to inspect the browser interface. Tests and fixture generators live outside packaged `ui`. Linux portable CI excludes the desktop package; local full desktop checks require GTK3/WebKitGTK4.1 development libraries. See the verification document for precise cloud setup, separate Windows cfg checks, scale reproduction, supported filesystem gates and limits.
