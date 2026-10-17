//! Read-only content preparation, anchored to retained directory and file handles.
use crate::*;
use std::{
    ffi::OsStr,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

pub trait InventoryFile: Read {
    fn observation(&self) -> io::Result<FileObservation>;
    /// Stable path to this held file, usable only while the owner remains alive.
    fn helper_path(&self) -> io::Result<PathBuf> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "safe metadata helper handoff is unavailable on this platform",
        ))
    }
}
fn stale() -> io::Error {
    io::Error::other("source identity, ancestry or material freshness changed")
}
fn identity(actual: &FileObservation, expected: &FileStamp, kind: EntryKind) -> io::Result<()> {
    if actual.kind != kind
        || expected.physical_id.is_none()
        || actual.stamp.physical_id != expected.physical_id
    {
        return Err(stale());
    }
    Ok(())
}
fn material(actual: &FileObservation, expected: &FileStamp) -> io::Result<()> {
    identity(actual, expected, EntryKind::File)?;
    // Reading can update access time. It does not change content freshness.
    if actual.stamp.size != expected.size
        || actual.stamp.modified_ns != expected.modified_ns
        || expected
            .created_ns
            .is_some_and(|ns| actual.stamp.created_ns != Some(ns))
        || actual.stamp.windows_attributes != expected.windows_attributes
    {
        return Err(stale());
    }
    Ok(())
}
/// Invokes the callback only after grant, root, ancestry and source verification.
/// Returns its result only after rechecking the held source and every parent binding.
/// The callback receives a reader, never a path or a writable handle.
pub fn with_verified_content<T>(
    inventory: &Inventory,
    entry_id: ids::EntryId,
    operation: impl FnOnce(&mut dyn Read) -> io::Result<T>,
) -> io::Result<T> {
    with_verified_file(inventory, entry_id, false, |file| operation(file))
}
/// Runs a synchronous read-only helper against a source pinned to the selected grant.
/// The callback must finish/reap its children before returning. Linux receives a parent
/// proc-fd path; Windows receives the original path with source and ancestry held against
/// replacement (and source writes). No unsafe absolute-path fallback is permitted.
pub fn with_verified_helper_path<T>(
    inventory: &Inventory,
    entry_id: ids::EntryId,
    operation: impl FnOnce(&Path) -> io::Result<T>,
) -> io::Result<T> {
    with_verified_file(inventory, entry_id, true, |file| {
        #[cfg(windows)]
        let path = {
            let _ = file;
            inventory
                .entries
                .iter()
                .find(|e| e.id == entry_id)
                .ok_or_else(stale)?
                .absolute_path
                .clone()
        };
        #[cfg(not(windows))]
        let path = file.helper_path()?;
        operation(&path)
    })
}
fn with_verified_file<T>(
    inventory: &Inventory,
    entry_id: ids::EntryId,
    helper: bool,
    operation: impl FnOnce(&mut dyn InventoryFile) -> io::Result<T>,
) -> io::Result<T> {
    let entry = inventory
        .entries
        .iter()
        .find(|e| e.id == entry_id)
        .ok_or_else(stale)?;
    let root = inventory
        .roots
        .iter()
        .find(|r| r.id == entry.root_id)
        .ok_or_else(stale)?;
    if entry.kind != EntryKind::File
        || !entry.issues.is_empty()
        || root.path != root.source_grant.path
        || !entry.absolute_path.starts_with(&root.path)
        || matches!(
            root.evidence.state,
            RootEvidenceState::Mismatch | RootEvidenceState::Unavailable
        )
        || root.evidence.observed_identity.is_none()
        || root
            .evidence
            .expected_identity
            .as_ref()
            .is_some_and(|id| Some(id) != root.evidence.observed_identity.as_ref())
    {
        return Err(stale());
    }
    let base = if entry.absolute_path == root.path {
        root.path.parent().ok_or_else(stale)?
    } else {
        &root.path
    };
    if base.join(decode_path(&entry.relative_path).map_err(|_| stale())?) != entry.absolute_path {
        return Err(stale());
    }
    let mut anchor = PathBuf::new();
    let mut names = Vec::new();
    for part in entry.absolute_path.components() {
        match part {
            Component::Prefix(_) | Component::RootDir => anchor.push(part.as_os_str()),
            Component::Normal(name) => names.push(name),
            _ => return Err(stale()),
        }
    }
    if !anchor.is_absolute() || names.is_empty() {
        return Err(stale());
    }
    let directory = if helper {
        platform::open_helper_directory(&anchor)?
    } else {
        platform::open_directory(&anchor)?
    };
    let before = directory.observation()?;
    identity(&before, &before.stamp, EntryKind::Directory)?;
    // A selected filesystem anchor is never visited as a descended child.
    // Bind it to the scan evidence before any content callback, including cached reads.
    if anchor == root.path && before.stamp.physical_id != root.evidence.observed_identity {
        return Err(stale());
    }
    let result = descend(
        inventory,
        root,
        entry,
        directory.as_ref(),
        &anchor,
        &names,
        operation,
    )?;
    identity(
        &platform::open_directory(&anchor)?.observation()?,
        &before.stamp,
        EntryKind::Directory,
    )?;
    Ok(result)
}
fn descend<T>(
    inventory: &Inventory,
    root: &RootBinding,
    entry: &EntrySnapshot,
    parent: &dyn InventoryDirectory,
    path: &Path,
    names: &[&OsStr],
    operation: impl FnOnce(&mut dyn InventoryFile) -> io::Result<T>,
) -> io::Result<T> {
    let name = names[0];
    let child_path = path.join(name);
    if names.len() == 1 {
        let mut file = parent.open_child_file(name)?;
        material(&file.observation()?, &entry.stamp)?;
        material(&parent.observe_child(name)?, &entry.stamp)?;
        if child_path == root.path && entry.stamp.physical_id != root.evidence.observed_identity {
            return Err(stale());
        }
        let result = operation(file.as_mut())?;
        material(&file.observation()?, &entry.stamp)?;
        material(&parent.observe_child(name)?, &entry.stamp)?;
        return Ok(result);
    }
    let child = parent.open_child_directory(name)?;
    let before = child.observation()?;
    identity(&before, &before.stamp, EntryKind::Directory)?;
    if child_path == root.path && before.stamp.physical_id != root.evidence.observed_identity {
        return Err(stale());
    }
    if let Some(expected) = inventory
        .entries
        .iter()
        .find(|e| e.absolute_path == child_path)
    {
        identity(&before, &expected.stamp, EntryKind::Directory)?;
    }
    identity(
        &parent.observe_child(name)?,
        &before.stamp,
        EntryKind::Directory,
    )?;
    let result = descend(
        inventory,
        root,
        entry,
        child.as_ref(),
        &child_path,
        &names[1..],
        operation,
    )?;
    identity(&child.observation()?, &before.stamp, EntryKind::Directory)?;
    identity(
        &parent.observe_child(name)?,
        &before.stamp,
        EntryKind::Directory,
    )?;
    Ok(result)
}
