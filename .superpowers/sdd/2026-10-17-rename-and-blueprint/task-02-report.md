# Task 02 report — no-replace filesystem adapters

Status: DONE_WITH_CONCERNS. Linux portable implementation and tests complete; Windows native qualification remains pending and production mutations fail closed. Canonical checklist was not edited. No helpers/subagents were spawned.

Base: `437ca1c475fa37800b280f58c9ba40dbd5bfbd21`.
Implementation commit: `73283568eae9fdb5b12bd0d1db87ab51f0f6cb0d` (`feat: add verified no-replace filesystem adapters`).

Report commit: separate documentation commit immediately following the implementation; exact hash is returned in the controller handoff.
Plan commit message: `feat: add verified no-replace filesystem adapters`.

## Scope and dependency boundary

Added `vidlib-fileops::error` and `platform/{mod,linux,windows}.rs`, platform integration tests and `docs/verification/filesystem-capabilities.md`. Existing duplicate public functions and behavior were preserved; lib.rs only gains module/reexports. The fileops manifest/lockfile add inventory, Linux-only rustix and Windows-only windows dependencies. Root Cargo.toml unchanged. Controller explicitly authorized the narrow inventory `DirectoryFacts` completion: `NamePolicy`, `NameViolation`, `name_policy`, `reserved_names`, and pure `validate_name`. No rename recipe/step type is imported.

## Exact public interfaces

