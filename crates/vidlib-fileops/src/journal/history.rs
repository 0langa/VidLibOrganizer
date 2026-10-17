//! Live bounded metadata queries. Keyset ranges and data revisions are separate:
//! inserts cannot shift a retained range; changed facts require an explicit refresh.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistorySort {
    Oldest,
    Newest,
    OperationId,
    State,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryQueryCursor {
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    after: u64,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    through: u64,
    key: String,
    sort: HistorySort,
    filter: String,
}
pub struct HistoryQuery {
    pub cursor: Option<HistoryQueryCursor>,
    pub limit: usize,
    pub sort: HistorySort,
    pub filter: String,
    pub expected_revision: u64,
}
#[derive(Debug)]
pub struct HistoryQueryPage {
    pub items: Vec<HistoryItem>,
    pub next_cursor: Option<HistoryQueryCursor>,
    pub revision: u64,
}
pub(super) fn read_revision(connection: &Connection) -> Result<u64, OpError> {
    let text: String = connection
        .query_row(
            "SELECT revision FROM history_revision WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let revision = text
        .parse::<u64>()
        .map_err(|_| invalid("invalid history data revision"))?;
    if revision.to_string() != text {
        return Err(invalid("noncanonical history data revision"));
    }
    Ok(revision)
}
pub(super) fn advance_revision(connection: &Connection) -> Result<(), OpError> {
    let next = read_revision(connection)?
        .checked_add(1)
        .ok_or_else(|| recovery("history data revision exhausted"))?;
    connection
        .execute(
            "UPDATE history_revision SET revision=?1 WHERE singleton=1",
            [next.to_string()],
        )
        .map_err(sql_error)?;
    Ok(())
}
// Reused factual eligibility semantics; no inspection/compiled paths are loaded.
const COLUMNS: &str = "o.rowid,o.id,o.state,i.original_operation,(SELECT COUNT(*) FROM steps s WHERE s.operation_id=o.id AND s.state='CompletionDurable' AND NOT EXISTS(SELECT 1 FROM recovery_effects e WHERE e.operation_id=o.id AND e.inverse_step=s.id)), NOT EXISTS(SELECT 1 FROM steps u WHERE u.operation_id=o.id AND u.state IN ('IntentDurable','Unresolved')) AND NOT EXISTS(SELECT 1 FROM operation_inverses ai JOIN operations ao ON ao.id=ai.operation_id WHERE ai.original_operation=o.id AND (ao.state IN ('Prepared','Running','RecoveryRequired') OR EXISTS(SELECT 1 FROM steps ast WHERE ast.operation_id=ao.id AND ast.state='CompletionDurable' AND NOT EXISTS(SELECT 1 FROM recovery_effects ae WHERE ae.operation_id=ao.id AND ae.inverse_step=ast.id))))";
impl Journal {
    pub fn history_revision(&self) -> Result<u64, OpError> {
        read_revision(&self.reader()?)
    }
    pub fn history_query(&self, query: HistoryQuery) -> Result<HistoryQueryPage, OpError> {
        if query.filter.len() > 4096 {
            return Err(invalid("history filter exceeds limit"));
        }
        let mut reader = self.reader()?;
        let tx = reader.transaction().map_err(sql_error)?;
        let revision = read_revision(&tx)?;
        if revision != query.expected_revision {
            return Err(OpError::new(
                ErrorCode::StaleState,
                Phase::Read,
                "History changed. Refresh the first page.",
            ));
        }
        let high_water: i64 = tx
            .query_row("SELECT COALESCE(MAX(rowid),0) FROM operations", [], |r| {
                r.get(0)
            })
            .map_err(sql_error)?;
        let through = query
            .cursor
            .as_ref()
            .map(|c| c.through)
            .unwrap_or(high_water as u64);
        let after = query.cursor.as_ref().map(|c| c.after).unwrap_or(0);
        let key = query.cursor.as_ref().map(|c| c.key.as_str()).unwrap_or("");
        if high_water < 0
            || through > high_water as u64
            || after > through
            || key.len() > 128
            || query
                .cursor
                .as_ref()
                .is_some_and(|c| c.sort != query.sort || c.filter != query.filter)
        {
            return Err(invalid("history cursor does not match query/range"));
        }
        let (condition, order) = match query.sort {
            HistorySort::Oldest => ("o.rowid>?4", "o.rowid ASC"),
            HistorySort::Newest => ("o.rowid<?4", "o.rowid DESC"),
            HistorySort::OperationId => (
                "(o.id>?5 OR (o.id=?5 AND o.rowid>?4))",
                "o.id ASC,o.rowid ASC",
            ),
            HistorySort::State => (
                "(o.state>?5 OR (o.state=?5 AND o.rowid>?4))",
                "o.state ASC,o.rowid ASC",
            ),
        };
        let sql=format!("SELECT {COLUMNS} FROM operations o JOIN history_visibility h ON h.operation_id=o.id AND h.visible=1 LEFT JOIN operation_inverses i ON i.operation_id=o.id WHERE o.rowid<=?1 AND instr(lower(o.id||' '||o.state||' '||CASE o.state WHEN 'RecoveryRequired' THEN 'recovery required' ELSE '' END||' '||COALESCE(i.original_operation,'')),?2)>0 AND (?3=0 OR {condition}) ORDER BY {order} LIMIT ?6");
        let limit = query.limit.clamp(1, 200);
        let values = {
            let mut stmt = tx.prepare(&sql).map_err(sql_error)?;
            let rows = stmt
                .query_map(
                    params![
                        through as i64,
                        query.filter.to_lowercase(),
                        query.cursor.is_some(),
                        after as i64,
                        key,
                        (limit + 1) as i64
                    ],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)? as u64,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, i64>(4)? as u64,
                            r.get::<_, bool>(5)?,
                        ))
                    },
                )
                .map_err(sql_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)?
        };
        let next_cursor = (values.len() > limit).then(|| {
            let (after, id, state, ..) = &values[limit - 1];
            HistoryQueryCursor {
                after: *after,
                through,
                key: match query.sort {
                    HistorySort::State => state.clone(),
                    HistorySort::OperationId => id.clone(),
                    _ => after.to_string(),
                },
                sort: query.sort,
                filter: query.filter.clone(),
            }
        });
        let items = values
            .into_iter()
            .take(limit)
            .map(|(_, id, state, inverse, completed_steps, eligible)| {
                Ok(HistoryItem {
                    operation_id: parse_id(&id)?,
                    state: from_json(&format!("\"{state}\""))?,
                    inverse_of: inverse.map(|id| parse_id(&id)).transpose()?,
                    completed_steps,
                    undo_available: eligible
                        && completed_steps > 0
                        && matches!(state.as_str(), "Completed" | "Cancelled" | "Failed"),
                })
            })
            .collect::<Result<Vec<_>, OpError>>()?;
        tx.commit().map_err(sql_error)?;
        Ok(HistoryQueryPage {
            items,
            next_cursor,
            revision,
        })
    }
}
