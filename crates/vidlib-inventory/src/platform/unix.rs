use crate::{
    DirectoryListing, EntryKind, FileObservation, FileStamp, InventoryDirectory, PathPlatform,
    PhysicalId,
};
use rustix::fs::{open, openat, statat, AtFlags, Dir, FileType, Mode, OFlags};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, Metadata},
    io,
    os::unix::ffi::OsStringExt,
    path::Path,
};

pub(crate) fn physical_id(_path: &Path, metadata: &Metadata) -> Option<PhysicalId> {
    use std::os::unix::fs::MetadataExt;
    Some(identity(metadata.dev(), metadata.ino()))
}
fn identity(device: u64, inode: u64) -> PhysicalId {
    PhysicalId {
        platform: PathPlatform::UnixBytes,
        volume_identity: device.to_be_bytes().to_vec(),
        file_identity: inode.to_be_bytes().to_vec(),
    }
}
pub(crate) fn attributes(_metadata: &Metadata) -> Option<u32> {
    None
}

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
pub(crate) fn open_directory(path: &Path) -> io::Result<Box<dyn InventoryDirectory>> {
    Ok(Box::new(UnixDirectory {
        file: open(path, DIRECTORY_FLAGS, Mode::empty())?.into(),
    }))
}
struct UnixDirectory {
    file: File,
}
struct UnixFile(File);
impl std::io::Read for UnixFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        std::io::Read::read(&mut self.0, buffer)
    }
}
impl crate::InventoryFile for UnixFile {
    fn observation(&self) -> io::Result<FileObservation> {
        let metadata = self.0.metadata()?;
        Ok(FileObservation {
            kind: crate::scan::kind(&metadata),
            stamp: super::stamp(&metadata, physical_id(Path::new(""), &metadata)),
        })
    }
}
impl InventoryDirectory for UnixDirectory {
    fn open_child_file(&self, name: &OsStr) -> io::Result<Box<dyn crate::InventoryFile + '_>> {
        crate::scan::validate_child_name(name)?;
        // NONBLOCK prevents an attacker-swapped FIFO from blocking preparation.
        let file = openat(
            &self.file,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )?;
        Ok(Box::new(UnixFile(file.into())))
    }
    fn observation(&self) -> io::Result<FileObservation> {
        let metadata = self.file.metadata()?;
        Ok(FileObservation {
            kind: EntryKind::Directory,
            stamp: super::stamp(&metadata, physical_id(Path::new(""), &metadata)),
        })
    }
    fn read_directory(&self) -> io::Result<DirectoryListing> {
        let directory = Dir::read_from(&self.file)?;
        let mut listing = DirectoryListing::default();
        for child in directory {
            match child {
                Ok(entry) => {
                    let name = entry.file_name().to_bytes();
                    if name != b"." && name != b".." {
                        listing.names.push(OsString::from_vec(name.to_vec()));
                    }
                }
                Err(error) => listing.errors.push(error.to_string()),
            }
        }
        Ok(listing)
    }
    fn observe_child(&self, name: &OsStr) -> io::Result<FileObservation> {
        crate::scan::validate_child_name(name)?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use rustix::fs::{statx, StatxFlags};
            match statx(
                &self.file,
                name,
                AtFlags::SYMLINK_NOFOLLOW,
                StatxFlags::BASIC_STATS | StatxFlags::BTIME,
            ) {
                Ok(stat) => {
                    let kind = kind(FileType::from_raw_mode(u32::from(stat.stx_mode)));
                    let nanos = |seconds: i64, nanoseconds: u32| {
                        i128::from(seconds) * 1_000_000_000 + i128::from(nanoseconds)
                    };
                    return Ok(FileObservation {
                        kind,
                        stamp: FileStamp {
                            physical_id: Some(identity(
                                rustix::fs::makedev(stat.stx_dev_major, stat.stx_dev_minor),
                                stat.stx_ino,
                            )),
                            size: (kind == EntryKind::File).then_some(stat.stx_size),
                            created_ns: (stat.stx_mask & StatxFlags::BTIME.bits() != 0)
                                .then_some(nanos(stat.stx_btime.tv_sec, stat.stx_btime.tv_nsec)),
                            modified_ns: Some(nanos(stat.stx_mtime.tv_sec, stat.stx_mtime.tv_nsec)),
                            accessed_ns: Some(nanos(stat.stx_atime.tv_sec, stat.stx_atime.tv_nsec)),
                            windows_attributes: None,
                        },
                    });
                }
                Err(rustix::io::Errno::NOSYS) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let stat = statat(&self.file, name, AtFlags::SYMLINK_NOFOLLOW)?;
        let kind = kind(FileType::from_raw_mode(stat.st_mode));
        Ok(FileObservation {
            kind,
            stamp: FileStamp {
                physical_id: Some(identity(stat.st_dev, stat.st_ino)),
                size: (kind == EntryKind::File).then_some(stat.st_size as u64),
                created_ns: None,
                modified_ns: Some(
                    i128::from(stat.st_mtime) * 1_000_000_000 + i128::from(stat.st_mtime_nsec),
                ),
                accessed_ns: Some(
                    i128::from(stat.st_atime) * 1_000_000_000 + i128::from(stat.st_atime_nsec),
                ),
                windows_attributes: None,
            },
        })
    }
    fn open_child_directory(&self, name: &OsStr) -> io::Result<Box<dyn InventoryDirectory + '_>> {
        crate::scan::validate_child_name(name)?;
        Ok(Box::new(Self {
            file: openat(&self.file, name, DIRECTORY_FLAGS, Mode::empty())?.into(),
        }))
    }
}
fn kind(file_type: FileType) -> EntryKind {
    match file_type {
        FileType::RegularFile => EntryKind::File,
        FileType::Directory => EntryKind::Directory,
        FileType::Symlink => EntryKind::Symlink,
        _ => EntryKind::Other,
    }
}
