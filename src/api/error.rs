use thiserror::Error;

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("Network HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("XML parsing error: {0}")]
    Xml(#[from] quick_xml::DeError),

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Yandex API error: {0}")]
    Yandex(String),

    #[error("Authorization pending, please authorize in browser")]
    AuthPending,

    #[error("Device code expired")]
    CodeExpired,

    #[error("Track download info not found or empty")]
    DownloadInfoNotFound,

    #[error("Invalid download info response")]
    InvalidDownloadInfo,
}
