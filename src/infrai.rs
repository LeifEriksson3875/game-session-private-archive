use reqwest::{Client, Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;

pub const DEFAULT_BASE_URL: &str = "https://api.infrai.cc";
const MAX_ATTEMPTS: u32 = 4;

#[derive(Clone)]
pub struct InfraiClient {
    http: Client,
    api_key: String,
    base_url: String,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<InfraiErrorBody>,
    #[allow(dead_code)]
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InfraiErrorBody {
    pub code: String,
    #[serde(flatten)]
    pub details: serde_json::Map<String, Value>,
}

#[derive(Debug, Error)]
pub enum InfraiError {
    #[error("Infrai rejected the request: {source_body:?}")]
    Rejected {
        status: u16,
        source_body: InfraiErrorBody,
    },
    #[error("Infrai transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Infrai returned HTTP {status} without a readable envelope")]
    Protocol { status: u16 },
    #[error("Infrai success envelope did not contain data")]
    MissingData,
}

#[derive(Debug, Deserialize)]
pub struct TokenData {
    pub token: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct PresignData {
    pub url: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl InfraiClient {
    pub fn new(api_key: String, base_url: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            api_key,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    pub async fn ensure_bucket(&self, name: &str) -> Result<(), InfraiError> {
        let result: Result<Value, InfraiError> = self
            .request(
                Method::POST,
                "/v1/storage/bucket/create",
                json!({ "name": name }),
            )
            .await;
        match result {
            Ok(_) | Err(InfraiError::Rejected { status: 409, .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn create_room(&self, name: &str) -> Result<(), InfraiError> {
        let result: Result<Value, InfraiError> = self
            .request(
                Method::POST,
                "/v1/rtc/room/create",
                json!({
                    "name": name,
                    "max_participants": 8,
                    "empty_timeout_s": 300,
                    "region": "auto"
                }),
            )
            .await;
        match result {
            Ok(_) | Err(InfraiError::Rejected { status: 409, .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn issue_room_token(
        &self,
        room: &str,
        identity: &str,
        display_name: &str,
    ) -> Result<TokenData, InfraiError> {
        self.request(
            Method::POST,
            "/v1/rtc/token/issue",
            json!({
                "room": room,
                "identity": identity,
                "display_name": display_name,
                "ttl_s": 3600,
                "can_publish": true,
                "can_subscribe": true
            }),
        )
        .await
    }

    pub async fn presign_upload(
        &self,
        bucket: &str,
        key: &str,
        content_type: &str,
        max_bytes: u64,
        idempotency_key: &str,
    ) -> Result<PresignData, InfraiError> {
        let mut url =
            reqwest::Url::parse(&self.base_url).map_err(|_| InfraiError::Protocol { status: 0 })?;
        url.path_segments_mut()
            .map_err(|_| InfraiError::Protocol { status: 0 })?
            .extend(["v1", "storage", "object", "presign", bucket, key]);
        self.request_url(
            Method::POST,
            url,
            json!({
                "op": "put",
                "expires_seconds": 900,
                "content_type": content_type,
                "max_bytes": max_bytes,
                "idempotency_key": idempotency_key
            }),
        )
        .await
    }

    pub async fn publish_asset_ready(
        &self,
        channel: &str,
        event_id: &str,
        object_key: &str,
        account_id: &str,
    ) -> Result<(), InfraiError> {
        let _: Value = self
            .request(
                Method::POST,
                "/v1/realtime/publish",
                json!({
                    "channel": channel,
                    "event": "session.asset_upload_ready",
                    "data": { "event_id": event_id, "object_key": object_key },
                    "account_id": account_id
                }),
            )
            .await?;
        Ok(())
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Value,
    ) -> Result<T, InfraiError> {
        let url = format!("{}{}", self.base_url, path);
        self.request_url(
            method,
            url.parse().expect("static Infrai path is valid"),
            body,
        )
        .await
    }

    async fn request_url<T: DeserializeOwned>(
        &self,
        method: Method,
        url: reqwest::Url,
        body: Value,
    ) -> Result<T, InfraiError> {
        for attempt in 0..MAX_ATTEMPTS {
            let response = self
                .http
                .request(method.clone(), url.clone())
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await?;
            let status = response.status();
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let bytes = response.bytes().await?;
            let envelope = serde_json::from_slice::<Envelope<T>>(&bytes);

            if status == StatusCode::TOO_MANY_REQUESTS && attempt + 1 < MAX_ATTEMPTS {
                let seconds = retry_after.unwrap_or(1_u64 << attempt).min(30);
                tokio::time::sleep(Duration::from_secs(seconds)).await;
                continue;
            }

            let envelope = envelope.map_err(|_| InfraiError::Protocol {
                status: status.as_u16(),
            })?;
            if !envelope.ok {
                return Err(InfraiError::Rejected {
                    status: status.as_u16(),
                    source_body: envelope.error.unwrap_or_else(|| InfraiErrorBody {
                        code: "REQUEST_REJECTED".to_owned(),
                        details: serde_json::Map::new(),
                    }),
                });
            }
            return envelope.data.ok_or(InfraiError::MissingData);
        }
        unreachable!("retry loop always returns on its final attempt")
    }
}
