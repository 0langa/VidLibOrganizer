use crate::{RenameError, RenameErrorCode};
use serde::{Deserialize, Serialize};
use vidlib_inventory::ids::{EntryId, GrantId};

/// Versioned portable configuration. Native paths never occur in recipes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeV1 {
    #[serde(deserialize_with = "deserialize_version")]
    pub schema_version: u32,
    #[serde(with = "vidlib_inventory::exact::u64_decimal")]
    pub revision: u64,
    pub selection: Vec<EntryId>,
    #[serde(default)]
    pub selection_filter: Option<crate::SelectionFilter>,
    /// Explicit primary vector; independent of display sorting.
    pub numbering_order: Vec<EntryId>,
    #[serde(default)]
    pub numbering_scope: crate::NumberingScope,
    pub rules: Vec<Rule>,
    pub companions: CompanionPolicy,
    pub destination: Destination,
    pub collision_policy: CollisionPolicy,
    pub metadata_changes: Vec<MetadataChange>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub enabled: bool,
    pub segment: SegmentSelector,
    pub parameters: RuleParameters,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuleParameters {
    Keep,
    FixedStem {
        value: String,
    },
    Template {
        parts: Vec<TemplatePart>,
    },
    Text {
        operation: crate::TextOperation,
    },
    Regex {
        rule: crate::RegexRule,
    },
    Number {
        rule: crate::NumberRule,
    },
    PadNumbers {
        #[serde(with = "vidlib_inventory::exact::u64_decimal")]
        padding: u64,
        scope: crate::MatchScope,
    },
    Date {
        operation: crate::DateOperation,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SegmentSelector {
    Stem,
    Extension,
    WholeName,
    /// One-based extended graphemes; negative from end, inclusive endpoints.
    Graphemes {
        target: NameSegment,
        start: i32,
        end: i32,
    },
    /// Flat, left-to-right literal pairs; excludes both delimiters unless requested.
    Between {
        target: NameSegment,
        opening: String,
        closing: String,
        occurrence: u32,
        include_delimiters: bool,
    },
    /// Select the token after the Nth delimiter, ending before the next.
    /// Literal, left-to-right, non-overlapping, one-based occurrences.
    Delimited {
        target: NameSegment,
        delimiter: String,
        occurrence: u32,
        include_delimiter: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameSegment {
    Stem,
    Extension,
    WholeName,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TemplatePart {
    Literal(String),
    Field {
        key: FieldKey,
        format: ValueFormat,
        fallback: Option<String>,
    },
    Optional(Vec<TemplatePart>),
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldKey {
    OriginalStem,
    CurrentStem,
    Extension,
    Parent { level: u32 },
    RelativeDirectory,
    Counter,
    Date { source: DateSource },
    SizeBytes,
    Media { property: MediaProperty },
    WindowsProperty { property: WindowsProperty },
    Hash { algorithm: HashAlgorithm },
    Uuid,
    Random,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateSource {
    Created,
    Modified,
    Accessed,
    Recorded,
    Now,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaProperty {
    Width,
    Height,
    DurationNs,
    FrameRate,
    VideoCodec,
    AudioCodec,
    AudioChannels,
    BitRate,
    Container,
    Title,
    Language,
}
/// Canonical Property System names; arbitrary lookup/coercion is not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WindowsProperty {
    #[serde(rename = "System.Author")]
    Author,
    #[serde(rename = "System.Title")]
    Title,
    #[serde(rename = "System.Media.DateEncoded")]
    DateEncoded,
    #[serde(rename = "System.Photo.DateTaken")]
    DateTaken,
}
impl WindowsProperty {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::Author => "System.Author",
            Self::Title => "System.Title",
            Self::DateEncoded => "System.Media.DateEncoded",
            Self::DateTaken => "System.Photo.DateTaken",
        }
    }
    pub fn is_date(self) -> bool {
        matches!(self, Self::DateEncoded | Self::DateTaken)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashAlgorithm {
    Crc32,
    Md5,
    Sha1,
    Sha512,
    Sha256,
    Blake3,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueFormat {
    Text,
    Integer { radix: Radix, padding: u32 },
    Decimal { places: u32 },
    Date { pattern: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Radix {
    Decimal,
    HexLower,
    HexUpper,
    AlphabeticUpper,
    AlphabeticLower,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompanionPolicy {
    None,
    Explicit {
        associations: Vec<CompanionAssociation>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionAssociation {
    pub primary: EntryId,
    pub companions: Vec<EntryId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Destination {
    SameDirectory,
    Granted {
        grant_id: GrantId,
        layout: Vec<TemplatePart>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollisionPolicy {
    Block,
    Suffix {
        separator: String,
        max_attempts: u32,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetadataChange {
    Timestamp {
        field: TimestampField,
        #[serde(with = "vidlib_inventory::exact::i128_decimal")]
        value_ns: i128,
    },
    WindowsAttributes {
        set: u32,
        clear: u32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampField {
    Created,
    Modified,
    Accessed,
}

impl RecipeV1 {
    /// Bounded import; schema/version validation happens before evaluation.
    pub fn from_json(bytes: &[u8]) -> Result<Self, RenameError> {
        if bytes.len() > 1024 * 1024 {
            return Err(RenameError::new(
                RenameErrorCode::RecipeTooLarge,
                "recipe exceeds 1 MiB",
            ));
        }
        let envelope: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
        if envelope
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .is_some_and(|v| v != 1)
        {
            return Err(RenameError::new(
                RenameErrorCode::UnknownRecipeVersion,
                "expected recipe version 1",
            ));
        }
        let recipe: Self = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
        recipe.validate()?;
        Ok(recipe)
    }

    pub fn validate(&self) -> Result<(), RenameError> {
        self.validate_and_compile_regex().map(|_| ())
    }

    /// Compilation is shared with evaluation so enabled expressions compile once
    /// per evaluation, before any rows, rather than again during validation.
    pub(crate) fn validate_and_compile_regex(
        &self,
    ) -> Result<std::collections::BTreeMap<&str, crate::CompiledRegexRule>, RenameError> {
        self.validate_structure()?;
        self.rules
            .iter()
            .filter(|rule| rule.enabled)
            .filter_map(|rule| {
                if let RuleParameters::Regex { rule: parameters } = &rule.parameters {
                    Some(
                        crate::CompiledRegexRule::compile(parameters)
                            .map(|compiled| (rule.id.as_str(), compiled))
                            .map_err(|mut error| {
                                error.message = format!("rule {}: {}", rule.id, error.message);
                                error
                            }),
                    )
                } else {
                    None
                }
            })
            .collect()
    }

    fn validate_structure(&self) -> Result<(), RenameError> {
        use std::collections::BTreeSet;
        if self.schema_version != 1 {
            return Err(RenameError::new(
                RenameErrorCode::UnknownRecipeVersion,
                "expected recipe version 1",
            ));
        }
        if self.rules.len() > 100 {
            return Err(RenameError::new(
                RenameErrorCode::TooManyRules,
                "recipe exceeds 100 rules",
            ));
        }
        if let Some(filter) = &self.selection_filter {
            filter.validate()?;
        }
        let selected: BTreeSet<_> = self.selection.iter().copied().collect();
        if selected.len() != self.selection.len() {
            return Err(invalid("duplicate selected entry"));
        }
        let mut companions = BTreeSet::new();
        if let CompanionPolicy::Explicit { associations } = &self.companions {
            let mut primaries = BTreeSet::new();
            for association in associations {
                if !selected.contains(&association.primary)
                    || !primaries.insert(association.primary)
                {
                    return Err(invalid("invalid or duplicated companion primary"));
                }
                for id in &association.companions {
                    if !selected.contains(id)
                        || *id == association.primary
                        || !companions.insert(*id)
                    {
                        return Err(invalid("invalid or duplicated companion ownership"));
                    }
                }
            }
            if !primaries.is_disjoint(&companions) {
                return Err(invalid("a companion cannot own companions"));
            }
        }
        let primary_set: BTreeSet<_> = selected.difference(&companions).copied().collect();
        let order: BTreeSet<_> = self.numbering_order.iter().copied().collect();
        if order.len() != self.numbering_order.len() || order != primary_set {
            return Err(RenameError::new(
                RenameErrorCode::InvalidNumberingOrder,
                "numbering order must list every primary exactly once",
            ));
        }
        let mut ids = BTreeSet::new();
        for rule in &self.rules {
            if rule.id.is_empty() || !ids.insert(&rule.id) {
                return Err(invalid("empty or duplicate rule ID"));
            }
            match &rule.segment {
                SegmentSelector::Delimited {
                    delimiter,
                    occurrence,
                    ..
                } if delimiter.is_empty() || *occurrence == 0 => {
                    return Err(invalid(
                        "delimiter must be nonempty and occurrence one-based",
                    ))
                }
                _ => {}
            }
            match &rule.parameters {
                RuleParameters::FixedStem { value } => {
                    if rule.segment != SegmentSelector::Stem || !valid_component(value) {
                        return Err(invalid(
                            "fixed stem requires a valid stem and the stem segment",
                        ));
                    }
                }
                RuleParameters::Template { parts } => validate_template(parts, 0)?,
                RuleParameters::Text { operation } => {
                    crate::text::validate_operation(operation).map_err(|e| invalid(e.message))?
                }
                RuleParameters::Number { rule } => crate::numbers::validate_number_rule(rule)?,
                RuleParameters::PadNumbers { padding, .. } if *padding > 4096 => {
                    return Err(invalid("padding exceeds 4096"))
                }
                RuleParameters::Date { operation } => {
                    let (crate::DateOperation::Insert { rule, .. }
                    | crate::DateOperation::Reformat { rule }) = operation;
                    crate::dates::validate_date_rule(rule)?;
                }
                RuleParameters::Keep
                | RuleParameters::Regex { .. }
                | RuleParameters::PadNumbers { .. } => {}
            }
        }
        if let Destination::Granted { layout, .. } = &self.destination {
            validate_template(layout, 0)?;
        }
        if let CollisionPolicy::Suffix {
            separator,
            max_attempts,
        } = &self.collision_policy
        {
            if *max_attempts == 0 || separator.contains(['/', '\\', '\0']) {
                return Err(invalid("invalid collision suffix policy"));
            }
        }
        let mut timestamps = BTreeSet::new();
        let mut attributes = false;
        for change in &self.metadata_changes {
            match change {
                MetadataChange::WindowsAttributes { set, clear }
                    if set & clear != 0 || attributes =>
                {
                    return Err(invalid("conflicting attribute changes"))
                }
                MetadataChange::WindowsAttributes { .. } => attributes = true,
                MetadataChange::Timestamp { field, .. } if !timestamps.insert(*field as u8) => {
                    return Err(invalid("duplicate timestamp change"))
                }
                _ => {}
            }
        }
        Ok(())
    }
}
pub(crate) fn invalid(message: impl Into<String>) -> RenameError {
    RenameError::new(RenameErrorCode::InvalidRecipe, message)
}
pub(crate) fn valid_component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', '\0'])
}

pub(crate) fn validate_template(parts: &[TemplatePart], depth: usize) -> Result<(), RenameError> {
    if depth > 32 {
        return Err(invalid("template nesting exceeds 32"));
    }
    for part in parts {
        match part {
            TemplatePart::Literal(_) => {}
            TemplatePart::Optional(inner) => validate_template(inner, depth + 1)?,
            TemplatePart::Field { key, format, .. } => {
                if matches!(key, FieldKey::Parent { level: 0 }) {
                    return Err(invalid("parent level must be positive"));
                }
                let numeric = matches!(
                    key,
                    FieldKey::Counter
                        | FieldKey::SizeBytes
                        | FieldKey::Random
                        | FieldKey::Media {
                            property: MediaProperty::Width
                                | MediaProperty::Height
                                | MediaProperty::DurationNs
                                | MediaProperty::AudioChannels
                                | MediaProperty::BitRate
                        }
                );
                let valid = match format {
                    ValueFormat::Text => !key.is_date(),
                    ValueFormat::Integer { padding, .. } => numeric && *padding <= 4096,
                    ValueFormat::Decimal { places } => {
                        matches!(
                            key,
                            FieldKey::Media {
                                property: MediaProperty::FrameRate
                            }
                        ) && *places <= 18
                    }
                    ValueFormat::Date { pattern } => key.is_date() && valid_date_pattern(pattern),
                };
                if !valid {
                    return Err(invalid(
                        "field and format are incompatible or format is invalid",
                    ));
                }
            }
        }
    }
    Ok(())
}
/// Pinned, locale-independent strftime subset: %Y %m %d %H %M %S %f %z %:z
/// and %%. Locale names, composite formats and arbitrary modifiers are excluded.
pub(crate) fn valid_date_pattern(pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.next() {
                Some('Y' | 'm' | 'd' | 'H' | 'M' | 'S' | 'f' | 'z' | '%') => {}
                Some(':') if chars.next() == Some('z') => {}
                _ => return false,
            }
        }
    }
    true
}

fn deserialize_version<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(d)?;
    if version == 1 {
        Ok(version)
    } else {
        Err(serde::de::Error::custom("unknown recipe version"))
    }
}

impl FieldKey {
    pub fn is_date(&self) -> bool {
        matches!(self, Self::Date { .. })
            || matches!(self,Self::WindowsProperty { property } if property.is_date())
    }
}
