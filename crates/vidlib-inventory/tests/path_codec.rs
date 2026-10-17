use std::path::{Path, PathBuf};
use vidlib_inventory::ids::*;
use vidlib_inventory::{
    decode_path, encode_path, FileStamp, InventoryRequest, NativePathRecord, PathPlatform,
};

#[test]
fn path_codec_round_trips_native_units() -> Result<(), Box<dyn std::error::Error>> {
    let native_path = PathBuf::from("media/空/episode.mkv");
    assert_eq!(decode_path(&encode_path(&native_path))?, native_path);
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let path = PathBuf::from(OsString::from_vec(b"media/\xff.mkv".to_vec()));
        assert_eq!(decode_path(&encode_path(&path))?, path);
    }
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        let path = PathBuf::from(OsString::from_wide(&[0x0061, 0xD800, 0x0062]));
        assert_eq!(decode_path(&encode_path(&path))?, path);
    }
    Ok(())
}

#[test]
fn windows_utf16_fixture_is_exact_and_foreign_paths_are_not_transcoded() {
    let record = NativePathRecord {
        platform: PathPlatform::WindowsUtf16Le,
        bytes: vec![0x61, 0, 0, 0xD8, 0x62, 0],
    };
    let json = serde_json::to_string(&record).unwrap();
    let restored: NativePathRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, record);
    #[cfg(not(windows))]
    assert!(decode_path(&record).is_err());
    #[cfg(windows)]
    assert_eq!(encode_path(&decode_path(&record).unwrap()), record);
    let odd = NativePathRecord {
        platform: PathPlatform::WindowsUtf16Le,
        bytes: vec![1],
    };
    assert!(decode_path(&odd).is_err());
}

#[test]
fn portable_relative_paths_reject_escape_and_native_display_is_not_lookup() {
    for value in ["../escape", "/absolute", "nested/../escape", ""] {
        assert!(
            encode_path(Path::new(value)).validate_relative().is_err(),
            "{value}"
        );
    }
    assert!(encode_path(Path::new("nested/episode.mkv"))
        .validate_relative()
        .is_ok());
}

#[test]
fn all_ids_are_distinct_uuid_strings() {
    macro_rules! check { ($($id:ident),*) => { $(
        let id = $id::new();
        assert_ne!(id, $id::new());
        let json = serde_json::to_value(id).unwrap();
        assert!(json.is_string());
        assert_eq!(serde_json::from_value::<$id>(json).unwrap(), id);
    )* }; }
    check!(
        InventoryId,
        RootId,
        EntryId,
        GrantId,
        SessionId,
        JobId,
        PlanId,
        GroupId,
        StepId,
        OperationId,
        EventId,
        PresetId,
        CollectionId,
        AttemptId,
        AttachmentId
    );
}

#[test]
fn json_quantities_and_nanosecond_timestamps_use_validated_decimal_strings() {
    let stamp = FileStamp {
        size: Some(u64::MAX),
        created_ns: Some(i128::MIN),
        modified_ns: Some(i128::MAX),
        accessed_ns: Some(-1),
        ..FileStamp::default()
    };
    let json = serde_json::to_value(&stamp).unwrap();
    assert_eq!(json["size"], u64::MAX.to_string());
    assert_eq!(json["created_ns"], i128::MIN.to_string());
    assert_eq!(json["modified_ns"], i128::MAX.to_string());
    assert_eq!(
        serde_json::from_value::<FileStamp>(json.clone()).unwrap(),
        stamp
    );
    for invalid in [
        serde_json::json!(9007199254740993u64),
        serde_json::json!("01"),
        serde_json::json!("-1"),
        serde_json::json!("18446744073709551616"),
    ] {
        let mut bad = json.clone();
        bad["size"] = invalid;
        assert!(serde_json::from_value::<FileStamp>(bad).is_err());
    }
    for invalid in ["+1", "01", "-0", "170141183460469231731687303715884105728"] {
        let mut bad = json.clone();
        bad["modified_ns"] = serde_json::json!(invalid);
        assert!(serde_json::from_value::<FileStamp>(bad).is_err());
    }
    let mut request = InventoryRequest::new(vec![]);
    request.generation = u64::MAX;
    assert_eq!(
        serde_json::to_value(request).unwrap()["generation"],
        u64::MAX.to_string()
    );
}