The following is the actual interface declaration in this commit (including method arguments and return types):

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Durability {
    Synced,
    Unsupported,
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct MutationOutcome {
    pub observation: FileObservation,
    pub durability: Durability,
}
#[derive(Clone, Debug, Default)]
pub struct MetadataUpdate {
    pub modified_ns: Option<i128>,
    pub accessed_ns: Option<i128>,
    pub created_ns: Option<i128>,
    pub readonly: Option<bool>,
    pub windows_attributes: Option<u32>,
}
#[derive(Debug)]
pub struct OwnedEntry {
    pub path: PathBuf,
    pub stamp: FileStamp,
}
#[derive(Debug)]
pub struct StagingFile {
    pub ownership: OwnedEntry,
    pub file: File,
}

/// Every implementation must reject uncertain capability, preserve a conflicting
/// destination, and verify fresh native identity before an operation.
/// Journal/coordinator authorization is the caller's responsibility.
pub trait Filesystem {
    fn capture_namespace(
        &self,
        grants: &GrantSet,
        candidates: &[PathBuf],
    ) -> Result<NamespaceSnapshot, OpError>;
    fn inspect(&self, path: &Path) -> Result<FileObservation, OpError>;
    fn verify(&self, path: &Path, expected: &FileStamp) -> Result<FileObservation, OpError>;
    fn rename_no_replace(
        &mut self,
        source: &Path,
        target: &Path,
        expected: &FileStamp,
    ) -> Result<MutationOutcome, OpError>;
    fn publish_staging(
        &mut self,
        owned: &OwnedEntry,
        target: &Path,
    ) -> Result<MutationOutcome, OpError>;
    fn create_directory(&mut self, path: &Path) -> Result<OwnedEntry, OpError>;
    fn create_staging(&mut self, path: &Path) -> Result<StagingFile, OpError>;
    fn refresh_staging(&self, staging: &mut StagingFile) -> Result<Durability, OpError>;
    fn open_verified(&self, path: &Path, expected: &FileStamp) -> Result<File, OpError>;
    fn set_metadata(
        &mut self,
        path: &Path,
        expected: &FileStamp,
        update: &MetadataUpdate,
    ) -> Result<MutationOutcome, OpError>;
    fn flush_directory(&self, path: &Path) -> Result<Durability, OpError>;
    fn remove_owned(&mut self, entry: &OwnedEntry) -> Result<Durability, OpError>;
}

#[derive(Clone, Debug, Default)]
pub struct PlatformFs {
    pub policy: ProtectedPolicy,
    /// Supplied from the persisted operation store while its read lease is held.
    pub namespace_epoch: u64,
    pub policy_epoch: u64,
}


```

Errors (`vidlib_fileops::{ErrorCode, Phase, OpError}` and `vidlib_fileops::error`):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    DestinationExists,
    SourceMissing,
    StaleState,
    ReparsePoint,
    InvalidPath,
    OutsideGrant,
    ProtectedEntry,
    PermissionDenied,
    Busy,
    CrossDevice,
    Unsupported,
    Io,
    RecoveryRequired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Inspect,
    Capture,
    Verify,
    Rename,
    Create,
    Read,
    Write,
    Metadata,
    Flush,
    Remove,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpError {
    pub code: ErrorCode,
    pub phase: Phase,
    pub operation_id: Option<OperationId>,
    pub step_id: Option<StepId>,
    pub item_id: Option<EntryId>,
    pub message: String,
    pub retryable: bool,
}

```

`OpError::new(code, phase, message)` and `OpError::unsupported(phase)` are public constructors; OpError implements Display/Error and Serialize/Deserialize. IDs remain optional until the higher layer has them; only Busy is retryable by default.

Shared inventory addition:

```rust
pub enum NamePolicy { Posix, Windows, Unknown }
pub enum NameViolation {
    UnknownPolicy, InvalidComponent, TooLong, ForbiddenCharacter,
    ReservedName, TrailingDotOrSpace,
}
// Added fields to DirectoryFacts:
pub name_policy: NamePolicy;
pub reserved_names: Vec<String>;
impl DirectoryFacts {
    pub fn validate_name(&self, name: &NativePathRecord) -> Result<(), NameViolation>;
}
```

Native paths remain Rust-owned. `PlatformFs` epoch fields must be populated from the persisted operation store under a read lease by its future caller. `NamespaceSnapshot`/`DirectoryFacts` stay owned by inventory, with all original exact-quantity serialization unchanged. The syscall seam is deliberately not a journal/coordinator; future callers must durably journal intents before effects and receipts afterward. Ownership/durability and streaming-content requirements are documented in the capability report.

## Named tests and what regressions they catch

- `existing_target_never_replaced`
- `target_created_after_inspection_is_not_overwritten`
- `directory_case_policy_is_per_parent`
- `reparse_ancestor_rejected`
- `rename_files_directories_and_case_only_preserves_identity`
- `stale_source_or_missing_identity_cannot_mutate`
- `protected_names_and_outside_grants_fail_before_mutation`
- `namespace_keeps_native_names_and_protected_entries`
- `create_new_staging_publish_and_owned_cleanup`
- `cleanup_does_not_remove_replaced_or_nonempty_owned_entries`
- `verified_read_and_metadata_setters_use_fresh_handles`
- `staging_cannot_bypass_an_explicitly_protected_root`
- `directories_and_symlink_destinations_are_never_replaced`
- `unknown_filesystem_is_read_only_to_mutation_seam`
- `tmpfs_native_capability_fixture`
- `planner_and_adapter_share_native_reserved_name_and_length_policy`
- `every_candidate_is_grant_checked_before_parent_deduplication`
- `staging_refresh_rejects_a_path_replaced_while_content_handle_is_open`

Additional unit test: `platform::tests::replaced_parent_binding_is_stale_even_while_old_handle_remains_valid` catches a detached descriptor being treated as a current pathname binding.

Windows integration tests (compiled; not executed):
- `unverified_windows_volume_fails_closed_for_file_folder_and_case_only`
- `locked_file_never_changes_when_capability_is_unverified`

Private Windows native qualification tests (compiled; not executed):
- `native_existing_target_never_replaced_for_files_and_directories`
- `native_locked_file_rejected`
- `native_case_only_and_directory_moves_preserve_identity`
- `native_target_created_after_inspection_is_not_overwritten`
- `native_directory_case_policy_is_per_parent` (explicit ignore: requires case-setting capability/permission)
- `native_reparse_ancestor_rejected` (explicit ignore: requires symlink/developer-mode permission)

Tests exercise real temp directories, OS calls, contents, native identities, and literal expected values. The independent inventory seam supplied initial FileStamp fixtures so the first no-replace red occurred at the operation. No mocked filesystem behavior is asserted. Linux per-parent sensitivity evidence is sensitive tmpfs/overlay versus unknown procfs; actual sensitive-versus-insensitive sibling NTFS evidence is pending, not fabricated with a mock.

## Witnessed red/green evidence

Toolchain throughout: `CARGO_HOME=/workspace/.tools/cargo RUSTUP_HOME=/workspace/.tools/rustup PATH=/workspace/.tools/cargo/bin:$PATH`.
An initial test compile typo (different byte-array lengths in an if expression) was corrected before counting red. Initial seam methods returned typed Unsupported with no effects; 11 real tests then failed as expected before the Linux effects were implemented.

### Initial red

```sh
cargo test -p vidlib-fileops --test platform
```

```text
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.48s
     Running tests/platform.rs (target/debug/deps/platform-53904100077d678f)

running 11 tests
test cleanup_does_not_remove_replaced_or_nonempty_owned_entries ... FAILED
test existing_target_never_replaced ... FAILED
test create_new_staging_publish_and_owned_cleanup ... FAILED
test directory_case_policy_is_per_parent ... FAILED
test protected_names_and_outside_grants_fail_before_mutation ... FAILED
test namespace_keeps_native_names_and_protected_entries ... FAILED
test rename_files_directories_and_case_only_preserves_identity ... FAILED
test target_created_after_inspection_is_not_overwritten ... FAILED
test reparse_ancestor_rejected ... FAILED
test stale_source_or_missing_identity_cannot_mutate ... FAILED
test verified_read_and_metadata_setters_use_fresh_handles ... FAILED

failures:

---- cleanup_does_not_remove_replaced_or_nonempty_owned_entries stdout ----

thread 'cleanup_does_not_remove_replaced_or_nonempty_owned_entries' (16096) panicked at crates/vidlib-fileops/tests/platform.rs:142:102:
called `Result::unwrap()` on an `Err` value: OpError { code: Unsupported, phase: Create, operation_id: None, step_id: None, item_id: None, message: "filesystem capability is not verified", retryable: false }

---- existing_target_never_replaced stdout ----

thread 'existing_target_never_replaced' (16099) panicked at crates/vidlib-fileops/tests/platform.rs:22:5:
assertion `left == right` failed
  left: Unsupported
 right: DestinationExists

---- create_new_staging_publish_and_owned_cleanup stdout ----

thread 'create_new_staging_publish_and_owned_cleanup' (16097) panicked at crates/vidlib-fileops/tests/platform.rs:124:52:
called `Result::unwrap()` on an `Err` value: OpError { code: Unsupported, phase: Create, operation_id: None, step_id: None, item_id: None, message: "filesystem capability is not verified", retryable: false }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- directory_case_policy_is_per_parent stdout ----

thread 'directory_case_policy_is_per_parent' (16098) panicked at crates/vidlib-fileops/tests/platform.rs:47:109:
called `Result::unwrap()` on an `Err` value: OpError { code: Unsupported, phase: Capture, operation_id: None, step_id: None, item_id: None, message: "filesystem capability is not verified", retryable: false }

---- protected_names_and_outside_grants_fail_before_mutation stdout ----

thread 'protected_names_and_outside_grants_fail_before_mutation' (16101) panicked at crates/vidlib-fileops/tests/platform.rs:104:9:
assertion `left == right` failed
  left: Unsupported
 right: ProtectedEntry

---- namespace_keeps_native_names_and_protected_entries stdout ----

thread 'namespace_keeps_native_names_and_protected_entries' (16100) panicked at crates/vidlib-fileops/tests/platform.rs:115:111:
called `Result::unwrap()` on an `Err` value: OpError { code: Unsupported, phase: Capture, operation_id: None, step_id: None, item_id: None, message: "filesystem capability is not verified", retryable: false }

---- rename_files_directories_and_case_only_preserves_identity stdout ----

thread 'rename_files_directories_and_case_only_preserves_identity' (16102) panicked at crates/vidlib-fileops/tests/platform.rs:83:76:
called `Result::unwrap()` on an `Err` value: OpError { code: Unsupported, phase: Rename, operation_id: None, step_id: None, item_id: None, message: "filesystem capability is not verified", retryable: false }

---- target_created_after_inspection_is_not_overwritten stdout ----

thread 'target_created_after_inspection_is_not_overwritten' (16105) panicked at crates/vidlib-fileops/tests/platform.rs:32:87:
called `Result::unwrap()` on an `Err` value: OpError { code: Unsupported, phase: Capture, operation_id: None, step_id: None, item_id: None, message: "filesystem capability is not verified", retryable: false }

---- reparse_ancestor_rejected stdout ----

thread 'reparse_ancestor_rejected' (16103) panicked at crates/vidlib-fileops/tests/platform.rs:70:5:
assertion `left == right` failed
  left: Unsupported
 right: ReparsePoint

---- stale_source_or_missing_identity_cannot_mutate stdout ----

thread 'stale_source_or_missing_identity_cannot_mutate' (16104) panicked at crates/vidlib-fileops/tests/platform.rs:95:5:
assertion `left == right` failed
  left: Unsupported
 right: StaleState

---- verified_read_and_metadata_setters_use_fresh_handles stdout ----

thread 'verified_read_and_metadata_setters_use_fresh_handles' (16106) panicked at crates/vidlib-fileops/tests/platform.rs:153:69:
called `Result::unwrap()` on an `Err` value: OpError { code: Unsupported, phase: Read, operation_id: None, step_id: None, item_id: None, message: "filesystem capability is not verified", retryable: false }


failures:
    cleanup_does_not_remove_replaced_or_nonempty_owned_entries
    create_new_staging_publish_and_owned_cleanup
    directory_case_policy_is_per_parent
    existing_target_never_replaced
    namespace_keeps_native_names_and_protected_entries
    protected_names_and_outside_grants_fail_before_mutation
    rename_files_directories_and_case_only_preserves_identity
    reparse_ancestor_rejected
    stale_source_or_missing_identity_cannot_mutate
    target_created_after_inspection_is_not_overwritten
    verified_read_and_metadata_setters_use_fresh_handles

test result: FAILED. 0 passed; 11 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p vidlib-fileops --test platform`

```

### Protected staging root regression red

```sh
cargo test -p vidlib-fileops --test platform
```

```text
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.35s
     Running tests/platform.rs (target/debug/deps/platform-53904100077d678f)

running 15 tests
test cleanup_does_not_remove_replaced_or_nonempty_owned_entries ... ok
test directories_and_symlink_destinations_are_never_replaced ... ok
test create_new_staging_publish_and_owned_cleanup ... ok
test existing_target_never_replaced ... ok
test namespace_keeps_native_names_and_protected_entries ... ok
test protected_names_and_outside_grants_fail_before_mutation ... ok
test rename_files_directories_and_case_only_preserves_identity ... ok
test directory_case_policy_is_per_parent ... ok
test reparse_ancestor_rejected ... ok
test stale_source_or_missing_identity_cannot_mutate ... ok
test staging_cannot_bypass_an_explicitly_protected_root ... FAILED
test target_created_after_inspection_is_not_overwritten ... ok
test unknown_filesystem_is_read_only_to_mutation_seam ... ok
test tmpfs_native_capability_fixture ... ok
test verified_read_and_metadata_setters_use_fresh_handles ... ok

failures:

---- staging_cannot_bypass_an_explicitly_protected_root stdout ----

thread 'staging_cannot_bypass_an_explicitly_protected_root' (16504) panicked at crates/vidlib-fileops/tests/platform.rs:167:49:
called `Result::unwrap_err()` on an `Ok` value: StagingFile { ownership: OwnedEntry { path: "/tmp/.tmpME7g06/.vidlib-staging-protected", stamp: FileStamp { physical_id: Some(PhysicalId { platform: UnixBytes, volume_identity: [0, 0, 0, 0, 0, 0, 0, 33], file_identity: [0, 0, 0, 0, 0, 0, 11, 60] }), size: Some(0), created_ns: Some(1792234503883732484), modified_ns: Some(1792234503883732484), accessed_ns: Some(1792234503883732484), windows_attributes: None } }, file: File { fd: 6, path: "/tmp/.tmpME7g06/.vidlib-staging-protected", read: true, write: true } }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    staging_cannot_bypass_an_explicitly_protected_root

test result: FAILED. 14 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p vidlib-fileops --test platform`

```

### Shared native name-policy red

```sh
cargo test -p vidlib-fileops --test platform planner_and_adapter_share
```

```text
   Compiling vidlib-inventory v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-inventory)
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.59s
     Running tests/platform.rs (target/debug/deps/platform-53904100077d678f)

running 1 test
test planner_and_adapter_share_native_reserved_name_and_length_policy ... FAILED

failures:

---- planner_and_adapter_share_native_reserved_name_and_length_policy stdout ----

thread 'planner_and_adapter_share_native_reserved_name_and_length_policy' (17270) panicked at crates/vidlib-fileops/tests/platform.rs:202:62:
assertion `left == right` failed
  left: Err(UnknownPolicy)
 right: Err(ReservedName)
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    planner_and_adapter_share_native_reserved_name_and_length_policy

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 15 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p vidlib-fileops --test platform`

```

### Parent binding red

```sh
cargo test -p vidlib-fileops --lib replaced_parent_binding
```

```text
   Compiling vidlib-inventory v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-inventory)
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.56s
     Running unittests src/lib.rs (target/debug/deps/vidlib_fileops-1d5a57864a55a22b)

