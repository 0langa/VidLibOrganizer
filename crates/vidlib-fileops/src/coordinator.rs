//! App-local advisory coordination. A lease owns one independently opened file;
//! neither lease nor handle is cloned. No SQLite connection crosses a thread.
use crate::{journal::Journal, ErrorCode, OpError, Phase};
use fs4::fs_std::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseKind {
    Read,
    Write,
    Recovery,
}
/// Backend-only exact values. IPC adapters must encode these as decimal strings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Epochs {
    pub namespace: u64,
    pub policy: u64,
}
/// Read under a lease. The legacy marker records uncertainty, not per-file effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoordinationStatus {
    pub epochs: Epochs,
    pub pending_journal: bool,
    pub interrupted_legacy_write: bool,
}
impl CoordinationStatus {
    pub fn requires_recovery(&self) -> bool {
        self.pending_journal || self.interrupted_legacy_write
    }
}
#[derive(Debug)]
pub struct Coordinator {
    app_data: PathBuf,
    active_writers: Arc<AtomicUsize>,
}
#[derive(Debug)]
pub struct Lease {
    // Move to one worker, but do not share write authority across concurrent workers.
    not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
    file: File,
    database: PathBuf,
    kind: LeaseKind,
    active_writers: Arc<AtomicUsize>,
}
impl Coordinator {
    pub fn open(app_data: &Path) -> Result<Self, OpError> {
        if !app_data.is_absolute() {
            return Err(OpError::new(
                ErrorCode::InvalidPath,
                Phase::Inspect,
                "app data must be an absolute local path",
            ));
        }
        std::fs::create_dir_all(app_data).map_err(io_error)?;
        let app_data = std::fs::canonicalize(app_data).map_err(io_error)?;
        validate_local(&app_data)?;
        let coordinator = Self {
            app_data,
            active_writers: Arc::new(AtomicUsize::new(0)),
        };
        coordinator.validate_managed_paths()?;
        let file = coordinator.open_lock()?;
        match FileExt::try_lock_exclusive(&file) {
            Ok(true) => {
                Journal::open(&coordinator.database_path())?;
                FileExt::unlock(&file).map_err(io_error)?;
            }
            Ok(false) => {
                Journal::coordination_status(&coordinator.database_path())?;
            }
            Err(e) => return Err(io_error(e)),
        }
        Ok(coordinator)
    }
    pub fn owns_lease(&self, lease: &Lease) -> bool {
        self.database_path() == lease.database
    }
    pub fn database_path(&self) -> PathBuf {
        self.app_data.join("operations.sqlite3")
    }
    pub fn blueprint_path(&self) -> PathBuf {
        self.app_data.join("blueprint.sqlite3")
    }
    fn validate_managed_paths(&self) -> Result<(), OpError> {
        for name in [
            "operations.lock",
            "operations.sqlite3",
            "operations.sqlite3-wal",
            "operations.sqlite3-shm",
            "blueprint.sqlite3",
            "blueprint.sqlite3-wal",
            "blueprint.sqlite3-shm",
        ] {
            match std::fs::symlink_metadata(self.app_data.join(name)) {
                Ok(metadata) => {
                    let linked = metadata.file_type().is_symlink();
                    #[cfg(windows)]
                    let linked = {
                        use std::os::windows::fs::MetadataExt;
                        linked || metadata.file_attributes() & 0x400 != 0
                    };
                    if linked {
                        return Err(OpError::new(
                            ErrorCode::ReparsePoint,
                            Phase::Inspect,
                            "managed app-data files must not be links or reparse points",
                        ));
                    }
                    if !metadata.is_file() {
                        return Err(OpError::new(
                            ErrorCode::InvalidPath,
                            Phase::Inspect,
                            "managed app-data path is not a regular file",
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        Ok(())
    }
    fn open_lock(&self) -> Result<File, OpError> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.app_data.join("operations.lock"))
            .map_err(io_error)
    }
    pub fn try_acquire(&self, kind: LeaseKind) -> Result<Lease, OpError> {
        self.validate_managed_paths()?;
        let file = self.open_lock()?;
        let locked = match kind {
            LeaseKind::Read => FileExt::try_lock_shared(&file),
            _ => FileExt::try_lock_exclusive(&file),
        }
        .map_err(io_error)?;
        if !locked {
            return Err(OpError::new(
                ErrorCode::Busy,
                Phase::Inspect,
                "another collection job is active",
            ));
        }
        let pending = Journal::coordination_status(&self.database_path())?.requires_recovery();
        if pending && kind != LeaseKind::Recovery {
            return Err(OpError::new(
                ErrorCode::RecoveryRequired,
                Phase::Inspect,
                "an interrupted operation requires recovery",
            ));
        }
        if kind != LeaseKind::Read {
            self.active_writers.fetch_add(1, Ordering::SeqCst);
        }
        Ok(Lease {
            not_sync: std::marker::PhantomData,
            file,
            database: self.database_path(),
            kind,
            active_writers: self.active_writers.clone(),
        })
    }
    /// Covers all local Write/Recovery holders, including future Rename jobs.
    pub fn has_active_writer(&self) -> bool {
        self.active_writers.load(Ordering::SeqCst) != 0
    }
}
impl Lease {
    pub fn begin_legacy_mutation(&self) -> Result<LegacyMutation<'_>, OpError> {
        if self.kind != LeaseKind::Write {
            return Err(OpError::new(
                ErrorCode::PermissionDenied,
                Phase::Write,
                "legacy actions require a normal write lease",
            ));
        }
        if Journal::coordination_status(&self.database)?.requires_recovery() {
            return Err(OpError::new(
                ErrorCode::RecoveryRequired,
                Phase::Write,
                "pending operation requires recovery",
            ));
        }
        Journal::open(&self.database)?.change_coordination(false, false, Some(true))?;
        Ok(LegacyMutation { lease: self })
    }
    pub fn kind(&self) -> LeaseKind {
        self.kind
    }
    pub fn status(&self) -> Result<CoordinationStatus, OpError> {
        Journal::coordination_status(&self.database)
    }
    pub fn epochs(&self) -> Result<Epochs, OpError> {
        Ok(self.status()?.epochs)
    }
    pub fn verify_epochs(&self, expected: Epochs) -> Result<(), OpError> {
        if self.epochs()? != expected {
            return Err(OpError::new(
                ErrorCode::StaleState,
                Phase::Verify,
                "collection namespace or protected-root policy changed; preview again",
            ));
        }
        Ok(())
    }
    fn writable(&self) -> Result<(), OpError> {
        if self.kind == LeaseKind::Read {
            return Err(OpError::new(
                ErrorCode::PermissionDenied,
                Phase::Write,
                "read lease cannot mutate collection state",
            ));
        }
        Ok(())
    }
    pub fn advance_namespace(&self) -> Result<Epochs, OpError> {
        self.writable()?;
        Journal::open(&self.database)?.change_coordination(true, false, None)
    }
    pub fn advance_policy(&self) -> Result<Epochs, OpError> {
        self.writable()?;
        Journal::open(&self.database)?.change_coordination(false, true, None)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Closing this independently opened handle also releases the OS lock on panic.
        let _ = FileExt::unlock(&self.file);
        if self.kind != LeaseKind::Read {
            self.active_writers.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
fn io_error(e: std::io::Error) -> OpError {
    OpError::new(
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            ErrorCode::PermissionDenied
        } else {
            ErrorCode::Io
        },
        Phase::Inspect,
        e.to_string(),
    )
}

#[cfg(target_os = "linux")]
fn validate_local(path: &Path) -> Result<(), OpError> {
    let directory = File::open(path).map_err(io_error)?;
    let fs = rustix::fs::fstatfs(&directory).map_err(|e| io_error(e.into()))?;
    // Local storage classification only; this does not enable any media mutation capability.
    if !matches!(
        fs.f_type as u64,
        0xef53 | 0x5846_5342 | 0x9123_683e | 0x0102_1994 | 0x794c_7630 | 0x2fc1_2fc1
    ) {
        return Err(OpError::new(
            ErrorCode::Unsupported,
            Phase::Inspect,
            "operation store requires a verified local filesystem",
        ));
    }
    Ok(())
}
#[cfg(windows)]
fn validate_local(path: &Path) -> Result<(), OpError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW},
    };
    let native: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut volume = vec![0u16; 32768];
    unsafe {
        GetVolumePathNameW(PCWSTR(native.as_ptr()), &mut volume)
            .map_err(|e| OpError::new(ErrorCode::Unsupported, Phase::Inspect, e.to_string()))?;
        // DRIVE_FIXED: remote, unknown and removable app-data volumes fail closed.
        if GetDriveTypeW(PCWSTR(volume.as_ptr())) != 3 {
            return Err(OpError::new(
                ErrorCode::Unsupported,
                Phase::Inspect,
                "operation store requires a fixed local volume",
            ));
        }
    }
    Ok(())
}
#[cfg(not(any(target_os = "linux", windows)))]
fn validate_local(_path: &Path) -> Result<(), OpError> {
    Err(OpError::unsupported(Phase::Inspect))
}

/// Transitional legacy action batch; not a per-item execution journal.
pub struct LegacyMutation<'a> {
    lease: &'a Lease,
}
impl LegacyMutation<'_> {
    /// Call only after a known outcome, including reported partial success.
    /// Dropping without finishing leaves the durable gate active, including on panic.
    pub fn finish(self) -> Result<Epochs, OpError> {
        Journal::open(&self.lease.database)?.change_coordination(true, false, Some(false))
    }
}
