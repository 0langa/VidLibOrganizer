//! Offline helper adapter. Executable paths are supplied by app resources, never PATH.
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use vidlib_inventory::JobControl;
use vidlib_rename::{
    metadata::*, FrozenFieldValue, FrozenTimezone, MediaProperty, WindowsProperty,
};

#[derive(Clone, Copy, Debug)]
pub struct ProcessLimits {
    pub timeout: Duration,
    pub stdout: usize,
    pub stderr: usize,
}
impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            stdout: 4 * 1024 * 1024,
            stderr: 64 * 1024,
        }
    }
}
fn issue(code: MetadataIssueCode, message: impl Into<String>) -> MetadataIssue {
    MetadataIssue {
        code,
        message: message.into(),
    }
}
fn drain(mut reader: impl Read, cap: usize, exceeded: Arc<AtomicBool>) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(bytes);
        }
        if count > cap.saturating_sub(bytes.len()) {
            exceeded.store(true, Ordering::Release);
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}
/// Direct spawn, concurrent bounded drains, monotonic deadline and kill + wait on
/// every exit path. Limits can be reduced by callers, never raised above policy.
pub fn run_bounded(
    command: &mut Command,
    control: &JobControl,
    limits: ProcessLimits,
) -> Result<Vec<u8>, MetadataIssue> {
    if control.is_cancelled() {
        return Err(issue(MetadataIssueCode::Cancelled, "metadata cancelled"));
    }
    if !Path::new(command.get_program()).is_absolute() {
        return Err(issue(
            MetadataIssueCode::Unavailable,
            "metadata helper requires an absolute bundled path",
        ));
    }
    let limits = ProcessLimits {
        timeout: limits.timeout.min(Duration::from_secs(10)),
        stdout: limits.stdout.min(4 * 1024 * 1024),
        stderr: limits.stderr.min(64 * 1024),
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let start = Instant::now();
    let mut child = ChildGuard {
        child: command.spawn().map_err(|e| {
            issue(
                MetadataIssueCode::Unavailable,
                format!("metadata helper could not start: {e}"),
            )
        })?,
        reaped: false,
    };
    #[cfg(windows)]
    let job = match process_job(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out_flag = overflow.clone();
    let err_flag = overflow.clone();
    let stdout = std::thread::Builder::new()
        .name("metadata-stdout".into())
        .spawn(move || drain(stdout, limits.stdout, out_flag))
        .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()))?;
    let stderr = std::thread::Builder::new()
        .name("metadata-stderr".into())
        .spawn(move || drain(stderr, limits.stderr, err_flag))
        .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()))?;
    let outcome = loop {
        if control.is_cancelled() {
            break Err(issue(MetadataIssueCode::Cancelled, "metadata cancelled"));
        }
        if overflow.load(Ordering::Acquire) {
            break Err(issue(
                MetadataIssueCode::OutputLimit,
                "metadata output exceeded its byte limit",
            ));
        }
        if start.elapsed() >= limits.timeout {
            break Err(issue(
                MetadataIssueCode::Timeout,
                "metadata probe exceeded its deadline",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(issue(
                        MetadataIssueCode::Io,
                        format!("metadata helper exited with {status}"),
                    ))
                }
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => break Err(issue(MetadataIssueCode::Io, e.to_string())),
        }
    };
    // Clean the whole owned process group/job, including inherited pipe writers.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    drop(job);
    let _ = child.kill();
    let reaped = child
        .wait()
        .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()));
    child.reaped = reaped.is_ok();
    let out = stdout
        .join()
        .map_err(|_| issue(MetadataIssueCode::Io, "stdout worker stopped"))?
        .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()));
    let err = stderr
        .join()
        .map_err(|_| issue(MetadataIssueCode::Io, "stderr worker stopped"))?
        .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()));
    reaped?;
    if overflow.load(Ordering::Acquire) {
        return Err(issue(
            MetadataIssueCode::OutputLimit,
            "metadata output exceeded its byte limit",
        ));
    }
    outcome?;
    err?;
    out
}