running 1 test
test platform::tests::replaced_parent_binding_is_stale_even_while_old_handle_remains_valid ... FAILED

failures:

---- platform::tests::replaced_parent_binding_is_stale_even_while_old_handle_remains_valid stdout ----

thread 'platform::tests::replaced_parent_binding_is_stale_even_while_old_handle_remains_valid' (18008) panicked at crates/vidlib-fileops/src/platform/mod.rs:621:73:
called `Result::unwrap_err()` on an `Ok` value: ()
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    platform::tests::replaced_parent_binding_is_stale_even_while_old_handle_remains_valid

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p vidlib-fileops --lib`

```

### Every candidate grant validation red

```sh
cargo test -p vidlib-fileops --test platform every_candidate
```

```text
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.34s
     Running tests/platform.rs (target/debug/deps/platform-53904100077d678f)

running 1 test
test every_candidate_is_grant_checked_before_parent_deduplication ... FAILED

failures:

---- every_candidate_is_grant_checked_before_parent_deduplication stdout ----

thread 'every_candidate_is_grant_checked_before_parent_deduplication' (18726) panicked at crates/vidlib-fileops/tests/platform.rs:455:100:
called `Result::unwrap_err()` on an `Ok` value: NamespaceSnapshot { parents: [DirectoryFacts { parent_path: "/tmp/.tmp39qt9T", source_grants: [GrantId(f6b5b708-0875-4b0f-87de-88bf31a18eae)], entries: [NamespaceEntry { name: NativePathRecord { platform: UnixBytes, bytes: [115, 101, 108, 101, 99, 116, 101, 100] }, absolute_path: "/tmp/.tmp39qt9T/selected", kind: File, stamp: FileStamp { physical_id: Some(PhysicalId { platform: UnixBytes, volume_identity: [0, 0, 0, 0, 0, 0, 0, 33], file_identity: [0, 0, 0, 0, 0, 0, 12, 51] }), size: Some(8), created_ns: Some(1792234879344991262), modified_ns: Some(1792234879344991262), accessed_ns: Some(1792234879344991262), windows_attributes: None }, protected: false }], case_sensitivity: Sensitive, normalization: Preserved, max_component_units: Some(255), max_path_units: None, forbidden_characters: ["\0", "/"], name_policy: Posix, reserved_names: [], trailing_dots_spaces_allowed: Some(true), capabilities: FilesystemCapabilities { read_only: Some(false), physical_identity: Supported, hard_links: Unknown, atomic_rename: Supported, durable_flush: Unknown, local_filesystem: Some(true) }, children_complete: true, window: ObservationWindow { started_ns: 1792234879347322105, finished_ns: 1792234879347456291 }, issues: [] }], grants: GrantSet { grants: [SourceGrant { id: GrantId(f6b5b708-0875-4b0f-87de-88bf31a18eae), path: "/tmp/.tmp39qt9T/selected", source: ValidatedManual }] }, policy: ProtectedPolicy { protected_roots: [], marker_name: ".vidlib-blueprint.json", staging_prefixes: [".vidlib-staging"] }, namespace_epoch: 0, policy_epoch: 0, window: ObservationWindow { started_ns: 1792234879347303639, finished_ns: 1792234879347465327 } }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    every_candidate_is_grant_checked_before_parent_deduplication

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 16 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p vidlib-fileops --test platform`

```

### Held staging ownership refresh red

```sh
cargo test -p vidlib-fileops --test platform staging
```

```text
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.66s
     Running tests/platform.rs (target/debug/deps/platform-53904100077d678f)

running 3 tests
test staging_cannot_bypass_an_explicitly_protected_root ... ok
test staging_refresh_rejects_a_path_replaced_while_content_handle_is_open ... FAILED
test create_new_staging_publish_and_owned_cleanup ... FAILED

failures:

---- staging_refresh_rejects_a_path_replaced_while_content_handle_is_open stdout ----

thread 'staging_refresh_rejects_a_path_replaced_while_content_handle_is_open' (19858) panicked at crates/vidlib-fileops/tests/platform.rs:480:5:
assertion `left == right` failed
  left: Unsupported
 right: StaleState
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- create_new_staging_publish_and_owned_cleanup stdout ----

