use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tempfile::{tempdir, TempDir};
use vidlib_inventory::{
    scan_with_fs, DirectoryListing, InventoryFs, InventoryRequest, JobControl, RootBinding,
};

pub struct Fixture {
    pub dir: TempDir,
}
impl Fixture {
    pub fn new() -> Self {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.mkv"), b"video").unwrap();
        fs::write(dir.path().join("b.mp4"), b"").unwrap();
        fs::write(dir.path().join("notes.txt"), b"notes").unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("nested/c.mov"), b"other").unwrap();
        fs::hard_link(dir.path().join("a.mkv"), dir.path().join("alias.mkv")).unwrap();
        Self { dir }
    }
    pub fn root(&self) -> &Path {
        self.dir.path()
    }
    pub fn request(&self) -> InventoryRequest {
        InventoryRequest::new(vec![RootBinding::new(self.root().to_path_buf()).unwrap()])
    }
}

pub struct FaultFs {
    pub fail_stat: Option<PathBuf>,
    pub fail_listing: Option<PathBuf>,
    pub partial_listing: Option<PathBuf>,
    pub cancel_after_stat: Option<usize>,
    pub control: JobControl,
    pub stat_count: AtomicUsize,
    pub swap_after_listing: Option<(PathBuf, PathBuf, PathBuf)>,
    pub omit_root_identity: bool,
    pub omit_handle_identity: bool,
    pub wrong_handle_identity: bool,
    pub invalid_directory_kind: bool,
    swapped: AtomicBool,
}
impl FaultFs {
    pub fn new(control: JobControl) -> Self {
        Self {
            fail_stat: None,
            fail_listing: None,
            partial_listing: None,
            cancel_after_stat: None,
            control,
            stat_count: AtomicUsize::new(0),
            swap_after_listing: None,
            omit_root_identity: false,
            omit_handle_identity: false,
            wrong_handle_identity: false,
            invalid_directory_kind: false,
            swapped: AtomicBool::new(false),
        }
    }
}
impl FaultFs {
    fn before_stat(&self, path: &Path) -> io::Result<()> {
        let count = self.stat_count.fetch_add(1, Ordering::SeqCst) + 1;
        if self.cancel_after_stat == Some(count) {
            self.control.cancel();
        }
        if self.fail_stat.as_deref() == Some(path) {
            return Err(io::Error::other("injected stat failure"));
        }
        Ok(())
    }
    fn after_listing(&self, path: &Path, listing: &mut DirectoryListing) -> io::Result<()> {
        if self.partial_listing.as_deref() == Some(path) {
            listing
                .errors
                .push("injected partial listing failure".into());
        }
        if let Some((target, moved, external)) = &self.swap_after_listing {
            if target == path && !self.swapped.swap(true, Ordering::SeqCst) {
                fs::rename(target, moved)?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(external, target)?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(external, target)?;
            }
        }
        Ok(())
    }
}
impl InventoryFs for FaultFs {
    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
        self.before_stat(path)?;
        fs::symlink_metadata(path)
    }
    fn physical_id(
        &self,
        path: &Path,
        metadata: &fs::Metadata,
    ) -> Option<vidlib_inventory::PhysicalId> {
        if self.omit_root_identity {
            None
        } else {
            vidlib_inventory::OsInventoryFs.physical_id(path, metadata)
        }
    }
    fn open_directory(
        &self,
        path: &Path,
    ) -> io::Result<Box<dyn vidlib_inventory::InventoryDirectory + '_>> {
        Ok(Box::new(FaultDirectory {
            adapter: self,
            path: path.to_path_buf(),
            inner: vidlib_inventory::OsInventoryFs.open_directory(path)?,
        }))
    }
}
struct FaultDirectory<'a> {
    adapter: &'a FaultFs,
    path: PathBuf,
    inner: Box<dyn vidlib_inventory::InventoryDirectory + 'a>,
}
impl vidlib_inventory::InventoryDirectory for FaultDirectory<'_> {
    fn observation(&self) -> io::Result<vidlib_inventory::FileObservation> {
        let mut observed = self.inner.observation()?;
        if self.adapter.omit_handle_identity {
            observed.stamp.physical_id = None;
        }
        if self.adapter.wrong_handle_identity {
            if let Some(identity) = &mut observed.stamp.physical_id {
                identity.file_identity = vec![0];
            }
        }
        if self.adapter.invalid_directory_kind {
            observed.kind = vidlib_inventory::EntryKind::Other;
        }
        Ok(observed)
    }
    fn read_directory(&self) -> io::Result<DirectoryListing> {
        if self.adapter.fail_listing.as_deref() == Some(&self.path) {
            return Err(io::Error::other("injected listing failure"));
        }
        let mut listing = self.inner.read_directory()?;
        self.adapter.after_listing(&self.path, &mut listing)?;
        Ok(listing)
    }
    fn observe_child(
        &self,
        name: &std::ffi::OsStr,
    ) -> io::Result<vidlib_inventory::FileObservation> {
        self.adapter.before_stat(&self.path.join(name))?;
        let mut observed = self.inner.observe_child(name)?;
        if self.adapter.omit_handle_identity {
            observed.stamp.physical_id = None;
        }
        Ok(observed)
    }
    fn open_child_directory(
        &self,
        name: &std::ffi::OsStr,
    ) -> io::Result<Box<dyn vidlib_inventory::InventoryDirectory + '_>> {
        Ok(Box::new(FaultDirectory {
            adapter: self.adapter,
            path: self.path.join(name),
            inner: self.inner.open_child_directory(name)?,
        }))
    }
}

pub fn fault_scan(fixture: &Fixture, fs: &FaultFs) -> vidlib_inventory::Inventory {
    scan_with_fs(&fixture.request(), &fs.control, fs).unwrap()
}
