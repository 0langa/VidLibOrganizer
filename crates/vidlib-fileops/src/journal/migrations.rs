//! Check schema identity before making any persistent change. Never import reports.
use super::{schema, OpError};
use crate::{ErrorCode, Phase};
use rusqlite::{Connection, TransactionBehavior};

pub(super) fn check(conn: &Connection) -> Result<i64, OpError> {
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
        return Ok(0);
    }
    let unsupported = || {
        OpError::new(
            ErrorCode::Unsupported,
            Phase::Read,
            "unknown or inconsistent operation journal schema; original retained",
        )
    };
    if !(1..=schema::VERSION).contains(&version) {
        return Err(unsupported());
    }
    let mut expected = vec![(1, format!("{:x}", md5::compute(schema::SQL)))];
    if version >= 2 {
        expected.push((2, format!("{:x}", md5::compute(schema::V2_SQL))));
    }
    let migrations = conn
        .prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|_| unsupported())?;
    if migrations != expected {
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
    Ok(version)
}
pub(super) fn migrate(conn: &mut Connection) -> Result<(), OpError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(super::sql_error)?;
    // Recheck under the writer lock: a concurrent migration may have won.
    let version = check(&tx)?;
    if version == 0 {
        tx.execute_batch(schema::SQL).map_err(super::sql_error)?;
        tx.execute(
            "INSERT INTO schema_migrations(version,checksum) VALUES(1,?1)",
            [format!("{:x}", md5::compute(schema::SQL))],
        )
        .map_err(super::sql_error)?;
    }
    if version < 2 {
        tx.execute_batch(schema::V2_SQL).map_err(super::sql_error)?;
        // Validate extracted references before committing any schema/data change.
        // This is a one-time migration read, never a per-effect inventory decode.
        let mut statement = tx
            .prepare("SELECT approved_plan FROM operations")
            .map_err(super::sql_error)?;
        let rows = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(super::sql_error)?;
        for row in rows {
            super::from_json::<vidlib_rename::PlanRef>(&row.map_err(super::sql_error)?)?;
        }
        tx.execute(
            "INSERT INTO schema_migrations(version,checksum) VALUES(2,?1)",
            [format!("{:x}", md5::compute(schema::V2_SQL))],
        )
        .map_err(super::sql_error)?;
        tx.pragma_update(None, "user_version", schema::VERSION)
            .map_err(super::sql_error)?;
    }
    tx.commit().map_err(super::sql_error)
}
