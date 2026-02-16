pub mod config;
pub mod sniff;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Credential types
// ---------------------------------------------------------------------------

/// An API key credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyCredential {
    pub key: String,
}

/// An OAuth credential set (refresh + access token).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredential {
    pub refresh: String,
    pub access: String,
    /// Expiry timestamp in milliseconds since epoch.
    pub expires: i64,
    /// Extra data (e.g. `projectId` for Google Cloud).
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// A setup-token credential (Anthropic).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupTokenCredential {
    pub token: String,
}

/// Union of all credential types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    ApiKey(ApiKeyCredential),
    OAuth(OAuthCredential),
    SetupToken(SetupTokenCredential),
}

impl Credential {
    /// Get the API key / bearer token from this credential.
    pub fn api_key(&self) -> Option<String> {
        match self {
            Credential::ApiKey(c) => Some(c.key.clone()),
            Credential::OAuth(c) => {
                // For Google Cloud Code Assist, return JSON with token + projectId
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

    /// Check if an OAuth credential is expired.
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

/// Describes the authentication method for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum AuthMethod {
    /// Simple API key (paste a key).
    ApiKey {
        env_var: Option<String>,
        hint: Option<String>,
    },
    /// OAuth flow (opens browser).
    OAuth {
        hint: Option<String>,
    },
    /// Anthropic setup-token (paste a setup-token from `claude setup-token`).
    SetupToken {
        hint: Option<String>,
    },
}

/// Describes a provider and how to authenticate to it.
#[derive(Debug, Clone)]
pub struct ProviderAuthInfo {
    /// The unique provider ID (e.g. "openai", "anthropic", "google", "gemini-cli", "antigravity").
    pub provider_id: String,
    /// Human-readable label.
    pub label: String,
    /// Group label (first-level menu). E.g. "Google" groups google, gemini-cli, antigravity.
    pub group: String,
    /// Description / hint.
    pub hint: String,
    /// The authentication methods this provider supports (in order of preference).
    pub auth_methods: Vec<AuthMethod>,
}

/// Returns the full list of supported providers with their auth info.
pub fn all_provider_auth_info() -> Vec<ProviderAuthInfo> {
    vec![
        ProviderAuthInfo {
            provider_id: "openai".into(),
            label: "OpenAI API Key".into(),
            group: "OpenAI".into(),
            hint: "GPT-4o, o1, o3, o4-mini, GPT-4.1, GPT-5.2".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("OPENAI_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "anthropic".into(),
            label: "Anthropic API Key".into(),
            group: "Anthropic".into(),
            hint: "Claude Opus 4.6, Sonnet 4.5, Haiku 3.5".into(),
            auth_methods: vec![
                AuthMethod::ApiKey {
                    env_var: Some("ANTHROPIC_API_KEY".into()),
                    hint: None,
                },
                AuthMethod::SetupToken {
                    hint: Some("Run `claude setup-token` to generate a token".into()),
                },
                AuthMethod::OAuth {
                    hint: Some("Claude Pro/Max subscription OAuth".into()),
                },
            ],
        },
        ProviderAuthInfo {
            provider_id: "google".into(),
            label: "Gemini API Key".into(),
            group: "Google".into(),
            hint: "Gemini 2.5 Pro/Flash, Gemini 2.0 Flash".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("GEMINI_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "gemini-cli".into(),
            label: "Gemini CLI OAuth".into(),
            group: "Google".into(),
            hint: "Google Cloud Code Assist (free tier)".into(),
            auth_methods: vec![AuthMethod::OAuth {
                hint: Some("Google OAuth for Cloud Code Assist".into()),
            }],
        },
        ProviderAuthInfo {
            provider_id: "antigravity".into(),
            label: "Antigravity OAuth".into(),
            group: "Google".into(),
            hint: "Gemini 3, Claude, GPT-OSS via Google Cloud".into(),
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
            provider_id: "xai".into(),
            label: "xAI (Grok) API Key".into(),
            group: "xAI".into(),
            hint: "Grok 3, Grok 3 Mini".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("XAI_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "groq".into(),
            label: "Groq API Key".into(),
            group: "Groq".into(),
            hint: "Llama, Mixtral, Gemma (ultra-fast)".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("GROQ_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "together".into(),
            label: "Together AI API Key".into(),
            group: "Together".into(),
            hint: "Llama, DeepSeek, Qwen open models".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("TOGETHER_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "siliconflow".into(),
            label: "SiliconFlow API Key".into(),
            group: "SiliconFlow".into(),
            hint: "DeepSeek, Qwen models (CN)".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("SILICONFLOW_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "zhipuai".into(),
            label: "ZhipuAI API Key".into(),
            group: "ZhipuAI".into(),
            hint: "GLM-4 models".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("ZHIPUAI_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "fireworks".into(),
            label: "Fireworks API Key".into(),
            group: "Fireworks".into(),
            hint: "Llama, DeepSeek, Qwen".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("FIREWORKS_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "nebius".into(),
            label: "Nebius API Key".into(),
            group: "Nebius".into(),
            hint: "Llama, DeepSeek, Qwen".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("NEBIUS_API_KEY".into()),
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

/// Returns groups: each group is (group_label, Vec<ProviderAuthInfo>).
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
