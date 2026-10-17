# Filesystem capability evidence — Task 02

Recorded 2026-10-17. This is a capability gate for native primitives, not completion
of Rename, its journal, recovery, undo, or the Windows release.

## Observed environment

- Linux x86_64, kernel `6.18.55` (`uname -a`: host `f338b284a8cc`).
- rustix 1.1.4; `renameat2(RENAME_NOREPLACE)`, descriptor-relative `openat`,
  `mkdirat`, `unlinkat`, `futimens`, `fsync`.
- `/tmp` and `/dev/shm`: **tmpfs**, magic `0x01021994`.
- Repository/`target/task02-overlay-tmp`: **overlayfs**, magic `0x794c7630`.
- `/proc`: procfs, magic `0x9fa0`, explicitly unsupported for mutation.
- Windows crate 0.62.2, Rust target `x86_64-pc-windows-gnu`, workspace-local MinGW
  SDK. Cargo check/Clippy of all fileops and inventory test targets succeeded.
  **No Windows binary or filesystem test ran.**

## Matrix

| OS/filesystem | Native evidence | Production mutation gate | Remaining limits |
| --- | --- | --- | --- |
| Linux/tmpfs | 18 platform tests pass using `/tmp`; explicit `/dev/shm` collision/success fixture | Enabled; syscall errors still fail closed | Volatile memory filesystem; no reboot/power-loss persistence |
| Linux/overlayfs | Same 18 platform tests pass with `TMPDIR` under workspace `target` | Enabled; syscall errors still fail closed | Lower/upper storage durability and locality unknown |
| Linux/procfs | Namespace facts retain unknown mutation/name policy; creation returns `Unsupported` | Disabled | Read-only observations only through this adapter |
| Linux/ext4, XFS, Btrfs, other local filesystems | Not mounted/tested here | Disabled | Add native matrix evidence before enabling families |
| Linux/network filesystems, FAT/exFAT, unknown filesystems | Not tested | Disabled | No guessed atomicity, case policy, or durability |
| Windows/NTFS files and directories | **Pending**; bindings/tests compile only | Disabled (`Unsupported`) | Collision, case-only, locking, per-directory case policy, metadata, flush and reparse runtime tests pending |
| Windows/exFAT/removable | **Pending**, no attached volume | Disabled | Record actual API results or explicit unsupported outcomes on native Windows |
| Windows/SMB/share | **Pending**, no share | Disabled | Server semantics, identities, disconnect/locking/flush behavior unverified |
| Other OS targets | No verified adapter | Disabled | Filesystem methods return typed `Unsupported`; unsupported inventory targets may impose their own compile limitations |

Enabled Linux family facts are conservative planning evidence, not a promise that
an individual kernel call will succeed. `ENOSYS`, `EINVAL`, and `EOPNOTSUPP` from
no-replace rename become `Unsupported`; cross-device rename becomes `CrossDevice`.
There is no check-then-overwriting-rename fallback. Every operation retains native
error handling, including read-only mounts, permissions, and existing names.

## Boundaries and invariants

Paths stay as native `PathBuf`/`NativePathRecord`. Every candidate is checked
against native source grants before parent deduplication; all relevant grant IDs
are retained. Captures list native names even when stat fails and report incomplete
coverage. They create no marker, staging entry, or other probe file. Existing
protected markers/staging names remain visible in namespace observations.

Each parent gets its own case, normalization, native component length, name policy,
reserved names, and capability facts. Shared `DirectoryFacts::validate_name` lets
the pure planner and adapter apply the same native syntax without importing one
another. Unknown policy fails closed. Windows per-directory case information is
queried through `FileCaseSensitiveInfo`; query failure means `Unknown`, never a
volume-wide guess. Windows device names, extension variants, forbidden characters,
trailing dots/spaces, native UTF-16 lengths, and unpaired surrogates are handled in
native units. Linux lengths are byte lengths. Unknown whole-path limits remain
`None`; there is no invented PATH_MAX for descriptor-relative Linux operations.

Linux traverses every ancestor with held directory descriptors and `O_NOFOLLOW`.
Source freshness/identity and parent bindings are rechecked before mutation, with
post-rename identity and binding checks. File/directory no-replace collisions,
including symlink destinations, retain both source and destination. Windows uses
handle-relative `NtCreateFile` with `FILE_OPEN_REPARSE_POINT`, handle information,
and `SetFileInformationByHandle(FileRenameInfo)` with `ReplaceIfExists=false`.
Its native implementation remains unreachable from public mutation methods until
qualification explicitly enables a volume capability; there is no public bypass.

