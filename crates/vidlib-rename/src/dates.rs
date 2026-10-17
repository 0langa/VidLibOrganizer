//! Explicit, locale-independent dates. Evaluation never reads the clock or local zone.
use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DstAmbiguity {
    #[default]
    Reject,
    Earliest,
    Latest,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NonexistentTime {
    #[default]
    Reject,
    NextValid,
    PreviousValid,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DateRule {
    pub input_format: Option<String>,
    pub output_format: String,
    pub timezone: Option<FrozenTimezone>,
    pub assume_timezone: Option<FrozenTimezone>,
    pub ambiguity: DstAmbiguity,
    pub nonexistent: NonexistentTime,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DateOperation {
    Insert {
        source: DateSource,
        #[serde(with = "crate::numbers::exact_position")]
        at: InsertPosition,
        prefix: String,
        suffix: String,
        rule: DateRule,
    },
    Reformat {
        rule: DateRule,
    },
}

use chrono::{DateTime, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenEnvironment {
    #[serde(with = "vidlib_inventory::exact::i128_decimal")]
    pub now_ns: i128,
    pub timezone: FrozenTimezone,
    pub ambiguity: DstAmbiguity,
    pub nonexistent: NonexistentTime,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DateValue {
    Instant {
        #[serde(with = "vidlib_inventory::exact::i128_decimal")]
        ns: i128,
    },
    Local {
        value: NaiveDateTime,
        timezone: Option<FrozenTimezone>,
    },
    Date {
        value: NaiveDate,
    },
    Text {
        value: String,
    },
    Missing,
}
fn issue(code: RuleIssueCode, message: impl Into<String>) -> RuleIssue {
    RuleIssue {
        code,
        native_code: None,
        message: message.into(),
    }
}
fn invalid_date(message: impl Into<String>) -> RuleIssue {
    issue(RuleIssueCode::DateInvalid, message)
}
/// This is the only host local-zone lookup. Call explicitly during input
/// preparation and retain the returned IANA zone; evaluation never calls it.
pub fn resolve_local_timezone() -> Result<FrozenTimezone, RenameError> {
    let name = iana_time_zone::get_timezone()
        .map_err(|e| RenameError::new(RenameErrorCode::InvalidFrozenInputs, e.to_string()))?;
    let zone = FrozenTimezone::Iana { name };
    if !valid_timezone(&zone) {
        return Err(RenameError::new(
            RenameErrorCode::InvalidFrozenInputs,
            "local zone is not in the bundled IANA database",
        ));
    }
    Ok(zone)
}
impl FrozenEnvironment {
    /// `now_ns` must be captured once by the caller, alongside the explicit zone.
    pub fn freeze(
        now_ns: i128,
        timezone: FrozenTimezone,
        ambiguity: DstAmbiguity,
        nonexistent: NonexistentTime,
    ) -> Result<Self, RenameError> {
        if !valid_timezone(&timezone) || instant(now_ns).is_err() {
            return Err(RenameError::new(
                RenameErrorCode::InvalidFrozenInputs,
                "invalid frozen date environment",
            ));
        }
        Ok(Self {
            now_ns,
            timezone,
            ambiguity,
            nonexistent,
        })
    }
}
pub(crate) fn valid_timezone(zone: &FrozenTimezone) -> bool {
    match zone {
        FrozenTimezone::Utc => true,
        FrozenTimezone::FixedOffset { seconds } => FixedOffset::east_opt(*seconds).is_some(),
        FrozenTimezone::Iana { name } => name.parse::<chrono_tz::Tz>().is_ok(),
    }
}
pub(crate) fn validate_date_rule(rule: &DateRule) -> Result<(), RenameError> {
    if !crate::recipe::valid_date_pattern(&rule.output_format)
        || rule
            .input_format
            .as_ref()
            .is_some_and(|pattern| !crate::recipe::valid_date_pattern(pattern))
        || rule
            .timezone
            .as_ref()
            .is_some_and(|zone| !valid_timezone(zone))
        || rule
            .assume_timezone
            .as_ref()
            .is_some_and(|zone| !valid_timezone(zone))
    {
        return Err(crate::recipe::invalid(
            "invalid date format or unconfirmed timezone",
        ));
    }
    Ok(())
}
fn instant(ns: i128) -> Result<DateTime<Utc>, RuleIssue> {
    let seconds = i64::try_from(ns.div_euclid(1_000_000_000))
        .map_err(|_| invalid_date("timestamp is outside chrono range"))?;
    DateTime::from_timestamp(seconds, ns.rem_euclid(1_000_000_000) as u32)
        .ok_or_else(|| invalid_date("timestamp is outside chrono range"))
}
fn parse_text(text: &str, rule: &DateRule) -> Result<DateValue, RuleIssue> {
    let Some(pattern) = &rule.input_format else {
        // Only a complete RFC3339 timestamp is self-describing without a format.
        let value = DateTime::parse_from_rfc3339(text).map_err(|_| {
            issue(
                RuleIssueCode::DateNeedsFormat,
                "date text requires an explicit input format",
            )
        })?;
        return parsed_instant(value);
    };
    let fields = chrono::format::StrftimeItems::new(pattern).collect::<Vec<_>>();
    if fields
        .iter()
        .any(|item| matches!(item, chrono::format::Item::Fixed(_)))
    {
        return parsed_instant(
            DateTime::parse_from_str(text, pattern).map_err(|e| invalid_date(e.to_string()))?,
        );
    }
    if fields.iter().any(|item| {
        matches!(
            item,
            chrono::format::Item::Numeric(
                chrono::format::Numeric::Hour
                    | chrono::format::Numeric::Minute
                    | chrono::format::Numeric::Second
                    | chrono::format::Numeric::Nanosecond,
                _
            )
        )
    }) {
        Ok(DateValue::Local {
            value: NaiveDateTime::parse_from_str(text, pattern)
                .map_err(|e| invalid_date(e.to_string()))?,
            timezone: None,
        })
    } else {
        Ok(DateValue::Date {
            value: NaiveDate::parse_from_str(text, pattern)
                .map_err(|e| invalid_date(e.to_string()))?,
        })
    }
}
fn parsed_instant(value: DateTime<FixedOffset>) -> Result<DateValue, RuleIssue> {
    if value.timestamp_subsec_nanos() >= 1_000_000_000 {
        return Err(invalid_date(
            "leap seconds cannot be represented as epoch nanoseconds",
        ));
    }
    Ok(DateValue::Instant {
        ns: i128::from(value.timestamp()) * 1_000_000_000
            + i128::from(value.timestamp_subsec_nanos()),
    })
}

fn choose<T: TimeZone>(
    result: LocalResult<DateTime<T>>,
    policy: DstAmbiguity,
) -> Result<DateTime<Utc>, RuleIssue> {
    match result {
        LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(a, b) => match policy {
            DstAmbiguity::Reject => Err(issue(
                RuleIssueCode::DateAmbiguous,
                "local time repeats; select earliest or latest explicitly",
            )),
            DstAmbiguity::Earliest => Ok(a.with_timezone(&Utc).min(b.with_timezone(&Utc))),
            DstAmbiguity::Latest => Ok(a.with_timezone(&Utc).max(b.with_timezone(&Utc))),
        },
        LocalResult::None => Err(issue(
            RuleIssueCode::DateNonexistent,
            "local time does not exist in the selected timezone",
        )),
    }
}
fn local_in_zone<T: TimeZone>(
    value: &NaiveDateTime,
    zone: &T,
    env: &FrozenEnvironment,
) -> Result<DateTime<Utc>, RuleIssue> {
    if value.and_utc().timestamp_subsec_nanos() >= 1_000_000_000 {
        return Err(invalid_date(
            "leap seconds cannot be represented as epoch nanoseconds",
        ));
    }
    let result = zone.from_local_datetime(value);
    if !matches!(result, LocalResult::None) || env.nonexistent == NonexistentTime::Reject {
        return choose(result, env.ambiguity);
    }
    // Explicit nearest valid wall-clock second, preserving subsecond precision.
    // The bound also covers IANA transitions skipping a complete calendar day.
    let direction = if env.nonexistent == NonexistentTime::NextValid {
        1
    } else {
        -1
    };
    for seconds in 1..=172_800 {
        let candidate = value
            .checked_add_signed(chrono::Duration::seconds(direction * seconds))
            .ok_or_else(|| invalid_date("local resolution overflow"))?;
        let result = zone.from_local_datetime(&candidate);
        if !matches!(result, LocalResult::None) {
            return choose(result, env.ambiguity);
        }
    }
    Err(issue(
        RuleIssueCode::DateNonexistent,
        "no valid local time within 48 hours",
    ))
}
fn local_to_instant(
    value: &NaiveDateTime,
    zone: &FrozenTimezone,
    env: &FrozenEnvironment,
) -> Result<DateTime<Utc>, RuleIssue> {
    match zone {
        FrozenTimezone::Utc => local_in_zone(value, &Utc, env),
        FrozenTimezone::FixedOffset { seconds } => local_in_zone(
            value,
            &FixedOffset::east_opt(*seconds).ok_or_else(|| invalid_date("invalid fixed offset"))?,
            env,
        ),
        FrozenTimezone::Iana { name } => local_in_zone(
            value,
            &name
                .parse::<chrono_tz::Tz>()
                .map_err(|_| invalid_date("unknown IANA timezone"))?,
            env,
        ),
    }
}
/// Format through the pinned chrono subset. Unknown local/recording zones never
/// acquire the environment zone implicitly; `assume_timezone` is a user choice.
pub fn format_date(
    value: &DateValue,
    rule: &DateRule,
    env: &FrozenEnvironment,
) -> Result<String, RuleIssue> {
    validate_date_rule(rule).map_err(|e| invalid_date(e.message))?;
    if !valid_timezone(&env.timezone) {
        return Err(invalid_date("invalid frozen timezone"));
    }
    if let DateValue::Text { value } = value {
        return format_date(&parse_text(value, rule)?, rule, env);
    }
    let mut output = String::new();
    if let DateValue::Date { value } = value {
        write!(&mut output, "{}", value.format(&rule.output_format))
            .map_err(|_| invalid_date("date-only value cannot supply requested time or offset"))?;
        return Ok(output);
    }
    let value = match value {
        DateValue::Instant { ns } => instant(*ns)?,
        DateValue::Local { value, timezone } => {
            let zone = timezone
                .as_ref()
                .or(rule.assume_timezone.as_ref())
                .ok_or_else(|| {
                    issue(
                        RuleIssueCode::DateTimezoneUnknown,
                        "recording/local timezone is unknown; select an explicit assumption",
                    )
                })?;
            local_to_instant(value, zone, env)?
        }
        DateValue::Missing => {
            return Err(issue(
                RuleIssueCode::DateMissing,
                "requested date is absent",
            ))
        }
        _ => unreachable!("text and date handled above"),
    };
    let zone = rule.timezone.as_ref().unwrap_or(&env.timezone);
    match zone {
        FrozenTimezone::Utc => write!(&mut output, "{}", value.format(&rule.output_format)),
        FrozenTimezone::FixedOffset { seconds } => write!(
            &mut output,
            "{}",
            value
                .with_timezone(
                    &FixedOffset::east_opt(*seconds)
                        .ok_or_else(|| invalid_date("invalid offset"))?
                )
                .format(&rule.output_format)
        ),
        FrozenTimezone::Iana { name } => write!(
            &mut output,
            "{}",
            value
                .with_timezone(
                    &name
                        .parse::<chrono_tz::Tz>()
                        .map_err(|_| invalid_date("unknown IANA timezone"))?
                )
                .format(&rule.output_format)
        ),
    }
    .map_err(|_| invalid_date("date cannot supply requested format"))?;
    Ok(output)
}

pub(crate) fn apply_date(
    input: &str,
    operation: &DateOperation,
    entry: &vidlib_inventory::EntrySnapshot,
    rule_id: &str,
    inputs: &FrozenInputs,
) -> Result<String, RuleIssue> {
    let (rule, source) = match operation {
        DateOperation::Reformat { rule } => (
            rule,
            DateValue::Text {
                value: input.into(),
            },
        ),
        DateOperation::Insert { rule, source, .. } => {
            (rule, date_source(entry, rule_id, *source, inputs)?)
        }
    };
    let env = FrozenEnvironment {
        now_ns: inputs.now_ns,
        timezone: inputs.timezone.clone(),
        ambiguity: rule.ambiguity,
        nonexistent: rule.nonexistent,
    };
    let formatted = format_date(&source, rule, &env)?;
    match operation {
        DateOperation::Reformat { .. } => Ok(formatted),
        DateOperation::Insert {
            at, prefix, suffix, ..
        } => crate::numbers::insert_value(input, at, format!("{prefix}{formatted}{suffix}")),
    }
}
fn date_source(
    entry: &vidlib_inventory::EntrySnapshot,
    rule_id: &str,
    source: DateSource,
    inputs: &FrozenInputs,
) -> Result<DateValue, RuleIssue> {
    if source == DateSource::Now {
        return Ok(DateValue::Instant { ns: inputs.now_ns });
    }
    if source == DateSource::Recorded {
        let frozen = inputs
            .values
            .iter()
            .find(|value| {
                value.entry_id == entry.id
                    && value.rule_id.as_deref() == Some(rule_id)
                    && value.key == FieldKey::Date { source }
            })
            .or_else(|| {
                inputs.values.iter().find(|value| {
                    value.entry_id == entry.id
                        && value.rule_id.is_none()
                        && value.key == FieldKey::Date { source }
                })
            });
        return match frozen.map(|value| &value.value) {
            Some(FrozenFieldValue::Timestamp {
                ns,
                timezone: Some(_),
            }) => Ok(DateValue::Instant { ns: *ns }),
            Some(FrozenFieldValue::Timestamp { ns, timezone: None }) => Ok(DateValue::Local {
                value: instant(*ns)?.naive_utc(),
                timezone: None,
            }),
            Some(FrozenFieldValue::LocalTimestamp { value, timezone }) => Ok(DateValue::Local {
                value: *value,
                timezone: timezone.clone(),
            }),
            _ => Ok(DateValue::Missing),
        };
    }
    Ok(match source {
        DateSource::Created => entry.stamp.created_ns,
        DateSource::Modified => entry.stamp.modified_ns,
        DateSource::Accessed => entry.stamp.accessed_ns,
        _ => unreachable!("recorded and now handled above"),
    }
    .map_or(DateValue::Missing, |ns| DateValue::Instant { ns }))
}
