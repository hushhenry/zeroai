//! Dynamic model fetching for OpenAI-compatible custom providers.

use crate::models::static_models::static_models_for_provider;
use crate::types::*;
use reqwest::Client;
use serde::Deserialize;
use std::fmt;

/// Error from fetching models list, with optional HTTP status for auth/API errors.
#[derive(Debug, Clone)]
pub struct FetchError {
    /// HTTP status code if the failure was an HTTP error (e.g. 401, 403, 404).
    pub status: Option<u16>,
    /// Human-readable message.
    pub message: String,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(s) = self.status {
            write!(f, "{} {}", s, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for FetchError {}

impl FetchError {
    /// True if this error is likely an auth/credential problem (401, 403, or 404).
    pub fn is_auth_error(&self) -> bool {
        self.status.map(|s| s == 401 || s == 403 || s == 404).unwrap_or(false)
    }
}

/// OpenAI-compatible models list response.
#[derive(Debug, Deserialize)]
struct OpenAIModelsResponse {
    #[serde(default)]
    data: Vec<OpenAIModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelEntry {
    id: String,
    #[allow(dead_code)]
    object: Option<String>,
}

/// Returns true if the provider is a custom OpenAI-compatible one (dynamic model list).
pub fn is_custom_provider(provider: &str) -> bool {
    provider.starts_with("custom:")
}

/// Fetch models for a provider. For static providers uses the static list; for custom
/// (name contains "custom:") fetches via HTTP GET from models_url or {base_url}/v1/models.
/// Returns a structured FetchError with status code on HTTP errors so callers can detect auth failures.
pub async fn fetch_models_for_provider(
    provider: &str,
    api_key: Option<&str>,
    models_url: Option<&str>,
) -> Result<Vec<ModelDef>, FetchError> {
    if !is_custom_provider(provider) {
        return Ok(static_models_for_provider(provider));
    }

    let base_url = provider
        .strip_prefix("custom:")
        .unwrap_or("")
        .trim()
        .trim_end_matches('/');
    if base_url.is_empty() || (!base_url.starts_with("http://") && !base_url.starts_with("https://")) {
        return Ok(Vec::new());
    }

    let url = match models_url {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => format!("{}/v1/models", base_url),
    };

    let client = Client::new();
    let mut req = client.get(&url);
    if let Some(key) = api_key {
        req = req.header("Authorization", format!("Bearer {}", key));
    }

    let resp = req.send().await.map_err(|e| FetchError {
        status: None,
        message: format!("Failed to fetch models list: {}", e),
    })?;

    let status = resp.status();
    if !status.is_success() {
        let code = status.as_u16();
        let body = resp.text().await.unwrap_or_default();
        let msg = if body.len() > 200 {
            format!("{}...", &body[..200])
        } else {
            body
        };
        return Err(FetchError {
            status: Some(code),
            message: msg,
        });
    }

    let body = resp.text().await.map_err(|e| FetchError {
        status: None,
        message: format!("Failed to read response body: {}", e),
    })?;

    let parsed: OpenAIModelsResponse = serde_json::from_str(&body).map_err(|e| FetchError {
        status: None,
        message: format!("Invalid models list JSON: {}", e),
    })?;

    let models = parsed
        .data
        .into_iter()
        .map(|e| ModelDef {
            id: e.id.clone(),
            name: e.id.clone(),
            api: Api::OpenaiCompletions,
            provider: provider.to_string(),
            base_url: base_url.to_string(),
            reasoning: false,
            input: vec![InputModality::Text],
            cost: ModelCost::default(),
            context_window: 128000,
            max_tokens: 16384,
            headers: None,
        })
        .collect();

    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_custom_provider_detects_prefix() {
        assert!(is_custom_provider("custom:https://api.example.com"));
        assert!(!is_custom_provider("openai"));
        assert!(!is_custom_provider("custom"));
    }

    #[test]
    fn parse_openai_models_response() {
        let json = r#"{
            "object": "list",
            "data": [
                { "id": "gpt-4o", "object": "model" },
                { "id": "gpt-3.5-turbo", "object": "model" }
            ]
        }"#;
        let parsed: OpenAIModelsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.data.len(), 2);
        assert_eq!(parsed.data[0].id, "gpt-4o");
        assert_eq!(parsed.data[1].id, "gpt-3.5-turbo");
    }

    #[test]
    fn parse_openai_models_response_minimal() {
        let json = r#"{"data":[{"id":"model-1"}]}"#;
        let parsed: OpenAIModelsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.data.len(), 1);
        assert_eq!(parsed.data[0].id, "model-1");
    }

    #[test]
    fn fallback_url_when_models_url_none() {
        let provider = "custom:https://api.example.com";
        let base_url = provider.strip_prefix("custom:").unwrap().trim().trim_end_matches('/');
        let url: String = format!("{}/v1/models", base_url);
        assert_eq!(url, "https://api.example.com/v1/models");
    }

    #[test]
    fn fetch_error_is_auth_error() {
        assert!(FetchError { status: Some(401), message: String::new() }.is_auth_error());
        assert!(FetchError { status: Some(403), message: String::new() }.is_auth_error());
        assert!(FetchError { status: Some(404), message: String::new() }.is_auth_error());
        assert!(!FetchError { status: Some(500), message: String::new() }.is_auth_error());
        assert!(!FetchError { status: None, message: String::new() }.is_auth_error());
    }
}