thread 'create_new_staging_publish_and_owned_cleanup' (19856) panicked at crates/vidlib-fileops/tests/platform.rs:247:52:
called `Result::unwrap()` on an `Err` value: OpError { code: Unsupported, phase: Write, operation_id: None, step_id: None, item_id: None, message: "filesystem capability is not verified", retryable: false }


failures:
    create_new_staging_publish_and_owned_cleanup
    staging_refresh_rejects_a_path_replaced_while_content_handle_is_open

test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 15 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p vidlib-fileops --test platform`

```

### Final portable green

```sh
cargo test -p vidlib-engine -p vidlib-fileops -p vidlib-inventory
```

```text
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.61s
     Running unittests src/lib.rs (target/debug/deps/vidlib_engine-d93bfedd144350c9)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/scan.rs (target/debug/deps/scan-c76ad2f0b7e7510f)

running 14 tests
test cancellation_before_scan_preserves_cancelled_state ... ok
test directory_links_are_not_followed_or_counted_as_extra_videos ... ok
test empty_library_completes_without_issues_or_hash_work ... ok
test fast_scan_groups_exact_sizes_recursively_without_content_comparison ... ok
test invalid_roots_produce_issues_without_discarding_valid_roots ... ok
test md5_separates_same_size_collisions_and_hashes_complete_candidates_only ... ok
test missing_candidate_reports_issue_and_keeps_other_duplicate_results ... ok
test cancellation_mid_hash_stops_before_finishing_current_file ... ok
test no_roots_is_a_validation_error ... ok
test overlapping_roots_and_repeated_paths_are_counted_once ... ok
test selected_directory_link_is_reported_instead_of_followed ... ok
test complete_hash_detects_a_difference_beyond_the_first_buffer ... ok
test files_changed_during_hashing_are_excluded_and_reported ... ok
test modification_time_change_without_length_change_invalidates_hash ... ok

test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s

     Running unittests src/lib.rs (target/debug/deps/vidlib_fileops-1d5a57864a55a22b)

running 2 tests
test platform::tests::replaced_parent_binding_is_stale_even_while_old_handle_remains_valid ... ok
test tests::changed_source_after_validation_cleans_partial_destination ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/actions.rs (target/debug/deps/actions-98c8373a47b420f9)

running 21 tests
test changed_modification_time_rejects_deletion ... ok
test changed_kept_copy_hash_prevents_deletion_even_with_restored_timestamp ... ok
test duplicate_selection_is_rejected ... ok
test empty_selection_is_rejected ... ok
test changed_selected_content_rejects_md5_actions_with_restored_timestamp ... ok
test deleting_a_selected_copy_keeps_the_original ... ok
test missing_kept_copy_prevents_deletion ... ok
test missing_md5_hash_rejects_actions ... ok
test invalid_move_destination_prevents_changes ... ok
test move_collision_never_overwrites_existing_file ... ok
test move_preserves_content_and_kept_original ... ok
test move_to_source_directory_is_rejected ... ok
test move_preserves_original_modification_timestamp ... ok
test selecting_every_copy_is_rejected_before_any_deletion ... ok
test stale_group_late_in_request_prevents_changes_to_an_earlier_group ... ok
test stale_selected_file_rejects_every_action ... ok
test size_candidates_can_be_deleted_without_content_equality ... ok
test symlink_source_is_rejected_without_removing_its_target ... ok
test unknown_path_rejects_the_entire_request ... ok
test selected_files_with_same_name_both_move_without_overwrite ... ok
test move_preserves_content_across_many_streaming_chunks ... ok

test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

     Running tests/platform.rs (target/debug/deps/platform-53904100077d678f)

running 18 tests
test cleanup_does_not_remove_replaced_or_nonempty_owned_entries ... ok
test directories_and_symlink_destinations_are_never_replaced ... ok
test existing_target_never_replaced ... ok
test every_candidate_is_grant_checked_before_parent_deduplication ... ok
test create_new_staging_publish_and_owned_cleanup ... ok
test namespace_keeps_native_names_and_protected_entries ... ok
test planner_and_adapter_share_native_reserved_name_and_length_policy ... ok
test protected_names_and_outside_grants_fail_before_mutation ... ok
test directory_case_policy_is_per_parent ... ok
test staging_cannot_bypass_an_explicitly_protected_root ... ok
test rename_files_directories_and_case_only_preserves_identity ... ok
test reparse_ancestor_rejected ... ok
test staging_refresh_rejects_a_path_replaced_while_content_handle_is_open ... ok
test stale_source_or_missing_identity_cannot_mutate ... ok
test unknown_filesystem_is_read_only_to_mutation_seam ... ok
test target_created_after_inspection_is_not_overwritten ... ok
test tmpfs_native_capability_fixture ... ok
test verified_read_and_metadata_setters_use_fresh_handles ... ok

test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/windows_platform.rs (target/debug/deps/windows_platform-99b9a609e9550729)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/vidlib_inventory-8cf31c7bb3797f0b)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/inventory.rs (target/debug/deps/inventory-54d7614fdaa102a6)

running 22 tests
test explicit_files_are_supported_and_relative_grants_are_rejected ... ok
test cancellation_preserves_coverage ... ok
test failed_stat_retains_listed_child ... ok
test failed_binding_observation_drops_previously_matched_evidence ... ok
test mismatched_identity_or_invalid_directory_handle_still_rejects_traversal ... ok
test links_are_visible_but_never_followed ... ok
test os_directory_listing_refuses_directory_links ... ok
test namespace_model_carries_exact_epochs_and_observed_parent_facts_without_fileops ... ok
test native_modified_timestamp_and_regular_file_size_match_os_observation ... ok
test non_utf8_names_remain_native_and_carry_transformation_issues ... ok
test overlap_deduplicates_paths_not_hardlinks ... ok
test overlapping_bindings_retain_their_own_observation_and_coverage_association ... ok
test precancelled_scan_records_unvisited_roots_and_progress_is_bounded ... ok
test overlapping_bindings_validate_evidence_before_deduplicating_traversal ... ok
test protected_marker_and_staging_names_remain_in_listing_but_are_excluded ... ok
test protected_subtrees_are_excluded_when_selected_directly ... ok
test partial_and_failed_listings_propagate_incomplete_coverage_to_ancestors ... ok
test protected_roots_conservatively_match_case_aliases_at_component_boundaries ... ok
test reserved_ascii_case_variants_are_conservatively_excluded_for_unknown_filesystems ... ok
test scan_keeps_unique_empty_and_nonvideo_entries ... ok
test replacing_a_listed_directory_with_an_external_link_never_observes_external_children ... ok
test unavailable_directory_identity_preserves_safe_entries_with_incomplete_coverage ... ok

test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/path_codec.rs (target/debug/deps/path_codec-4d9b5506ad20490f)

running 5 tests
test path_codec_round_trips_native_units ... ok
test portable_relative_paths_reject_escape_and_native_display_is_not_lookup ... ok
test all_ids_are_distinct_uuid_strings ... ok
test windows_utf16_fixture_is_exact_and_foreign_paths_are_not_transcoded ... ok
test json_quantities_and_nanosecond_timestamps_use_validated_decimal_strings ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests vidlib_engine

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests vidlib_fileops

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests vidlib_inventory

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


```

### Overlayfs native green

```sh
TMPDIR=/workspace/VidLibOrganizer/.worktrees/rename-blueprint/target/task02-overlay-tmp cargo test -p vidlib-fileops --test platform
```

