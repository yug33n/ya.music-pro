pub mod client;
pub mod error;
pub mod models;

pub use client::YandexApiClient;
pub use error::ApiError;
pub use models::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DatabaseRepository;

    #[tokio::test]
    async fn test_client_init_and_token_db_persistence() {
        let db = DatabaseRepository::in_memory().unwrap();
        let client = YandexApiClient::new();

        assert_eq!(client.get_token().await, None);
        assert!(!client.load_token_from_db(&db).await.unwrap());

        client.set_token(Some("test_token_12345".to_string())).await;
        assert_eq!(
            client.get_token().await,
            Some("test_token_12345".to_string())
        );

        client.save_token_to_db(&db).await.unwrap();

        // Create a new client and verify loading from DB
        let client2 = YandexApiClient::new();
        assert!(client2.load_token_from_db(&db).await.unwrap());
        assert_eq!(
            client2.get_token().await,
            Some("test_token_12345".to_string())
        );
    }

    #[test]
    fn test_device_code_model_deserialization() {
        let json_data = r#"{
            "device_code": "dev12345",
            "user_code": "abcd-efgh",
            "verification_url": "https://ya.ru/device",
            "expires_in": 300,
            "interval": 5
        }"#;

        let res: DeviceCodeResponse = serde_json::from_str(json_data).unwrap();
        assert_eq!(res.device_code, "dev12345");
        assert_eq!(res.user_code, "abcd-efgh");
        assert_eq!(res.verification_url, "https://ya.ru/device");
        assert_eq!(res.expires_in, 300);
        assert_eq!(res.interval, 5);
    }
}
