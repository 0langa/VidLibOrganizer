use thiserror::Error;

pub type VidLibResult<T> = Result<T, VidLibError>;

#[derive(Debug, Error)]
pub enum VidLibError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("metadata error: {0}")]
    Metadata(String),
    #[error("duplicate analysis error: {0}")]
    Duplicates(String),
    #[error("file operation error: {0}")]
    FileOps(String),
    #[error("ml error: {0}")]
    Ml(String),
    #[error("operation aborted: {0}")]
    Aborted(String),
}

impl From<std::io::Error> for VidLibError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<rusqlite::Error> for VidLibError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value.to_string())
    }
}

impl From<serde_json::Error> for VidLibError {
    fn from(value: serde_json::Error) -> Self {
        Self::Validation(value.to_string())
    }
}

pub fn format_user_error(error: &VidLibError) -> String {
    match error {
        VidLibError::Validation(message) => format!("Validation error: {message}"),
        VidLibError::InvalidPath(message) => format!("Invalid path: {message}"),
        VidLibError::Database(message) => format!("Database error: {message}"),
        VidLibError::Io(message) => format!("I/O error: {message}"),
        VidLibError::Metadata(message) => format!("Metadata error: {message}"),
        VidLibError::Duplicates(message) => format!("Duplicate analysis error: {message}"),
        VidLibError::FileOps(message) => format!("File operation error: {message}"),
        VidLibError::Ml(message) => format!("ML error: {message}"),
        VidLibError::Aborted(message) => format!("Operation aborted: {message}"),
    }
}
