use std::{
    process::Command,
    time::{Duration, Instant},
};
use vidlib_inventory::JobControl;
use vidlib_rename::{metadata::*, *};
use vidlib_tauri::metadata::*;

#[test]
#[ignore = "subprocess fixture; only invoked by bounded parent"]
fn helper_fixture() {
    use std::io::Write;
    let Some(mode) = std::env::var_os("VIDLIB_TEST_HELPER_MODE") else {
        return;
    };
    if let Some(path) = std::env::var_os("VIDLIB_TEST_PID") {
        std::fs::write(path, std::process::id().to_string()).unwrap();
    }
    match mode.to_str().unwrap() {
        "hang" => loop {
            std::thread::sleep(Duration::from_millis(10));
        },
        "stdout" => loop {
            if std::io::stdout().write_all(&[b'x'; 8192]).is_err() {
                break;
            }
        },
        "stderr" => loop {
            if std::io::stderr().write_all(&[b'x'; 8192]).is_err() {
                break;
            }
        },
        _ => panic!("unexpected fixture mode"),
    }
}
fn fixture_command(mode: &str) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args(["--exact", "helper_fixture", "--ignored", "--nocapture"]);
    c.env("VIDLIB_TEST_HELPER_MODE", mode);
    c
}
#[test]
fn helper_timeout_kills_and_reaps() {
    let d = tempfile::tempdir().unwrap();
    let pid = d.path().join("pid");
    let mut c = fixture_command("hang");
    c.env("VIDLIB_TEST_PID", &pid);
    let start = Instant::now();
    let e = run_bounded(
        &mut c,
        &JobControl::default(),
        ProcessLimits {
            timeout: Duration::from_millis(200),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, MetadataIssueCode::Timeout);
    assert!(start.elapsed() < Duration::from_secs(3));
    #[cfg(unix)]
    {
        let pid: i32 = std::fs::read_to_string(pid).unwrap().parse().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "child must be reaped");
    }
}
#[test]
fn helper_cancellation_kills_and_reaps() {
    let control = JobControl::default();
    let copy = control.clone();
    let cancel = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        copy.cancel();
    });
    let e = run_bounded(
        &mut fixture_command("hang"),
        &control,
        ProcessLimits::default(),
    )
    .unwrap_err();
    cancel.join().unwrap();
    assert_eq!(e.code, MetadataIssueCode::Cancelled);
}
#[test]
fn stdout_stderr_and_json_are_bounded() {
    for mode in ["stdout", "stderr"] {
        let e = run_bounded(
            &mut fixture_command(mode),
            &JobControl::default(),
            ProcessLimits::default(),
        )
        .unwrap_err();
        assert_eq!(e.code, MetadataIssueCode::OutputLimit, "{mode}");
    }
    let fields = [PropertyKey::Media(MediaProperty::Width)];
    assert_eq!(
        parse_media_info(&vec![b' '; 4 * 1024 * 1024 + 1], &fields, "25.04")
            .unwrap_err()
            .code,
        MetadataIssueCode::OutputLimit
    );
    assert_eq!(
        parse_media_info(b"{", &fields, "25.04").unwrap_err().code,
        MetadataIssueCode::InvalidData
    );
}
#[test]
fn fixture_video_duration_resolution_codec_title_date() {
    // Explicit developer machine binary is injected only here, never resolved by production.
    #[cfg(not(windows))]
    let exe = std::path::PathBuf::from("/usr/bin/mediainfo");
    #[cfg(windows)]
    let exe = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/mediainfo/windows-x86_64/MediaInfo.exe");
    #[cfg(not(windows))]
    let version = "25.04";
    #[cfg(windows)]
    let version = "25.10";
    let p = MediaInfoProvider::from_executable(exe, version.into());
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/media/metadata.mp4");
    let fields = [
        PropertyKey::Media(MediaProperty::Width),
        PropertyKey::Media(MediaProperty::Height),
        PropertyKey::Media(MediaProperty::DurationNs),
        PropertyKey::Media(MediaProperty::VideoCodec),
        PropertyKey::Recorded,
        PropertyKey::Media(MediaProperty::Title),
        PropertyKey::Media(MediaProperty::Language),
    ];
    let record = p.probe(&path, &fields, &JobControl::default()).unwrap();
    assert_eq!(
        record.values[&fields[0]].value,
        MetadataState::Value(FrozenFieldValue::Integer(64))
    );
    assert_eq!(
        record.values[&fields[1]].value,
        MetadataState::Value(FrozenFieldValue::Integer(48))
    );
    assert_eq!(
        record.values[&fields[2]].value,
        MetadataState::Value(FrozenFieldValue::Integer(1_000_000_000))
    );
    assert_eq!(
        record.values[&fields[3]].value,
        MetadataState::Value(FrozenFieldValue::Text("MPEG-4 Visual".into()))
    );
    assert_eq!(
        record.values[&PropertyKey::Recorded].timezone,
        TimezoneEvidence::Unknown
    );
    assert_eq!(
        record.values[&fields[5]].value,
        MetadataState::Value(FrozenFieldValue::Text("VidLib fixture".into()))
    );
    assert_eq!(
        record.values[&fields[6]].value,
        MetadataState::Value(FrozenFieldValue::Text("en".into()))
    );
    assert!(matches!(
        record.values[&PropertyKey::Recorded].value,
        MetadataState::Value(FrozenFieldValue::LocalTimestamp { timezone: None, .. })
    ));
}

