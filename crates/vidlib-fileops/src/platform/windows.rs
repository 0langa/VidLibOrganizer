//! Windows bindings are compile-checked only. No volume is enabled for mutation
//! until the native qualification matrix in docs/verification is completed.
use super::*;
use ::windows::{
    core::PWSTR,
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            NtCreateFile, FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
            FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
        },
    },
    Win32::{
        Foundation::{RtlNtStatusToDosError, HANDLE, UNICODE_STRING},
        Storage::FileSystem::*,
        System::IO::IO_STATUS_BLOCK,
    },
};
use std::{
    ffi::{OsStr, OsString},
    fs::OpenOptions,
    mem::{offset_of, size_of},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
};
use vidlib_inventory::{
    CaseSensitivity, DirectoryListing, NameNormalization, PathPlatform, PhysicalId,
};

fn handle(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}
fn win_error(error: ::windows::core::Error, phase: Phase) -> OpError {
    let raw = (error.code().0 as u32 & 0xffff) as i32;
    match raw {
        32 | 33 => OpError::new(
            ErrorCode::Busy,
            phase,
            "file is locked or sharing access is denied",
        ),
        80 | 183 => OpError::new(
            ErrorCode::DestinationExists,
            phase,
            "destination already exists",
        ),
        1 | 50 | 87 | 120 => OpError::unsupported(phase),
        _ => io_error(std::io::Error::from_raw_os_error(raw), phase),
    }
}
fn relative(
    parent: &File,
    name: &OsStr,
    access: FILE_ACCESS_RIGHTS,
    create: bool,
    directory: bool,
    exclusive_mutation: bool,
) -> Result<File, OpError> {
    let mut units: Vec<_> = name.encode_wide().collect();
    let length = u16::try_from(units.len() * 2).map_err(|_| {
        OpError::new(
            ErrorCode::InvalidPath,
            Phase::Inspect,
            "native name too long",
        )
    })?;
    let native_name = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: PWSTR(units.as_mut_ptr()),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: handle(parent),
        ObjectName: &native_name,
        ..Default::default()
    };
    let mut status = IO_STATUS_BLOCK::default();
    let mut output = HANDLE::default();
    let options = FILE_OPEN_REPARSE_POINT
        | FILE_OPEN_FOR_BACKUP_INTENT
        | FILE_SYNCHRONOUS_IO_NONALERT
        | if directory {
            FILE_DIRECTORY_FILE
        } else if create {
            FILE_NON_DIRECTORY_FILE
        } else {
            Default::default()
        };
    // A single validated component is resolved under a live parent. Reparse
    // points are opened themselves and checked before use, never traversed.
    let result = unsafe {
        NtCreateFile(
            &mut output,
            access | SYNCHRONIZE,
            &attributes,
            &mut status,
            None,
            FILE_ATTRIBUTE_NORMAL,
            if exclusive_mutation {
                FILE_SHARE_READ
            } else {
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
            },
            if create { FILE_CREATE } else { FILE_OPEN },
            options,
            None,
            0,
        )
    };
    if result.0 < 0 {
        let raw = unsafe { RtlNtStatusToDosError(result) };
        return Err(win_error(
            ::windows::core::Error::from_hresult(::windows::core::HRESULT::from_win32(raw)),
            Phase::Inspect,
        ));
    }
    // Successful NtCreateFile returns exactly one owned handle.
    Ok(unsafe { File::from_raw_handle(output.0) })
}
pub(super) fn directory(path: &Path) -> Result<File, OpError> {
    let mut components = path.components();
    let prefix = components.next().ok_or_else(|| {
        OpError::new(
            ErrorCode::InvalidPath,
            Phase::Inspect,
            "volume prefix required",
        )
    })?;
    let mut root = PathBuf::from(prefix.as_os_str());
    if !matches!(components.next(), Some(std::path::Component::RootDir)) {
        return Err(OpError::new(
            ErrorCode::InvalidPath,
            Phase::Inspect,
            "absolute Windows volume path required",
        ));
    }
    root.push(r"\");
    let mut file = OpenOptions::new()
        .access_mode((FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES).0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
        .open(root)
        .map_err(|e| io_error(e, Phase::Inspect))?;
    reject_reparse(&file)?;
    for component in components {
        if let std::path::Component::Normal(name) = component {
            file = open_child(&file, name, true)?;
        }
    }
    Ok(file)
}
fn reject_reparse(file: &File) -> Result<(), OpError> {
    if observation(file)?.kind == EntryKind::ReparsePoint {
        Err(OpError::new(
            ErrorCode::ReparsePoint,
            Phase::Inspect,
            "reparse path component rejected",
        ))
    } else {
        Ok(())
    }
}
pub(super) fn open_child(parent: &File, name: &OsStr, directory: bool) -> Result<File, OpError> {
    let file = relative(
        parent,
        name,
        FILE_READ_DATA | FILE_READ_ATTRIBUTES,
        false,
        directory,
        false,
    )?;
    reject_reparse(&file)?;
    Ok(file)
}
pub(super) fn open_mutable_child(
    parent: &File,
    name: &OsStr,
    phase: Phase,
) -> Result<File, OpError> {
    let access = if phase == Phase::Metadata {
        FILE_WRITE_ATTRIBUTES
    } else {
        DELETE
    };
    let file = relative(
        parent,
        name,
        FILE_READ_ATTRIBUTES | access,
        false,
        false,
        true,
    )?;
    reject_reparse(&file)?;
    Ok(file)
}
pub(super) fn observation(file: &File) -> Result<FileObservation, OpError> {
    let metadata = file.metadata().map_err(|e| io_error(e, Phase::Inspect))?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle(file), &mut info) }
        .map_err(|e| win_error(e, Phase::Inspect))?;
    let kind = if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        EntryKind::ReparsePoint
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let identity = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok(FileObservation {
        kind,
        stamp: FileStamp {
            physical_id: (identity != 0).then(|| PhysicalId {
                platform: PathPlatform::WindowsUtf16Le,
                volume_identity: info.dwVolumeSerialNumber.to_be_bytes().to_vec(),
                file_identity: identity.to_be_bytes().to_vec(),
            }),
            size: (kind == EntryKind::File).then_some(metadata.len()),
            created_ns: metadata.created().ok().map(epoch_ns),
            modified_ns: metadata.modified().ok().map(epoch_ns),
            accessed_ns: metadata.accessed().ok().map(epoch_ns),
            windows_attributes: Some(metadata.file_attributes()),
        },
    })
}
pub(super) fn child_observation(parent: &File, name: &OsStr) -> Result<FileObservation, OpError> {
    observation(&relative(
        parent,
        name,
        FILE_READ_ATTRIBUTES,
        false,
        false,
        false,
    )?)
}
pub(super) fn listing(file: &File) -> Result<DirectoryListing, OpError> {
    let mut buffer = vec![0u64; 8192];
    let bytes = buffer.len() * size_of::<u64>();
    let name_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let mut listing = DirectoryListing::default();
    let mut class = FileIdBothDirectoryRestartInfo;
    loop {
        buffer.fill(0);
        let result = unsafe {
            GetFileInformationByHandleEx(
                handle(file),
                class,
                buffer.as_mut_ptr().cast(),
                bytes as u32,
            )
        };
        class = FileIdBothDirectoryInfo;
        if let Err(error) = result {
            if error.code().0 as u32 & 0xffff != 18 {
                listing.errors.push(error.to_string());
            }
            break;
        }
        let pointer = buffer.as_ptr().cast::<u8>();
        let mut offset = 0;
        loop {
            if offset + size_of::<FILE_ID_BOTH_DIR_INFO>() > bytes {
                return Err(OpError::new(
                    ErrorCode::Io,
                    Phase::Capture,
                    "invalid directory record bounds",
                ));
            }
            let entry = unsafe {
                pointer
                    .add(offset)
                    .cast::<FILE_ID_BOTH_DIR_INFO>()
                    .read_unaligned()
            };
            let length = entry.FileNameLength as usize;
            if !length.is_multiple_of(2) || length > bytes - offset - name_offset {
                return Err(OpError::new(
                    ErrorCode::Io,
                    Phase::Capture,
                    "invalid directory name length",
                ));
            }
            let units: Vec<_> = (0..length / 2)
                .map(|index| unsafe {
                    pointer
                        .add(offset + name_offset + index * 2)
                        .cast::<u16>()
                        .read_unaligned()
                })
                .collect();
            if units != [46] && units != [46, 46] {
                listing.names.push(OsString::from_wide(&units));
            }
            if entry.NextEntryOffset == 0 {
                break;
            }
            let next = entry.NextEntryOffset as usize;
            if next < name_offset + length || next > bytes - offset {
                return Err(OpError::new(
                    ErrorCode::Io,
                    Phase::Capture,
                    "invalid directory record offset",
                ));
            }
            offset += next;
        }
    }
    Ok(listing)
}
pub(super) fn facts(file: &File) -> Result<DirectoryFacts, OpError> {
    let mut case = FILE_CASE_SENSITIVE_INFO::default();
    let case_sensitivity = match unsafe {
        GetFileInformationByHandleEx(
            handle(file),
            FileCaseSensitiveInfo,
            (&mut case as *mut FILE_CASE_SENSITIVE_INFO).cast(),
            size_of::<FILE_CASE_SENSITIVE_INFO>() as u32,
        )
    } {
        Ok(()) => {
            if case.Flags & 1 != 0 {
                CaseSensitivity::Sensitive
            } else {
                CaseSensitivity::Insensitive
            }
        }
        Err(_) => CaseSensitivity::Unknown,
    };
    let mut maximum = 0;
    let mut flags = 0;
    let volume = unsafe {
        GetVolumeInformationByHandleW(
            handle(file),
            None,
            None,
            Some(&mut maximum),
            Some(&mut flags),
            None,
        )
    };
    let mut facts = DirectoryFacts {
        name_policy: vidlib_inventory::NamePolicy::Windows,
        reserved_names: windows_reserved_names(),
        case_sensitivity,
        normalization: NameNormalization::Unknown,
        max_component_units: volume.is_ok().then_some(u64::from(maximum)),
        forbidden_characters: ["\0", "<", ">", ":", "\"", "/", "\\", "|", "?", "*"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        trailing_dots_spaces_allowed: Some(false),
        ..Default::default()
    };
    if volume.is_ok() {
        facts.capabilities.read_only = Some(flags & 0x0008_0000 != 0);
    }
    // Query success is observation, never native qualification evidence.
    Ok(facts)
}
pub(super) fn mutation_supported(_parent: &File, phase: Phase) -> Result<(), OpError> {
    Err(OpError::unsupported(phase))
}
pub(super) fn rename(
    _source_parent: &File,
    _source_name: &OsStr,
    source: &File,
    target_parent: &File,
    target_name: &OsStr,
) -> Result<(), OpError> {
    let units: Vec<_> = target_name.encode_wide().collect();
    let length = units
        .len()
        .checked_mul(2)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| {
            OpError::new(
                ErrorCode::InvalidPath,
                Phase::Rename,
                "target name too long",
            )
        })?;
    let byte_count = (offset_of!(FILE_RENAME_INFO, FileName) + length as usize)
        .max(size_of::<FILE_RENAME_INFO>());
    // u64 backing aligns FILE_RENAME_INFO; initialized trailing bytes hold exactly
    // FileNameLength bytes. ReplaceIfExists remains false, never overwrite flags.
    let mut buffer = vec![0u64; byte_count.div_ceil(8)];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = handle(target_parent);
        (*info).FileNameLength = length;
        std::ptr::copy_nonoverlapping(
            units.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            units.len(),
        );
        SetFileInformationByHandle(
            handle(source),
            FileRenameInfo,
            info.cast(),
            byte_count as u32,
        )
    }
    .map_err(|e| win_error(e, Phase::Rename))
}
pub(super) fn create(parent: &File, name: &OsStr, directory: bool) -> Result<File, OpError> {
    let file = relative(
        parent,
        name,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE,
        true,
        directory,
        false,
    )?;
    reject_reparse(&file)?;
    Ok(file)
}
// Delete-pending and close-time link removal have not been qualified on native
// Windows. Refuse owned cleanup before the effect until its evidence contract is
// implemented and tested; Linux link-count semantics must not be assumed here.
pub(super) fn removal_link_count(_file: &File) -> Result<u64, OpError> {
    Err(OpError::unsupported(Phase::Remove))
}

