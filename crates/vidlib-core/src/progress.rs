use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProgressSnapshot {
    pub current_path: Option<String>,
    pub processed_files: u64,
    pub discovered_files: u64,
    pub bytes_processed: u64,
    pub percent: f32,
    pub message: String,
}

impl ProgressSnapshot {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            current_path: None,
            processed_files: 0,
            discovered_files: 0,
            bytes_processed: 0,
            percent: 0.0,
            message: message.into(),
        }
    }
}
