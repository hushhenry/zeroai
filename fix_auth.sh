#!/bin/bash
cd ai/src/auth
# Modify all_provider_auth_info to include setup-token as a separate sub-provider for clarity
# or modify the grouping. 
# Better: add an explicit "Anthropic (setup-token)" sub-provider to the list.

cat > mod.rs << 'RS'
pub mod config;
pub mod sniff;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Credential types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyCredential {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredential {
    pub refresh: String,
    pub access: String,
    pub expires: i64,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupTokenCredential {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    ApiKey(ApiKeyCredential),
    OAuth(OAuthCredential),
    SetupToken(SetupTokenCredential),
}

impl Credential {
    pub fn api_key(&self) -> Option<String> {
        match self {
            Credential::ApiKey(c) => Some(c.key.clone()),
            Credential::OAuth(c) => {
                if let Some(project_id) = c.extra.get("projectId") {
                    if let Some(pid) = project_id.as_str() {
                        return Some(
                            serde_json::json!({
                                "token": c.access,
                                "projectId": pid
                            })
                            .to_string(),
                        );
                    }
                }
                Some(c.access.clone())
            }
            Credential::SetupToken(c) => Some(c.token.clone()),
        }
    }

    pub fn is_expired(&self) -> bool {
        match self {
            Credential::OAuth(c) => chrono::Utc::now().timestamp_millis() >= c.expires,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Auth method descriptors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum AuthMethod {
    ApiKey {
        env_var: Option<String>,
        hint: Option<String>,
    },
    OAuth {
        hint: Option<String>,
    },
    SetupToken {
        hint: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ProviderAuthInfo {
    pub provider_id: String,
    pub label: String,
    pub group: String,
    pub hint: String,
    pub auth_methods: Vec<AuthMethod>,
}

pub fn all_provider_auth_info() -> Vec<ProviderAuthInfo> {
    vec![
        ProviderAuthInfo {
            provider_id: "openai".into(),
            label: "OpenAI API Key".into(),
            group: "OpenAI".into(),
            hint: "GPT-4o, o1, o3, o4-mini".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("OPENAI_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "anthropic".into(),
            label: "Anthropic API Key".into(),
            group: "Anthropic".into(),
            hint: "Standard API (via key)".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("ANTHROPIC_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "anthropic-setup".into(),
            label: "Anthropic Setup Token".into(),
            group: "Anthropic".into(),
            hint: "From claude setup-token".into(),
            auth_methods: vec![AuthMethod::SetupToken {
                hint: Some("Run `claude setup-token` to generate".into()),
            }],
        },
        ProviderAuthInfo {
            provider_id: "google".into(),
            label: "Gemini API Key".into(),
            group: "Google".into(),
            hint: "AI Studio (API key)".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("GEMINI_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "gemini-cli".into(),
            label: "Gemini CLI OAuth".into(),
            group: "Google".into(),
            hint: "Cloud Code Assist (OAuth)".into(),
            auth_methods: vec![AuthMethod::OAuth {
                hint: Some("Google OAuth for Cloud Code Assist".into()),
            }],
        },
        ProviderAuthInfo {
            provider_id: "antigravity".into(),
            label: "Antigravity OAuth".into(),
            group: "Google".into(),
            hint: "Google Cloud Enterprise".into(),
            auth_methods: vec![AuthMethod::OAuth {
                hint: Some("Google OAuth for Antigravity".into()),
            }],
        },
        ProviderAuthInfo {
            provider_id: "deepseek".into(),
            label: "DeepSeek API Key".into(),
            group: "DeepSeek".into(),
            hint: "DeepSeek V3, R1".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("DEEPSEEK_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "openrouter".into(),
            label: "OpenRouter API Key".into(),
            group: "OpenRouter".into(),
            hint: "Multi-provider gateway".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("OPENROUTER_API_KEY".into()),
                hint: None,
            }],
        },
    ]
}

pub fn provider_groups() -> Vec<(String, Vec<ProviderAuthInfo>)> {
    let all = all_provider_auth_info();
    let mut groups: Vec<(String, Vec<ProviderAuthInfo>)> = Vec::new();
    for info in all {
        if let Some(g) = groups.iter_mut().find(|(label, _)| label == &info.group) {
            g.1.push(info);
        } else {
            let label = info.group.clone();
            groups.push((label, vec![info]));
        }
    }
    groups
}
RS
