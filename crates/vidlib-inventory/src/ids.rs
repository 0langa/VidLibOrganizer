//! Opaque object identifiers. Possessing an ID does not grant permission.
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

macro_rules! ids {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl $name {
            pub fn new() -> Self { Self(Uuid::new_v4()) }
            pub fn from_uuid(value: Uuid) -> Self { Self(value) }
            pub fn as_uuid(&self) -> &Uuid { &self.0 }
        }
        impl Default for $name { fn default() -> Self { Self::new() } }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(Self) }
        }
    )+};
}
ids!(
    InventoryId,
    RootId,
    EntryId,
    GrantId,
    SessionId,
    JobId,
    PlanId,
    GroupId,
    StepId,
    OperationId,
    EventId,
    PresetId,
    CollectionId,
    AttemptId,
    AttachmentId
);
