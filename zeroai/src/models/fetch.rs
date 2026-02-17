//! Dynamic model fetching for OpenAI-compatible custom providers.

use crate::models::static_models::static_models_for_provider;
use crate::types::*;
use anyhow::Context;
use reqwest::Client;
use serde::Deserialize;

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
pub async fn fetch_models_for_provider(
    provider: &str,
    api_key: Option<&str>,
    models_url: Option<&str>,
) -> anyhow::Result<Vec<ModelDef>> {
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

    let resp = req.send().await.context("Failed to fetch models list")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Models list request failed: {} {}",
            status,
            if body.len() > 200 { format!("{}...", &body[..200]) } else { body }
        );
    }

    let body = resp.text().await.context("Failed to read models response body")?;
    let parsed: OpenAIModelsResponse = serde_json::from_str(&body).context("Invalid models list JSON")?;

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
}
