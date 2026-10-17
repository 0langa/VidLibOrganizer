use std::{
    cell::Cell,
    fs::File,
    path::{Path, PathBuf},
};
use vidlib_fileops::{platform::*, *};
use vidlib_inventory::*;
/// Native effects with process termination at externally observable phase seams.
pub struct CopyFs {
    pub native: PlatformFs,
    pub metadata_observation: Option<(PathBuf, FileStamp)>,
    pub boundary: String,
    pub creates: Cell<usize>,
    pub cancel: Option<JobControl>,
    fired: Cell<bool>,
    metadata_calls: usize,
}
impl CopyFs {
    pub fn new(boundary: &str) -> Self {
        Self {
            native: PlatformFs::default(),
            metadata_observation: None,
            boundary: boundary.into(),
            creates: Cell::new(0),
            cancel: None,
            fired: Cell::new(false),
            metadata_calls: 0,
        }
    }
    fn replace_parent(&self, path: &Path, boundary: &str) {
        if self.boundary == boundary {
            let parent = path.parent().unwrap();
            std::fs::rename(parent, parent.with_extension("prior")).unwrap();
            std::fs::create_dir(parent).unwrap();
        }
    }
    fn check_closed_writer(&self, path: &Path) {
        if self.boundary != "closed-writer" {
            return;
        }
        for entry in std::fs::read_dir("/proc/self/fd").unwrap().flatten() {
            if std::fs::read_link(entry.path()).is_ok_and(|p| p == path) {
                let info =
                    std::fs::read_to_string(Path::new("/proc/self/fdinfo").join(entry.file_name()))
                        .unwrap();
                let flags = info
                    .lines()
                    .find_map(|line| line.strip_prefix("flags:\t"))
                    .unwrap();
                assert_eq!(
                    u32::from_str_radix(flags, 8).unwrap() & 3,
                    0,
                    "staging writer remains open at read/publication boundary"
                );
            }
        }
    }
    fn at(&self, name: &str) {
        if self.boundary == name {
            std::process::exit(86);
        }
    }
}
impl Filesystem for CopyFs {
    fn capture_namespace(&self, g: &GrantSet, c: &[PathBuf]) -> Result<NamespaceSnapshot, OpError> {
        let mut snapshot = self.native.capture_namespace(g, c)?;
        if let Some((path, stamp)) = &self.metadata_observation {
            for parent in &mut snapshot.parents {
                if parent.parent_path == *path {
                    parent.stamp = stamp.clone();
                }
                for entry in &mut parent.entries {
                    if entry.absolute_path == *path {
                        entry.stamp = stamp.clone();
                    }
                }
            }
        }
        if self.boundary == "changed-descendant-attributes" && self.creates.get() > 0 {
            for parent in &mut snapshot.parents {
                for entry in &mut parent.entries {
                    if entry.kind == EntryKind::Directory
                        && entry.absolute_path.file_name().is_some_and(|n| n == "sub")
                    {
                        entry.stamp.windows_attributes = Some(1);
                    }
                }
            }
        }
        if self.boundary == "unstatable-child" && self.creates.get() > 0 {
            for parent in &mut snapshot.parents {
                if parent.parent_path.file_name().is_some_and(|n| n == "sub") {
                    parent.children_complete = false;
                }
            }
        }
        Ok(snapshot)
    }
    fn inspect(&self, p: &Path) -> Result<FileObservation, OpError> {
        let mut observation = self.native.inspect(p)?;
        if let Some((path, stamp)) = &self.metadata_observation {
            if path == p {
                observation.stamp = stamp.clone();
            }
        }
        if self.boundary == "changed-directory-attributes"
            && observation.kind == EntryKind::Directory
            && p.file_name().is_some_and(|n| n == "sub")
        {
            observation.stamp.windows_attributes = Some(1);
        }
        Ok(observation)
    }
    fn verify(&self, p: &Path, s: &FileStamp) -> Result<FileObservation, OpError> {
        if self
            .metadata_observation
            .as_ref()
            .is_some_and(|(path, _)| path == p)
        {
            let actual = self.inspect(p)?;
            if actual.stamp != *s {
                return Err(OpError::new(
                    ErrorCode::StaleState,
                    Phase::Verify,
                    "injected metadata poststate differs",
                ));
            }
            return Ok(actual);
        }
        self.native.verify(p, s)
    }
    fn rename_reviewed(&mut self, r: &RenameRequest<'_>) -> Result<MutationOutcome, OpError> {
        self.at("before-rename");
        let x = self.native.rename_reviewed(r)?;
        self.at("after-rename");
        Ok(x)
    }
    fn rename_reserved(&mut self, r: &RenameReservation) -> Result<MutationOutcome, OpError> {
        self.at("before-rename");
        if self.boundary == "change-atime-before-rename" {
            let before = self.native.inspect(r.source())?.stamp;
            self.native.set_metadata(
                r.source(),
                &before,
                &MetadataUpdate {
                    accessed_ns: Some(1_650_000_000_000_000_000),
                    ..Default::default()
                },
            )?;
        }
        let directory = self.native.inspect(r.source())?.kind == EntryKind::Directory;
        let mut x = self.native.rename_reserved(r)?;
        if self.boundary == "after-quarantine-receipt" {
            self.fired.set(true);
        }
        if self.boundary == "change-atime-after-rename" {
            let path = r.target();
            std::fs::File::open(path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_accessed(
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_650_000_000),
                ))
                .unwrap();
            x.observation = self.native.inspect(path)?;
        }
        if directory {
            self.at("after-directory-quarantine");
        }
        self.at("after-rename");
        Ok(x)
    }
    fn rename_no_replace(
        &mut self,
        s: &Path,
        t: &Path,
        e: &FileStamp,
    ) -> Result<MutationOutcome, OpError> {
        self.native.rename_no_replace(s, t, e)
    }
    fn create_directory(&mut self, p: &Path) -> Result<OwnedEntry, OpError> {
        self.at("before-directory");
        let x = self.native.create_directory(p)?;
        self.at("after-directory");
        Ok(x)
    }
    fn create_staging(&mut self, p: &Path) -> Result<StagingFile, OpError> {
        self.replace_parent(p, "replace-parent-before-create");
        self.at("before-create");
        let x = self.native.create_staging(p)?;
        self.creates.set(self.creates.get() + 1);
        self.at("after-create");
        Ok(x)
    }
    fn create_directory_reviewed(
        &mut self,
        p: &Path,
        parent: &PhysicalId,
    ) -> Result<OwnedEntry, OpError> {
        self.at("before-directory");
        let x = self.native.create_directory_reviewed(p, parent)?;
        self.at("after-directory");
        Ok(x)
    }
    fn create_staging_reviewed(
        &mut self,
        p: &Path,
        parent: &PhysicalId,
    ) -> Result<StagingFile, OpError> {
        self.replace_parent(p, "replace-parent-before-create");
        self.at("before-create");
        let x = self.native.create_staging_reviewed(p, parent)?;
        self.creates.set(self.creates.get() + 1);
        self.at("after-create");
        Ok(x)
    }
    fn publish_staging_reviewed(
        &mut self,
        o: &OwnedEntry,
        t: &Path,
        sp: &PhysicalId,
        tp: &PhysicalId,
    ) -> Result<MutationOutcome, OpError> {
        self.check_closed_writer(&o.path);
        self.at("before-publish");
        if self.boundary == "publication-race" && !self.fired.replace(true) {
            std::fs::write(t, b"external winner").unwrap();
        }
        if self.boundary == "publication-permission" && !self.fired.replace(true) {
            return Err(OpError::new(
                ErrorCode::PermissionDenied,
                Phase::Rename,
                "injected publication permission denial",
            ));
        }
        let x = self.native.publish_staging_reviewed(o, t, sp, tp)?;
        if self.boundary == "cancel-after-publish" {
            self.cancel.as_ref().unwrap().cancel();
        }
        self.at("after-publish");
        Ok(x)
    }
    fn remove_owned_reviewed(
        &mut self,
        o: &OwnedEntry,
        p: &PhysicalId,
    ) -> Result<Durability, OpError> {
        self.at("before-cleanup");
        let directory = self.native.inspect(&o.path)?.kind == EntryKind::Directory;
        let x = self.native.remove_owned_reviewed(o, p)?;
        if directory {
            self.at("after-owned-directory-remove");
        }
        self.at("after-cleanup");
        Ok(x)
    }
    fn open_owned_staging(
        &mut self,
        e: &OwnedEntry,
        p: &PhysicalId,
    ) -> Result<StagingFile, OpError> {
        self.at("before-stream");
        if matches!(self.boundary.as_str(), "mid-stream" | "cancel-mid-stream")
            && !self.fired.replace(true)
        {
            let path = e.path.clone();
            let control = self.cancel.clone();
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while std::time::Instant::now() < deadline {
                    if std::fs::metadata(&path)
                        .is_ok_and(|m| m.len() >= 1024 * 1024 && m.len() < 16 * 1024 * 1024)
                    {
                        if let Some(control) = control {
                            control.cancel();
                            return;
                        }
                        std::process::exit(86);
                    }
                    std::thread::yield_now();
                }
            });
        }
        self.native.open_owned_staging(e, p)
    }
    fn refresh_staging(&self, s: &mut StagingFile) -> Result<Durability, OpError> {
        let x = self.native.refresh_staging(s)?;
        if self.boundary == "disk-full"
            && s.ownership.stamp.size.unwrap_or(0) > 0
            && !self.fired.replace(true)
        {
            return Err(OpError::new(
                ErrorCode::Io,
                Phase::Write,
                std::io::Error::from_raw_os_error(28).to_string(),
            ));
        }
        if s.ownership.stamp.size.unwrap_or(0) > 0 {
            self.at("after-stream");
        }
        Ok(x)
    }
    fn preserve_staging_metadata(
        &self,
        s: &mut StagingFile,
        u: &MetadataUpdate,
    ) -> Result<Durability, OpError> {
        self.native.preserve_staging_metadata(s, u)
    }
    fn open_verified(&self, p: &Path, s: &FileStamp) -> Result<File, OpError> {
        // The next historical-digest check runs after durable rename completion,
        // before the cleanup intent. The parent asserts that exact journal state.
        if self.boundary == "after-quarantine-receipt" && self.fired.get() {
            self.at("after-quarantine-receipt");
        }
        self.check_closed_writer(p);
        self.native.open_verified(p, s)
    }
    fn publish_staging(&mut self, o: &OwnedEntry, t: &Path) -> Result<MutationOutcome, OpError> {
        self.at("before-publish");
        let x = self.native.publish_staging(o, t)?;
        self.at("after-publish");
        Ok(x)
    }
    fn set_metadata(
        &mut self,
        p: &Path,
        s: &FileStamp,
        u: &MetadataUpdate,
    ) -> Result<MutationOutcome, OpError> {
        self.metadata_calls += 1;
        if self.boundary == "metadata-second-fail" && self.metadata_calls == 2 {
            return Err(OpError::new(
                ErrorCode::Io,
                Phase::Metadata,
                "injected second-field failure",
            ));
        }
        if self.metadata_calls == 2 {
            self.at("before-second-metadata");
        }
        self.at("before-metadata");
        let result = self.native.set_metadata(p, s, u)?;
        self.at(&format!("after-metadata-{}", self.metadata_calls));
        self.at("after-metadata");
        Ok(result)
    }
    fn flush_directory(&self, p: &Path) -> Result<Durability, OpError> {
        self.native.flush_directory(p)
    }
    fn remove_owned(&mut self, o: &OwnedEntry) -> Result<Durability, OpError> {
        self.at("before-cleanup");
        let x = self.native.remove_owned(o)?;
        self.at("after-cleanup");
        Ok(x)
    }
    fn remove_quarantined(&mut self, o: &OwnedRemoval) -> Result<Durability, OpError> {
        if self.boundary == "change-atime-before-cleanup" {
            std::fs::File::open(o.path())
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_accessed(
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_650_000_000),
                ))
                .unwrap();
        }
        let directory = self.native.inspect(o.path())?.kind == EntryKind::Directory;
        if self.boundary == "remove-permission" && !self.fired.replace(true) {
            return Err(OpError::new(
                ErrorCode::PermissionDenied,
                Phase::Remove,
                "injected removal permission denial",
            ));
        }
        self.at("before-remove");
        let x = self.native.remove_quarantined(o)?;
        if directory {
            self.at("after-directory-remove");
        }
        self.at("after-remove");
        Ok(x)
    }
}
