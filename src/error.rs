use thiserror::Error;

#[derive(Error, Debug)]
pub enum WindchillError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON parsing failed: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    ConfigError(#[from] config::ConfigError),

    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Document not found: {0}")]
    DocumentNotFound(String),

    #[error("Checkout failed: {0}")]
    CheckoutError(String),

    #[error("Upload failed: {0}")]
    UploadError(String),

    #[error("Invalid response from server: {0}")]
    InvalidResponse(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, WindchillError>;
