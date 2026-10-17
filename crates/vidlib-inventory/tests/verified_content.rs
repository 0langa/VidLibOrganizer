use vidlib_inventory::*;
fn fixture() -> (tempfile::TempDir, Inventory, ids::EntryId) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/source"), b"abc").unwrap();
    let inv = scan(
        &InventoryRequest::new(vec![RootBinding::new(dir.path().to_owned()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let id = inv
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::File)
        .unwrap()
        .id;
    (dir, inv, id)
}
#[test]
fn held_content_checks_before_and_after_and_ignores_read_access_time() {
    let (dir, inv, id) = fixture();
    let content = with_verified_content(&inv, id, |file| {
        let mut bytes = vec![];
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
    .unwrap();
    assert_eq!(content, b"abc");
    assert!(with_verified_content(&inv, id, |_| {
        std::fs::write(dir.path().join("sub/source"), b"changed")?;
        Ok(())
    })
    .is_err());
}
#[test]
fn replacement_and_parent_rebinding_fail_closed() {
    let (dir, inv, id) = fixture();
    assert!(with_verified_content(&inv, id, |_| {
        std::fs::rename(dir.path().join("sub"), dir.path().join("old"))?;
        std::fs::create_dir(dir.path().join("sub"))?;
        std::fs::hard_link(dir.path().join("old/source"), dir.path().join("sub/source"))?;
        Ok(())
    })
    .is_err());
    assert!(
        with_verified_content(&inv, id, |_| Ok(())).is_err(),
        "replaced parent rejected even with same source inode"
    );
}
#[test]
fn wrong_grant_and_root_evidence_never_read_content() {
    let (_dir, mut inv, id) = fixture();
    inv.roots[0].source_grant.path = inv.roots[0].path.join("other");
    assert!(with_verified_content(&inv, id, |_| -> std::io::Result<()> {
        panic!("outside grant")
    })
    .is_err());
}
#[cfg(unix)]
#[test]
fn source_and_ancestor_links_are_rejected() {
    use std::os::unix::fs::symlink;
    let (dir, inv, id) = fixture();
    std::fs::rename(dir.path().join("sub/source"), dir.path().join("sub/real")).unwrap();
    symlink("real", dir.path().join("sub/source")).unwrap();
    assert!(
        with_verified_content(&inv, id, |_| -> std::io::Result<()> { panic!("link read") })
            .is_err()
    );
    std::fs::remove_file(dir.path().join("sub/source")).unwrap();
    std::fs::rename(dir.path().join("sub/real"), dir.path().join("sub/source")).unwrap();
    std::fs::rename(dir.path().join("sub"), dir.path().join("real_sub")).unwrap();
    symlink("real_sub", dir.path().join("sub")).unwrap();
    assert!(with_verified_content(&inv, id, |_| -> std::io::Result<()> {
        panic!("ancestor link read")
    })
    .is_err());
}
#[test]
fn unavailable_creation_stamp_does_not_invent_a_requirement() {
    let (_dir, mut inv, id) = fixture();
    inv.entries
        .iter_mut()
        .find(|e| e.id == id)
        .unwrap()
        .stamp
        .created_ns = None;
    let text = with_verified_content(&inv, id, |reader| {
        let mut s = String::new();
        reader.read_to_string(&mut s)?;
        Ok(s)
    })
    .unwrap();
    assert_eq!(text, "abc");
}
#[test]
fn access_time_only_change_is_not_material_and_file_grants_work() {
    let (dir, _, _) = fixture();
    let path = dir.path().join("sub/source");
    let inv = scan(
        &InventoryRequest::new(vec![RootBinding::new(path.clone()).unwrap()]),
        &JobControl::default(),
    )
    .unwrap();
    let id = inv.entries[0].id;
    std::fs::File::open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_accessed(std::time::UNIX_EPOCH))
        .unwrap();
    assert_eq!(
        with_verified_content(&inv, id, |reader| {
            let mut s = String::new();
            reader.read_to_string(&mut s)?;
            Ok(s)
        })
        .unwrap(),
        "abc"
    );
}
#[test]
fn root_and_source_replacements_fail_closed() {
    let (dir, inv, id) = fixture();
    std::fs::rename(dir.path().join("sub/source"), dir.path().join("old-source")).unwrap();
    std::fs::write(dir.path().join("sub/source"), b"xyz").unwrap();
    assert!(with_verified_content(&inv, id, |_| -> std::io::Result<()> {
        panic!("replacement reader called")
    })
    .is_err());
    let (dir, inv, id) = fixture();
    assert!(with_verified_content(&inv, id, |_| {
        std::fs::rename(dir.path(), dir.path().with_extension("moved"))?;
        std::fs::create_dir(dir.path())?;
        Ok(())
    })
    .is_err());
    std::fs::remove_dir_all(dir.path().with_extension("moved")).unwrap();
}

#[test]
fn selected_anchor_root_must_match_scan_identity_before_content_callback() {
    let (_dir, mut inventory, id) = fixture();
    let prior_root_identity = inventory.roots[0].evidence.observed_identity.clone();
    let anchor = inventory.roots[0]
        .path
        .ancestors()
        .last()
        .unwrap()
        .to_owned();
    // Build an anchor-root observation without recursively scanning the whole drive.
    // Both identities are real observations; only the selected root binding changes.
    let mut root = RootBinding::new(anchor.clone()).unwrap();
    root.evidence.observed_identity = OsInventoryFs
        .open_directory(&anchor)
        .unwrap()
        .observation()
        .unwrap()
        .stamp
        .physical_id;
    assert_ne!(root.evidence.observed_identity, prior_root_identity);
    for entry in &mut inventory.entries {
        entry.root_id = root.id;
        entry.relative_path = encode_path(entry.absolute_path.strip_prefix(&anchor).unwrap());
    }
    inventory.roots = vec![root];
    let bytes = with_verified_content(&inventory, id, |reader| {
        let mut bytes = vec![];
        reader.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
    .unwrap();
    assert_eq!(bytes, b"abc", "a correctly bound anchor remains readable");

    // Model a redirected/replaced root whose descendant still has its original stamp.
    inventory.roots[0].evidence.observed_identity = prior_root_identity;
    let called = std::cell::Cell::new(false);
    let result = with_verified_content(&inventory, id, |_| {
        called.set(true);
        Ok(())
    });
    assert!(
        !called.get(),
        "stale anchor root must be rejected before content callback"
    );
    assert!(
        result.is_err(),
        "descendant identity cannot authorize a different root"
    );
}

#[test]
fn helper_rejects_wrong_grant_and_stale_source_before_callback() {
    let (dir, mut inv, id) = fixture();
    let grant = inv.roots[0].source_grant.path.clone();
    inv.roots[0].source_grant.path = grant.join("other");
    assert!(
        with_verified_helper_path(&inv, id, |_| -> std::io::Result<()> {
            panic!("wrong grant reached helper")
        })
        .is_err()
    );
    inv.roots[0].source_grant.path = grant;
    std::fs::rename(dir.path().join("sub/source"), dir.path().join("old")).unwrap();
    std::fs::write(dir.path().join("sub/source"), b"xyz").unwrap();
    assert!(
        with_verified_helper_path(&inv, id, |_| -> std::io::Result<()> {
            panic!("replacement source reached helper")
        })
        .is_err()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn helper_path_pins_source_during_swap_and_releases_descriptor_after_callback() {
    let (dir, inv, id) = fixture();
    let source = dir.path().join("sub/source");
    let backup = dir.path().join("backup");
    let alias = with_verified_helper_path(&inv, id, |path| {
        assert_ne!(path, source);
        std::fs::rename(&source, &backup)?;
        std::fs::write(&source, b"xyz")?;
        assert_eq!(std::fs::read(path)?, b"abc");
        std::fs::remove_file(&source)?;
        std::fs::rename(&backup, &source)?;
        Ok(path.to_owned())
    })
    .unwrap();
    assert_ne!(
        std::fs::read_link(&alias).ok().as_ref(),
        Some(&source),
        "held source descriptor must close; another test may reuse its fd number"
    );
    assert!(
        with_verified_helper_path(&inv, id, |_| {
            std::fs::rename(&source, &backup)?;
            std::fs::write(&source, b"xyz")?;
            Ok(())
        })
        .is_err(),
        "persistent replacement rejected by postcheck"
    );
}

#[cfg(unix)]
#[test]
fn helper_rejects_source_and_ancestor_links_before_callback() {
    use std::os::unix::fs::symlink;
    let (dir, inv, id) = fixture();
    std::fs::rename(dir.path().join("sub/source"), dir.path().join("sub/real")).unwrap();
    symlink("real", dir.path().join("sub/source")).unwrap();
    assert!(
        with_verified_helper_path(&inv, id, |_| -> std::io::Result<()> {
            panic!("source link reached helper")
        })
        .is_err()
    );
    std::fs::remove_file(dir.path().join("sub/source")).unwrap();
    std::fs::rename(dir.path().join("sub/real"), dir.path().join("sub/source")).unwrap();
    std::fs::rename(dir.path().join("sub"), dir.path().join("real_sub")).unwrap();
    symlink("real_sub", dir.path().join("sub")).unwrap();
    assert!(
        with_verified_helper_path(&inv, id, |_| -> std::io::Result<()> {
            panic!("ancestor link reached helper")
        })
        .is_err()
    );
}

#[cfg(windows)]
#[test]
fn helper_pins_windows_source_and_ancestry_without_blocking_unrelated_children() {
    let (dir, inv, id) = fixture();
    let parent = dir.path().join("sub");
    let source = parent.join("source");
    with_verified_helper_path(&inv, id, |path| {
        assert_eq!(std::fs::read(path)?, b"abc");
        assert!(std::fs::write(&source, b"xyz").is_err());
        assert!(std::fs::rename(&source, parent.join("moved")).is_err());
        assert!(std::fs::rename(&parent, dir.path().join("moved_parent")).is_err());
        assert!(std::fs::rename(dir.path(), dir.path().with_extension("moved")).is_err());
        std::fs::write(parent.join("unrelated"), b"allowed")?;
        Ok(())
    })
    .unwrap();
    std::fs::write(&source, b"xyz").unwrap();
    std::fs::rename(&source, parent.join("moved")).unwrap();
    std::fs::rename(&parent, dir.path().join("moved_parent")).unwrap();
}
