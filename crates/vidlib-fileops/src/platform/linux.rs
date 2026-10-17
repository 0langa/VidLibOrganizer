use super::*;
use rustix::fs::{self, AtFlags, Mode, OFlags, RenameFlags};
use std::{
    ffi::OsString,
    os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, PermissionsExt},
    },
};
use vidlib_inventory::{
    CaseSensitivity, DirectoryListing, FilesystemCapabilities, NameNormalization, PathPlatform,
    PhysicalId, Support,
};

const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
pub(super) fn directory(path: &Path) -> Result<File, OpError> {
    let mut file: File = fs::open("/", DIRECTORY | OFlags::NOATIME, Mode::empty())
        .or_else(|e| {
            if e == rustix::io::Errno::PERM {
                fs::open("/", DIRECTORY, Mode::empty())
            } else {
                Err(e)
            }
        })
        .map_err(|e| io_error(e.into(), Phase::Inspect))?
        .into();
    for component in path.components() {
        if let std::path::Component::Normal(name) = component {
            file = open_child(&file, name, true)?;
        }
    }
    Ok(file)
}
pub(super) fn open_child(
    parent: &File,
    name: &std::ffi::OsStr,
    directory: bool,
) -> Result<File, OpError> {
    let flags = if directory {
        DIRECTORY
    } else {
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC
    };
    // Avoid changing an explicitly controlled atime while proving namespace or
    // full content. Unowned files may deny NOATIME; retain ordinary read support.
    fs::openat(parent, name, flags | OFlags::NOATIME, Mode::empty())
        .or_else(|e| {
            if e == rustix::io::Errno::PERM {
                fs::openat(parent, name, flags, Mode::empty())
            } else {
                Err(e)
            }
        })
        .map(File::from)
        .map_err(|e| {
            if e == rustix::io::Errno::LOOP
                || (e == rustix::io::Errno::NOTDIR
                    && fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).is_ok_and(|s| {
                        fs::FileType::from_raw_mode(s.st_mode) == fs::FileType::Symlink
                    }))
            {
                OpError::new(
                    ErrorCode::ReparsePoint,
                    Phase::Inspect,
                    "linked path component rejected",
                )
            } else {
                io_error(e.into(), Phase::Inspect)
            }
        })
}
pub(super) fn observation(file: &File) -> Result<FileObservation, OpError> {
    let m = file.metadata().map_err(|e| io_error(e, Phase::Inspect))?;
    let kind = if m.is_file() {
        EntryKind::File
    } else if m.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::Other
    };
    Ok(FileObservation {
        kind,
        stamp: FileStamp {
            physical_id: Some(PhysicalId {
                platform: PathPlatform::UnixBytes,
                volume_identity: m.dev().to_be_bytes().to_vec(),
                file_identity: m.ino().to_be_bytes().to_vec(),
            }),
            size: m.is_file().then_some(m.len()),
            created_ns: m.created().ok().map(epoch_ns),
            modified_ns: m.modified().ok().map(epoch_ns),
            accessed_ns: m.accessed().ok().map(epoch_ns),
            windows_attributes: None,
        },
    })
}
pub(super) fn listing(file: &File) -> Result<DirectoryListing, OpError> {
    let mut result = DirectoryListing::default();
    for entry in fs::Dir::read_from(file).map_err(|e| io_error(e.into(), Phase::Capture))? {
        match entry {
            Ok(entry) => {
                let bytes = entry.file_name().to_bytes();
                if bytes != b"." && bytes != b".." {
                    result.names.push(OsString::from_vec(bytes.to_vec()));
                }
            }
            Err(e) => result.errors.push(e.to_string()),
        }
    }
    Ok(result)
}
pub(super) fn child_observation(
    parent: &File,
    name: &std::ffi::OsStr,
) -> Result<FileObservation, OpError> {
    // O_PATH cannot block on a FIFO and observes a link itself without following it.
    let file: File = fs::openat(
        parent,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| io_error(e.into(), Phase::Capture))?
    .into();
    let mut observation = observation(&file)?;
    if file
        .metadata()
        .map_err(|e| io_error(e, Phase::Capture))?
        .file_type()
        .is_symlink()
    {
        observation.kind = EntryKind::Symlink;
    }
    Ok(observation)
}
pub(super) fn facts(file: &File) -> Result<DirectoryFacts, OpError> {
    let fs = fs::fstatfs(file).map_err(|e| io_error(e.into(), Phase::Capture))?;
    // Enable only filesystem families exercised by our native capability suite.
    // Unknown, network, and untested disk filesystems remain read-only to this seam.
    let verified = matches!(fs.f_type as u64, 0x794c_7630 | 0x0102_1994); // overlayfs, tmpfs
    let readonly = fs.f_flags & 1 != 0;
    Ok(DirectoryFacts {
        case_sensitivity: if verified {
            CaseSensitivity::Sensitive
        } else {
            CaseSensitivity::Unknown
        },
        normalization: if verified {
            NameNormalization::Preserved
        } else {
            NameNormalization::Unknown
        },
        max_component_units: u64::try_from(fs.f_namelen).ok(),
        // Calls below are descriptor-relative: there is no artificial PATH_MAX.
        max_path_units: None,
        name_policy: if verified {
            vidlib_inventory::NamePolicy::Posix
        } else {
            vidlib_inventory::NamePolicy::Unknown
        },
        forbidden_characters: vec!["\0".into(), "/".into()],
        trailing_dots_spaces_allowed: verified.then_some(true),
        capabilities: FilesystemCapabilities {
            read_only: Some(readonly),
            physical_identity: Support::Supported,
            hard_links: Support::Unknown,
            atomic_rename: if verified && !readonly {
                Support::Supported
            } else {
                Support::Unknown
            },
            durable_flush: Support::Unknown,
            local_filesystem: if fs.f_type as u64 == 0x0102_1994 {
                Some(true)
            } else {
                None
            },
        },
        ..Default::default()
    })
}
pub(super) fn mutation_supported(parent: &File, phase: Phase) -> Result<(), OpError> {
    let facts = facts(parent)?;
    if facts.capabilities.read_only == Some(true) {
        return Err(OpError::new(
            ErrorCode::PermissionDenied,
            phase,
            "filesystem is read-only",
        ));
    }
    if facts.capabilities.atomic_rename != Support::Supported {
        return Err(OpError::unsupported(phase));
    }
    Ok(())
}
pub(super) fn rename(
    source_parent: &File,
    source_name: &std::ffi::OsStr,
    _source: &File,
    target_parent: &File,
    target_name: &std::ffi::OsStr,
) -> Result<(), OpError> {
    fs::renameat_with(
        source_parent,
        source_name,
        target_parent,
        target_name,
        RenameFlags::NOREPLACE,
    )
    .map_err(|e| {
        if matches!(
            e,
            rustix::io::Errno::NOSYS | rustix::io::Errno::INVAL | rustix::io::Errno::OPNOTSUPP
        ) {
            OpError::unsupported(Phase::Rename)
        } else {
            io_error(e.into(), Phase::Rename)
        }
    })
}
pub(super) fn create(
    parent: &File,
    name: &std::ffi::OsStr,
    directory: bool,
) -> Result<File, OpError> {
    if directory {
        fs::mkdirat(parent, name, Mode::from_raw_mode(0o700))
            .map_err(|e| io_error(e.into(), Phase::Create))?;
        open_child(parent, name, true).map_err(|_| {
            OpError::new(
                ErrorCode::RecoveryRequired,
                Phase::Create,
                "directory created but ownership could not be captured",
            )
        })
    } else {
        fs::openat(
            parent,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map(File::from)
        .map_err(|e| io_error(e.into(), Phase::Create))
    }
}
pub(super) fn removal_link_count(file: &File) -> Result<u64, OpError> {
    file.metadata()
        .map(|metadata| metadata.nlink())
        .map_err(|error| io_error(error, Phase::Remove))
}

pub(super) fn remove(
    parent: &File,
    name: &std::ffi::OsStr,
    _file: &File,
    directory: bool,
) -> Result<(), OpError> {
    #[cfg(test)]
    boundary_fixture::run(boundary_fixture::Boundary::RemoveBeforeEffect);
    fs::unlinkat(
        parent,
        name,
        if directory {
            AtFlags::REMOVEDIR
        } else {
            AtFlags::empty()
        },
    )
    .map_err(|e| io_error(e.into(), Phase::Remove))?;
    #[cfg(test)]
    boundary_fixture::run(boundary_fixture::Boundary::RemoveAfterEffect);
    Ok(())
}
pub(super) fn metadata(file: &File, update: &MetadataUpdate) -> Result<(), OpError> {
    #[cfg(test)]
    boundary_fixture::run(boundary_fixture::Boundary::MetadataBeforeEffect);
    if update.created_ns.is_some() || update.windows_attributes.is_some() {
        return Err(OpError::unsupported(Phase::Metadata));
    }
    let time = |value: Option<i128>| -> Result<rustix::fs::Timespec, OpError> {
        match value {
            None => Ok(rustix::fs::Timespec {
                tv_sec: 0,
                tv_nsec: fs::UTIME_OMIT,
            }),
            Some(ns) => Ok(rustix::fs::Timespec {
                tv_sec: ns.div_euclid(1_000_000_000).try_into().map_err(|_| {
                    OpError::new(
                        ErrorCode::InvalidPath,
                        Phase::Metadata,
                        "timestamp outside native range",
                    )
                })?,
                tv_nsec: ns.rem_euclid(1_000_000_000) as _,
            }),
        }
    };
    let times = fs::Timestamps {
        last_access: time(update.accessed_ns)?,
        last_modification: time(update.modified_ns)?,
    };
    if update.accessed_ns.is_some() || update.modified_ns.is_some() {
        fs::futimens(file, &times).map_err(|e| io_error(e.into(), Phase::Metadata))?;
    }
    if let Some(readonly) = update.readonly {
        let mut permissions = file
            .metadata()
            .map_err(|e| io_error(e, Phase::Metadata))?
            .permissions();
        let mode = permissions.mode();
        permissions.set_mode(if readonly {
            mode & !0o222
        } else {
            mode | 0o200
        });
        file.set_permissions(permissions).map_err(|e| {
            OpError::new(
                ErrorCode::RecoveryRequired,
                Phase::Metadata,
                format!("metadata may be partially applied: {e}"),
            )
        })?;
    }
    #[cfg(test)]
    boundary_fixture::run(boundary_fixture::Boundary::MetadataAfterEffect);
    Ok(())
}
pub(super) fn flush(file: &File) -> Durability {
    match file.sync_all() {
        Ok(()) => Durability::Synced,
        Err(e) if matches!(e.raw_os_error(), Some(22 | 95)) => Durability::Unsupported,
        Err(e) => Durability::Failed(e.to_string()),
    }
}

pub(super) fn open_mutable_child(
    parent: &File,
    name: &std::ffi::OsStr,
    _phase: Phase,
) -> Result<File, OpError> {
    open_child(parent, name, false)
}

pub(super) fn open_writer(parent: &File, name: &std::ffi::OsStr) -> Result<File, OpError> {
    fs::openat(
        parent,
        name,
        OFlags::RDWR | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|e| io_error(e.into(), Phase::Write))
}
