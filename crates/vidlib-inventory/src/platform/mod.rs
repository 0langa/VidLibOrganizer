use crate::{FileStamp, PhysicalId};
use std::{
    fs::Metadata,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn epoch_ns(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    }
}
pub(crate) fn now_ns() -> i128 {
    epoch_ns(SystemTime::now())
}

pub(crate) fn stamp(metadata: &Metadata, identity: Option<PhysicalId>) -> FileStamp {
    FileStamp {
        physical_id: identity,
        size: metadata.is_file().then_some(metadata.len()),
        created_ns: metadata.created().ok().map(epoch_ns),
        modified_ns: metadata.modified().ok().map(epoch_ns),
        accessed_ns: metadata.accessed().ok().map(epoch_ns),
        windows_attributes: attributes(metadata),
    }
}

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(unix)]
pub(crate) use unix::{attributes, open_directory, physical_id};
#[cfg(windows)]
pub(crate) use windows::{attributes, open_directory, physical_id};

#[cfg(unix)]
pub(crate) use unix::open_directory as open_helper_directory;
#[cfg(windows)]
pub(crate) use windows::open_helper_directory;
