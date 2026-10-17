//! Every child opens the same app-data directory and independently opens the OS lock.
use std::process::Command;
use vidlib_fileops::{
    coordinator::{Coordinator, LeaseKind},
    ErrorCode,
};

fn child(path: &std::path::Path, mode: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "subprocess_fixture", "--nocapture"])
        .env("VIDLIB_COORDINATOR_FIXTURE", path)
        .env("VIDLIB_COORDINATOR_MODE", mode)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn subprocess_fixture() {
    let Some(path) = std::env::var_os("VIDLIB_COORDINATOR_FIXTURE") else {
        return;
    };
    let c = Coordinator::open(std::path::Path::new(&path)).unwrap();
    match std::env::var("VIDLIB_COORDINATOR_MODE").unwrap().as_str() {
        "busy" => assert_eq!(
            c.try_acquire(LeaseKind::Write).unwrap_err().code,
            ErrorCode::Busy
        ),
        "interrupt" => {
            let lease = c.try_acquire(LeaseKind::Write).unwrap();
            let _mutation = lease.begin_legacy_mutation().unwrap();
            std::fs::write(
                std::path::Path::new(&path).join("legacy-effect"),
                b"changed",
            )
            .unwrap();
            std::process::exit(0); // Deliberately bypass RAII to simulate abrupt process exit.
        }
        "bump" => {
            c.try_acquire(LeaseKind::Write)
                .unwrap()
                .advance_namespace()
                .unwrap();
        }
        _ => panic!("unknown fixture mode"),
    }
}

// Dropping one reader must not unlock another reader's separately opened handle.
#[test]
fn readers_exclude_writer() {
    let d = tempfile::tempdir().unwrap();
    let c = Coordinator::open(d.path()).unwrap();
    let a = c.try_acquire(LeaseKind::Read).unwrap();
    let b = c.try_acquire(LeaseKind::Read).unwrap();
    assert_eq!(
        c.try_acquire(LeaseKind::Write).unwrap_err().code,
        ErrorCode::Busy
    );
    drop(a);
    child(d.path(), "busy");
    drop(b);
    assert!(c.try_acquire(LeaseKind::Write).is_ok());
}

#[test]
fn writer_excludes_second_process() {
    let d = tempfile::tempdir().unwrap();
    let c = Coordinator::open(d.path()).unwrap();
    let _writer = c.try_acquire(LeaseKind::Write).unwrap();
    child(d.path(), "busy");
    assert_eq!(
        c.try_acquire(LeaseKind::Read).unwrap_err().code,
        ErrorCode::Busy
    );
    assert_eq!(
        c.try_acquire(LeaseKind::Recovery).unwrap_err().code,
        ErrorCode::Busy
    );
}

#[test]
fn panic_releases_lease() {
    let d = tempfile::tempdir().unwrap();
    let c = Coordinator::open(d.path()).unwrap();
    let _ = std::panic::catch_unwind(|| {
        let _lease = c.try_acquire(LeaseKind::Write).unwrap();
        assert!(c.has_active_writer());
        child(d.path(), "busy");
        panic!("fixture unwind");
    });
    assert!(c.try_acquire(LeaseKind::Write).is_ok());
    assert!(!c.has_active_writer());
    child(d.path(), "bump");
}

// The process restart gate must query persistent operation state, not a cached flag.
#[test]
fn startup_pending_journal_blocks_writes() {
    let d = tempfile::tempdir().unwrap();
    let c = Coordinator::open(d.path()).unwrap();
    let db = rusqlite::Connection::open(d.path().join("operations.sqlite3")).unwrap();
    db.execute("INSERT INTO operations(id,state,inspection,groups_json,approved_plan) VALUES('pending','Prepared','{}','[]','{}')", []).unwrap();
    let restarted = Coordinator::open(d.path()).unwrap();
    for coordinator in [&c, &restarted] {
        assert_eq!(
            coordinator.try_acquire(LeaseKind::Write).unwrap_err().code,
            ErrorCode::RecoveryRequired
        );
        assert_eq!(
            coordinator.try_acquire(LeaseKind::Read).unwrap_err().code,
            ErrorCode::RecoveryRequired
        );
        assert!(coordinator.try_acquire(LeaseKind::Recovery).is_ok());
    }
}

// A mutation in another process must revoke a previously captured plan epoch.
#[test]
fn other_process_epoch_revokes_old_plan() {
    let d = tempfile::tempdir().unwrap();
    let c = Coordinator::open(d.path()).unwrap();
    let captured = c.try_acquire(LeaseKind::Read).unwrap().epochs().unwrap();
    child(d.path(), "bump");
    let writer = c.try_acquire(LeaseKind::Write).unwrap();
    assert_eq!(
        writer.verify_epochs(captured).unwrap_err().code,
        ErrorCode::StaleState
    );
    let current = writer.epochs().unwrap();
    assert_eq!(current.namespace, 1);
    assert_eq!(current.policy, 0);
    writer.advance_policy().unwrap();
    assert_eq!(writer.epochs().unwrap().policy, 1);
    assert_eq!(
        writer.verify_epochs(current).unwrap_err().code,
        ErrorCode::StaleState
    );
}

