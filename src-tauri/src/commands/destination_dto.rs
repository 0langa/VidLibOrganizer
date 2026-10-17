//! Strict command wire types. Portable recipe serialization stays independent.
use serde::Deserialize;
use vidlib_rename::{self as core, DateSource, HashAlgorithm, WindowsProperty};

#[derive(Clone, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum TemplatePartDto {
    Literal(String),
    Field {
        key: FieldKeyDto,
        format: ValueFormatDto,
        fallback: Option<String>,
    },
    Optional(Vec<TemplatePartDto>),
}
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum FieldKeyDto {
    OriginalStem {},
    CurrentStem {},
    Extension {},
    Parent { level: u32 },
    RelativeDirectory {},
    Counter {},
    Date { source: DateSource },
    SizeBytes {},
    Media { property: MediaPropertyDto },
    WindowsProperty { property: WindowsProperty },
    Hash { algorithm: HashAlgorithm },
    Uuid {},
    Random {},
}
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ValueFormatDto {
    Text {},
    Integer { radix: RadixDto, padding: u32 },
    Decimal { places: u32 },
    Date { pattern: String },
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RadixDto {
    Decimal,
    HexLower,
    HexUpper,
    AlphabeticUpper,
    AlphabeticLower,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaPropertyDto {
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
impl From<TemplatePartDto> for core::TemplatePart {
    fn from(value: TemplatePartDto) -> Self {
        match value {
            TemplatePartDto::Literal(text) => Self::Literal(text),
            TemplatePartDto::Field {
                key,
                format,
                fallback,
            } => Self::Field {
                key: key.into(),
                format: format.into(),
                fallback,
            },
            TemplatePartDto::Optional(parts) => {
                Self::Optional(parts.into_iter().map(Into::into).collect())
            }
        }
    }
}
impl From<FieldKeyDto> for core::FieldKey {
    fn from(value: FieldKeyDto) -> Self {
        match value {
            FieldKeyDto::OriginalStem {} => Self::OriginalStem,
            FieldKeyDto::CurrentStem {} => Self::CurrentStem,
            FieldKeyDto::Extension {} => Self::Extension,
            FieldKeyDto::Parent { level } => Self::Parent { level },
            FieldKeyDto::RelativeDirectory {} => Self::RelativeDirectory,
            FieldKeyDto::Counter {} => Self::Counter,
            FieldKeyDto::Date { source } => Self::Date { source },
            FieldKeyDto::SizeBytes {} => Self::SizeBytes,
            FieldKeyDto::Media { property } => Self::Media {
                property: property.into(),
            },
            FieldKeyDto::WindowsProperty { property } => Self::WindowsProperty { property },
            FieldKeyDto::Hash { algorithm } => Self::Hash { algorithm },
            FieldKeyDto::Uuid {} => Self::Uuid,
            FieldKeyDto::Random {} => Self::Random,
        }
    }
}
impl From<ValueFormatDto> for core::ValueFormat {
    fn from(value: ValueFormatDto) -> Self {
        match value {
            ValueFormatDto::Text {} => Self::Text,
            ValueFormatDto::Integer { radix, padding } => Self::Integer {
                radix: radix.into(),
                padding,
            },
            ValueFormatDto::Decimal { places } => Self::Decimal { places },
            ValueFormatDto::Date { pattern } => Self::Date { pattern },
        }
    }
}
impl From<RadixDto> for core::Radix {
    fn from(value: RadixDto) -> Self {
        match value {
            RadixDto::Decimal => Self::Decimal,
            RadixDto::HexLower => Self::HexLower,
            RadixDto::HexUpper => Self::HexUpper,
            RadixDto::AlphabeticUpper => Self::AlphabeticUpper,
            RadixDto::AlphabeticLower => Self::AlphabeticLower,
        }
    }
}
impl From<MediaPropertyDto> for core::MediaProperty {
    fn from(value: MediaPropertyDto) -> Self {
        match value {
            MediaPropertyDto::Width => Self::Width,
            MediaPropertyDto::Height => Self::Height,
            MediaPropertyDto::DurationNs => Self::DurationNs,
            MediaPropertyDto::FrameRate => Self::FrameRate,
            MediaPropertyDto::VideoCodec => Self::VideoCodec,
            MediaPropertyDto::AudioCodec => Self::AudioCodec,
            MediaPropertyDto::AudioChannels => Self::AudioChannels,
            MediaPropertyDto::BitRate => Self::BitRate,
            MediaPropertyDto::Container => Self::Container,
            MediaPropertyDto::Title => Self::Title,
            MediaPropertyDto::Language => Self::Language,
        }
    }
}
