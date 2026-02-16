use super::*;
use crate::oauth::pkce::generate_pkce;
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use std::collections::HashMap;

const CLIENT_ID_HEX: &str = "3638313235353830393339352d6f6f386674326f707264726e7039653361716636617633686d6469623133356a2e617070732e676f6f676c6575736572636f6e74656e742e636f6d";
const CLIENT_SECRET_HEX: &str = "474f435350582d347548674d506d2d316f37536b2d67655636437535636c584673786c";

fn get_client_id() -> String {
    let bytes = (0..CLIENT_ID_HEX.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&CLIENT_ID_HEX[i..i + 2], 16).unwrap_or_default())
        .collect::<Vec<u8>>();
    String::from_utf8(bytes).unwrap_or_default()
}

fn get_client_secret() -> String {
    let bytes = (0..CLIENT_SECRET_HEX.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&CLIENT_SECRET_HEX[i..i + 2], 16).unwrap_or_default())
        .collect::<Vec<u8>>();
    String::from_utf8(bytes).unwrap_or_default()
}
const REDIRECT_URI: &str = "http://localhost:8085/oauth2callback";
const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
];
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const CODE_ASSIST_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com";

fn decode(b64: &str) -> String {
    String::from_utf8(STANDARD.decode(b64).unwrap_or_default()).unwrap_or_default()
}

/// Google Gemini CLI OAuth provider (Cloud Code Assist).
pub struct GeminiCliOAuthProvider;

#[async_trait]
impl OAuthProvider for GeminiCliOAuthProvider {
    fn id(&self) -> &str {
        "gemini-cli"
    }

    fn name(&self) -> &str {
        "Google Cloud Code Assist (Gemini CLI)"
    }

    async fn login(&self, callbacks: &dyn OAuthCallbacks) -> anyhow::Result<OAuthCredentials> {
        let client_id = decode(&get_client_id());
        let pkce = generate_pkce();

        let scopes = SCOPES.join(" ");
        let params = [
            ("client_id", client_id.as_str()),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT_URI),
            ("scope", &scopes),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", &pkce.verifier),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ];

        let auth_url = format!("{}?{}", AUTH_URL, serde_urlencoded::to_string(&params)?);

        callbacks.on_auth(OAuthAuthInfo {
            url: auth_url,
            instructions: Some("Complete the sign-in in your browser. The callback will be received on localhost:8085.".into()),
        });

        // In a TUI/CLI context, we'd start a local server here.
        // For now, prompt the user to paste the redirect URL.
        let redirect_url = callbacks
            .on_prompt(OAuthPrompt {
                message: "Paste the redirect URL from your browser (or wait for automatic callback):".into(),
                placeholder: Some("http://localhost:8085/oauth2callback?code=...&state=...".into()),
            })
            .await?;

        let parsed = url::Url::parse(&redirect_url)?;
        let code = parsed
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.to_string())
            .ok_or_else(|| anyhow::anyhow!("No authorization code in redirect URL"))?;

        callbacks.on_progress("Exchanging authorization code for tokens...");

