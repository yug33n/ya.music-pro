use crate::api::error::ApiError;
use crate::api::models::{
    AccountStatus, DeviceCodeResponse, TokenErrorResponse, TokenResponse, YandexApiResponse,
};
use crate::db::DatabaseRepository;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::RwLock;

pub const DEFAULT_CLIENT_ID: &str = "23cabb944e3444b1b94ba960432363e1";
pub const DEFAULT_CLIENT_SECRET: &str = "53bc75ac20044548ae010c7322560249";
pub const API_BASE_URL: &str = "https://api.music.yandex.net";
pub const OAUTH_BASE_URL: &str = "https://oauth.yandex.ru";

#[derive(Clone)]
pub struct YandexApiClient {
    http: Client,
    token: Arc<RwLock<Option<String>>>,
    client_id: String,
    client_secret: String,
}

impl YandexApiClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("YandexMusic/5.30.0 (Windows 10; x64)")
            .build()
            .expect("Failed to initialize reqwest client");

        Self {
            http: client,
            token: Arc::new(RwLock::new(None)),
            client_id: DEFAULT_CLIENT_ID.to_string(),
            client_secret: DEFAULT_CLIENT_SECRET.to_string(),
        }
    }

    pub fn with_token(token: &str) -> Self {
        let client = Self::new();
        let tok = token.to_string();
        tokio::spawn({
            let arc_tok = client.token.clone();
            async move {
                *arc_tok.write().await = Some(tok);
            }
        });
        client
    }

    pub async fn set_token(&self, token: Option<String>) {
        *self.token.write().await = token;
    }

    pub async fn get_token(&self) -> Option<String> {
        self.token.read().await.clone()
    }

    pub async fn load_token_from_db(&self, db: &DatabaseRepository) -> Result<bool, ApiError> {
        if let Some(tok) = db.get_config("access_token")? {
            if !tok.is_empty() {
                self.set_token(Some(tok)).await;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn save_token_to_db(&self, db: &DatabaseRepository) -> Result<(), ApiError> {
        if let Some(ref tok) = *self.token.read().await {
            db.set_config("access_token", tok)?;
        }
        Ok(())
    }

    /// Step 1: Request Device Code for ya.ru/device flow
    pub async fn request_device_code(&self) -> Result<DeviceCodeResponse, ApiError> {
        let url = format!("{}/device/code", OAUTH_BASE_URL);
        let params = [("client_id", self.client_id.as_str())];

        let resp = self
            .http
            .post(&url)
            .form(&params)
            .send()
            .await?
            .error_for_status()?;

        let body: DeviceCodeResponse = resp.json().await?;
        Ok(body)
    }

    /// Step 2: Poll token endpoint until user authorizes code
    pub async fn poll_device_token(&self, device_code: &str) -> Result<TokenResponse, ApiError> {
        let url = format!("{}/token", OAUTH_BASE_URL);
        let params = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("code", device_code),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
        ];

        let resp = self.http.post(&url).form(&params).send().await?;

        let status = resp.status();
        let text = resp.text().await?;

        if status.is_success() {
            let token_resp: TokenResponse = serde_json::from_str(&text)?;
            self.set_token(Some(token_resp.access_token.clone())).await;
            return Ok(token_resp);
        }

        // Try to parse error
        if let Ok(err_resp) = serde_json::from_str::<TokenErrorResponse>(&text) {
            match err_resp.error.as_str() {
                "authorization_pending" => return Err(ApiError::AuthPending),
                "expired_token" | "invalid_grant" => return Err(ApiError::CodeExpired),
                other => return Err(ApiError::Yandex(other.to_string())),
            }
        }

        Err(ApiError::Yandex(format!("HTTP {}: {}", status, text)))
    }

    /// Helper to get authorization headers
    pub async fn auth_headers(&self) -> Result<HeaderMap, ApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("YandexMusic/5.30.0 (Windows 10; x64)"),
        );

        if let Some(ref token) = *self.token.read().await {
            let auth_val = format!("OAuth {}", token);
            if let Ok(val) = HeaderValue::from_str(&auth_val) {
                headers.insert(AUTHORIZATION, val);
            }
        }

        Ok(headers)
    }

    /// Fetch account status
    pub async fn get_account_status(&self) -> Result<AccountStatus, ApiError> {
        let url = format!("{}/account/status", API_BASE_URL);
        let headers = self.auth_headers().await?;

        let resp = self
            .http
            .get(&url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?;

        let api_resp: YandexApiResponse<AccountStatus> = resp.json().await?;
        api_resp
            .result
            .ok_or_else(|| ApiError::Yandex("Missing account status result in response".to_string()))
    }

    pub fn http(&self) -> &Client {
        &self.http
    }
}

impl Default for YandexApiClient {
    fn default() -> Self {
        Self::new()
    }
}
