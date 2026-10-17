//! Typed template rendering over immutable observations; no I/O or clock access.
use crate::*;
use vidlib_inventory::{decode_path, EntrySnapshot};

pub struct TemplateContext<'a> {
    pub scope: FrozenScope,
    pub entry: &'a EntrySnapshot,
    pub original: &'a NameParts,
    pub current: &'a NameParts,
    pub rule_id: &'a str,
    pub inputs: &'a FrozenInputs,
}
fn issue(code: RuleIssueCode, message: impl Into<String>) -> RuleIssue {
    RuleIssue {
        code,
        message: message.into(),
        native_code: None,
    }
}
fn missing(key: &FieldKey) -> RuleIssue {
    issue(
        RuleIssueCode::MetadataMissing,
        format!("required field is missing: {key:?}"),
    )
}
pub fn render_template(
    parts: &[TemplatePart],
    context: &TemplateContext<'_>,
) -> Result<String, RuleIssue> {
    crate::recipe::validate_template(parts, 0)
        .map_err(|e| issue(RuleIssueCode::InvalidInput, e.message))?;
    render(parts, context)
}
fn render(parts: &[TemplatePart], context: &TemplateContext<'_>) -> Result<String, RuleIssue> {
    let mut output = String::new();
    for part in parts {
        match part {
            TemplatePart::Literal(text) => output.push_str(text),
            TemplatePart::Optional(parts) => match render(parts, context) {
                Ok(text) => output.push_str(&text),
                Err(error) if error.code == RuleIssueCode::MetadataMissing => {}
                Err(error) => return Err(error),
            },
            TemplatePart::Field {
                key,
                format,
                fallback,
            } => {
                let raw = value(key, context)?;
                let value = raw.resolved();
                match value {
                    FrozenFieldValue::Missing {
                        decision: MissingDecision::Fallback(text),
                    } => output.push_str(&text),
                    FrozenFieldValue::Missing { .. } => {
                        if let Some(text) = fallback {
                            output.push_str(text);
                        } else {
                            return Err(match &raw {
                                FrozenFieldValue::Metadata(value) => value.missing_issue(),
                                _ => missing(key),
                            });
                        }
                    }
                    value => output.push_str(&formatted(key, &value, format, context)?),
                }
            }
        }
    }
    Ok(output)
}
fn frozen_value<'a>(
    inputs: &'a FrozenInputs,
    entry: &EntrySnapshot,
    rule_id: &str,
    key: &FieldKey,
    scope: FrozenScope,
) -> Option<&'a FrozenFieldValue> {
    inputs
        .values
        .iter()
        .find(|v| {
            v.scope == scope
                && v.entry_id == entry.id
                && v.rule_id.as_deref() == Some(rule_id)
                && &v.key == key
        })
        .or_else(|| {
            inputs.values.iter().find(|v| {
                v.entry_id == entry.id
                    && v.rule_id.is_none()
                    && &v.key == key
                    && (v.scope == scope || matches!(key, FieldKey::Hash { .. }))
            })
        })
        .map(|v| &v.value)
}
fn value(key: &FieldKey, context: &TemplateContext<'_>) -> Result<FrozenFieldValue, RuleIssue> {
    let text = |s: &str| FrozenFieldValue::Text(s.to_owned());
    Ok(match key {
        FieldKey::OriginalStem => text(&context.original.stem),
        FieldKey::CurrentStem => text(&context.current.stem),
        FieldKey::Extension => text(&context.current.ext),
        FieldKey::Parent { level } => context
            .entry
            .absolute_path
            .ancestors()
            .nth(*level as usize)
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map(text)
            .unwrap_or_else(absent),
        FieldKey::RelativeDirectory => {
            let path = decode_path(&context.entry.relative_path)
                .map_err(|e| issue(RuleIssueCode::InvalidInput, e.to_string()))?;
            path.parent()
                .and_then(|p| p.to_str())
                .map(text)
                .unwrap_or_else(absent)
        }
        FieldKey::SizeBytes => context
            .entry
            .stamp
            .size
            .map(|v| FrozenFieldValue::Integer(i128::from(v)))
            .unwrap_or_else(absent),
        FieldKey::Date {
            source: DateSource::Now,
        } => FrozenFieldValue::Timestamp {
            ns: context.inputs.now_ns,
            timezone: Some(context.inputs.timezone.clone()),
        },
        FieldKey::Date {
            source: DateSource::Created | DateSource::Modified | DateSource::Accessed,
        } => {
            let ns = match key {
                FieldKey::Date {
                    source: DateSource::Created,
                } => context.entry.stamp.created_ns,
                FieldKey::Date {
                    source: DateSource::Modified,
                } => context.entry.stamp.modified_ns,
                _ => context.entry.stamp.accessed_ns,
            };
            ns.map(|ns| FrozenFieldValue::Timestamp {
                ns,
                timezone: Some(FrozenTimezone::Utc),
            })
            .unwrap_or_else(absent)
        }
        _ => frozen_value(
            context.inputs,
            context.entry,
            context.rule_id,
            key,
            context.scope,
        )
        .cloned()
        .unwrap_or_else(absent),
    })
}
fn absent() -> FrozenFieldValue {
    FrozenFieldValue::Missing {
        decision: MissingDecision::Block,
    }
}
fn formatted(
    key: &FieldKey,
    value: &FrozenFieldValue,
    format: &ValueFormat,
    context: &TemplateContext<'_>,
) -> Result<String, RuleIssue> {
    match (value, format) {
        (FrozenFieldValue::Text(text), ValueFormat::Text) => Ok(text.clone()),
        (FrozenFieldValue::TextList(text), ValueFormat::Text) => Ok(text.join("; ")),
        (FrozenFieldValue::Integer(value), ValueFormat::Text) => Ok(value.to_string()),
        (FrozenFieldValue::Integer(value), ValueFormat::Integer { radix, padding }) => {
            format_number(*value, *radix, u64::from(*padding))
        }
        (FrozenFieldValue::Decimal { coefficient, scale }, ValueFormat::Decimal { places }) => {
            decimal(*coefficient, *scale, *places)
        }
        (FrozenFieldValue::Decimal { coefficient, scale }, ValueFormat::Text) => {
            decimal(*coefficient, *scale, *scale)
        }
        (_, ValueFormat::Date { pattern }) => {
            let date = if let FieldKey::Date { source } = key {
                crate::dates::date_source_scoped(
                    context.entry,
                    context.rule_id,
                    *source,
                    context.inputs,
                    context.scope,
                )?
            } else if key.is_date() {
                match value {
                    FrozenFieldValue::Timestamp { ns, .. } => DateValue::Instant { ns: *ns },
                    FrozenFieldValue::LocalTimestamp { value, timezone } => DateValue::Local {
                        value: *value,
                        timezone: timezone.clone(),
                    },
                    _ => DateValue::Missing,
                }
            } else {
                return Err(issue(
                    RuleIssueCode::InvalidInput,
                    "date format requires a date field",
                ));
            };
            format_date(
                &date,
                &DateRule {
                    input_format: None,
                    output_format: pattern.clone(),
                    timezone: None,
                    assume_timezone: None,
                    ambiguity: DstAmbiguity::Reject,
                    nonexistent: NonexistentTime::Reject,
                },
                &FrozenEnvironment {
                    now_ns: context.inputs.now_ns,
                    timezone: context.inputs.timezone.clone(),
                    ambiguity: DstAmbiguity::Reject,
                    nonexistent: NonexistentTime::Reject,
                },
            )
        }
        _ => Err(issue(
            RuleIssueCode::InvalidInput,
            "field value and formatter are incompatible",
        )),
    }
}
/// Exact decimal formatting, rounded half away from zero; never passes through f64.
fn decimal(coefficient: i128, scale: u32, places: u32) -> Result<String, RuleIssue> {
    if scale > 18 || places > 18 {
        return Err(issue(
            RuleIssueCode::InvalidInput,
            "decimal precision exceeds 18",
        ));
    }
    let mut magnitude = coefficient.unsigned_abs();
    if scale > places {
        let divisor = 10u128.pow(scale - places);
        magnitude = magnitude / divisor + u128::from(magnitude % divisor >= divisor / 2);
    }
    let mut digits = magnitude.to_string();
    if places > scale {
        digits.extend(std::iter::repeat_n('0', (places - scale) as usize));
    }
    if places > 0 {
        let places = places as usize;
        if digits.len() <= places {
            digits = format!("{}{}", "0".repeat(places + 1 - digits.len()), digits);
        }
        digits.insert(digits.len() - places, '.');
    }
    if coefficient < 0 && magnitude != 0 {
        digits.insert(0, '-');
    }
    Ok(digits)
}