        let client_secret = decode(&get_client_secret());
        let client = reqwest::Client::new();
        let resp = client
            .post(TOKEN_URL)
            .form(&[
                ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()),
                ("code", &code),
                ("grant_type", "authorization_code"),
                ("redirect_uri", REDIRECT_URI),
                ("code_verifier", &pkce.verifier),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            anyhow::bail!("Token exchange failed: {}", body);
        }

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: i64,
        }

        let token: TokenResp = resp.json().await?;
        let refresh = token.refresh_token
            .ok_or_else(|| anyhow::anyhow!("No refresh token received"))?;

        callbacks.on_progress("Discovering Cloud Code Assist project...");
        let project_id = discover_project(&token.access_token, callbacks).await?;

        let expires = chrono::Utc::now().timestamp_millis() + token.expires_in * 1000 - 5 * 60 * 1000;

        let mut extra = HashMap::new();
        extra.insert("projectId".into(), serde_json::json!(project_id));

        Ok(OAuthCredentials {
            refresh,
            access: token.access_token,
            expires,
            extra,
        })
    }

    async fn refresh_token(&self, credentials: &OAuthCredentials) -> anyhow::Result<OAuthCredentials> {
        let project_id = credentials
            .extra
            .get("projectId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing projectId in credentials"))?
            .to_string();

        let client_id = decode(&get_client_id());
        let client_secret = decode(&get_client_secret());
        let client = reqwest::Client::new();

        let resp = client
            .post(TOKEN_URL)
            .form(&[
                ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()),
                ("refresh_token", &credentials.refresh),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await?;
            anyhow::bail!("Google Cloud token refresh failed: {}", body);
        }

        #[derive(Deserialize)]
        struct RefreshResp {
            access_token: String,
            expires_in: i64,
            refresh_token: Option<String>,
        }

        let data: RefreshResp = resp.json().await?;
        let expires = chrono::Utc::now().timestamp_millis() + data.expires_in * 1000 - 5 * 60 * 1000;

        let mut extra = HashMap::new();
        extra.insert("projectId".into(), serde_json::json!(project_id));

        Ok(OAuthCredentials {
            refresh: data.refresh_token.unwrap_or_else(|| credentials.refresh.clone()),
            access: data.access_token,
            expires,
            extra,
        })
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        let project_id = credentials
            .extra
            .get("projectId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        serde_json::json!({
            "token": credentials.access,
            "projectId": project_id
        })
        .to_string()
    }
}

/// Discover or provision a Cloud Code Assist project.
async fn discover_project(
    access_token: &str,
    callbacks: &dyn OAuthCallbacks,
) -> anyhow::Result<String> {
    // Check env var first
    if let Ok(project) = std::env::var("GOOGLE_CLOUD_PROJECT") {
        if !project.is_empty() {
            return Ok(project);
        }
    }
    if let Ok(project) = std::env::var("GOOGLE_CLOUD_PROJECT_ID") {
        if !project.is_empty() {
            return Ok(project);
        }
    }

    callbacks.on_progress("Checking for existing Cloud Code Assist project...");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1internal:loadCodeAssist", CODE_ASSIST_ENDPOINT))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "google-cloud-sdk vscode_cloudshelleditor/0.1")
        .header("X-Goog-Api-Client", "gl-node/22.17.0")
        .json(&serde_json::json!({
            "metadata": {
                "ideType": "IDE_UNSPECIFIED",
                "platform": "PLATFORM_UNSPECIFIED",
                "pluginType": "GEMINI"
            }
        }))
        .send()
        .await?;

    if resp.status().is_success() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LoadResp {
            cloudaicompanion_project: Option<String>,
        }

        let data: LoadResp = resp.json().await?;
        if let Some(project) = data.cloudaicompanion_project {
            if !project.is_empty() {
                return Ok(project);
            }
        }
    }

    // Try onboarding with free tier
    callbacks.on_progress("Provisioning Cloud Code Assist project (this may take a moment)...");

    let onboard_resp = client
        .post(format!("{}/v1internal:onboardUser", CODE_ASSIST_ENDPOINT))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "google-cloud-sdk vscode_cloudshelleditor/0.1")
        .header("X-Goog-Api-Client", "gl-node/22.17.0")
        .json(&serde_json::json!({
            "tierId": "free-tier",
            "metadata": {
                "ideType": "IDE_UNSPECIFIED",
                "platform": "PLATFORM_UNSPECIFIED",
                "pluginType": "GEMINI"
            }
        }))
        .send()
        .await?;

    if !onboard_resp.status().is_success() {
        let body = onboard_resp.text().await?;
        anyhow::bail!("onboardUser failed: {}", body);
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct OnboardResp {
        done: Option<bool>,
        name: Option<String>,
        response: Option<OnboardResponse>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OnboardResponse {
        cloudaicompanion_project: Option<ProjectId>,
    }

    #[derive(Deserialize)]
    struct ProjectId {
        id: Option<String>,
    }

    let data: OnboardResp = onboard_resp.json().await?;

    if let Some(resp) = data.response {
        if let Some(project) = resp.cloudaicompanion_project {
            if let Some(id) = project.id {
                return Ok(id);
            }
        }
    }

    anyhow::bail!(
        "Could not discover or provision a Google Cloud project. \
         Try setting GOOGLE_CLOUD_PROJECT environment variable."
    )
}
