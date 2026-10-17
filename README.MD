# VidLibOrganizer

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

## Development

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build -p vidlib-tauri
```

- `crates/vidlib-engine`: traversal, size/MD5 grouping and cancellation.
- `crates/vidlib-fileops`: validated move/delete operations.
- `src-tauri`: background jobs, saved folders and desktop integration.
- `ui`: accessible, dependency-free HTML/CSS/JavaScript interface.

Open `ui/index.html` through a static HTTP server to inspect the interface; its explicit preview uses example data and cannot touch files. The desktop application is required for real library access. The old metadata/ML prototype has been retired; its history remains in Git. See `docs/spec.md` and `docs/implementation-plan.md` for the current design.
