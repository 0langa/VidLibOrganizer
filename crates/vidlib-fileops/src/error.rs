use serde::{Deserialize, Serialize};
use vidlib_inventory::ids::{EntryId, OperationId, StepId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    DestinationExists,
    SourceMissing,
    StaleState,
    ReparsePoint,
    InvalidPath,
    OutsideGrant,
    ProtectedEntry,
    PermissionDenied,
    Busy,
    CrossDevice,
    Unsupported,
    Io,
    RecoveryRequired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Inspect,
    Capture,
    Verify,
    Rename,
    Create,
    Read,
    Write,
    Metadata,
    Flush,
    Remove,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpError {
    pub code: ErrorCode,
    pub phase: Phase,
    pub operation_id: Option<OperationId>,
    pub step_id: Option<StepId>,
    pub item_id: Option<EntryId>,
    pub message: String,
    pub retryable: bool,
}
impl OpError {
    pub fn new(code: ErrorCode, phase: Phase, message: impl Into<String>) -> Self {
        Self {
            code,
            phase,
            operation_id: None,
            step_id: None,
            item_id: None,
            message: message.into(),
            retryable: matches!(code, ErrorCode::Busy),
        }
    }
    pub fn unsupported(phase: Phase) -> Self {
        Self::new(
            ErrorCode::Unsupported,
            phase,
            "filesystem capability is not verified",
        )
    }
}
impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for OpError {}