pub(super) fn remove(
    _parent: &File,
    _name: &OsStr,
    file: &File,
    _directory: bool,
) -> Result<(), OpError> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            handle(file),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .map_err(|e| win_error(e, Phase::Remove))
}
pub(super) fn metadata(file: &File, update: &MetadataUpdate) -> Result<(), OpError> {
    let time = |value: Option<i128>| -> Result<i64, OpError> {
        match value {
            None => Ok(0),
            Some(ns) if ns % 100 == 0 => (ns / 100 + 116_444_736_000_000_000i128)
                .try_into()
                .ok()
                .filter(|ticks| *ticks > 0)
                .ok_or_else(|| {
                    OpError::new(
                        ErrorCode::Unsupported,
                        Phase::Metadata,
                        "timestamp outside Windows settable range",
                    )
                }),
            _ => Err(OpError::new(
                ErrorCode::Unsupported,
                Phase::Metadata,
                "Windows timestamp requires exact 100-nanosecond precision",
            )),
        }
    };
    let mut info = FILE_BASIC_INFO {
        CreationTime: time(update.created_ns)?,
        LastAccessTime: time(update.accessed_ns)?,
        LastWriteTime: time(update.modified_ns)?,
        ..Default::default()
    };
    if let Some(attributes) = update.windows_attributes {
        let supported = (FILE_ATTRIBUTE_READONLY
            | FILE_ATTRIBUTE_HIDDEN
            | FILE_ATTRIBUTE_SYSTEM
            | FILE_ATTRIBUTE_ARCHIVE
            | FILE_ATTRIBUTE_NORMAL)
            .0;
        let current = file
            .metadata()
            .map_err(|e| io_error(e, Phase::Metadata))?
            .file_attributes();
        if (attributes ^ current) & !supported != 0 {
            return Err(OpError::unsupported(Phase::Metadata));
        }
        info.FileAttributes = if attributes & !FILE_ATTRIBUTE_NORMAL.0 == 0 {
            FILE_ATTRIBUTE_NORMAL.0
        } else {
            attributes & !FILE_ATTRIBUTE_NORMAL.0
        };
    }
    if let Some(readonly) = update.readonly {
        if info.FileAttributes == 0 {
            info.FileAttributes = file
                .metadata()
                .map_err(|e| io_error(e, Phase::Metadata))?
                .file_attributes();
        }
        if readonly {
            info.FileAttributes |= FILE_ATTRIBUTE_READONLY.0;
        } else {
            info.FileAttributes &= !FILE_ATTRIBUTE_READONLY.0;
        }
        if info.FileAttributes == 0 {
            info.FileAttributes = FILE_ATTRIBUTE_NORMAL.0;
        }
    }
    unsafe {
        SetFileInformationByHandle(
            handle(file),
            FileBasicInfo,
            (&info as *const FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    }
    .map_err(|e| win_error(e, Phase::Metadata))
}
pub(super) fn flush(file: &File) -> Durability {
    match unsafe { FlushFileBuffers(handle(file)) } {
        Ok(()) => Durability::Synced,
        Err(error) if matches!(error.code().0 as u32 & 0xffff, 1 | 5 | 6 | 50 | 87) => {
            Durability::Unsupported
        }
        Err(error) => Durability::Failed(error.to_string()),
    }
}

fn windows_reserved_names() -> Vec<String> {
    let mut names: Vec<_> = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    for prefix in ["COM", "LPT"] {
        for suffix in ["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"] {
            names.push(format!("{prefix}{suffix}"));
        }
    }
    names
}

#[cfg(test)]
mod qualification {
    use super::*;
    /// These tests deliberately call private native primitives. They qualify
    /// APIs without enabling any production volume gate. Run on native Windows;
    /// successful compilation on another OS is not test evidence.
    fn move_native(source: &Path, target: &Path) -> Result<(), OpError> {
        let (source_parent, source_name) = super::super::parent(source)?;
        let (target_parent, target_name) = super::super::parent(target)?;
        let file = open_mutable_child(&source_parent, source_name, Phase::Rename)?;
        rename(
            &source_parent,
            source_name,
            &file,
            &target_parent,
            target_name,
        )
    }
    #[test]
    fn native_existing_target_never_replaced_for_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        for is_directory in [false, true] {
            let source = dir
                .path()
                .join(if is_directory { "source-dir" } else { "source" });
            let target = dir
                .path()
                .join(if is_directory { "target-dir" } else { "target" });
            if is_directory {
                std::fs::create_dir(&source).unwrap();
                std::fs::create_dir(&target).unwrap();
                std::fs::write(source.join("child"), b"source").unwrap();
                std::fs::write(target.join("child"), b"existing").unwrap();
            } else {
                std::fs::write(&source, b"source").unwrap();
                std::fs::write(&target, b"existing").unwrap();
            }
            assert_eq!(
                move_native(&source, &target).unwrap_err().code,
                ErrorCode::DestinationExists
            );
            assert_eq!(
                std::fs::read(if is_directory {
                    source.join("child")
                } else {
                    source
                })
                .unwrap(),
                b"source"
            );
            assert_eq!(
                std::fs::read(if is_directory {
                    target.join("child")
                } else {
                    target
                })
                .unwrap(),
                b"existing"
            );
        }
    }
    #[test]
    fn native_staging_writer_must_close_before_publication_handle_opens() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("staging");
        let target = dir.path().join("published");
        let parent = directory(dir.path()).unwrap();
        let mut writer = create(&parent, source.file_name().unwrap(), false).unwrap();
        std::io::Write::write_all(&mut writer, b"verified bytes").unwrap();
        writer.sync_all().unwrap();
        assert_eq!(
            move_native(&source, &target).unwrap_err().code,
            ErrorCode::Busy
        );
        drop(writer);
        move_native(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"verified bytes");
        assert!(!source.exists());
    }
    #[test]
    fn native_locked_file_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::write(&source, b"safe").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(&source)
            .unwrap();
        assert_eq!(
            move_native(&source, &dir.path().join("target"))
                .unwrap_err()
                .code,
            ErrorCode::Busy
        );
        drop(held);
        assert_eq!(std::fs::read(source).unwrap(), b"safe");
    }
    #[test]
    fn native_metadata_explicit_flags_times_and_precision() {
        let dir = tempfile::tempdir().unwrap();
        for is_directory in [false, true] {
            let path = dir
                .path()
                .join(if is_directory { "folder" } else { "file" });
            if is_directory {
                std::fs::create_dir(&path).unwrap();
            } else {
                std::fs::write(&path, b"metadata").unwrap();
            }
            let (parent, name) = super::super::parent(&path).unwrap();
            let file = open_mutable_child(&parent, name, Phase::Metadata).unwrap();
            let original = observation(&file).unwrap().stamp;
            for bit in [1, 2, 4, 32] {
                let before = observation(&file)
                    .unwrap()
                    .stamp
                    .windows_attributes
                    .unwrap();
                let desired = (before | bit) & !FILE_ATTRIBUTE_NORMAL.0;
                metadata(
                    &file,
                    &MetadataUpdate {
                        windows_attributes: Some(desired),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(
                    observation(&file).unwrap().stamp.windows_attributes,
                    Some(desired)
                );
                let cleared = desired & !bit;
                metadata(
                    &file,
                    &MetadataUpdate {
                        windows_attributes: Some(cleared),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(
                    observation(&file)
                        .unwrap()
                        .stamp
                        .windows_attributes
                        .unwrap()
                        & bit,
                    0
                );
                assert_eq!(
                    observation(&file)
                        .unwrap()
                        .stamp
                        .windows_attributes
                        .unwrap()
                        & !0xa7,
                    before & !0xa7
                );
            }
            let value = 1_600_000_000_123_456_700;
            metadata(
                &file,
                &MetadataUpdate {
                    created_ns: Some(value),
                    modified_ns: Some(value + 100),
                    accessed_ns: Some(value + 200),
                    ..Default::default()
                },
            )
            .unwrap();
            let after = observation(&file).unwrap().stamp;
            assert_eq!(
                (after.created_ns, after.modified_ns, after.accessed_ns),
                (Some(value), Some(value + 100), Some(value + 200))
            );
            assert_eq!(
                metadata(
                    &file,
                    &MetadataUpdate {
                        modified_ns: Some(value + 1),
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .code,
                ErrorCode::Unsupported
            );
            assert_eq!(
                observation(&file).unwrap().stamp.modified_ns,
                Some(value + 100)
            );
            metadata(
                &file,
                &MetadataUpdate {
                    created_ns: original.created_ns,
                    modified_ns: original.modified_ns,
                    accessed_ns: original.accessed_ns,
                    windows_attributes: original.windows_attributes,
                    ..Default::default()
                },
            )
            .unwrap();
        }
    }
    #[test]
    fn native_case_only_and_directory_moves_preserve_identity() {
        let dir = tempfile::tempdir().unwrap();
        for is_directory in [false, true] {
            let source = dir
                .path()
                .join(if is_directory { "Folder" } else { "File" });
            let target = dir
                .path()
                .join(if is_directory { "folder" } else { "file" });
            if is_directory {
                std::fs::create_dir(&source).unwrap();
            } else {
                std::fs::write(&source, b"safe").unwrap();
            }
            let before = PlatformFs::default().inspect(&source).unwrap();
            move_native(&source, &target).unwrap();
            let after = PlatformFs::default().inspect(&target).unwrap();
            assert_eq!(before.stamp.physical_id, after.stamp.physical_id);
            let names = listing(&directory(dir.path()).unwrap()).unwrap().names;
            assert!(names.contains(&target.file_name().unwrap().to_os_string()));
            assert!(!names.contains(&source.file_name().unwrap().to_os_string()));
        }
    }
    #[test]
    fn native_target_created_after_inspection_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        std::fs::write(&source, b"source").unwrap();
        let (parent, name) = super::super::parent(&source).unwrap();
        let file = open_mutable_child(&parent, name, Phase::Rename).unwrap();
        std::fs::write(&target, b"late").unwrap();
        assert_eq!(
            rename(&parent, name, &file, &parent, target.file_name().unwrap())
                .unwrap_err()
                .code,
            ErrorCode::DestinationExists
        );
        assert_eq!(std::fs::read(target).unwrap(), b"late");
        assert_eq!(std::fs::read(source).unwrap(), b"source");
    }
    #[test]
    #[ignore = "requires native NTFS directory case-sensitivity support and permission; record explicit unsupported otherwise"]
    fn native_directory_case_policy_is_per_parent() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("insensitive");
        let b = dir.path().join("sensitive");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let change = OpenOptions::new()
            .access_mode((FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES).0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(&b)
            .unwrap();
        let flags = FILE_CASE_SENSITIVE_INFO { Flags: 1 };
        unsafe {
            SetFileInformationByHandle(
                handle(&change),
                FileCaseSensitiveInfo,
                (&flags as *const FILE_CASE_SENSITIVE_INFO).cast(),
                size_of::<FILE_CASE_SENSITIVE_INFO>() as u32,
            )
        }
        .unwrap();
        let insensitive = facts(&directory(&a).unwrap()).unwrap();
        let sensitive = facts(&directory(&b).unwrap()).unwrap();
        assert_eq!(insensitive.case_sensitivity, CaseSensitivity::Insensitive);
        assert_eq!(sensitive.case_sensitivity, CaseSensitivity::Sensitive);
    }
    #[test]
    #[ignore = "requires native Windows symlink permission/developer mode"]
    fn native_reparse_ancestor_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("source"), b"safe").unwrap();
        let alias = dir.path().join("alias");
        std::os::windows::fs::symlink_dir(&real, &alias).unwrap();
        assert_eq!(
            PlatformFs::default()
                .inspect(&alias.join("source"))
                .unwrap_err()
                .code,
            ErrorCode::ReparsePoint
        );
    }
}

pub(super) fn open_writer(parent: &File, name: &OsStr) -> Result<File, OpError> {
    let file = relative(
        parent,
        name,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE,
        false,
        false,
        true,
    )?;
    reject_reparse(&file)?;
    Ok(file)
}