```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.09s
     Running tests/platform.rs (target/debug/deps/platform-53904100077d678f)

running 18 tests
test cleanup_does_not_remove_replaced_or_nonempty_owned_entries ... ok
test directories_and_symlink_destinations_are_never_replaced ... ok
test existing_target_never_replaced ... ok
test every_candidate_is_grant_checked_before_parent_deduplication ... ok
test planner_and_adapter_share_native_reserved_name_and_length_policy ... ok
test directory_case_policy_is_per_parent ... ok
test namespace_keeps_native_names_and_protected_entries ... ok
test protected_names_and_outside_grants_fail_before_mutation ... ok
test reparse_ancestor_rejected ... ok
test staging_cannot_bypass_an_explicitly_protected_root ... ok
test staging_refresh_rejects_a_path_replaced_while_content_handle_is_open ... ok
test target_created_after_inspection_is_not_overwritten ... ok
test stale_source_or_missing_identity_cannot_mutate ... ok
test unknown_filesystem_is_read_only_to_mutation_seam ... ok
test tmpfs_native_capability_fixture ... ok
test rename_files_directories_and_case_only_preserves_identity ... ok
test verified_read_and_metadata_setters_use_fresh_handles ... ok
test create_new_staging_publish_and_owned_cleanup ... ok

test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s


```

### Linux Clippy

```sh
cargo clippy -p vidlib-engine -p vidlib-fileops -p vidlib-inventory --all-targets -- -D warnings
```

```text
    Checking vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.41s

```

### Windows-target Clippy, compile-only

```sh
source /workspace/.tools/windows-check-env.sh
cargo clippy -p vidlib-fileops -p vidlib-inventory --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

```text
    Checking vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.39s

```

### Windows binary requested on Linux

```sh
cargo test -p vidlib-fileops --test windows_platform -- --nocapture
```

```text
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.76s
     Running tests/windows_platform.rs (target/debug/deps/windows_platform-99b9a609e9550729)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


```

`cargo fmt --all -- --check`: exit 0, no output. Final Linux and Windows-target Clippy pass with warnings denied. Interim Clippy diagnostics were test slice clones and a Windows bool `.into()`; both corrected. An initial Windows module/crate-name ambiguity was corrected before the successful check. These were compile/lint iterations, not native runtime reds.

## Baseline full-workspace limitations

Bare `cargo test` was run as required by TDD and failed before tests because unconfigured system development libraries were absent. Exact first output:

```text
    Blocking waiting for file lock on package cache
    Blocking waiting for file lock on package cache
    Blocking waiting for file lock on build directory
   Compiling glib-sys v0.18.1
   Compiling gobject-sys v0.18.0
   Compiling gio-sys v0.18.1
   Compiling gdk-sys v0.18.2
warning: glib-sys@0.18.1:
error: failed to run custom build command for `glib-sys v0.18.1`

Caused by:
  process didn't exit successfully: `/workspace/VidLibOrganizer/.worktrees/rename-blueprint/target/debug/build/glib-sys-d365bb08a382de74/build-script-build` (exit status: 1)
  --- stdout
  cargo:rerun-if-env-changed=GLIB_2.0_NO_PKG_CONFIG
  cargo:rerun-if-env-changed=PKG_CONFIG_x86_64-unknown-linux-gnu
  cargo:rerun-if-env-changed=PKG_CONFIG_x86_64_unknown_linux_gnu
  cargo:rerun-if-env-changed=HOST_PKG_CONFIG
  cargo:rerun-if-env-changed=PKG_CONFIG
  cargo:rerun-if-env-changed=PKG_CONFIG_PATH_x86_64-unknown-linux-gnu
  cargo:rerun-if-env-changed=PKG_CONFIG_PATH_x86_64_unknown_linux_gnu
  cargo:rerun-if-env-changed=HOST_PKG_CONFIG_PATH
  cargo:rerun-if-env-changed=PKG_CONFIG_PATH
  cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR_x86_64-unknown-linux-gnu
  cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR_x86_64_unknown_linux_gnu
  cargo:rerun-if-env-changed=HOST_PKG_CONFIG_LIBDIR
  cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR
  cargo:rerun-if-env-changed=PKG_CONFIG_SYSROOT_DIR_x86_64-unknown-linux-gnu
  cargo:rerun-if-env-changed=PKG_CONFIG_SYSROOT_DIR_x86_64_unknown_linux_gnu
  cargo:rerun-if-env-changed=HOST_PKG_CONFIG_SYSROOT_DIR
  cargo:rerun-if-env-changed=PKG_CONFIG_SYSROOT_DIR
  cargo:warning=
  pkg-config exited with status code 1
  > PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1 pkg-config --libs --cflags glib-2.0 'glib-2.0 >= 2.70'

  pkg-config output:
    Package glib-2.0 was not found in the pkg-config search path.
    Perhaps you should add the directory containing `glib-2.0.pc'
    to the PKG_CONFIG_PATH environment variable
    Package 'glib-2.0', required by 'virtual:world', not found
    Package 'glib-2.0', required by 'virtual:world', not found

  The system library `glib-2.0` required by crate `glib-sys` was not found.
  The file `glib-2.0.pc` needs to be installed and the PKG_CONFIG_PATH environment variable must contain its parent directory.
  The PKG_CONFIG_PATH environment variable is not set.

  HINT: if you have installed the library, try setting PKG_CONFIG_PATH to the directory containing `glib-2.0.pc`.

warning: build failed, waiting for other jobs to finish...

```

A follow-up exclusion command mistakenly named `vidlib-organizer` rather than the actual desktop crate `vidlib-tauri`; Cargo warned that the excluded package did not exist and again attempted the desktop. Its build failures named glib-sys/glib-2.0, gobject-sys/gobject-2.0, gio-sys/gio-2.0, and gdk-sys/gdk-3.0. This was corrected by explicitly selecting all three portable workspace crates in the successful 82-test command above. No test failure was omitted. The controller notes that a separately configured desktop SDK reaches a preexisting missing `icons/icon.png` failure owned by Task05; that configured desktop check was not run or claimed by this task.

## Pending, unsupported, and material limits

- Windows NTFS file/folder/case-only/locking/per-parent sensitivity/reparse behavior, Windows metadata/flush, exFAT/removable and SMB runtime evidence are pending. All Windows production mutation capabilities remain disabled. No native Windows red/green result is claimed; only compilation/Clippy is available. The native bindings cause no production mutation effects while the gate is closed.
- Other Linux filesystems, including ext4/XFS/Btrfs/FAT/exFAT/network, are intentionally unverified/unsupported for mutation. Procfs unsupported behavior was tested. Unknown OS adapters return Unsupported where inventory supports the target.
- tmpfs and overlayfs were actually tested separately; initial `/tmp` evidence was tmpfs, not overlayfs. The explicit overlay run followed mount identification.
- Linux atomic no-replace prevents destination replacement. Its source basename cannot be atomically matched to an inode; unlink and mkdir/open have related identity gaps. Held descriptors, immediate revalidation and poststate checks reduce races but do not claim immunity to a hostile privileged process. Unexpected applied state or rebound parent yields RecoveryRequired.
- Flush acknowledgment is distinct from power-loss guarantee. tmpfs is volatile; overlay backing durability/locality is unknown. `durable_flush` stays Unknown. Process-termination recovery belongs to later journal/reconciliation tasks; this adapter does not claim recovered batches or transaction atomicity.
- New directory creation returns ownership before an explicit parent flush; staged writing must use `refresh_staging` before publication so path replacement cannot silently change ownership. Streaming content hashing/count verification belongs to the copy layer.
- The full portable suite and targeted checks are green. Desktop baseline and native Windows qualification remain concerns, not reasons to block authorized portable work.

