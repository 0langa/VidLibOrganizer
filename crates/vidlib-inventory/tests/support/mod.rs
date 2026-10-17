use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
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
        }
    }
}
impl InventoryFs for FaultFs {
    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
        let count = self.stat_count.fetch_add(1, Ordering::SeqCst) + 1;
        if self.cancel_after_stat == Some(count) {
            self.control.cancel();
        }
        if self.fail_stat.as_deref() == Some(path) {
            return Err(io::Error::other("injected stat failure"));
        }
        fs::symlink_metadata(path)
    }
    fn read_directory(&self, path: &Path) -> io::Result<DirectoryListing> {
        if self.fail_listing.as_deref() == Some(path) {
            return Err(io::Error::other("injected listing failure"));
        }
        let mut listing = vidlib_inventory::OsInventoryFs.read_directory(path)?;
        if self.partial_listing.as_deref() == Some(path) {
            listing
                .errors
                .push("injected partial listing failure".into());
        }
        Ok(listing)
    }
}

pub fn fault_scan(fixture: &Fixture, fs: &FaultFs) -> vidlib_inventory::Inventory {
    scan_with_fs(&fixture.request(), &fs.control, fs).unwrap()
}
