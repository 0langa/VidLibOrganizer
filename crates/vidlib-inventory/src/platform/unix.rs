use crate::PhysicalId;
use std::{fs::Metadata, path::Path};

pub(crate) fn physical_id(_path: &Path, metadata: &Metadata) -> Option<PhysicalId> {
    use std::os::unix::fs::MetadataExt;
    Some(PhysicalId {
        platform: crate::PathPlatform::UnixBytes,
        volume_identity: metadata.dev().to_be_bytes().to_vec(),
        file_identity: metadata.ino().to_be_bytes().to_vec(),
    })
}
pub(crate) fn attributes(_metadata: &Metadata) -> Option<u32> {
    None
}

pub(crate) fn read_directory(path: &Path) -> std::io::Result<crate::DirectoryListing> {
    use rustix::fs::{open, Dir, Mode, OFlags};
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let directory = Dir::new(fd)?;
    let mut listing = crate::DirectoryListing::default();
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