struct ChildGuard {
    child: std::process::Child,
    reaped: bool,
}
impl std::ops::Deref for ChildGuard {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}
impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub struct MediaInfoProvider {
    executable: PathBuf,
    version: String,
}
impl MediaInfoProvider {
    /// Backend-only injection seam; UI cannot provide an executable or source path.
    pub fn from_executable(executable: PathBuf, version: String) -> Self {
        Self {
            executable,
            version,
        }
    }
}
// Slots are shared across adapter instances/windows: native drive/share on Windows,
// physical volume on Unix. Windows scheduling performs no network/volume query.
fn volume_slot(path: &Path) -> Result<Arc<Mutex<()>>, MetadataIssue> {
    use std::sync::{OnceLock, Weak};
    type Slots = BTreeMap<Vec<u8>, Weak<Mutex<()>>>;
    static SLOTS: OnceLock<Mutex<Slots>> = OnceLock::new();
    #[cfg(unix)]
    let key = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path)
            .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()))?
            .dev()
            .to_be_bytes()
            .to_vec()
    };
    #[cfg(windows)]
    let key = windows_drive_key(path);
    let mut slots = SLOTS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| issue(MetadataIssueCode::Io, "metadata volume slots unavailable"))?;
    slots.retain(|_, slot| slot.strong_count() > 0);
    if let Some(slot) = slots.get(&key).and_then(Weak::upgrade) {
        return Ok(slot);
    }
    let slot = Arc::new(Mutex::new(()));
    slots.insert(key, Arc::downgrade(&slot));
    Ok(slot)
}
#[cfg(windows)]
fn windows_drive_key(path: &Path) -> Vec<u8> {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Vec::new();
    };
    match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => vec![letter.to_ascii_uppercase()],
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            use std::os::windows::ffi::OsStrExt;
            let text =
                format!("{}\\{}", server.to_string_lossy(), share.to_string_lossy()).to_uppercase();
            std::ffi::OsStr::new(&text)
                .encode_wide()
                .flat_map(u16::to_le_bytes)
                .collect()
        }
        // Unusual device namespaces conservatively share a single slot. This key
        // is scheduling data only; the exact native source path is never changed.
        _ => Vec::new(),
    }
}
fn acquire_slot<'a>(
    slot: &'a Mutex<()>,
    control: &JobControl,
) -> Result<std::sync::MutexGuard<'a, ()>, MetadataIssue> {
    loop {
        if control.is_cancelled() {
            return Err(issue(
                MetadataIssueCode::Cancelled,
                "metadata cancelled while queued",
            ));
        }
        match slot.try_lock() {
            Ok(slot) => return Ok(slot),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err(issue(MetadataIssueCode::Io, "metadata slot unavailable")),
        }
    }
}
fn remaining(start: Instant) -> Result<ProcessLimits, MetadataIssue> {
    let timeout = Duration::from_secs(10)
        .checked_sub(start.elapsed())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| {
            issue(
                MetadataIssueCode::Timeout,
                "metadata per-file deadline exceeded",
            )
        })?;
    Ok(ProcessLimits {
        timeout,
        ..Default::default()
    })
}
impl MediaInfoProvider {
    fn probe_with_budget(
        &self,
        path: &Path,
        fields: &[PropertyKey],
        control: &JobControl,
        start: Instant,
    ) -> Result<MetadataRecord, MetadataIssue> {
        let mut command = Command::new(&self.executable);
        command
            .arg("--Output=JSON")
            .arg("--Language=raw")
            .arg("--Full")
            .arg(path);
        let record = parse_media_info(
            &run_bounded(&mut command, control, remaining(start)?)?,
            fields,
            &self.version,
        )?;
        remaining(start)?;
        Ok(record)
    }
}
impl MetadataProvider for MediaInfoProvider {
    fn version(&self) -> &str {
        &self.version
    }
    fn probe(
        &self,
        path: &Path,
        fields: &[PropertyKey],
        control: &JobControl,
    ) -> Result<MetadataRecord, MetadataIssue> {
        if fields.is_empty() {
            return Ok(MetadataRecord {
                values: BTreeMap::new(),
            });
        }
        if !path.is_absolute() {
            return Err(issue(
                MetadataIssueCode::InvalidData,
                "metadata source must be a native absolute path",
            ));
        }
        let slot = volume_slot(path)?;
        let _guard = acquire_slot(&slot, control)?;
        self.probe_with_budget(path, fields, control, Instant::now())
    }
}
/// App-owned resource configuration. No UI-selected executable or startup fetch.
pub struct DesktopMetadataProvider {
    media: Option<MediaInfoProvider>,
    property_executable: Option<PathBuf>,
    version: String,
}
impl DesktopMetadataProvider {
    pub fn unavailable() -> Self {
        Self {
            media: None,
            property_executable: std::env::current_exe().ok(),
            version: format!("MediaInfo-unavailable/{PROPERTY_VERSION}"),
        }
    }
    pub fn from_resources(resource_dir: &Path) -> Self {
        let mut provider = Self::unavailable();
        if cfg!(all(windows, target_arch = "x86_64")) {
            let executable = resource_dir.join("mediainfo/windows-x86_64/MediaInfo.exe");
            // The pinned executable hash is checked before each cache-miss spawn.
            provider.media = Some(MediaInfoProvider::from_executable(
                executable,
                "25.10".into(),
            ));
            provider.version = format!("MediaInfo-25.10/{PROPERTY_VERSION}");
        }
        provider
    }
}
impl MetadataProvider for DesktopMetadataProvider {
    fn version(&self) -> &str {
        &self.version
    }
    fn probe(
        &self,
        path: &Path,
        fields: &[PropertyKey],
        control: &JobControl,
    ) -> Result<MetadataRecord, MetadataIssue> {
        if fields.is_empty() {
            return Ok(MetadataRecord {
                values: BTreeMap::new(),
            });
        }
        let slot = volume_slot(path)?;
        let _guard = acquire_slot(&slot, control)?;
        let start = Instant::now();
        let media = fields
            .iter()
            .filter(|f| !matches!(f, PropertyKey::Windows(_)))
            .cloned()
            .collect::<Vec<_>>();
        let mut record = MetadataRecord {
            values: BTreeMap::new(),
        };
        if !media.is_empty() {
            let result = self
                .media
                .as_ref()
                .ok_or_else(|| {
                    issue(
                        MetadataIssueCode::Unavailable,
                        "bundled MediaInfo is unavailable for this architecture",
                    )
                })
                .and_then(|provider| {
                    verify_packaged_executable(&provider.executable)?;
                    provider.probe_with_budget(path, &media, control, start)
                });
            match result {
                Ok(output) => record = output,
                Err(error) => {
                    for key in &media {
                        record.values.insert(
                            key.clone(),
                            MetadataValue {
                                value: MetadataState::Error(error.clone()),
                                source_key: format!("{key:?}"),
                                provider: "MediaInfo".into(),
                                version: "25.10".into(),
                                timezone: TimezoneEvidence::Unknown,
                            },
                        );
                    }
                }
            }
        }
        let properties = fields
            .iter()
            .filter_map(|f| {
                if let PropertyKey::Windows(p) = f {
                    Some(*p)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if !properties.is_empty() {
            let result = probe_windows_process(
                self.property_executable.as_deref(),
                path,
                &properties,
                control,
                start,
            );
            match result {
                Ok(values) => record.values.extend(values),
                Err(error) => {
                    for property in properties {
                        record.values.insert(
                            PropertyKey::Windows(property),
                            MetadataValue {
                                value: MetadataState::Error(error.clone()),
                                source_key: property.canonical_name().into(),
                                provider: "WindowsPropertySystem".into(),
                                version: PROPERTY_VERSION.into(),
                                timezone: TimezoneEvidence::Unknown,
                            },
                        );
                    }
                }
            }
        }
        Ok(record)
    }
}
fn probe_windows_process(
    executable: Option<&Path>,
    path: &Path,
    properties: &[WindowsProperty],
    control: &JobControl,
    start: Instant,
) -> Result<Vec<(PropertyKey, MetadataValue)>, MetadataIssue> {
    if !cfg!(windows) {
        return Ok(properties
            .iter()
            .map(|p| {
                (
                    PropertyKey::Windows(*p),
                    normalize_windows_property(*p, WindowsRawValue::Unsupported),
                )
            })
            .collect());
    }
    let executable = executable.ok_or_else(|| {
        issue(
            MetadataIssueCode::Unavailable,
            "native property helper unavailable",
        )
    })?;
    let mut command = Command::new(executable);
    command
        .arg("--vidlib-property-probe")
        .arg(path)
        .args(properties.iter().map(|p| p.canonical_name()));
    let bytes = run_bounded(&mut command, control, remaining(start)?)?;
    let values: Vec<(PropertyKey, MetadataValue)> = serde_json::from_slice(&bytes)
        .map_err(|e| issue(MetadataIssueCode::InvalidData, e.to_string()))?;
    if values.len() != properties.len()
        || values
            .iter()
            .zip(properties)
            .any(|((key, value), property)| {
                key != &PropertyKey::Windows(*property)
                    || value.source_key != property.canonical_name()
                    || value.version != PROPERTY_VERSION
            })
    {
        return Err(issue(
            MetadataIssueCode::InvalidData,
            "native property helper response mismatch",
        ));
    }
    Ok(values)
}
pub fn verify_packaged_executable(path: &Path) -> Result<(), MetadataIssue> {
    use sha2::{Digest, Sha256};
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../tools/media-info-manifest.json"))
            .expect("committed manifest");
    let expected = manifest["artifacts"][0]["files"]["MediaInfo.exe"]["sha256"]
        .as_str()
        .expect("manifest executable hash");
    let file = std::fs::File::open(path).map_err(|e| {
        issue(
            MetadataIssueCode::Unavailable,
            format!("bundled metadata executable unavailable: {e}"),
        )
    })?;
    let mut reader = file.take(32 * 1024 * 1024);
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()))?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    if format!("{:x}", digest.finalize()) != expected {
        return Err(issue(
            MetadataIssueCode::InvalidData,
            "bundled MediaInfo SHA-256 mismatch",
        ));
    }
    Ok(())
}
fn decimal(value: &str) -> Option<(i128, u32)> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 18
    {
        return None;
    }
    Some((
        format!("{whole}{fraction}").parse().ok()?,
        fraction.len() as u32,
    ))
}
fn recorded(value: &str) -> Option<(FrozenFieldValue, TimezoneEvidence)> {
    if let Ok(date) = chrono::DateTime::parse_from_rfc3339(value) {
        let zone = FrozenTimezone::FixedOffset {
            seconds: date.offset().local_minus_utc(),
        };
        return Some((
            FrozenFieldValue::Timestamp {
                ns: i128::from(date.timestamp()) * 1_000_000_000
                    + i128::from(date.timestamp_subsec_nanos()),
                timezone: Some(zone.clone()),
            },
            TimezoneEvidence::Known(zone),
        ));
    }
    let (text, zone) = if let Some(text) = value
        .strip_prefix("UTC ")
        .or_else(|| value.strip_suffix(" UTC"))
    {
        (text, Some(FrozenTimezone::Utc))
    } else {
        (value, None)
    };
    let date = ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(text, f).ok())?;
    Some((
        FrozenFieldValue::LocalTimestamp {
            value: date,
            timezone: zone.clone(),
        },
        zone.map(TimezoneEvidence::Known)
            .unwrap_or(TimezoneEvidence::Unknown),
    ))
}
pub fn parse_media_info(
    bytes: &[u8],
    fields: &[PropertyKey],
    version: &str,
) -> Result<MetadataRecord, MetadataIssue> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(issue(
            MetadataIssueCode::OutputLimit,
            "metadata JSON exceeds 4 MiB",
        ));
    }
    let document: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| {
        issue(
            MetadataIssueCode::InvalidData,
            format!("invalid metadata JSON: {e}"),
        )
    })?;
    if document["creatingLibrary"]["version"].as_str() != Some(version) {
        return Err(issue(
            MetadataIssueCode::InvalidData,
            "metadata provider version mismatch",
        ));
    }
    let tracks = document["media"]["track"].as_array().ok_or_else(|| {
        issue(
            MetadataIssueCode::InvalidData,
            "metadata track array missing",
        )
    })?;
    let mut values = BTreeMap::new();
    for field in fields {
        let (track, key) = match field {
            PropertyKey::Windows(property) => {
                values.insert(
                    field.clone(),
                    MetadataValue {
                        value: MetadataState::Unsupported,
                        source_key: property.canonical_name().into(),
                        provider: "WindowsPropertySystem".into(),
                        version: PROPERTY_VERSION.into(),
                        timezone: TimezoneEvidence::Unknown,
                    },
                );
                continue;
            }
            PropertyKey::Recorded => ("General", "Recorded_Date"),
            PropertyKey::Media(property) => match property {
                MediaProperty::Title => ("General", "Title"),
                MediaProperty::Language => ("Audio", "Language"),
                MediaProperty::Width => ("Video", "Width"),
                MediaProperty::Height => ("Video", "Height"),
                MediaProperty::DurationNs => ("General", "Duration"),
                MediaProperty::FrameRate => ("Video", "FrameRate"),
                MediaProperty::VideoCodec => ("Video", "Format"),
                MediaProperty::AudioCodec => ("Audio", "Format"),
                MediaProperty::AudioChannels => ("Audio", "Channels"),
                MediaProperty::BitRate => ("General", "OverallBitRate"),
                MediaProperty::Container => ("General", "Format"),
            },
        };
        let raw = tracks
            .iter()
            .find(|t| t["@type"].as_str() == Some(track))
            .and_then(|t| t.get(key));
        let mut timezone = if *field == PropertyKey::Recorded {
            TimezoneEvidence::Unknown
        } else {
            TimezoneEvidence::NotApplicable
        };
        let state = match raw {
            None | Some(serde_json::Value::Null) => MetadataState::Missing,
            Some(serde_json::Value::String(raw)) if raw.is_empty() => MetadataState::Missing,
            Some(serde_json::Value::String(raw)) => {
                let typed = match field {
                    PropertyKey::Recorded => recorded(raw).map(|(value, zone)| {
                        timezone = zone;
                        value
                    }),
                    PropertyKey::Media(
                        MediaProperty::Width
                        | MediaProperty::Height
                        | MediaProperty::AudioChannels
                        | MediaProperty::BitRate,
                    ) => raw
                        .parse::<i128>()
                        .ok()
                        .filter(|n| *n >= 0)
                        .map(FrozenFieldValue::Integer),
                    PropertyKey::Media(MediaProperty::DurationNs) => decimal(raw)
                        .and_then(|(coefficient, scale)| {
                            if scale <= 9 {
                                coefficient.checked_mul(10i128.pow(9 - scale))
                            } else {
                                let divisor = 10i128.pow(scale - 9);
                                (coefficient % divisor == 0).then_some(coefficient / divisor)
                            }
                        })
                        .map(FrozenFieldValue::Integer),
                    PropertyKey::Media(MediaProperty::FrameRate) => {
                        decimal(raw).map(|(coefficient, scale)| FrozenFieldValue::Decimal {
                            coefficient,
                            scale,
                        })
                    }
                    _ => Some(FrozenFieldValue::Text(raw.clone())),
                };
                typed.map(MetadataState::Value).unwrap_or_else(|| {
                    MetadataState::Error(issue(
                        MetadataIssueCode::InvalidData,
                        format!("invalid {track}.{key} type"),
                    ))
                })
            }
            _ => MetadataState::Error(issue(
                MetadataIssueCode::InvalidData,
                format!("invalid {track}.{key} JSON type"),
            )),
        };
        values.insert(
            field.clone(),
            MetadataValue {
                value: state,
                source_key: format!("{track}.{key}"),
                provider: "MediaInfo".into(),
                version: version.into(),
                timezone,
            },
        );
    }
    Ok(MetadataRecord { values })
}

