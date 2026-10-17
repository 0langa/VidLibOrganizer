# Verification

## Automated checks
- `cargo test --workspace`: 42 passing tests (14 scan fixtures, 23 file-operation tests, 5 desktop state/preferences tests).
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `node --check` on every UI JavaScript module: passed.
- `cargo build --release -p vidlib-tauri`: passed; optimized executable produced.
- Independent backend review: identified lost modification timestamps during moves; fixed with a failing-then-passing regression test.

Windows junction fixtures require a test process with permission to create junctions. Running these inside the restricted automation sandbox fails with access denied; running the suite with the approved host permissions passes. No tests use the user's media library.

## Desktop smoke test
Exercised the actual Windows app with a disposable four-file fixture library:
- Added a folder through the UI and verified it persisted across app restart.
- Fast mode found one group containing three files of identical byte length.
- MD5 mode correctly excluded the same-sized different-content file and retained the two identical files, showing the known MD5 for `abc`.
- Selecting one of two copies disabled selection of the last retained copy.
- The permanent-deletion confirmation listed the exact selected path and bytes; cancellation left the file untouched. Actual permanent deletion is exercised by automated real-filesystem tests.
- Keep all cleared selection and disabled group checkboxes; Undo restored review.
- Open containing folder opened the correct Windows Explorer directory.
- Move selected opened the native folder chooser and confirmation; the selected fixture reached the destination with intact contents, the source was removed, the keeper remained, and the result group disappeared.
- The optimized executable launches and loads saved folders.
- Removed the disposable folder from saved libraries and verified the configuration returned to its original empty list. The optimized application remains open, ready for the user's folders.

The Codex browser preview could not reach Windows localhost, so visual and interaction checks used the native desktop app. No 1–3 TB library benchmark or physical external-drive disconnect test was performed. Cross-volume moves use the same verified streaming copy path as same-volume moves; the tests exercise that path with disposable local folders.