#[test]
fn windows_property_types_are_normalized() {
    let title = normalize_windows_property(
        WindowsProperty::Title,
        WindowsRawValue::Text("A title".into()),
    );
    assert_eq!(
        title.value,
        MetadataState::Value(FrozenFieldValue::Text("A title".into()))
    );
    let author = normalize_windows_property(
        WindowsProperty::Author,
        WindowsRawValue::TextList(vec!["One".into(), "Two".into()]),
    );
    assert_eq!(
        author.value,
        MetadataState::Value(FrozenFieldValue::TextList(vec!["One".into(), "Two".into()]))
    );
    let date = normalize_windows_property(
        WindowsProperty::DateEncoded,
        WindowsRawValue::FileTime(116_444_736_000_000_001),
    );
    assert_eq!(
        date.value,
        MetadataState::Value(FrozenFieldValue::Timestamp {
            ns: 100,
            timezone: Some(FrozenTimezone::Utc)
        })
    );
    assert_eq!(date.source_key, "System.Media.DateEncoded");
    assert_eq!(
        normalize_windows_property(WindowsProperty::Title, WindowsRawValue::Empty).value,
        MetadataState::Missing
    );
    assert!(
        matches!(
            normalize_windows_property(WindowsProperty::Title, WindowsRawValue::FileTime(0)).value,
            MetadataState::Error(_)
        ),
        "never stringify a mismatched PROPVARIANT"
    );
}
#[test]
fn embedded_title_language_dates_and_requested_types() {
    let doc = br#"{"creatingLibrary":{"version":"25.10"},"media":{"track":[{"@type":"General","Title":"Fixture title","Recorded_Date":"2024-02-03 04:05:06","Duration":"1.000000001"},{"@type":"Audio","Language":"en"},{"@type":"Video","Width":"bad"}]}}"#;
    let fields = [
        PropertyKey::Media(MediaProperty::Title),
        PropertyKey::Media(MediaProperty::Language),
        PropertyKey::Recorded,
        PropertyKey::Media(MediaProperty::DurationNs),
    ];
    let r = parse_media_info(doc, &fields, "25.10").unwrap();
    assert_eq!(
        r.values.len(),
        4,
        "unrequested malformed Width is not parsed"
    );
    assert_eq!(
        r.values[&fields[0]].value,
        MetadataState::Value(FrozenFieldValue::Text("Fixture title".into()))
    );
    assert_eq!(
        r.values[&fields[1]].value,
        MetadataState::Value(FrozenFieldValue::Text("en".into()))
    );
    assert_eq!(r.values[&fields[2]].timezone, TimezoneEvidence::Unknown);
    assert_eq!(
        r.values[&fields[3]].value,
        MetadataState::Value(FrozenFieldValue::Integer(1_000_000_001))
    );
    let wrong = [PropertyKey::Media(MediaProperty::Width)];
    assert!(matches!(
        parse_media_info(doc, &wrong, "25.10").unwrap().values[&wrong[0]].value,
        MetadataState::Error(_)
    ));
    assert_eq!(
        parse_media_info(doc, &fields, "other").unwrap_err().code,
        MetadataIssueCode::InvalidData
    );
}

