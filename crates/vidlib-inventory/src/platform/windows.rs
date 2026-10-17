use crate::{EntryKind, FileObservation, InventoryDirectory, PhysicalId};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, Metadata, OpenOptions},
    io,
    mem::{offset_of, size_of},
    os::windows::{
        ffi::OsStrExt,
        ffi::OsStringExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
};
use windows_sys::Wdk::{
    Foundation::OBJECT_ATTRIBUTES,
    Storage::FileSystem::{
        NtCreateFile, FILE_OPEN, FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT,
        FILE_SYNCHRONOUS_IO_NONALERT,
    },
};
use windows_sys::Win32::{
    Foundation::{RtlNtStatusToDosError, ERROR_NO_MORE_FILES, UNICODE_STRING},
    Storage::FileSystem::{
        FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
    },
    System::IO::IO_STATUS_BLOCK,
};

fn open_path(path: &Path, access: u32) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(access)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}
pub(crate) fn physical_id(path: &Path, _metadata: &Metadata) -> Option<PhysicalId> {
    identity(&open_path(path, 0).ok()?).ok()
}
fn identity(file: &File) -> io::Result<PhysicalId> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // The initialized output structure and read-only handle remain live through the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok(PhysicalId {
        platform: crate::PathPlatform::WindowsUtf16Le,
        volume_identity: info.dwVolumeSerialNumber.to_be_bytes().to_vec(),
        file_identity: index.to_be_bytes().to_vec(),
    })
}
pub(crate) fn attributes(metadata: &Metadata) -> Option<u32> {
    use std::os::windows::fs::MetadataExt;
    Some(metadata.file_attributes())
}
fn observation(file: &File) -> io::Result<FileObservation> {
    let metadata = file.metadata()?;
    let kind = crate::scan::kind(&metadata);
    let mut stamp = super::stamp(&metadata, identity(file).ok());
    if kind != EntryKind::File {
        stamp.size = None;
    }
    Ok(FileObservation { kind, stamp })
}
pub(crate) fn open_directory(path: &Path) -> io::Result<Box<dyn InventoryDirectory>> {
    directory(open_path(path, FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)?)
}
fn directory(file: File) -> io::Result<Box<dyn InventoryDirectory>> {
    if observation(&file)?.kind != EntryKind::Directory {
        return Err(io::Error::other(
            "directory listing requires a non-reparse directory",
        ));
    }
    Ok(Box::new(WindowsDirectory { file }))
}
struct WindowsDirectory {
    file: File,
}
impl InventoryDirectory for WindowsDirectory {
    fn observation(&self) -> io::Result<FileObservation> {
        observation(&self.file)
    }
    fn read_directory(&self) -> io::Result<crate::DirectoryListing> {
        read_directory(&self.file)
    }
    fn observe_child(&self, name: &OsStr) -> io::Result<FileObservation> {
        observation(&open_relative(&self.file, name, FILE_READ_ATTRIBUTES)?)
    }
    fn open_child_directory(&self, name: &OsStr) -> io::Result<Box<dyn InventoryDirectory + '_>> {
        directory(open_relative(
            &self.file,
            name,
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES,
        )?)
    }
}
fn open_relative(parent: &File, name: &OsStr, access: u32) -> io::Result<File> {
    crate::scan::validate_child_name(name)?;
    let mut units: Vec<_> = name.encode_wide().collect();
    let length = u16::try_from(units.len() * 2)
        .map_err(|_| io::Error::other("native child name too long"))?;
    let native_name = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: units.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &native_name,
        ..OBJECT_ATTRIBUTES::default()
    };
    let mut status_block = IO_STATUS_BLOCK::default();
    let mut handle = std::ptr::null_mut();
    // RootDirectory anchors this single exact child name to the still-live parent
    // handle. OPEN_REPARSE_POINT inspects the child itself; no absolute lookup occurs.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut status_block,
            std::ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            FILE_OPEN_REPARSE_POINT | FILE_OPEN_FOR_BACKUP_INTENT | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    // A successful synchronous NtCreateFile transfers one owned handle to File.
    Ok(unsafe { File::from_raw_handle(handle) })
}
fn read_directory(file: &File) -> io::Result<crate::DirectoryListing> {
    // u64 storage provides the alignment required by FILE_ID_BOTH_DIR_INFO.
    let mut buffer = vec![0u64; 8192];
    let buffer_bytes = buffer.len() * size_of::<u64>();
    let name_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let mut listing = crate::DirectoryListing::default();
    let mut class = FileIdBothDirectoryRestartInfo;
    loop {
        buffer.fill(0);
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                class,
                buffer.as_mut_ptr().cast(),
                buffer_bytes as u32,
            )
        };
        class = FileIdBothDirectoryInfo;
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
                listing.errors.push(error.to_string());
            }
            break;
        }
        let bytes = buffer.as_ptr().cast::<u8>();
        let mut offset = 0usize;
        loop {
            if offset + size_of::<FILE_ID_BOTH_DIR_INFO>() > buffer_bytes {
                listing.errors.push("invalid directory entry bounds".into());
                return Ok(listing);
            }
            // Kernel offsets/lengths are checked before reading variable name data.
            // read_unaligned also handles filesystems with less strict record alignment.
            let entry = unsafe {
                bytes
                    .add(offset)
                    .cast::<FILE_ID_BOTH_DIR_INFO>()
                    .read_unaligned()
            };
            let name_len = entry.FileNameLength as usize;
            if !name_len.is_multiple_of(2) || name_len > buffer_bytes - offset - name_offset {
                listing
                    .errors
                    .push("invalid native directory name length".into());
                return Ok(listing);
            }
            let mut units = Vec::with_capacity(name_len / 2);
            for index in 0..name_len / 2 {
                units.push(unsafe {
                    bytes
                        .add(offset + name_offset + index * 2)
                        .cast::<u16>()
                        .read_unaligned()
                });
            }
            if units != [46] && units != [46, 46] {
                listing.names.push(OsString::from_wide(&units));
            }
            if entry.NextEntryOffset == 0 {
                break;
            }
            let next = entry.NextEntryOffset as usize;
            if next < name_offset + name_len || next > buffer_bytes - offset {
                listing
                    .errors
                    .push("invalid native directory entry offset".into());
                return Ok(listing);
            }
            offset += next;
        }
    }
    Ok(listing)
}
