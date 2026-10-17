//! Typed, read-only metadata boundary. Producers live in desktop adapters.
use crate::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use vidlib_inventory::JobControl;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "property",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PropertyKey {
    Media(MediaProperty),
    Recorded,
    Windows(WindowsProperty),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "zone",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TimezoneEvidence {
    Unknown,
    NotApplicable,
    Known(FrozenTimezone),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataIssueCode {
    Timeout,
    Cancelled,
    OutputLimit,
    InvalidData,
    Unavailable,
    Io,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataIssue {
    pub code: MetadataIssueCode,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MetadataState {
    Value(FrozenFieldValue),
    Missing,
    Unsupported,
    Error(MetadataIssue),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataValue {
    pub value: MetadataState,
    pub source_key: String,
    pub provider: String,
    pub version: String,
    pub timezone: TimezoneEvidence,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataRecord {
    pub values: BTreeMap<PropertyKey, MetadataValue>,
}
pub trait MetadataProvider: Send + Sync {
    fn version(&self) -> &str;
    fn probe(
        &self,
        path: &Path,
        fields: &[PropertyKey],
        control: &JobControl,
    ) -> Result<MetadataRecord, MetadataIssue>;
}
impl FrozenFieldValue {
    pub fn resolved(&self) -> Self {
        match self {
            Self::Metadata(value) => match &value.value {
                MetadataState::Value(value) if !matches!(value, Self::Metadata(_)) => value.clone(),
                _ => Self::Missing {
                    decision: MissingDecision::Block,
                },
            },
            value => value.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataCacheKey {
    pub physical_id: Option<vidlib_inventory::PhysicalId>,
    pub path: std::path::PathBuf,
    pub size: Option<u64>,
    pub modified_ns: Option<i128>,
    pub provider_version: String,
    pub fields: Vec<PropertyKey>,
}
#[derive(Clone)]
pub(crate) struct CacheEntry {
    key: MetadataCacheKey,
    record: MetadataRecord,
}
impl PropertyKey {
    pub fn from_field(key: &FieldKey) -> Option<Self> {
        match key {
            FieldKey::WindowsProperty { property } => Some(Self::Windows(*property)),
            FieldKey::Media { property } => Some(Self::Media(*property)),
            FieldKey::Date {
                source: DateSource::Recorded,
            } => Some(Self::Recorded),
            _ => None,
        }
    }
}
impl FrozenInputCache {
    /// Produces requested metadata under native inventory verification, publishing
    /// only after all preparation succeeds. Generated UUID/random/clock data survive.
    pub fn prepare_with_metadata(
        &mut self,
        request: &InputRequirements,
        inventory: &vidlib_inventory::Inventory,
        control: &JobControl,
        provider: &dyn MetadataProvider,
    ) -> Result<FrozenInputs, RenameError> {
        let mut next = self.clone();
        let mut output = next.prepare(request, inventory, control)?;
        let mut fields = BTreeMap::<
            vidlib_inventory::ids::EntryId,
            std::collections::BTreeSet<PropertyKey>,
        >::new();
        for field in &request.fields {
            if let Some(key) = PropertyKey::from_field(&field.key) {
                fields.entry(field.entry_id).or_default().insert(key);
            }
        }
        for (id, fields) in fields {
            let entry = inventory
                .entries
                .iter()
                .find(|e| e.id == id)
                .expect("prepare validates IDs");
            if entry.kind != vidlib_inventory::EntryKind::File {
                continue;
            }
            let key = MetadataCacheKey {
                physical_id: entry.stamp.physical_id.clone(),
                path: entry.absolute_path.clone(),
                size: entry.stamp.size,
                modified_ns: entry.stamp.modified_ns,
                provider_version: provider.version().into(),
                fields: fields.into_iter().collect(),
            };
            let old = next
                .metadata_cache
                .iter()
                .find(|cached| cached.key == key)
                .map(|cached| cached.record.clone());
            let record = vidlib_inventory::with_verified_helper_path(inventory, id, |safe_path| {
                if control.is_cancelled() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "metadata cancelled",
                    ));
                }
                Ok(old.unwrap_or_else(|| {
                    provider
                        .probe(safe_path, &key.fields, control)
                        .unwrap_or_else(|error| MetadataRecord {
                            values: key
                                .fields
                                .iter()
                                .map(|field| {
                                    (
                                        field.clone(),
                                        MetadataValue {
                                            value: MetadataState::Error(error.clone()),
                                            source_key: format!("{field:?}"),
                                            provider: "metadata".into(),
                                            version: provider.version().into(),
                                            timezone: TimezoneEvidence::Unknown,
                                        },
                                    )
                                })
                                .collect(),
                        })
                }))
            })
            .map_err(|e| {
                RenameError::new(
                    if control.is_cancelled() {
                        RenameErrorCode::Cancelled
                    } else if e.kind() == std::io::ErrorKind::Unsupported {
                        RenameErrorCode::InvalidFrozenInputs
                    } else {
                        RenameErrorCode::StaleInputs
                    },
                    e.to_string(),
                )
            })?;
            if control.is_cancelled() {
                return Err(RenameError::new(
                    RenameErrorCode::Cancelled,
                    "metadata cancelled",
                ));
            }
            // Failed probes remain retryable; never freeze transient error placeholders.
            if !record
                .values
                .values()
                .any(|v| matches!(v.value, MetadataState::Error(_)))
            {
                next.metadata_cache
                    .retain(|cached| cached.key.path != key.path);
                next.metadata_cache.push(CacheEntry {
                    key: key.clone(),
                    record: record.clone(),
                });
            }
            for value in output.values.iter_mut().filter(|v| v.entry_id == id) {
                let Some(field) = PropertyKey::from_field(&value.key) else {
                    continue;
                };
                // Explicit user fallback/optional choices retain authority over automatic inputs.
                if matches!(
                    value.value,
                    FrozenFieldValue::Missing {
                        decision: MissingDecision::Fallback(_) | MissingDecision::OmitOptional
                    }
                ) {
                    continue;
                }
                let metadata = record.values.get(&field).cloned().unwrap_or(MetadataValue {
                    value: MetadataState::Missing,
                    source_key: format!("{field:?}"),
                    provider: "metadata".into(),
                    version: provider.version().into(),
                    timezone: TimezoneEvidence::Unknown,
                });
                value.value = FrozenFieldValue::Metadata(Box::new(metadata));
                let retained = next.retained.as_mut().expect("prepared retention");
                retained.values.retain(|old| {
                    !(old.entry_id == value.entry_id
                        && old.rule_id == value.rule_id
                        && old.key == value.key)
                });
                retained.values.push(value.clone());
            }
        }
        *self = next;
        Ok(output)
    }
}

impl MetadataValue {
    pub(crate) fn missing_issue(&self) -> RuleIssue {
        let detail = match &self.value {
            MetadataState::Error(error) => format!("{:?}: {}", error.code, error.message),
            MetadataState::Unsupported => "field is unsupported".into(),
            _ => "field is missing".into(),
        };
        RuleIssue {
            code: RuleIssueCode::MetadataMissing,
            native_code: None,
            message: format!(
                "{} {} [{}]: {}",
                self.provider, self.version, self.source_key, detail
            ),
        }
    }
}
