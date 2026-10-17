//! Check schema identity before making any persistent change. Never import reports.
use super::{schema, OpError};
use crate::{ErrorCode, Phase};
use rusqlite::{Connection, TransactionBehavior};

pub(super) fn check(conn: &Connection) -> Result<bool, OpError> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(super::sql_error)?;
    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )
        .map_err(super::sql_error)?;
    if version == 0 && tables == 0 {
        return Ok(true);
    }
    let unsupported = || {
        OpError::new(
            ErrorCode::Unsupported,
            Phase::Read,
            "unknown or inconsistent operation journal schema; original retained",
        )
    };
    if version != schema::VERSION {
        return Err(unsupported());
    }
    let expected = format!("{:x}", md5::compute(schema::SQL));
    let migrations = conn
        .prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|_| unsupported())?;
    if migrations != vec![(schema::VERSION, expected)] {
        return Err(unsupported());
    }
    for table in [
        "operations",
        "steps",
        "receipts",
        "operation_events",
        "outbox_ack",
        "history_visibility",
    ] {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [table],
                |r| r.get(0),
            )
            .map_err(super::sql_error)?;
        if !exists {
            return Err(unsupported());
        }
    }
    Ok(false)
}
pub(super) fn migrate(conn: &mut Connection) -> Result<(), OpError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(super::sql_error)?;
    // Recheck under the writer lock: a concurrent initialization may have won.
    if check(&tx)? {
        tx.execute_batch(schema::SQL).map_err(super::sql_error)?;
        tx.execute(
            "INSERT INTO schema_migrations(version,checksum) VALUES(?1,?2)",
            rusqlite::params![schema::VERSION, format!("{:x}", md5::compute(schema::SQL))],
        )
        .map_err(super::sql_error)?;
        tx.pragma_update(None, "user_version", schema::VERSION)
            .map_err(super::sql_error)?;
    }
    tx.commit().map_err(super::sql_error)
}