#[derive(Clone, Debug)]
pub enum WindowsRawValue {
    Empty,
    Text(String),
    TextList(Vec<String>),
    FileTime(u64),
    Unsupported,
}
pub fn normalize_windows_property(
    property: WindowsProperty,
    value: WindowsRawValue,
) -> MetadataValue {
    let mut timezone = TimezoneEvidence::NotApplicable;
    let value = match (property, value) {
        (_, WindowsRawValue::Empty) => MetadataState::Missing,
        (_, WindowsRawValue::Unsupported) => MetadataState::Unsupported,
        (WindowsProperty::Title, WindowsRawValue::Text(value)) => {
            MetadataState::Value(FrozenFieldValue::Text(value))
        }
        (WindowsProperty::Author, WindowsRawValue::TextList(value)) => {
            MetadataState::Value(FrozenFieldValue::TextList(value))
        }
        (
            WindowsProperty::DateEncoded | WindowsProperty::DateTaken,
            WindowsRawValue::FileTime(ticks),
        ) => {
            let ns = (i128::from(ticks) - 116_444_736_000_000_000) * 100;
            timezone = TimezoneEvidence::Known(FrozenTimezone::Utc);
            MetadataState::Value(FrozenFieldValue::Timestamp {
                ns,
                timezone: Some(FrozenTimezone::Utc),
            })
        }
        _ => MetadataState::Error(issue(
            MetadataIssueCode::InvalidData,
            "canonical property returned an incompatible PROPVARIANT type",
        )),
    };
    MetadataValue {
        value,
        source_key: property.canonical_name().into(),
        provider: "WindowsPropertySystem".into(),
        version: PROPERTY_VERSION.into(),
        timezone,
    }
}
pub const PROPERTY_VERSION: &str = "typed-properties-v1";