Linux `renameat2` cannot atomically compare a source basename against an expected
inode. The analogous basename check-to-unlink gap and mkdir-to-open gap also exist.
Held handles and immediate/pre/post checks reduce these races; they do not claim
immunity to an uncooperative privileged process replacing entries during the
syscall boundary. Unexpected observed poststate returns `RecoveryRequired`, with
no blind retry or overwriting compensation. A parent that changes binding after
an applied operation also returns `RecoveryRequired`. A directory creation whose
post-create identity cannot be captured is unresolved, not reported as absent.

Staging creation uses exclusive create, with exact path and native identity in
`OwnedEntry`. `refresh_staging` checks the still-open content handle against its
recorded identity, verifies the path still names that object, refreshes size/times,
and reports file/parent flush outcomes. It must precede publication after writing;
callers must not replace ownership with an unrelated pathname observation.
Publication uses the same atomic no-replace move. Cleanup checks recorded state
and only removes an empty owned directory; it never recursively deletes a tree.
Explicit protected roots remain protected even within canonical staging names.

`open_verified` supplies the held native file for streaming content I/O. Higher
copy layers must verify byte count/digest and reverify source state after reading;
this primitive does not claim a whole-file hash or frozen content snapshot.
Creation returns ownership before parent flush; callers use `flush_directory` for
a new directory and `refresh_staging` for a written staging file, and journal these
phases explicitly. The seam contains no recipe or execution-step types.

Linux modification/access time and readonly setters are tested with exact
nanosecond observations. Creation time and Windows attributes are unsupported on
Linux, rejected before mutation. Clearing readonly restores owner-write only,
without granting group/world write. Windows `FILE_BASIC_INFO` setters and
`FlushFileBuffers` compile but remain **runtime unverified** and gated off;
Windows timestamp requests must be exactly representable in 100 ns ticks.

`Durability::Synced` means the relevant OS flush call acknowledged success.
`Unsupported` and `Failed(message)` are distinct outcomes, including after an
otherwise applied mutation; a flush failure does not fabricate a rolled-back
operation. `FilesystemCapabilities::durable_flush` remains `Unknown` because no
hardware durability was established. Successful tmpfs fsync is not persistence.

Process-termination recovery will be implemented/proved by the journal and
reconciliation tasks. This task proves native primitives only. Neither a future
SQLite journal nor successful fsync makes a whole batch atomic or proves
hardware/power-loss survival. Unresolved operation state requires reconciliation
by identity, not a guessed durable receipt. The existing duplicate fileops API
and behavior are preserved; this task does not retroactively claim new journal
or adapter guarantees for that interface.

## Reproduction and pending native Windows qualification

```sh
export CARGO_HOME=/workspace/.tools/cargo
export RUSTUP_HOME=/workspace/.tools/rustup
export PATH=/workspace/.tools/cargo/bin:$PATH
cargo test -p vidlib-engine -p vidlib-fileops -p vidlib-inventory
TMPDIR="$PWD/target/task02-overlay-tmp" cargo test -p vidlib-fileops --test platform
cargo clippy -p vidlib-engine -p vidlib-fileops -p vidlib-inventory --all-targets -- -D warnings
cargo fmt --all -- --check
source /workspace/.tools/windows-check-env.sh
cargo clippy -p vidlib-fileops -p vidlib-inventory --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

The platform suite has 18 named integration tests, plus the held-parent rebinding
unit test. The complete portable suite has 82 tests. On Linux,
`cargo test -p vidlib-fileops --test windows_platform -- --nocapture` executes
**zero** tests; it is not Windows evidence.

Run on real Windows, separately on selected NTFS, exFAT/removable, and share
locations (set TEMP/TMP to a disposable fixture directory on each volume):

```text
cargo test -p vidlib-fileops --test windows_platform -- --nocapture
cargo test -p vidlib-fileops --lib platform::windows::qualification -- --nocapture
cargo test -p vidlib-fileops --lib platform::windows::qualification -- --ignored --nocapture
```

The private native qualification tests exercise collisions for files/folders,
late arrival, locked source, and case-only/directory identity. The two explicitly
ignored tests require NTFS case-setting permission and symlink/developer-mode
permission respectively; run them explicitly and record real results or explicit
unsupported outcomes. The public Windows integration tests guard the disabled
production capability gate. Passing qualification tests does not automatically
enable the gate: reviewed evidence and a deliberate capability update are required.
Windows metadata/flush/removable/share qualification beyond these fixtures remains
pending. No native Windows red or green result is claimed in this Linux session.