#[test]
fn packaged_resource_is_resolved_from_tauri_staging_and_hash_checked() {
    let staging = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let exe = staging.join("mediainfo/windows-x86_64/MediaInfo.exe");
    verify_packaged_executable(&exe).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(staging.join("mediainfo/manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["version"], "25.10");
    for notice in [
        "LICENSE",
        "License.html",
        "ZenLib-LICENSE.txt",
        "TinyXML2-NOTICE.txt",
        "AES-NOTICE.txt",
        "zlib-LICENSE.txt",
    ] {
        assert!(
            !std::fs::read(staging.join("mediainfo/windows-x86_64").join(notice))
                .unwrap()
                .is_empty()
        );
    }
    let d = tempfile::tempdir().unwrap();
    let corrupt = d.path().join("MediaInfo.exe");
    std::fs::write(&corrupt, b"untrusted replacement").unwrap();
    assert_eq!(
        verify_packaged_executable(&corrupt).unwrap_err().code,
        MetadataIssueCode::InvalidData
    );
}

#[cfg(windows)]
#[test]
fn native_property_subprocess_reads_canonical_title_without_starting_webview() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/media/metadata.mp4");
    let mut command = Command::new(env!("CARGO_BIN_EXE_vidlib-tauri"));
    command
        .arg("--vidlib-property-probe")
        .arg(path)
        .args(["System.Title", "System.Author"]);
    let bytes = run_bounded(
        &mut command,
        &JobControl::default(),
        ProcessLimits::default(),
    )
    .unwrap();
    let values: Vec<(PropertyKey, MetadataValue)> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(values[0].0, PropertyKey::Windows(WindowsProperty::Title));
    assert_eq!(
        values[0].1.value,
        MetadataState::Value(FrozenFieldValue::Text("VidLib fixture".into()))
    );
    assert_eq!(values[1].0, PropertyKey::Windows(WindowsProperty::Author));
}

#[test]
fn requested_wrong_json_type_is_error_not_absence() {
    let doc=br#"{"creatingLibrary":{"version":"25.10"},"media":{"track":[{"@type":"Video","Width":{"value":"64"}}]}}"#;
    let key = PropertyKey::Media(MediaProperty::Width);
    let result = parse_media_info(doc, std::slice::from_ref(&key), "25.10").unwrap();
    assert!(
        matches!(result.values[&key].value, MetadataState::Error(_)),
        "a present incompatible field must not become Missing"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn transient_path_swap_cannot_feed_the_helper_substituted_media() {
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };
    struct SwappingProvider {
        original: PathBuf,
        replacement: PathBuf,
        calls: AtomicUsize,
    }
    impl MetadataProvider for SwappingProvider {
        fn version(&self) -> &str {
            "25.04"
        }
        fn probe(
            &self,
            path: &Path,
            fields: &[PropertyKey],
            control: &JobControl,
        ) -> Result<MetadataRecord, MetadataIssue> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let backup = self.original.with_extension("held");
            std::fs::rename(&self.original, &backup).unwrap();
            std::fs::copy(&self.replacement, &self.original).unwrap();
            let result = MediaInfoProvider::from_executable(
                PathBuf::from("/usr/bin/mediainfo"),
                "25.04".into(),
            )
            .probe(path, fields, control);
            std::fs::remove_file(&self.original).unwrap();
            std::fs::rename(backup, &self.original).unwrap();
            result
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.mp4");
    let replacement = dir.path().join("replacement.mp4");
    let bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/media/metadata.mp4"),
    )
    .unwrap();
    std::fs::write(&original, &bytes).unwrap();
    let mut other = bytes.clone();
    for start in (0..other.len() - 14).collect::<Vec<_>>() {
        if &other[start..start + 14] == b"VidLib fixture" {
            other[start..start + 14].copy_from_slice(b"Forged fixture");
        }
    }
    std::fs::write(&replacement, other).unwrap();
    let inv = vidlib_inventory::scan(
        &vidlib_inventory::InventoryRequest::new(vec![vidlib_inventory::RootBinding::new(
            original.clone(),
        )
        .unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let id = inv.entries[0].id;
    let req = InputRequirements {
        primary_order: vec![id],
        numbering_scope: Default::default(),
        timezone: FrozenTimezone::Utc,
        fields: vec![RequestedInput {
            scope: Default::default(),
            entry_id: id,
            rule_id: Some("r".into()),
            key: FieldKey::Media {
                property: MediaProperty::Title,
            },
        }],
    };
    let provider = SwappingProvider {
        original,
        replacement,
        calls: AtomicUsize::new(0),
    };
    let inputs = FrozenInputCache::default()
        .prepare_with_metadata(&req, &inv, &JobControl::default(), &provider)
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        inputs.values[0].value.resolved(),
        FrozenFieldValue::Text("VidLib fixture".into()),
        "helper must consume the held granted source despite a restored path swap"
    );
}
