use chrono::{DateTime, Utc};
use ignore::WalkBuilder;
use std::cell::Cell;
use std::collections::HashSet;
use std::path::Path;
use uuid::Uuid;
use vidlib_core::{
    ProgressSnapshot, ScanSummary, ScanWarningKind, ScanWarningRecord, VidLibResult,
    VideoEntry,
};
use vidlib_duplicates::{chunk_hash, exact_hash, perceptual_hash_from_image_bytes};
use vidlib_metadata::{MediaMetadata, MetadataProvider};

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "m4v", "webm", "mpg", "mpeg",
];

pub struct ScanOptions {
    pub recursive: bool,
    pub compute_exact_hash: bool,
    pub compute_chunk_hash: bool,
    pub ignore_hidden: bool,
    pub skip_extensions: Vec<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            recursive: true,
            compute_exact_hash: false,
            compute_chunk_hash: true,
            ignore_hidden: false,
            skip_extensions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub videos: Vec<VideoEntry>,
    pub summary: ScanSummary,
    pub warnings: Vec<ScanWarningRecord>,
}

#[derive(Debug, Default)]
pub struct CancellationToken {
    cancelled: Cell<bool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Cell::new(false),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.set(true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }
}

pub fn scan_path(
    root: &Path,
    options: &ScanOptions,
    metadata_provider: &dyn MetadataProvider,
    mut progress: impl FnMut(ProgressSnapshot),
    cancellation: Option<&CancellationToken>,
) -> VidLibResult<ScanResult> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(options.ignore_hidden)
        .git_ignore(false)
        .git_exclude(false)
        .parents(false);
    if !options.recursive {
        builder.max_depth(Some(1));
    }

    let mut videos = Vec::new();
    let skipped_extensions: HashSet<String> = options
        .skip_extensions
        .iter()
        .map(|ext| ext.to_ascii_lowercase())
        .collect();
    let mut discovered = 0_u64;
    let mut skipped = 0_u64;
    let mut failed = 0_u64;
    let mut warnings = Vec::new();
    for result in builder.build() {
        if cancellation.map(|token| token.is_cancelled()).unwrap_or(false) {
            warnings.push(ScanWarningRecord {
                id: Uuid::new_v4(),
                library_id: None,
                path: Some(root.to_path_buf()),
                kind: ScanWarningKind::Cancelled,
                message: "scan cancelled".into(),
                created_at: Utc::now(),
            });
            break;
        }
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                failed += 1;
                warnings.push(ScanWarningRecord {
                    id: Uuid::new_v4(),
                    library_id: None,
                    path: Some(root.to_path_buf()),
                    kind: ScanWarningKind::Walk,
                    message: format!("walk error: {err}"),
                    created_at: Utc::now(),
                });
                continue;
            }
        };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.into_path();
        if !is_video_file(&path) {
            skipped += 1;
            continue;
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| skipped_extensions.contains(&ext.to_ascii_lowercase()))
            .unwrap_or(false)
        {
            skipped += 1;
            continue;
        }

        discovered += 1;
        let file_metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) => {
                failed += 1;
                warnings.push(ScanWarningRecord {
                    id: Uuid::new_v4(),
                    library_id: None,
                    path: Some(path.clone()),
                    kind: ScanWarningKind::Metadata,
                    message: format!("metadata error for {}: {err}", path.display()),
                    created_at: Utc::now(),
                });
                continue;
            }
        };
        let media = match metadata_provider.extract(&path) {
            Ok(media) => media,
            Err(err) => {
                warnings.push(ScanWarningRecord {
                    id: Uuid::new_v4(),
                    library_id: None,
                    path: Some(path.clone()),
                    kind: ScanWarningKind::Metadata,
                    message: format!("metadata extraction error for {}: {err}", path.display()),
                    created_at: Utc::now(),
                });
                MediaMetadata::default()
            }
        };
        let exact_hash_value = if options.compute_exact_hash {
            match exact_hash(&path) {
                Ok(value) => Some(value),
                Err(err) => {
                    warnings.push(ScanWarningRecord {
                        id: Uuid::new_v4(),
                        library_id: None,
                        path: Some(path.clone()),
                        kind: ScanWarningKind::Hash,
                        message: format!("exact hash failed for {}: {err}", path.display()),
                        created_at: Utc::now(),
                    });
                    None
                }
            }
        } else {
            None
        };
        let chunk_hash_value = if options.compute_chunk_hash {
            match chunk_hash(&path) {
                Ok(value) => Some(value),
                Err(err) => {
                    warnings.push(ScanWarningRecord {
                        id: Uuid::new_v4(),
                        library_id: None,
                        path: Some(path.clone()),
                        kind: ScanWarningKind::Hash,
                        message: format!("chunk hash failed for {}: {err}", path.display()),
                        created_at: Utc::now(),
                    });
                    None
                }
            }
        } else {
            None
        };
        let perceptual_hash = match std::fs::read(&path) {
            Ok(bytes) => match perceptual_hash_from_image_bytes(&bytes) {
                Ok(hash) => Some(hash),
                Err(err) => {
                    warnings.push(ScanWarningRecord {
                        id: Uuid::new_v4(),
                        library_id: None,
                        path: Some(path.clone()),
                        kind: ScanWarningKind::Hash,
                        message: format!(
                            "perceptual hash failed for {}: {err}",
                            path.display()
                        ),
                        created_at: Utc::now(),
                    });
                    None
                }
            },
            Err(err) => {
                warnings.push(ScanWarningRecord {
                    id: Uuid::new_v4(),
                    library_id: None,
                    path: Some(path.clone()),
                    kind: ScanWarningKind::Hash,
                    message: format!("file read failed for {}: {err}", path.display()),
                    created_at: Utc::now(),
                });
                None
            }
        };
        let video_entry = VideoEntry {
            id: Uuid::new_v4(),
            file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
            extension: path.extension().map(|e| e.to_string_lossy().to_lowercase()),
            path: path.clone(),
            size_bytes: file_metadata.len(),
            modified_at: modified_at(file_metadata.modified().ok()),
            duration_seconds: media.duration_seconds,
            width: media.width,
            height: media.height,
            codec: media.codec,
            exact_hash: exact_hash_value,
            chunk_hash: chunk_hash_value,
            perceptual_hash,
            tags: Vec::new(),
        };
        progress(ProgressSnapshot {
            current_path: Some(video_entry.path.to_string_lossy().into_owned()),
            processed_files: discovered,
            discovered_files: discovered,
            bytes_processed: video_entry.size_bytes,
            percent: 0.0,
            message: format!("Indexed {}", video_entry.file_name),
        });
        videos.push(video_entry);
    }

    Ok(ScanResult {
        summary: ScanSummary {
            root: root.to_path_buf(),
            discovered_files: discovered,
            indexed_files: videos.len() as u64,
            skipped_files: skipped,
            failed_files: failed,
            warnings: warnings.iter().map(|warning| warning.message.clone()).collect(),
        },
        warnings,
        videos,
    })
}

pub fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            VIDEO_EXTENSIONS
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(ext))
        })
        .unwrap_or(false)
}

fn modified_at(time: Option<std::time::SystemTime>) -> Option<DateTime<Utc>> {
    time.map(DateTime::<Utc>::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    struct NoopMetadata;

    impl MetadataProvider for NoopMetadata {
        fn extract(&self, _path: &Path) -> VidLibResult<MediaMetadata> {
            Ok(MediaMetadata::default())
        }
    }

    #[test]
    fn filters_to_video_files() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.mp4"), b"a").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"b").unwrap();
        let result = scan_path(
            dir.path(),
            &ScanOptions::default(),
            &NoopMetadata,
            |_| {},
            None,
        )
        .unwrap();
        assert_eq!(result.videos.len(), 1);
        assert_eq!(result.videos[0].file_name, "a.mp4");
    }

    #[test]
    fn skips_configured_extensions() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.mp4"), b"a").unwrap();
        let result = scan_path(
            dir.path(),
            &ScanOptions {
                skip_extensions: vec!["mp4".into()],
                ..ScanOptions::default()
            },
            &NoopMetadata,
            |_| {},
            None,
        )
        .unwrap();
        assert!(result.videos.is_empty());
    }
}
