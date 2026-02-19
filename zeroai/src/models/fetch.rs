//! Dynamic model fetching for OpenAI-compatible providers.
//!
//! Most providers expose an OpenAI-compatible GET /v1/models endpoint.
//! Base URLs come from auth::provider_base_url (single source for API and models).

use crate::auth;
use crate::models::static_models::static_models_for_provider;
use crate::types::*;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

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

/// Ollama native /api/tags response.
#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
}

/// Returns true if the provider is a custom OpenAI-compatible one (dynamic model list).
pub fn is_custom_provider(provider: &str) -> bool {
    provider.starts_with("custom:")
}

/// Providers that have a base_url but do not expose OpenAI-compatible GET /models (proprietary API).
/// openai-codex: OAuth token lacks api.model.read; only static model list is used.
const STATIC_ONLY_PROVIDERS: &[&str] = &[
    "google", "anthropic", "anthropic-setup-token", "synthetic", "cloudflare-ai-gateway",
    "github-copilot", "amazon-bedrock", "openai-codex", "qwen-portal",
];

/// Returns true if a provider supports dynamic model listing (GET /models).
pub fn supports_dynamic_models(provider: &str) -> bool {
    is_custom_provider(provider)
        || (auth::provider_base_url(provider).is_some() && !STATIC_ONLY_PROVIDERS.contains(&provider))
}

/// Create a default `ModelDef` for a model ID on a known dynamic provider.
/// Returns `None` if the provider is not a known dynamic provider or custom provider.
pub fn default_model_def_for_provider(provider: &str, model_id: &str) -> Option<ModelDef> {
    let base_url = if is_custom_provider(provider) {
        let u = provider.strip_prefix("custom:").unwrap_or("").trim().trim_end_matches('/');
        if u.is_empty() { return None; }
        u.to_string()
    } else {
        auth::provider_base_url(provider)?.to_string()
    };

    Some(ModelDef {
        id: model_id.to_string(),
        name: model_id.to_string(),
        api: Api::OpenaiCompletions,
        provider: provider.to_string(),
        base_url,
        reasoning: looks_like_reasoning_model(model_id),
        input: vec![InputModality::Text],
        cost: ModelCost::default(),
        context_window: 128000,
        max_tokens: 16384,
        headers: None,
    })
}

/// Fetch models for a provider.
///
/// - **Custom providers** (`custom:https://...`): always fetches dynamically.
/// - **Known dynamic providers** (OpenAI, DeepSeek, etc.): fetches dynamically,
///   merges metadata from the static catalog, and falls back to static on error.
/// - **Static-only providers** (Anthropic, Google, Bedrock, etc.): returns the static catalog.
pub async fn fetch_models_for_provider(
    provider: &str,
    api_key: Option<&str>,
    models_url: Option<&str>,
) -> Result<Vec<ModelDef>, FetchError> {
    // Custom providers: pure dynamic fetch
    if is_custom_provider(provider) {
        return fetch_custom_provider(provider, api_key, models_url).await;
    }

    // Providers that support GET /models: fetch live list (no static fallback on failure)
    if supports_dynamic_models(provider) {
        if let Some(base_url) = auth::provider_base_url(provider) {
            let url = match models_url {
                Some(u) if !u.trim().is_empty() => u.trim().to_string(),
                _ => format!("{}/models", base_url),
            };

            let dynamic_result = if provider == "ollama" {
                fetch_ollama_models(base_url, api_key).await
            } else {
                fetch_openai_compatible_models(&url, api_key).await
            };

            match dynamic_result {
                Ok(ids) => return Ok(merge_dynamic_with_static(provider, base_url, &ids)),
                Err(e) => return Err(e),
            }
        }
    }
    // Custom provider without base_url from auth uses fetch_custom_provider (already returned above)

    // Static-only providers (have base_url but no GET /models, or unknown)
    Ok(static_models_for_provider(provider))
}

/// Fetch model IDs from an OpenAI-compatible /models endpoint.
async fn fetch_openai_compatible_models(url: &str, api_key: Option<&str>) -> Result<Vec<String>, FetchError> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| FetchError { status: None, message: format!("HTTP client error: {}", e) })?;

    let mut req = client.get(url);
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
        let msg = if body.len() > 200 { format!("{}...", &body[..200]) } else { body };
        return Err(FetchError { status: Some(code), message: msg });
    }

    let body = resp.text().await.map_err(|e| FetchError {
        status: None,
        message: format!("Failed to read response body: {}", e),
    })?;

    let parsed: OpenAIModelsResponse = serde_json::from_str(&body).map_err(|e| FetchError {
        status: None,
        message: format!("Invalid models list JSON: {}", e),
    })?;

    Ok(parsed.data.into_iter().map(|e| e.id).collect())
}

/// Fetch model names from Ollama's native /api/tags endpoint.
async fn fetch_ollama_models(base_url: &str, api_key: Option<&str>) -> Result<Vec<String>, FetchError> {
    // Ollama's native API lives at the root, not under /v1
    let api_base = base_url.trim_end_matches("/v1").trim_end_matches('/');
    let url = format!("{}/api/tags", api_base);

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| FetchError { status: None, message: format!("HTTP client error: {}", e) })?;

    let mut req = client.get(&url);
    if let Some(key) = api_key {
        req = req.header("Authorization", format!("Bearer {}", key));
    }

    let resp = req.send().await.map_err(|e| FetchError {
        status: None,
        message: format!("Failed to fetch Ollama models: {}", e),
    })?;

    if !resp.status().is_success() {
        // Fall back to OpenAI-compatible endpoint
        let compat_url = format!("{}/models", base_url);
        return fetch_openai_compatible_models(&compat_url, api_key).await;
    }

    let body = resp.text().await.map_err(|e| FetchError {
        status: None,
        message: format!("Failed to read Ollama response: {}", e),
    })?;

    let parsed: OllamaTagsResponse = serde_json::from_str(&body).map_err(|e| FetchError {
        status: None,
        message: format!("Invalid Ollama tags JSON: {}", e),
    })?;

    Ok(parsed.models.into_iter().map(|m| m.name).collect())
}

