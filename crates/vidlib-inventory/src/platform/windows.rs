use crate::PhysicalId;
use std::{fs::Metadata, path::Path};

pub(crate) fn physical_id(path: &Path, _metadata: &Metadata) -> Option<PhysicalId> {
    use std::{
        fs::OpenOptions,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let file = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // A live read-only handle is passed; the OS writes exactly this initialized structure.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return None;
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some(PhysicalId {
        platform: crate::PathPlatform::WindowsUtf16Le,
        volume_identity: info.dwVolumeSerialNumber.to_be_bytes().to_vec(),
        file_identity: index.to_be_bytes().to_vec(),
    })
}
pub(crate) fn attributes(metadata: &Metadata) -> Option<u32> {
    use std::os::windows::fs::MetadataExt;
    Some(metadata.file_attributes())
}

/// Enumerate the opened directory itself. Path-based FindFirstFile/read_dir
/// would follow a directory reparse point swapped in after the earlier stat.
pub(crate) fn read_directory(path: &Path) -> std::io::Result<crate::DirectoryListing> {
    use std::{
        ffi::OsString,
        fs::OpenOptions,
        io,
        mem::{offset_of, size_of},
        os::windows::{ffi::OsStringExt, fs::OpenOptionsExt, io::AsRawHandle},
    };
    use windows_sys::Win32::{
        Foundation::ERROR_NO_MORE_FILES,
        Storage::FileSystem::{
            FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandle,
            GetFileInformationByHandleEx, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE,
        },
    };
    let file = OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // The handle remains live throughout both metadata validation and enumeration.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
    {
        return Err(io::Error::other(
            "directory listing requires a non-reparse directory",
        ));
    }
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