// Read authority may never change epochs.
#[test]
fn read_lease_cannot_advance_epochs() {
    let d = tempfile::tempdir().unwrap();
    let c = Coordinator::open(d.path()).unwrap();
    let reader = c.try_acquire(LeaseKind::Read).unwrap();
    assert_eq!(
        reader.advance_namespace().unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        reader.advance_policy().unwrap_err().code,
        ErrorCode::PermissionDenied
    );
}

#[test]
fn relative_app_data_is_rejected() {
    assert_eq!(
        Coordinator::open(std::path::Path::new("relative"))
            .unwrap_err()
            .code,
        ErrorCode::InvalidPath
    );
}

// An abandoned legacy batch is uncertainty, never permission for another mutation.
#[test]
fn interrupted_legacy_mutation_survives_restart() {
    let d = tempfile::tempdir().unwrap();
    let c = Coordinator::open(d.path()).unwrap();
    let _ = std::panic::catch_unwind(|| {
        let lease = c.try_acquire(LeaseKind::Write).unwrap();
        let _mutation = lease.begin_legacy_mutation().unwrap();
        std::fs::write(d.path().join("changed"), b"effect before panic").unwrap();
        panic!("interrupted batch");
    });
    let restarted = Coordinator::open(d.path()).unwrap();
    assert_eq!(
        restarted.try_acquire(LeaseKind::Write).unwrap_err().code,
        ErrorCode::RecoveryRequired
    );
    assert!(restarted.try_acquire(LeaseKind::Recovery).is_ok());
}

#[test]
fn known_legacy_outcome_advances_epoch_and_clears_marker() {
    let d = tempfile::tempdir().unwrap();
    let c = Coordinator::open(d.path()).unwrap();
    {
        let lease = c.try_acquire(LeaseKind::Write).unwrap();
        lease.begin_legacy_mutation().unwrap().finish().unwrap();
    }
    assert_eq!(
        c.try_acquire(LeaseKind::Read)
            .unwrap()
            .epochs()
            .unwrap()
            .namespace,
        1
    );
}

#[test]
fn failed_epoch_commit_keeps_recovery_gate() {
    let d = tempfile::tempdir().unwrap();
    let c = Coordinator::open(d.path()).unwrap();
    {
        let lease = c.try_acquire(LeaseKind::Write).unwrap();
        let mutation = lease.begin_legacy_mutation().unwrap();
        let db = rusqlite::Connection::open(c.database_path()).unwrap();
        db.execute_batch("CREATE TABLE commit_parent(id INTEGER PRIMARY KEY); CREATE TABLE commit_child(id INTEGER REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER fail_epoch AFTER UPDATE OF namespace_epoch ON coordination WHEN NEW.namespace_epoch <> OLD.namespace_epoch BEGIN INSERT INTO commit_child VALUES(9); END;").unwrap();
        assert!(mutation.finish().is_err());
    }
    assert_eq!(
        c.try_acquire(LeaseKind::Write).unwrap_err().code,
        ErrorCode::RecoveryRequired
    );
}

#[cfg(unix)]
#[test]
fn managed_store_links_are_rejected() {
    use std::os::unix::fs::symlink;
    for name in ["operations.lock", "operations.sqlite3", "blueprint.sqlite3"] {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("external");
        std::fs::write(&target, b"external bytes").unwrap();
        symlink(&target, dir.path().join(name)).unwrap();
        assert_eq!(
            Coordinator::open(dir.path()).unwrap_err().code,
            ErrorCode::ReparsePoint,
            "{name}"
        );
        assert_eq!(std::fs::read(target).unwrap(), b"external bytes");
    }
}

// OS locks must be released by process death, while durable uncertainty survives it.
#[test]
fn abrupt_process_exit_releases_lock_but_retains_legacy_gate() {
    let dir = tempfile::tempdir().unwrap();
    let coordinator = Coordinator::open(dir.path()).unwrap();
    child(dir.path(), "interrupt");
    assert_eq!(
        std::fs::read(dir.path().join("legacy-effect")).unwrap(),
        b"changed"
    );
    assert_eq!(
        coordinator.try_acquire(LeaseKind::Write).unwrap_err().code,
        ErrorCode::RecoveryRequired
    );
    let recovery = coordinator.try_acquire(LeaseKind::Recovery).unwrap();
    let status = recovery.status().unwrap();
    assert!(status.interrupted_legacy_write);
    assert!(!status.pending_journal);
    assert!(status.requires_recovery());
    assert_eq!(status.epochs.namespace, 0);
    assert!(recovery.begin_legacy_mutation().is_err());
}
