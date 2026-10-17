//! Read-only, native-path observations shared by Rename and Blueprint.
//!
//! Native paths stay in Rust. Display strings are presentation only; identity is
//! per observed path, including distinct hard-link aliases. Coverage describes a
//! time-bounded observation, never authority for a future mutation.
pub mod control;
pub mod exact;
pub mod ids;
pub mod model;
pub mod path;
pub use path as path_codec;
mod platform;
pub mod scan;

pub use control::{JobControl, Progress, ProgressPhase};
pub use model::*;
pub use path::{decode_path, encode_path, NativePathRecord, PathCodecError, PathPlatform};
pub use scan::{
    scan, scan_with_fs, DirectoryListing, FileObservation, InventoryDirectory, InventoryError,
    InventoryFs, OsInventoryFs,
};

pub mod content;
pub use content::{with_verified_content, InventoryFile};
