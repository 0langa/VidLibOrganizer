//! Unicode text operations. Positions are one-based extended graphemes.
use crate::{recipe::valid_component, NameSegment, SegmentSelector};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Eq)]
/// Separate filename components; a private separator flag preserves trailing dots.
pub struct NameParts {
    pub stem: String,
    pub ext: String,
    extension_present: bool,
}
impl NameParts {
    pub fn from_filename(name: &str) -> Self {
        // This is component text, never a path. Preserve every byte so validation
        // can report invalid components rather than discarding a prefix/parent.
        match name.rfind('.').filter(|index| *index > 0) {
            Some(index) => Self {
                stem: name[..index].into(),
                ext: name[index + 1..].into(),
                extension_present: true,
            },
            None => Self::stem_only(name),
        }
    }

    pub fn stem_only(stem: &str) -> Self {
        Self {
            stem: stem.into(),
            ext: String::new(),
            extension_present: false,
        }
    }
    pub fn filename(&self) -> String {
        if self.extension_present || !self.ext.is_empty() {
            format!("{}.{}", self.stem, self.ext)
        } else {
            self.stem.clone()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRule {
    pub segment: SegmentSelector,
    pub operation: TextOperation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleIssueCode {
    InvalidInput,
    RegexLimit,
    RegexNative,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleIssue {
    pub code: RuleIssueCode,
    pub native_code: Option<i32>,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
/// Before a one-based logical grapheme (negative from end), or explicitly append.
pub enum InsertPosition {
    Before { position: i32 },
    End,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchScope {
    First,
    All,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CropSide {
    Before,
    After,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseMode {
    Keep,
    Upper,
    Lower,
    Title,
    Sentence,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RomanCase {
    Keep,
    Upper,
    Lower,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteralReplacement {
    pub from: String,
    pub to: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterTranslation {
    pub from: char,
    pub to: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextOperation {
    Keep,
    Fixed {
        value: String,
    },
    Remove,
    Prefix {
        text: String,
    },
    Suffix {
        text: String,
    },
    Insert {
        at: InsertPosition,
        text: String,
    },
    /// Entries execute in order; each entry matches left-to-right without overlap.
    Replace {
        replacements: Vec<LiteralReplacement>,
        scope: MatchScope,
        case_sensitive: bool,
    },
    RemoveCharacters {
        characters: String,
    },
    RemoveRange {
        start: i32,
        end: i32,
    },
    Crop {
        delimiter: String,
        occurrence: u32,
        side: CropSide,
        include_delimiter: bool,
    },
    /// Source and destination refer to the selected text before this operation.
    /// A move inside its source range is an issue; copying there is supported.
    MoveCopy {
        start: i32,
        end: i32,
        destination: InsertPosition,
        copy: bool,
    },
    Reverse,
    Trim,
    CollapseWhitespace {
        replacement: String,
    },
    /// Simultaneous scalar-character translation; inserted text is not translated again.
    Translate {
        translations: Vec<CharacterTranslation>,
    },
    /// Explicit opt-in, locale-independent transliteration via deunicode.
    Transliterate,
    /// Exceptions restore whole-word spelling before optional canonical ASCII Roman handling.
    /// None disables Roman handling; it is independent of the base case mode.
    Case {
        mode: CaseMode,
        exceptions: Vec<String>,
        roman: Option<RomanCase>,
    },
}

fn issue(message: &str) -> RuleIssue {
    RuleIssue {
        code: RuleIssueCode::InvalidInput,
        native_code: None,
        message: message.into(),
    }
}
impl std::fmt::Display for RuleIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for RuleIssue {}

impl SegmentSelector {
    pub fn target(&self) -> NameSegment {
        match self {
            Self::Stem => NameSegment::Stem,
            Self::Extension => NameSegment::Extension,
            Self::WholeName => NameSegment::WholeName,
            Self::Graphemes { target, .. }
            | Self::Delimited { target, .. }
            | Self::Between { target, .. } => *target,
        }
    }
}
impl NameParts {
    pub fn segment(&self, target: NameSegment) -> String {
        match target {
            NameSegment::Stem => self.stem.clone(),
            NameSegment::Extension => self.ext.clone(),
            NameSegment::WholeName => self.filename(),
        }
    }
}

/// Transform only the selector's current input. A failed rule leaves input untouched.
/// Removing/fixing to an empty extension removes its separator. Unchanged output
/// preserves spelling (including a trailing dot). WholeName explicitly re-splits it.
pub fn apply_text(input: &NameParts, rule: &TextRule) -> Result<NameParts, RuleIssue> {
    validate_operation(&rule.operation)?;
    apply_segment(
        input,
        &rule.segment,
        matches!(
            rule.operation,
            TextOperation::Remove | TextOperation::Fixed { .. }
        ),
        |selected| transform(selected, &rule.operation),
    )
}

/// Shared current-input selection, reassembly and component validation.
pub(crate) fn apply_segment(
    input: &NameParts,
    selector: &SegmentSelector,
    clear_empty_extension: bool,
    transform: impl FnOnce(&str) -> Result<String, RuleIssue>,
) -> Result<NameParts, RuleIssue> {
    let target = selector.target();
    let current = input.segment(target);
    let range = resolve_segment(&current, selector)?;
    let replacement = transform(&current[range.clone()])?;
    let mut transformed = current;
    transformed.replace_range(range, &replacement);
    let mut output = input.clone();
    match target {
        NameSegment::Stem => output.stem = transformed,
        NameSegment::Extension => {
            if transformed != input.ext || clear_empty_extension {
                output.extension_present = !transformed.is_empty();
            }
            output.ext = transformed;
        }
        NameSegment::WholeName => {
            if !valid_component(&transformed) {
                return Err(issue("rule produced an invalid filename component"));
            }
            output = NameParts::from_filename(&transformed);
        }
    }
    if !valid_component(&output.filename()) {
        return Err(issue("rule produced an invalid filename component"));
    }
    Ok(output)
}

fn boundaries(current: &str) -> Vec<usize> {
    current
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(current.len()))
        .collect()
}
fn index(position: i32, count: usize) -> Result<usize, RuleIssue> {
    let count = i64::try_from(count).map_err(|_| issue("name is too long"))?;
    let index = if position > 0 {
        i64::from(position) - 1
    } else {
        count + i64::from(position)
    };
    if position == 0 || index < 0 || index >= count {
        return Err(issue("grapheme position is zero or out of range"));
    }
    Ok(index as usize)
}
fn grapheme_range(current: &str, start: i32, end: i32) -> Result<Range<usize>, RuleIssue> {
    let bounds = boundaries(current);
    let start = index(start, bounds.len() - 1)?;
    let end = index(end, bounds.len() - 1)?;
    if start > end {
        return Err(issue("grapheme range is reversed"));
    }
    Ok(bounds[start]..bounds[end + 1])
}
fn literal_matches(current: &str, delimiter: &str) -> Result<Vec<Range<usize>>, RuleIssue> {
    if delimiter.is_empty() {
        return Err(issue("delimiter must not be empty"));
    }
    let bounds = boundaries(current);
    current
        .match_indices(delimiter)
        .map(|(start, found)| {
            let end = start + found.len();
            if bounds.binary_search(&start).is_err() || bounds.binary_search(&end).is_err() {
                Err(issue("delimiter splits a grapheme"))
            } else {
                Ok(start..end)
            }
        })
        .collect()
}
fn occurrence(matches: &[Range<usize>], occurrence: u32) -> Result<Range<usize>, RuleIssue> {
    occurrence
        .checked_sub(1)
        .and_then(|i| matches.get(i as usize))
        .cloned()
        .ok_or_else(|| issue("delimiter occurrence is zero or absent"))
}

/// Returns checked zero-based, half-open UTF-8 byte offsets into `current`.
/// `current` is already the selected stem, extension or whole filename.
/// Delimited selects after the Nth delimiter through the next delimiter/end.
/// Between selects flat, left-to-right, non-overlapping opening/closing pairs.
pub fn resolve_segment(
    current: &str,
    selector: &SegmentSelector,
) -> Result<Range<usize>, RuleIssue> {
    match selector {
        SegmentSelector::Stem | SegmentSelector::Extension | SegmentSelector::WholeName => {
            Ok(0..current.len())
        }
        SegmentSelector::Graphemes { start, end, .. } => grapheme_range(current, *start, *end),
        SegmentSelector::Delimited {
            delimiter,
            occurrence: n,
            include_delimiter,
            ..
        } => {
            let matches = literal_matches(current, delimiter)?;
            let found = occurrence(&matches, *n)?;
            let end = matches
                .get(*n as usize)
                .map_or(current.len(), |next| next.start);
            Ok((if *include_delimiter {
                found.start
            } else {
                found.end
            })..end)
        }
        SegmentSelector::Between {
            opening,
            closing,
            occurrence: n,
            include_delimiters,
            ..
        } => {
            if *n == 0 {
                return Err(issue("pair occurrence must be one-based"));
            }
            let openings = literal_matches(current, opening)?;
            let closings = literal_matches(current, closing)?;
            let mut cursor = 0;
            for pair in 1..=*n {
                let open = openings
                    .iter()
                    .find(|r| r.start >= cursor)
                    .ok_or_else(|| issue("opening delimiter is absent"))?;
                let close = closings
                    .iter()
                    .find(|r| r.start >= open.end)
                    .ok_or_else(|| issue("closing delimiter is absent"))?;
                if pair == *n {
                    return Ok(if *include_delimiters {
                        open.start..close.end
                    } else {
                        open.end..close.start
                    });
                }
                cursor = close.end;
            }
            Err(issue("pair occurrence is absent"))
        }
    }
}

pub(crate) fn validate_operation(operation: &TextOperation) -> Result<(), RuleIssue> {
    use std::collections::BTreeSet;
    match operation {
        TextOperation::Replace { replacements, .. }
            if replacements.iter().any(|r| r.from.is_empty()) =>
        {
            return Err(issue("replacement source must not be empty"))
        }
        TextOperation::Crop {
            delimiter,
            occurrence,
            ..
        } if delimiter.is_empty() || *occurrence == 0 => {
            return Err(issue(
                "crop needs a nonempty delimiter and one-based occurrence",
            ))
        }
        TextOperation::Translate { translations } => {
            let mut seen = BTreeSet::new();
            if translations.iter().any(|t| !seen.insert(t.from)) {
                return Err(issue("duplicate character translation"));
            }
        }
        TextOperation::Case { exceptions, .. } => {
            let mut seen = BTreeSet::new();
            for value in exceptions {
                if value.unicode_words().collect::<Vec<_>>() != [value.as_str()]
                    || !seen.insert(fold(value))
                {
                    return Err(issue("case exceptions must be distinct whole words"));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn insertion(current: &str, at: &InsertPosition) -> Result<usize, RuleIssue> {
    match at {
        InsertPosition::End => Ok(current.len()),
        InsertPosition::Before { position } => {
            let bounds = boundaries(current);
            Ok(bounds[index(*position, bounds.len() - 1)?])
        }
    }
}
fn transform(current: &str, operation: &TextOperation) -> Result<String, RuleIssue> {
    use TextOperation::*;
    Ok(match operation {
        Keep => current.into(),
        Fixed { value } => value.clone(),
        Remove => String::new(),
        Prefix { text } => format!("{text}{current}"),
        Suffix { text } => format!("{current}{text}"),
        Insert { at, text } => {
            let mut value = current.to_owned();
            value.insert_str(insertion(current, at)?, text);
            value
        }
        Replace {
            replacements,
            scope,
            case_sensitive,
        } => {
            let mut value = current.to_owned();
            for replacement in replacements {
                value = replace_literal(&value, replacement, *scope, *case_sensitive);
            }
            value
        }
        RemoveCharacters { characters } => current
            .chars()
            .filter(|c| !characters.contains(*c))
            .collect(),
        RemoveRange { start, end } => {
            let mut value = current.to_owned();
            value.replace_range(grapheme_range(current, *start, *end)?, "");
            value
        }
        Crop {
            delimiter,
            occurrence: n,
            side,
            include_delimiter,
        } => {
            let found = occurrence(&literal_matches(current, delimiter)?, *n)?;
            match side {
                CropSide::Before => current[..if *include_delimiter {
                    found.end
                } else {
                    found.start
                }]
                    .into(),
                CropSide::After => current[if *include_delimiter {
                    found.start
                } else {
                    found.end
                }..]
                    .into(),
            }
        }
        MoveCopy {
            start,
            end,
            destination,
            copy,
        } => {
            let range = grapheme_range(current, *start, *end)?;
            let mut dest = insertion(current, destination)?;
            let selected = &current[range.clone()];
            let mut value = current.to_owned();
            if !copy {
                if dest > range.start && dest < range.end {
                    return Err(issue("move destination overlaps the source"));
                }
                if dest >= range.end {
                    dest -= range.len();
                }
                value.replace_range(range, "");
            }
            value.insert_str(dest, selected);
            value
        }
        Reverse => current.graphemes(true).rev().collect(),
        Trim => current.trim().into(),
        CollapseWhitespace { replacement } => {
            let mut value = String::new();
            let mut whitespace = false;
            for c in current.chars() {
                if c.is_whitespace() {
                    if !whitespace {
                        value.push_str(replacement);
                    }
                    whitespace = true;
                } else {
                    whitespace = false;
                    value.push(c);
                }
            }
            value
        }
        Translate { translations } => {
            let mut value = String::new();
            for c in current.chars() {
                if let Some(t) = translations.iter().find(|t| t.from == c) {
                    value.push_str(&t.to);
                } else {
                    value.push(c);
                }
            }
            value
        }
        Transliterate => deunicode::deunicode(current),
        Case {
            mode,
            exceptions,
            roman,
        } => convert_case(current, *mode, exceptions, *roman),
    })
}

fn fold(value: &str) -> String {
    caseless::default_case_fold_str(value)
}
fn replace_literal(
    current: &str,
    replacement: &LiteralReplacement,
    scope: MatchScope,
    case_sensitive: bool,
) -> String {
    let bounds = boundaries(current);
    let needle = if case_sensitive {
        replacement.from.clone()
    } else {
        fold(&replacement.from)
    };
    let mut output = String::new();
    let mut cursor = 0;
    let mut replaced = false;
    for (i, start) in bounds.iter().copied().enumerate().take(bounds.len() - 1) {
        if start < cursor || (replaced && scope == MatchScope::First) {
            continue;
        }
        let mut found = None;
        for end in bounds[i + 1..].iter().copied() {
            let candidate = &current[start..end];
            let candidate = if case_sensitive {
                candidate.to_owned()
            } else {
                fold(candidate)
            };
            if candidate == needle {
                found = Some(end);
                break;
            }
            if candidate.len() > needle.len() {
                break;
            }
        }
        if let Some(end) = found {
            output.push_str(&current[cursor..start]);
            output.push_str(&replacement.to);
            cursor = end;
            replaced = true;
        }
    }
    output.push_str(&current[cursor..]);
    output
}

fn title_first(value: &str) -> String {
    let mut output = String::new();
    let mut changed = false;
    for c in value.chars() {
        if !changed
            && (c.is_lowercase()
                || c.is_uppercase()
                || unicode_case_mapping::to_titlecase(c) != [0; 3])
        {
            let mapped = unicode_case_mapping::to_titlecase(c);
            if mapped == [0; 3] {
                output.push(c);
            } else {
                output.extend(mapped.into_iter().filter_map(|cp| {
                    if cp == 0 {
                        None
                    } else {
                        char::from_u32(cp)
                    }
                }));
            }
            changed = true;
        } else {
            output.push(c);
        }
    }
    output
}
fn convert_case(
    current: &str,
    mode: CaseMode,
    exceptions: &[String],
    roman: Option<RomanCase>,
) -> String {
    // Restore exceptions/Roman from the original words even when full case mappings expand.
    let mut converted = String::new();
    let mut sentence_start = true;
    for sentence in current.split_sentence_bounds() {
        for part in sentence.split_word_bounds() {
            let is_word = part.unicode_words().next().is_some();
            if !is_word {
                converted.push_str(part);
                continue;
            }
            let word = if let Some(exception) = exceptions.iter().find(|e| fold(e) == fold(part)) {
                exception.clone()
            } else if let Some(roman_mode) = roman.filter(|_| is_roman(part)) {
                match roman_mode {
                    RomanCase::Keep => part.into(),
                    RomanCase::Upper => part.to_uppercase(),
                    RomanCase::Lower => part.to_lowercase(),
                }
            } else {
                match mode {
                    CaseMode::Keep => part.into(),
                    CaseMode::Upper => part.to_uppercase(),
                    CaseMode::Lower => part.to_lowercase(),
                    CaseMode::Title => title_first(&part.to_lowercase()),
                    CaseMode::Sentence if sentence_start => title_first(&part.to_lowercase()),
                    CaseMode::Sentence => part.to_lowercase(),
                }
            };
            // Numeric/uncased words do not consume sentence capitalization.
            // Unicode Lt letters are neither Lowercase nor Uppercase, but have
            // a nonidentity lowercase mapping and must consume it as well.
            if part.chars().any(|c| {
                c.is_lowercase()
                    || c.is_uppercase()
                    || unicode_case_mapping::to_lowercase(c) != [0; 2]
            }) {
                sentence_start = false;
            }
            converted.push_str(&word);
        }
        sentence_start = true;
    }
    converted
}
fn is_roman(word: &str) -> bool {
    let word = word.to_ascii_uppercase();
    if word.is_empty() || !word.bytes().all(|c| b"IVXLCDM".contains(&c)) {
        return false;
    }
    // Canonical ASCII numerals 1..3999; avoid rewriting words such as MIXED or invalid IIV.
    let tokens = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut rest = word.as_str();
    let mut value = 0;
    for (number, token) in tokens {
        let mut count = 0;
        while rest.starts_with(token) {
            count += 1;
            value += number;
            rest = &rest[token.len()..];
        }
        if count
            > if [1000, 100, 10, 1].contains(&number) {
                3
            } else {
                1
            }
        {
            return false;
        }
    }
    if !rest.is_empty() || value > 3999 {
        return false;
    }
    let mut canonical = String::new();
    for (number, token) in tokens {
        while value >= number {
            canonical.push_str(token);
            value -= number;
        }
    }
    canonical == word
}
