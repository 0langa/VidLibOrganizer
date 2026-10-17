use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PathPlatform {
    WindowsUtf16Le,
    UnixBytes,
}

/// Exact native units, including unpaired Windows surrogates and non-UTF8 Unix bytes.
/// This is not a display string and a foreign platform is never transcoded.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NativePathRecord {
    pub platform: PathPlatform,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathCodecError {
    UnsupportedPlatform,
    InvalidEncoding,
    InvalidRelativePath,
}
impl fmt::Display for PathCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedPlatform => "native path belongs to a different platform",
            Self::InvalidEncoding => "malformed native path encoding",
            Self::InvalidRelativePath => "path must contain only normal root-relative components",
        })
    }
}
impl Error for PathCodecError {}

#[cfg(unix)]
pub fn encode_path(path: &Path) -> NativePathRecord {
    use std::os::unix::ffi::OsStrExt;
    NativePathRecord {
        platform: PathPlatform::UnixBytes,
        bytes: path.as_os_str().as_bytes().to_vec(),
    }
}
#[cfg(windows)]
pub fn encode_path(path: &Path) -> NativePathRecord {
    use std::os::windows::ffi::OsStrExt;
    NativePathRecord {
        platform: PathPlatform::WindowsUtf16Le,
        bytes: path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect(),
    }
}

pub fn decode_path(record: &NativePathRecord) -> Result<PathBuf, PathCodecError> {
    if record.platform == PathPlatform::WindowsUtf16Le && !record.bytes.len().is_multiple_of(2) {
        return Err(PathCodecError::InvalidEncoding);
    }
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        if record.platform == PathPlatform::UnixBytes {
            return Ok(PathBuf::from(OsString::from_vec(record.bytes.clone())));
        }
    }
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        if record.platform == PathPlatform::WindowsUtf16Le {
            let units: Vec<_> = record
                .bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            return Ok(PathBuf::from(OsString::from_wide(&units)));
        }
    }
    Err(PathCodecError::UnsupportedPlatform)
}

impl NativePathRecord {
    /// Validate before persisting/importing a portable root-relative entry.
    /// Validation uses the record's own platform syntax, even on another OS.
    pub fn validate_relative(&self) -> Result<(), PathCodecError> {
        let units: Vec<u16> = match self.platform {
            PathPlatform::UnixBytes => self.bytes.iter().map(|b| u16::from(*b)).collect(),
            PathPlatform::WindowsUtf16Le => {
                if !self.bytes.len().is_multiple_of(2) {
                    return Err(PathCodecError::InvalidEncoding);
                }
                self.bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect()
            }
        };
        let windows = self.platform == PathPlatform::WindowsUtf16Le;
        if units.is_empty() || units.contains(&0) || (windows && units.contains(&58)) {
            return Err(PathCodecError::InvalidRelativePath);
        }
        for component in units.split(|unit| *unit == 47 || (windows && *unit == 92)) {
            if component.is_empty() || component == [46] || component == [46, 46] {
                return Err(PathCodecError::InvalidRelativePath);
            }
        }
        Ok(())
    }
}

pub(crate) fn conservative_path_is_within(path: &Path, root: &Path) -> bool {
    use std::path::Component;
    let mut path = path.components();
    root.components().all(|expected| {
        let Some(actual) = path.next() else {
            return false;
        };
        match (actual, expected) {
            (Component::RootDir, Component::RootDir) => true,
            _ => conservative_name_matches(actual.as_os_str(), expected.as_os_str(), false),
        }
    })
}

/// Policy comparison only: the original native name remains unchanged.
#[cfg(unix)]
pub(crate) fn conservative_name_matches(
    name: &std::ffi::OsStr,
    pattern: &std::ffi::OsStr,
    prefix: bool,
) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let name = name.as_bytes();
    let pattern = pattern.as_bytes();
    (prefix || name.len() == pattern.len())
        && name.len() >= pattern.len()
        && name[..pattern.len()].eq_ignore_ascii_case(pattern)
}
#[cfg(windows)]
pub(crate) fn conservative_name_matches(
    name: &std::ffi::OsStr,
    pattern: &std::ffi::OsStr,
    prefix: bool,
) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
    let name: Vec<_> = name.encode_wide().collect();
    let pattern: Vec<_> = pattern.encode_wide().collect();
    if (!prefix && name.len() != pattern.len()) || name.len() < pattern.len() {
        return false;
    }
    let Ok(length) = i32::try_from(pattern.len()) else {
        return true;
    };
    if length == 0 {
        return true;
    }
    // Both buffers are live and contain at least `length` native UTF16 units.
    // Comparison does not transcode unpaired surrogates or reconstruct a path.
    let comparison =
        unsafe { CompareStringOrdinal(name.as_ptr(), length, pattern.as_ptr(), length, 1) };
    comparison == CSTR_EQUAL || comparison == 0 // API uncertainty protects conservatively.
}
