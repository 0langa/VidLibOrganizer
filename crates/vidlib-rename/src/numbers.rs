//! Checked counters assigned only from an explicit frozen primary vector.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use vidlib_inventory::ids::EntryId;

pub type NumberingOrder = Vec<EntryId>;
pub type NumberAssignments = BTreeMap<EntryId, String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NumberRule {
    #[serde(with = "vidlib_inventory::exact::i128_decimal")]
    pub start: i128,
    #[serde(with = "vidlib_inventory::exact::i128_decimal")]
    pub step: i128,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub repeat_each: u64,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub padding: u64,
    pub radix: Radix,
    pub reset_by_folder: bool,
    #[serde(with = "exact_position")]
    pub at: InsertPosition,
    pub prefix: String,
    pub suffix: String,
}

pub(crate) fn validate_number_rule(rule: &NumberRule) -> Result<(), RenameError> {
    if rule.repeat_each == 0 || rule.padding > 4096 {
        return Err(crate::recipe::invalid(
            "repeat_each must be positive; padding is limited to 4096",
        ));
    }
    Ok(())
}

/// `entries` may have any display order. Only `order` consumes ordinals. The
/// caller supplies primaries after companion ownership has been resolved.
pub fn assign_numbers(
    entries: &[vidlib_inventory::EntrySnapshot],
    order: &NumberingOrder,
    rule: &NumberRule,
) -> Result<NumberAssignments, RenameError> {
    validate_number_rule(rule)?;
    let inventory_len = entries.len();
    let entries: BTreeMap<_, _> = entries.iter().map(|entry| (entry.id, entry)).collect();
    if entries.len() != inventory_len {
        return Err(RenameError::new(
            RenameErrorCode::InvalidInventory,
            "duplicate numbering inventory entry",
        ));
    }
    let mut assigned = BTreeMap::new();
    let mut folder_ordinals = BTreeMap::new();
    for (ordinal, id) in order.iter().enumerate() {
        let entry = entries.get(id).ok_or_else(|| {
            RenameError::new(RenameErrorCode::UnknownEntry, "numbering entry is absent")
        })?;
        if assigned.contains_key(id) {
            return Err(RenameError::new(
                RenameErrorCode::InvalidNumberingOrder,
                "duplicate primary in numbering order",
            ));
        }
        let ordinal = if rule.reset_by_folder {
            let next = folder_ordinals
                .entry(entry.absolute_path.parent())
                .or_insert(0_i128);
            let ordinal = *next;
            *next = next.checked_add(1).ok_or_else(overflow)?;
            ordinal
        } else {
            i128::try_from(ordinal).map_err(|_| overflow())?
        };
        let value = (ordinal / i128::from(rule.repeat_each))
            .checked_mul(rule.step)
            .and_then(|step| rule.start.checked_add(step))
            .ok_or_else(overflow)?;
        let formatted = format_number(value, rule.radix, rule.padding)
            .map_err(|issue| RenameError::new(RenameErrorCode::InvalidRuleInput, issue.message))?;
        assigned.insert(*id, formatted);
    }
    Ok(assigned)
}
fn overflow() -> RenameError {
    RenameError::new(
        RenameErrorCode::ArithmeticOverflow,
        "counter arithmetic overflow",
    )
}
fn issue(message: &str) -> RuleIssue {
    RuleIssue {
        code: RuleIssueCode::InvalidInput,
        native_code: None,
        message: message.into(),
    }
}
/// Padding belongs to the magnitude, after a possible minus sign. Alphabetic
/// counters use spreadsheet digits and have no zero digit or numeric padding.
pub fn format_number(value: i128, radix: Radix, padding: u64) -> Result<String, RuleIssue> {
    if padding > 4096 {
        return Err(issue("padding exceeds 4096"));
    }
    let magnitude = value.unsigned_abs();
    let digits = match radix {
        Radix::Decimal => magnitude.to_string(),
        Radix::HexLower => format!("{magnitude:x}"),
        Radix::HexUpper => format!("{magnitude:X}"),
        Radix::AlphabeticUpper | Radix::AlphabeticLower => {
            if value <= 0 {
                return Err(issue("alphabetic counters must be positive"));
            }
            let mut remainder = magnitude;
            let mut digits = Vec::new();
            let base = if radix == Radix::AlphabeticUpper {
                b'A'
            } else {
                b'a'
            };
            while remainder != 0 {
                remainder -= 1;
                digits.push((base + (remainder % 26) as u8) as char);
                remainder /= 26;
            }
            return Ok(digits.into_iter().rev().collect());
        }
    };
    Ok(format!(
        "{}{:0>width$}",
        if value < 0 { "-" } else { "" },
        digits,
        width = padding as usize
    ))
}
/// Pad ASCII decimal runs without parsing them, preserving arbitrarily large
/// existing numbers, signs, surrounding text and leading zeroes.
pub fn pad_existing_numbers(
    input: &str,
    padding: u64,
    scope: MatchScope,
) -> Result<String, RuleIssue> {
    if padding > 4096 {
        return Err(issue("padding exceeds 4096"));
    }
    let mut result = String::new();
    let mut rest = input;
    let mut matched = false;
    while let Some(start) = rest.find(|c: char| c.is_ascii_digit()) {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if !matched || scope == MatchScope::All {
            result.extend(std::iter::repeat_n(
                '0',
                (padding as usize).saturating_sub(end),
            ));
        }
        result.push_str(&rest[..end]);
        rest = &rest[end..];
        matched = true;
        if scope == MatchScope::First {
            break;
        }
    }
    result.push_str(rest);
    Ok(result)
}

pub(crate) fn insert_value(
    input: &str,
    at: &InsertPosition,
    value: String,
) -> Result<String, RuleIssue> {
    // Reuse the same Unicode grapheme insertion semantics as text rules.
    crate::text::transform(
        input,
        &TextOperation::Insert {
            at: at.clone(),
            text: value,
        },
    )
}

/// Existing text-rule indices retain their schema. Newly introduced number/date
/// insertion fields use canonical decimal strings even for bounded indices.
pub(crate) mod exact_position {
    use crate::InsertPosition;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    #[derive(Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum Position {
        Before {
            #[serde(with = "super::exact_i32")]
            position: i32,
        },
        End,
    }
    pub fn serialize<S: Serializer>(value: &InsertPosition, s: S) -> Result<S::Ok, S::Error> {
        match value {
            InsertPosition::End => Position::End,
            InsertPosition::Before { position } => Position::Before {
                position: *position,
            },
        }
        .serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<InsertPosition, D::Error> {
        Ok(match Position::deserialize(d)? {
            Position::End => InsertPosition::End,
            Position::Before { position } => InsertPosition::Before { position },
        })
    }
}
pub(crate) mod exact_i32 {
    use serde::{Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &i32, s: S) -> Result<S::Ok, S::Error> {
        vidlib_inventory::exact::i128_decimal::serialize(&i128::from(*value), s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
        i32::try_from(vidlib_inventory::exact::i128_decimal::deserialize(d)?)
            .map_err(serde::de::Error::custom)
    }
}