See `docs/verification/filesystem-capabilities.md` for the matrix, APIs, caller obligations, commands, and precise pending cases.

Final repository checks: `git diff --check` and `git diff --cached --check` succeeded with no output before the implementation commit. Only the eleven task/authorized shared-contract files were committed; no canonical checklist edits.


## Review fix round 1 of 5 — Important metadata/removal outcome findings

FIX_BASE: `8fe6590376d7f1bfb1cb8abee092df6ec5dc3040`.
Review source: `/workspace/scratch/task02-review.md`, both Important findings under Issues. The fix is a separate commit titled `fix: verify metadata and owned cleanup poststates`; its exact SHA is returned in the controller handoff. No public interface changes, canonical checklist edits, or subagents.

### Changes

1. Metadata now revalidates the child pathname identity/freshness immediately before calling the native setter and compares the requested child's poststate to the held object's resulting stamp after the effect. Pre-effect replacement is StaleState without applying the update. Missing/rebound/unobservable post-effect child state is RecoveryRequired, never an ordinary MutationOutcome.
2. Linux cleanup captures the held object's link count before unlink, checks it afterward, and inspects the resulting name. A file must lose exactly one link; a removed directory must have zero links; the requested name must be absent. A replacement unlinked while the owned object survives elsewhere, a name recreated after unlink, or unavailable evidence produces RecoveryRequired. Existing hard-link aliases remain intact and supported when exactly the intended observed link disappears.
3. Windows owned-cleanup deletion evidence is explicitly Unsupported before any effect. Linux link-count behavior is not assumed for Windows delete-pending/close semantics. The existing Windows global mutation gate remains disabled; native runtime qualification is still pending.

The deterministic native-boundary fixture is private and `cfg(all(test, target_os = "linux"))`/`cfg(test)` only. One-shot thread-local callbacks and RAII cleanup prevent cross-test interference. It adds no production IPC switch or runtime test configuration. Effects still run through the real production adapter and native syscalls. The tests intentionally reproduce the accepted unavoidable Linux syscall race and assert an unresolved outcome rather than claiming the replacement was protected by atomic source binding.

### Named tests / witnessed red

- `metadata_rebinding_before_final_validation_is_stale_without_effect`: replacing the path after the mutable handle opens must prevent any metadata effect.
- `metadata_rebinding_at_native_effect_requires_recovery`: replacement at the native setter boundary updates the held original but must not record the requested path as complete.
- `metadata_path_missing_after_native_effect_requires_recovery`: unlink after the setter must not produce an ordinary successful result.
- `owned_removal_rebinding_at_unlink_requires_recovery_for_files_and_directories`: moving the owned entry and installing a replacement just before unlink must leave evidence of unresolved owned removal.
- `owned_removal_recreated_name_after_unlink_requires_recovery`: a new name binding after unlink must not be described as completed cleanup.
- `owned_removal_of_one_hard_link_preserves_other_alias`: intended removal of one alias remains supported without deleting another alias. This compatibility guard already passed before the fix; it is not claimed as an intended red.

The first five named tests failed before the behavioral fix because the adapter returned successful outcomes. All six then passed, including both file/directory iterations of the unlink regression. The original held-parent test and all existing platform/legacy tests remain green.

### Intended red

```sh
cargo test -p vidlib-fileops --lib boundary_regressions
```

```text
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.85s
     Running unittests src/lib.rs (target/debug/deps/vidlib_fileops-1d5a57864a55a22b)

running 6 tests
test platform::boundary_regressions::metadata_rebinding_at_native_effect_requires_recovery ... FAILED
test platform::boundary_regressions::metadata_path_missing_after_native_effect_requires_recovery ... FAILED
test platform::boundary_regressions::metadata_rebinding_before_final_validation_is_stale_without_effect ... FAILED
test platform::boundary_regressions::owned_removal_of_one_hard_link_preserves_other_alias ... ok
test platform::boundary_regressions::owned_removal_rebinding_at_unlink_requires_recovery_for_files_and_directories ... FAILED
test platform::boundary_regressions::owned_removal_recreated_name_after_unlink_requires_recovery ... FAILED

failures:

---- platform::boundary_regressions::metadata_rebinding_at_native_effect_requires_recovery stdout ----

thread 'platform::boundary_regressions::metadata_rebinding_at_native_effect_requires_recovery' (20783) panicked at crates/vidlib-fileops/src/platform/mod.rs:806:27:
called `Result::unwrap_err()` on an `Ok` value: MutationOutcome { observation: FileObservation { kind: File, stamp: FileStamp { physical_id: Some(PhysicalId { platform: UnixBytes, volume_identity: [0, 0, 0, 0, 0, 0, 0, 33], file_identity: [0, 0, 0, 0, 0, 0, 15, 56] }), size: Some(5), created_ns: Some(1792235439660880325), modified_ns: Some(1700000000123456789), accessed_ns: Some(1792235439660880325), windows_attributes: None } }, durability: Synced }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- platform::boundary_regressions::metadata_path_missing_after_native_effect_requires_recovery stdout ----

thread 'platform::boundary_regressions::metadata_path_missing_after_native_effect_requires_recovery' (20782) panicked at crates/vidlib-fileops/src/platform/mod.rs:816:131:
called `Result::unwrap_err()` on an `Ok` value: MutationOutcome { observation: FileObservation { kind: File, stamp: FileStamp { physical_id: Some(PhysicalId { platform: UnixBytes, volume_identity: [0, 0, 0, 0, 0, 0, 0, 33], file_identity: [0, 0, 0, 0, 0, 0, 15, 57] }), size: Some(5), created_ns: Some(1792235439660880325), modified_ns: Some(1700000000123456789), accessed_ns: Some(1792235439660880325), windows_attributes: None } }, durability: Synced }

---- platform::boundary_regressions::metadata_rebinding_before_final_validation_is_stale_without_effect stdout ----

thread 'platform::boundary_regressions::metadata_rebinding_before_final_validation_is_stale_without_effect' (20784) panicked at crates/vidlib-fileops/src/platform/mod.rs:795:27:
called `Result::unwrap_err()` on an `Ok` value: MutationOutcome { observation: FileObservation { kind: File, stamp: FileStamp { physical_id: Some(PhysicalId { platform: UnixBytes, volume_identity: [0, 0, 0, 0, 0, 0, 0, 33], file_identity: [0, 0, 0, 0, 0, 0, 15, 61] }), size: Some(5), created_ns: Some(1792235439662782527), modified_ns: Some(1700000000123456789), accessed_ns: Some(1792235439662782527), windows_attributes: None } }, durability: Synced }

---- platform::boundary_regressions::owned_removal_rebinding_at_unlink_requires_recovery_for_files_and_directories stdout ----

thread 'platform::boundary_regressions::owned_removal_rebinding_at_unlink_requires_recovery_for_files_and_directories' (20786) panicked at crates/vidlib-fileops/src/platform/mod.rs:827:53:
called `Result::unwrap_err()` on an `Ok` value: Synced

---- platform::boundary_regressions::owned_removal_recreated_name_after_unlink_requires_recovery stdout ----

thread 'platform::boundary_regressions::owned_removal_recreated_name_after_unlink_requires_recovery' (20787) panicked at crates/vidlib-fileops/src/platform/mod.rs:837:49:
called `Result::unwrap_err()` on an `Ok` value: Synced


failures:
    platform::boundary_regressions::metadata_path_missing_after_native_effect_requires_recovery
    platform::boundary_regressions::metadata_rebinding_at_native_effect_requires_recovery
    platform::boundary_regressions::metadata_rebinding_before_final_validation_is_stale_without_effect
    platform::boundary_regressions::owned_removal_rebinding_at_unlink_requires_recovery_for_files_and_directories
    platform::boundary_regressions::owned_removal_recreated_name_after_unlink_requires_recovery

test result: FAILED. 1 passed; 5 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p vidlib-fileops --lib`

