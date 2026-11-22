use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: Option<String>,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenErrorResponse {
    pub error: String,
    pub error_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YandexApiResponse<T> {
    pub result: Option<T>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStatus {
    pub account: Option<AccountInfo>,
    pub permissions: Option<AccountPermissions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub uid: Option<u64>,
    pub login: Option<String>,
    pub full_name: Option<String>,
    pub display_name: Option<String>,
    pub registered: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountPermissions {
    pub until: Option<String>,
    pub values: Option<Vec<String>>,
    pub default: Option<Vec<String>>,
}