/// Merge dynamically discovered model IDs with the static catalog.
///
/// For each dynamic ID that matches a static entry, the static metadata (reasoning,
/// cost, context window, etc.) is preserved. Dynamic IDs not in the static catalog
/// get sensible defaults. Static models not in the dynamic list are omitted (the
/// provider no longer offers them).
fn merge_dynamic_with_static(provider: &str, base_url: &str, dynamic_ids: &[String]) -> Vec<ModelDef> {
    let static_models = static_models_for_provider(provider);
    let static_map: HashMap<&str, &ModelDef> = static_models.iter().map(|m| (m.id.as_str(), m)).collect();

    dynamic_ids
        .iter()
        .map(|id| {
            if let Some(s) = static_map.get(id.as_str()) {
                (*s).clone()
            } else {
                ModelDef {
                    id: id.clone(),
                    name: id.clone(),
                    api: Api::OpenaiCompletions,
                    provider: provider.to_string(),
                    base_url: base_url.to_string(),
                    reasoning: looks_like_reasoning_model(id),
                    input: vec![InputModality::Text],
                    cost: ModelCost::default(),
                    context_window: 128000,
                    max_tokens: 16384,
                    headers: None,
                }
            }
        })
        .collect()
}

/// Heuristic: model IDs containing these substrings likely support reasoning/thinking.
fn looks_like_reasoning_model(id: &str) -> bool {
    let lower = id.to_lowercase();
    lower.contains("thinking") || lower.contains("reason") || lower.contains("-r1")
        || lower.contains("/r1") || lower.contains("o1") || lower.contains("o3")
}

/// Fetch models for a custom provider (custom:https://...).
async fn fetch_custom_provider(
    provider: &str,
    api_key: Option<&str>,
    models_url: Option<&str>,
) -> Result<Vec<ModelDef>, FetchError> {
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

    let ids = fetch_openai_compatible_models(&url, api_key).await?;

    let models = ids
        .into_iter()
        .map(|id| ModelDef {
            name: id.clone(),
            reasoning: looks_like_reasoning_model(&id),
            id,
            api: Api::OpenaiCompletions,
            provider: provider.to_string(),
            base_url: base_url.to_string(),
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
    fn parse_ollama_tags_response() {
        let json = r#"{"models":[{"name":"llama3:latest"},{"name":"codellama:7b"}]}"#;
        let parsed: OllamaTagsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.models.len(), 2);
        assert_eq!(parsed.models[0].name, "llama3:latest");
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

    #[test]
    fn dynamic_providers_have_base_urls() {
        assert!(auth::provider_base_url("openai").is_some());
        assert!(auth::provider_base_url("openai-codex").is_some());
        assert!(auth::provider_base_url("qwen").is_some());
        assert!(auth::provider_base_url("deepseek").is_some());
        assert!(auth::provider_base_url("together").is_some());
        assert!(auth::provider_base_url("ollama").is_some());
        assert!(auth::provider_base_url("google").is_some());
        assert!(auth::provider_base_url("amazon-bedrock").is_some());
        // Unknown provider
        assert!(auth::provider_base_url("unknown-provider").is_none());
    }

    #[test]
    fn supports_dynamic_models_covers_custom_and_known() {
        assert!(supports_dynamic_models("custom:https://example.com"));
        assert!(supports_dynamic_models("openai"));
        assert!(supports_dynamic_models("groq"));
        assert!(!supports_dynamic_models("anthropic"));
        assert!(!supports_dynamic_models("google"));
        // openai-codex OAuth token lacks api.model.read; static list only
        assert!(!supports_dynamic_models("openai-codex"));
    }

    #[test]
    fn merge_preserves_static_metadata() {
        let dynamic_ids = vec!["gpt-4o".to_string(), "gpt-new-model".to_string()];
        let merged = merge_dynamic_with_static("openai", "https://api.openai.com/v1", &dynamic_ids);
        assert_eq!(merged.len(), 2);
        // gpt-4o should have static metadata (128000 context from static)
        assert_eq!(merged[0].id, "gpt-4o");
        assert_eq!(merged[0].context_window, 128000);
        // gpt-new-model gets defaults
        assert_eq!(merged[1].id, "gpt-new-model");
        assert_eq!(merged[1].provider, "openai");
    }

    #[test]
    fn reasoning_heuristic() {
        assert!(looks_like_reasoning_model("deepseek-r1"));
        assert!(looks_like_reasoning_model("deepseek-ai/DeepSeek-R1"));
        assert!(looks_like_reasoning_model("o1-preview"));
        assert!(looks_like_reasoning_model("o3-mini"));
        assert!(looks_like_reasoning_model("qwen-thinking-2.5"));
        assert!(!looks_like_reasoning_model("gpt-4o"));
        assert!(!looks_like_reasoning_model("llama-3.3-70b"));
    }
}