#[cfg(windows)]
struct ProcessJob(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(windows)]
fn process_job(child: &std::process::Child) -> Result<ProcessJob, MetadataIssue> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{Foundation::HANDLE, System::JobObjects::*};
    unsafe {
        let job = ProcessJob(
            CreateJobObjectW(None, None)
                .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()))?,
        );
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of_val(&info) as u32,
        )
        .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()))?;
        AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle()))
            .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()))?;
        Ok(job)
    }
}

/// Runs only in the isolated native-property process, on its COM-owning thread.
#[cfg(windows)]
pub fn read_windows_properties(
    path: &Path,
    fields: &[WindowsProperty],
) -> Result<Vec<(PropertyKey, MetadataValue)>, MetadataIssue> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::PROPERTYKEY,
            System::{
                Com::{
                    CoInitializeEx, CoUninitialize,
                    StructuredStorage::{PropVariantClear, PROPVARIANT},
                    COINIT_MULTITHREADED,
                },
                Variant::*,
            },
            UI::Shell::PropertiesSystem::*,
        },
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    struct Value(PROPVARIANT);
    impl Drop for Value {
        fn drop(&mut self) {
            unsafe {
                let _ = PropVariantClear(&mut self.0);
            }
        }
    }
    unsafe fn bounded_text(pointer: windows::core::PWSTR) -> Result<String, MetadataIssue> {
        if pointer.is_null() {
            return Err(issue(
                MetadataIssueCode::InvalidData,
                "null property string",
            ));
        }
        for length in 0..32768 {
            if unsafe { *pointer.0.add(length) } == 0 {
                return String::from_utf16(unsafe {
                    std::slice::from_raw_parts(pointer.0, length)
                })
                .map_err(|_| issue(MetadataIssueCode::InvalidData, "invalid property UTF-16"));
            }
        }
        Err(issue(
            MetadataIssueCode::OutputLimit,
            "property string exceeds 32768 UTF-16 units",
        ))
    }
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| issue(MetadataIssueCode::Unavailable, e.to_string()))?;
        let _apartment = Apartment;
        let path = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let store: IPropertyStore =
            SHGetPropertyStoreFromParsingName(PCWSTR(path.as_ptr()), None, GPS_DEFAULT)
                .map_err(|e| issue(MetadataIssueCode::Unavailable, e.to_string()))?;
        let mut output = vec![];
        for property in fields {
            let name = property
                .canonical_name()
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let mut key = PROPERTYKEY::default();
            PSGetPropertyKeyFromName(PCWSTR(name.as_ptr()), &mut key)
                .map_err(|e| issue(MetadataIssueCode::Unavailable, e.to_string()))?;
            let result = (|| -> Result<MetadataValue, MetadataIssue> {
                let value = Value(
                    store
                        .GetValue(&key)
                        .map_err(|e| issue(MetadataIssueCode::Io, e.to_string()))?,
                );
                let data = &value.0.Anonymous.Anonymous;
                let raw = match data.vt {
                    VT_EMPTY | VT_NULL => WindowsRawValue::Empty,
                    VT_LPWSTR => WindowsRawValue::Text(bounded_text(data.Anonymous.pwszVal)?),
                    VT_FILETIME => {
                        let time = data.Anonymous.filetime;
                        WindowsRawValue::FileTime(
                            (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime),
                        )
                    }
                    kind if kind.0 == VT_VECTOR.0 | VT_LPWSTR.0 => {
                        let strings = data.Anonymous.calpwstr;
                        if strings.cElems > 1024 || (strings.cElems > 0 && strings.pElems.is_null())
                        {
                            return Err(issue(
                                MetadataIssueCode::OutputLimit,
                                "property vector exceeds 1024 strings",
                            ));
                        }
                        let mut text = Vec::new();
                        for i in 0..strings.cElems as usize {
                            text.push(bounded_text(*strings.pElems.add(i))?);
                        }
                        WindowsRawValue::TextList(text)
                    }
                    _ => WindowsRawValue::Unsupported,
                };
                Ok(normalize_windows_property(*property, raw))
            })();
            output.push((
                PropertyKey::Windows(*property),
                result.unwrap_or_else(|e| MetadataValue {
                    value: MetadataState::Error(e),
                    source_key: property.canonical_name().into(),
                    provider: "WindowsPropertySystem".into(),
                    version: PROPERTY_VERSION.into(),
                    timezone: TimezoneEvidence::Unknown,
                }),
            ));
        }
        Ok(output)
    }
}
/// Handles the private helper mode before Tauri/WebView or app storage starts.
/// Source argv remains native UTF-16 on Windows; canonical fields are an allowlist.
pub fn property_helper_main() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--vidlib-property-probe")) {
        return None;
    }
    #[cfg(windows)]
    {
        let result = (|| {
            let path =
                PathBuf::from(args.next().ok_or_else(|| {
                    issue(MetadataIssueCode::InvalidData, "property source missing")
                })?);
            if !path.is_absolute() {
                return Err(issue(
                    MetadataIssueCode::InvalidData,
                    "absolute property source required",
                ));
            }
            let fields = args
                .map(|arg| {
                    serde_json::from_str::<WindowsProperty>(&format!(
                        "\"{}\"",
                        arg.to_string_lossy()
                    ))
                    .map_err(|_| {
                        issue(MetadataIssueCode::InvalidData, "unknown canonical property")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if fields.is_empty() || fields.len() > 4 {
                return Err(issue(
                    MetadataIssueCode::InvalidData,
                    "invalid property count",
                ));
            }
            read_windows_properties(&path, &fields)
        })();
        match result {
            Ok(values) => {
                use std::io::Write;
                let bytes = serde_json::to_vec(&values).expect("serializable metadata");
                if bytes.len() > 4 * 1024 * 1024 {
                    return Some(2);
                }
                Some(if std::io::stdout().write_all(&bytes).is_ok() {
                    0
                } else {
                    2
                })
            }
            Err(error) => {
                eprintln!("{}", error.message);
                Some(2)
            }
        }
    }
    #[cfg(not(windows))]
    Some(2)
}

#[cfg(test)]
mod coordination_tests {
    use super::*;
    #[test]
    fn file_deadline_is_shared_across_provider_phases() {
        let start = Instant::now() - Duration::from_millis(9750);
        let budget = remaining(start).unwrap();
        assert!(
            budget.timeout <= Duration::from_millis(250),
            "a second provider cannot receive a fresh ten seconds"
        );
        assert_eq!(
            remaining(Instant::now() - Duration::from_secs(11))
                .unwrap_err()
                .code,
            MetadataIssueCode::Timeout
        );
    }
    #[test]
    fn same_volume_slots_are_shared_and_queue_is_cancellable() {
        let d = tempfile::tempdir().unwrap();
        let one = d.path().join("one");
        let two = d.path().join("two");
        std::fs::write(&one, b"1").unwrap();
        std::fs::write(&two, b"2").unwrap();
        let first = volume_slot(&one).unwrap();
        let second = volume_slot(&two).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let held = first.lock().unwrap();
        let control = JobControl::default();
        let cancel = control.clone();
        let waiter = std::thread::spawn(move || acquire_slot(&second, &control).err().unwrap());
        cancel.cancel();
        assert_eq!(waiter.join().unwrap().code, MetadataIssueCode::Cancelled);
        drop(held);
        drop(acquire_slot(&first, &JobControl::default()).unwrap());
    }
}

#[cfg(all(test, windows))]
mod windows_drive_tests {
    use super::*;
    #[test]
    fn extended_native_prefixes_share_drive_slots() {
        assert_eq!(
            windows_drive_key(Path::new(r"C:\media\one.mp4")),
            windows_drive_key(Path::new(r"\\?\c:\media\two.mp4"))
        );
        assert_eq!(
            windows_drive_key(Path::new(r"\\server\share\one.mp4")),
            windows_drive_key(Path::new(r"\\?\UNC\SERVER\SHARE\two.mp4"))
        );
        assert_ne!(
            windows_drive_key(Path::new(r"C:\one.mp4")),
            windows_drive_key(Path::new(r"D:\two.mp4"))
        );
    }
}
