//! Strict desktop controls; portable recipes retain their historical encoding.
use serde::Deserialize;
use vidlib_rename::metadata::{AttributeChange, MetadataChangeSet, TimestampChange};
#[derive(Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataChangeSetDto {
    hidden: AttributeChange,
    read_only: AttributeChange,
    archive: AttributeChange,
    system: AttributeChange,
    created: TimestampChangeDto,
    modified: TimestampChangeDto,
    accessed: TimestampChangeDto,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum TimestampChangeDto {
    Keep {},
    Set {
        #[serde(rename = "valueNs", with = "vidlib_inventory::exact::i128_decimal")]
        value_ns: i128,
    },
    Offset {
        #[serde(rename = "offsetNs", with = "vidlib_inventory::exact::i128_decimal")]
        offset_ns: i128,
    },
    Increment {
        #[serde(rename = "startNs", with = "vidlib_inventory::exact::i128_decimal")]
        start_ns: i128,
        #[serde(rename = "stepNs", with = "vidlib_inventory::exact::i128_decimal")]
        step_ns: i128,
    },
}
impl Default for TimestampChangeDto {
    fn default() -> Self {
        Self::Keep {}
    }
}
impl From<TimestampChangeDto> for TimestampChange {
    fn from(value: TimestampChangeDto) -> Self {
        match value {
            TimestampChangeDto::Keep {} => Self::Keep,
            TimestampChangeDto::Set { value_ns } => Self::Set { value_ns },
            TimestampChangeDto::Offset { offset_ns } => Self::Offset { offset_ns },
            TimestampChangeDto::Increment { start_ns, step_ns } => {
                Self::Increment { start_ns, step_ns }
            }
        }
    }
}
impl From<MetadataChangeSetDto> for MetadataChangeSet {
    fn from(v: MetadataChangeSetDto) -> Self {
        Self {
            hidden: v.hidden,
            read_only: v.read_only,
            archive: v.archive,
            system: v.system,
            created: v.created.into(),
            modified: v.modified.into(),
            accessed: v.accessed.into(),
        }
    }
}