```

### Focused green

```sh
cargo test -p vidlib-fileops --lib boundary_regressions
```

```text
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.47s
     Running unittests src/lib.rs (target/debug/deps/vidlib_fileops-1d5a57864a55a22b)

running 6 tests
test platform::boundary_regressions::metadata_rebinding_before_final_validation_is_stale_without_effect ... ok
test platform::boundary_regressions::metadata_rebinding_at_native_effect_requires_recovery ... ok
test platform::boundary_regressions::metadata_path_missing_after_native_effect_requires_recovery ... ok
test platform::boundary_regressions::owned_removal_of_one_hard_link_preserves_other_alias ... ok
test platform::boundary_regressions::owned_removal_rebinding_at_unlink_requires_recovery_for_files_and_directories ... ok
test platform::boundary_regressions::owned_removal_recreated_name_after_unlink_requires_recovery ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s


```

### Portable regression green

```sh
cargo test -p vidlib-engine -p vidlib-fileops -p vidlib-inventory
```

```text
    Blocking waiting for file lock on package cache
    Blocking waiting for file lock on package cache
   Compiling vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.80s
     Running unittests src/lib.rs (target/debug/deps/vidlib_engine-d93bfedd144350c9)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/scan.rs (target/debug/deps/scan-c76ad2f0b7e7510f)

running 14 tests
test cancellation_before_scan_preserves_cancelled_state ... ok
test directory_links_are_not_followed_or_counted_as_extra_videos ... ok
test empty_library_completes_without_issues_or_hash_work ... ok
test fast_scan_groups_exact_sizes_recursively_without_content_comparison ... ok
test invalid_roots_produce_issues_without_discarding_valid_roots ... ok
test md5_separates_same_size_collisions_and_hashes_complete_candidates_only ... ok
test missing_candidate_reports_issue_and_keeps_other_duplicate_results ... ok
test cancellation_mid_hash_stops_before_finishing_current_file ... ok
test no_roots_is_a_validation_error ... ok
test overlapping_roots_and_repeated_paths_are_counted_once ... ok
test selected_directory_link_is_reported_instead_of_followed ... ok
test files_changed_during_hashing_are_excluded_and_reported ... ok
test modification_time_change_without_length_change_invalidates_hash ... ok
test complete_hash_detects_a_difference_beyond_the_first_buffer ... ok

test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s

     Running unittests src/lib.rs (target/debug/deps/vidlib_fileops-1d5a57864a55a22b)

running 8 tests
test platform::boundary_regressions::metadata_path_missing_after_native_effect_requires_recovery ... ok
test platform::boundary_regressions::metadata_rebinding_before_final_validation_is_stale_without_effect ... ok
test platform::boundary_regressions::metadata_rebinding_at_native_effect_requires_recovery ... ok
test platform::boundary_regressions::owned_removal_of_one_hard_link_preserves_other_alias ... ok
test platform::boundary_regressions::owned_removal_recreated_name_after_unlink_requires_recovery ... ok
test platform::tests::replaced_parent_binding_is_stale_even_while_old_handle_remains_valid ... ok
test platform::boundary_regressions::owned_removal_rebinding_at_unlink_requires_recovery_for_files_and_directories ... ok
test tests::changed_source_after_validation_cleans_partial_destination ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/actions.rs (target/debug/deps/actions-98c8373a47b420f9)

running 21 tests
test changed_modification_time_rejects_deletion ... ok
test deleting_a_selected_copy_keeps_the_original ... ok
test empty_selection_is_rejected ... ok
test changed_kept_copy_hash_prevents_deletion_even_with_restored_timestamp ... ok
test duplicate_selection_is_rejected ... ok
test invalid_move_destination_prevents_changes ... ok
test missing_md5_hash_rejects_actions ... ok
test changed_selected_content_rejects_md5_actions_with_restored_timestamp ... ok
test missing_kept_copy_prevents_deletion ... ok
test move_collision_never_overwrites_existing_file ... ok
test move_preserves_content_and_kept_original ... ok
test move_to_source_directory_is_rejected ... ok
test selecting_every_copy_is_rejected_before_any_deletion ... ok
test move_preserves_original_modification_timestamp ... ok
test stale_group_late_in_request_prevents_changes_to_an_earlier_group ... ok
test size_candidates_can_be_deleted_without_content_equality ... ok
test symlink_source_is_rejected_without_removing_its_target ... ok
test stale_selected_file_rejects_every_action ... ok
test unknown_path_rejects_the_entire_request ... ok
test selected_files_with_same_name_both_move_without_overwrite ... ok
test move_preserves_content_across_many_streaming_chunks ... ok

test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

     Running tests/platform.rs (target/debug/deps/platform-53904100077d678f)

running 18 tests
test cleanup_does_not_remove_replaced_or_nonempty_owned_entries ... ok
test directory_case_policy_is_per_parent ... ok
test directories_and_symlink_destinations_are_never_replaced ... ok
test every_candidate_is_grant_checked_before_parent_deduplication ... ok
test create_new_staging_publish_and_owned_cleanup ... ok
test namespace_keeps_native_names_and_protected_entries ... ok
test existing_target_never_replaced ... ok
test planner_and_adapter_share_native_reserved_name_and_length_policy ... ok
test protected_names_and_outside_grants_fail_before_mutation ... ok
test reparse_ancestor_rejected ... ok
test staging_cannot_bypass_an_explicitly_protected_root ... ok
test rename_files_directories_and_case_only_preserves_identity ... ok
test staging_refresh_rejects_a_path_replaced_while_content_handle_is_open ... ok
test target_created_after_inspection_is_not_overwritten ... ok
test unknown_filesystem_is_read_only_to_mutation_seam ... ok
test stale_source_or_missing_identity_cannot_mutate ... ok
test tmpfs_native_capability_fixture ... ok
test verified_read_and_metadata_setters_use_fresh_handles ... ok

test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/windows_platform.rs (target/debug/deps/windows_platform-99b9a609e9550729)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/vidlib_inventory-8cf31c7bb3797f0b)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/inventory.rs (target/debug/deps/inventory-54d7614fdaa102a6)

