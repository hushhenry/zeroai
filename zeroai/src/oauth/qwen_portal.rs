//! Qwen OAuth via Device Authorization Grant (RFC 8628).
//! Matches Qwen Code CLI: https://github.com/QwenLM/qwen-code (packages/core/src/qwen/qwenOAuth2.ts)
//! Uses https://chat.qwen.ai (not portal.qwen.ai); flow is device/code -> poll token.

use super::*;
use crate::oauth::pkce::generate_pkce;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

const CLIENT_ID: &str = "f0304373b74a44d2b584a3fb70ca9e56";
const DEVICE_CODE_URL: &str = "https://chat.qwen.ai/api/v1/oauth2/device/code";
const TOKEN_URL: &str = "https://chat.qwen.ai/api/v1/oauth2/token";
const SCOPE: &str = "openid profile email model.completion";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const DEVICE_CODE_EXPIRY_BUFFER_SECS: u64 = 60;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
enum TokenResponse {
    Success {
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<i64>,
    },
    Pending { status: String },
    Error {
        error: String,
        #[serde(default)]
        error_description: Option<String>,
    },
}

pub struct QwenPortalOAuthProvider;

#[async_trait]
impl OAuthProvider for QwenPortalOAuthProvider {
    fn id(&self) -> &str { "qwen-portal" }
    fn name(&self) -> &str { "Qwen Portal OAuth" }

    async fn login(&self, callbacks: &dyn OAuthCallbacks) -> anyhow::Result<OAuthCredentials> {
        let pkce = generate_pkce();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        // 1. Request device code (same as qwen-code)
        let device_resp = client
            .post(DEVICE_CODE_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", CLIENT_ID),
                ("scope", SCOPE),
                ("code_challenge", pkce.challenge.as_str()),
                ("code_challenge_method", "S256"),
            ])
            .send()
            .await?;

        let device_status = device_resp.status();
        if !device_status.is_success() {
            let body = device_resp.text().await.unwrap_or_default();
            anyhow::bail!("Device authorization failed: {} {}", device_status, body);
        }

        let device: DeviceCodeResponse = device_resp.json().await?;

        callbacks.on_auth(OAuthAuthInfo {
            url: device.verification_uri_complete.clone(),
            instructions: Some(
                "Open this URL in your browser, sign in with your Qwen account, and authorize. \
                 This window will wait for you to complete authorization."
                    .into(),
            ),
        });

        let expires_at = std::time::Instant::now()
            + Duration::from_secs(device.expires_in.saturating_sub(DEVICE_CODE_EXPIRY_BUFFER_SECS));
        let mut poll_interval = POLL_INTERVAL;

        loop {
            if std::time::Instant::now() >= expires_at {
                anyhow::bail!("Device code expired. Please start the login flow again.");
            }

            callbacks.on_progress("Waiting for authorization...");

            let token_resp = client
                .post(TOKEN_URL)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Accept", "application/json")
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("client_id", CLIENT_ID),
                    ("device_code", &device.device_code),
                    ("code_verifier", &pkce.verifier),
                ])
                .send()
                .await?;

            let status = token_resp.status();
            let body = token_resp.text().await.unwrap_or_default();
            let token: TokenResponse = serde_json::from_str(&body).unwrap_or(TokenResponse::Error {
                error: "invalid_response".into(),
                error_description: Some(body.clone()),
            });

            match token {
                TokenResponse::Success {
                    access_token,
                    refresh_token,
                    expires_in,
                } => {
                    let refresh = refresh_token
                        .unwrap_or_else(|| String::new());
                    let expires = chrono::Utc::now().timestamp_millis()
                        + expires_in.unwrap_or(3600) * 1000
                        - 300_000; // 5 min buffer
                    return Ok(OAuthCredentials {
                        refresh,
                        access: access_token,
                        expires,
                        extra: HashMap::new(),
                    });
                }
                TokenResponse::Pending { status: _ } => {
                    tokio::time::sleep(poll_interval).await;
                    if poll_interval < Duration::from_secs(10) {
                        poll_interval = Duration::from_secs((poll_interval.as_secs_f64() * 1.5) as u64).min(Duration::from_secs(10));
                    }
                    continue;
                }
                TokenResponse::Error { error, error_description } => {
                    if status.as_u16() == 429 || error == "slow_down" {
                        poll_interval = (poll_interval * 15) / 10;
                        tokio::time::sleep(poll_interval).await;
                        continue;
                    }
                    if error == "authorization_pending" {
                        tokio::time::sleep(poll_interval).await;
                        continue;
                    }
                    let msg = error_description.unwrap_or_else(|| error.clone());
                    anyhow::bail!("Token exchange failed: {} - {}", error, msg);
                }
            }
        }
    }

    async fn refresh_token(&self, credentials: &OAuthCredentials) -> anyhow::Result<OAuthCredentials> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        let resp = client
            .post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &credentials.refresh),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await?;

        let refresh_status = resp.status();
        if !refresh_status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Token refresh failed: {} {}", refresh_status, body);
        }

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            #[serde(default)]
            refresh_token: Option<String>,
            expires_in: i64,
        }
        let token: TokenResp = resp.json().await?;
        let expires = chrono::Utc::now().timestamp_millis() + token.expires_in * 1000 - 300_000;

        Ok(OAuthCredentials {
            refresh: token.refresh_token.unwrap_or_else(|| credentials.refresh.clone()),
            access: token.access_token,
            expires,
            extra: HashMap::new(),
        })
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        credentials.access.clone()
    }
}