running 22 tests
test explicit_files_are_supported_and_relative_grants_are_rejected ... ok
test cancellation_preserves_coverage ... ok
test failed_stat_retains_listed_child ... ok
test failed_binding_observation_drops_previously_matched_evidence ... ok
test mismatched_identity_or_invalid_directory_handle_still_rejects_traversal ... ok
test namespace_model_carries_exact_epochs_and_observed_parent_facts_without_fileops ... ok
test links_are_visible_but_never_followed ... ok
test native_modified_timestamp_and_regular_file_size_match_os_observation ... ok
test os_directory_listing_refuses_directory_links ... ok
test overlap_deduplicates_paths_not_hardlinks ... ok
test non_utf8_names_remain_native_and_carry_transformation_issues ... ok
test overlapping_bindings_retain_their_own_observation_and_coverage_association ... ok
test overlapping_bindings_validate_evidence_before_deduplicating_traversal ... ok
test precancelled_scan_records_unvisited_roots_and_progress_is_bounded ... ok
test protected_marker_and_staging_names_remain_in_listing_but_are_excluded ... ok
test protected_roots_conservatively_match_case_aliases_at_component_boundaries ... ok
test protected_subtrees_are_excluded_when_selected_directly ... ok
test replacing_a_listed_directory_with_an_external_link_never_observes_external_children ... ok
test partial_and_failed_listings_propagate_incomplete_coverage_to_ancestors ... ok
test reserved_ascii_case_variants_are_conservatively_excluded_for_unknown_filesystems ... ok
test scan_keeps_unique_empty_and_nonvideo_entries ... ok
test unavailable_directory_identity_preserves_safe_entries_with_incomplete_coverage ... ok

test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/path_codec.rs (target/debug/deps/path_codec-4d9b5506ad20490f)

running 5 tests
test path_codec_round_trips_native_units ... ok
test all_ids_are_distinct_uuid_strings ... ok
test portable_relative_paths_reject_escape_and_native_display_is_not_lookup ... ok
test json_quantities_and_nanosecond_timestamps_use_validated_decimal_strings ... ok
test windows_utf16_fixture_is_exact_and_foreign_paths_are_not_transcoded ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests vidlib_engine

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests vidlib_fileops

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests vidlib_inventory

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


```

### Overlay native regression green

```sh
TMPDIR=/workspace/VidLibOrganizer/.worktrees/rename-blueprint/target/task02-overlay-tmp cargo test -p vidlib-fileops
```

```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.11s
     Running unittests src/lib.rs (target/debug/deps/vidlib_fileops-1d5a57864a55a22b)

running 8 tests
test platform::boundary_regressions::metadata_path_missing_after_native_effect_requires_recovery ... ok
test platform::boundary_regressions::metadata_rebinding_before_final_validation_is_stale_without_effect ... ok
test platform::boundary_regressions::metadata_rebinding_at_native_effect_requires_recovery ... ok
test platform::tests::replaced_parent_binding_is_stale_even_while_old_handle_remains_valid ... ok
test platform::boundary_regressions::owned_removal_recreated_name_after_unlink_requires_recovery ... ok
test platform::boundary_regressions::owned_removal_rebinding_at_unlink_requires_recovery_for_files_and_directories ... ok
test tests::changed_source_after_validation_cleans_partial_destination ... ok
test platform::boundary_regressions::owned_removal_of_one_hard_link_preserves_other_alias ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/actions.rs (target/debug/deps/actions-98c8373a47b420f9)

running 21 tests
test deleting_a_selected_copy_keeps_the_original ... ok
test changed_modification_time_rejects_deletion ... ok
test changed_kept_copy_hash_prevents_deletion_even_with_restored_timestamp ... ok
test duplicate_selection_is_rejected ... ok
test empty_selection_is_rejected ... ok
test invalid_move_destination_prevents_changes ... ok
test changed_selected_content_rejects_md5_actions_with_restored_timestamp ... ok
test missing_kept_copy_prevents_deletion ... ok
test missing_md5_hash_rejects_actions ... ok
test move_preserves_original_modification_timestamp ... ok
test move_preserves_content_and_kept_original ... ok
test move_collision_never_overwrites_existing_file ... ok
test move_to_source_directory_is_rejected ... ok
test selecting_every_copy_is_rejected_before_any_deletion ... ok
test stale_group_late_in_request_prevents_changes_to_an_earlier_group ... ok
test size_candidates_can_be_deleted_without_content_equality ... ok
test stale_selected_file_rejects_every_action ... ok
test unknown_path_rejects_the_entire_request ... ok
test symlink_source_is_rejected_without_removing_its_target ... ok
test selected_files_with_same_name_both_move_without_overwrite ... ok
test move_preserves_content_across_many_streaming_chunks ... ok

test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s

     Running tests/platform.rs (target/debug/deps/platform-53904100077d678f)

running 18 tests
test cleanup_does_not_remove_replaced_or_nonempty_owned_entries ... ok
test directory_case_policy_is_per_parent ... ok
test directories_and_symlink_destinations_are_never_replaced ... ok
test namespace_keeps_native_names_and_protected_entries ... ok
test existing_target_never_replaced ... ok
test every_candidate_is_grant_checked_before_parent_deduplication ... ok
test planner_and_adapter_share_native_reserved_name_and_length_policy ... ok
test protected_names_and_outside_grants_fail_before_mutation ... ok
test staging_cannot_bypass_an_explicitly_protected_root ... ok
test reparse_ancestor_rejected ... ok
test staging_refresh_rejects_a_path_replaced_while_content_handle_is_open ... ok
test target_created_after_inspection_is_not_overwritten ... ok
test stale_source_or_missing_identity_cannot_mutate ... ok
test unknown_filesystem_is_read_only_to_mutation_seam ... ok
test tmpfs_native_capability_fixture ... ok
test verified_read_and_metadata_setters_use_fresh_handles ... ok
test rename_files_directories_and_case_only_preserves_identity ... ok
test create_new_staging_publish_and_owned_cleanup ... ok

test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/windows_platform.rs (target/debug/deps/windows_platform-99b9a609e9550729)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests vidlib_fileops

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


```

### Linux Clippy

```sh
cargo clippy -p vidlib-fileops -p vidlib-inventory --all-targets -- -D warnings
```

```text
    Checking vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.41s

```

### Windows-target Clippy, compile only

```sh
source /workspace/.tools/windows-check-env.sh
cargo clippy -p vidlib-fileops -p vidlib-inventory --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

```text
    Blocking waiting for file lock on package cache
    Blocking waiting for file lock on package cache
    Blocking waiting for file lock on build directory
    Checking vidlib-fileops v0.2.0 (/workspace/VidLibOrganizer/.worktrees/rename-blueprint/crates/vidlib-fileops)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.14s

```

`cargo fmt --all -- --check` succeeded with no output. Portable total: **88 passing tests**. The overlay fileops run passes 8 unit + 21 legacy action + 18 platform tests (**47**); the explicit `/dev/shm` fixture remains tmpfs even in that run. No full/unconfigured desktop rerun was needed; the previously recorded baseline limitation is unchanged.

The Minor Windows staging writer-sharing lifecycle finding is deliberately deferred to copy/publication integration and final review, as directed by the controller. No native Windows test result is claimed. These changes address both Important outcome-reporting findings while preserving the documented unavoidable Linux atomic source-binding limitation.
