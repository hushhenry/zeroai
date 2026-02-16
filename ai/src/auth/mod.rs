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
        // OpenAI Group
        ProviderAuthInfo {
            provider_id: "openai".into(),
            label: "OpenAI API key".into(),
            group: "OpenAI".into(),
            hint: "Standard API key".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("OPENAI_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "openai-codex".into(),
            label: "OpenAI Codex (ChatGPT OAuth)".into(),
            group: "OpenAI".into(),
            hint: "Uses ChatGPT Plus/Pro session".into(),
            auth_methods: vec![AuthMethod::OAuth {
                hint: Some("OAuth flow for ChatGPT session".into()),
            }],
        },
        // Anthropic Group
        ProviderAuthInfo {
            provider_id: "anthropic".into(),
            label: "Anthropic API key".into(),
            group: "Anthropic".into(),
            hint: "setup-token + API key".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("ANTHROPIC_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "anthropic".into(),
            label: "Anthropic token (paste setup-token)".into(),
            group: "Anthropic".into(),
            hint: "setup-token + API key".into(),
            auth_methods: vec![AuthMethod::SetupToken {
                hint: Some("run `claude setup-token` elsewhere, then paste the token here".into()),
            }],
        },
        // vLLM Group
        ProviderAuthInfo {
            provider_id: "vllm".into(),
            label: "vLLM (custom URL + model)".into(),
            group: "vLLM".into(),
            hint: "Local/self-hosted OpenAI-compatible".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("VLLM_API_KEY".into()),
                hint: None,
            }],
        },
        // MiniMax Group
        ProviderAuthInfo {
            provider_id: "minimax".into(),
            label: "MiniMax M2.5".into(),
            group: "MiniMax".into(),
            hint: "M2.5 (recommended)".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("MINIMAX_API_KEY".into()),
                hint: None,
            }],
        },
        // Moonshot Group
        ProviderAuthInfo {
            provider_id: "moonshot".into(),
            label: "Kimi API key (.ai)".into(),
            group: "Moonshot AI (Kimi K2.5)".into(),
            hint: "Kimi K2.5 + Kimi Coding".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("MOONSHOT_API_KEY".into()),
                hint: None,
            }],
        },
        // Google Group
        ProviderAuthInfo {
            provider_id: "google".into(),
            label: "Google Gemini API key".into(),
            group: "Google".into(),
            hint: "Gemini API key + OAuth".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("GEMINI_API_KEY".into()),
                hint: None,
            }],
        },
        ProviderAuthInfo {
            provider_id: "antigravity".into(),
            label: "Google Antigravity OAuth".into(),
            group: "Google".into(),
            hint: "Gemini API key + OAuth".into(),
            auth_methods: vec![AuthMethod::OAuth {
                hint: Some("Uses the bundled Antigravity auth plugin".into()),
            }],
        },
        ProviderAuthInfo {
            provider_id: "gemini-cli".into(),
            label: "Google Gemini CLI OAuth".into(),
            group: "Google".into(),
            hint: "Gemini API key + OAuth".into(),
            auth_methods: vec![AuthMethod::OAuth {
                hint: Some("Uses the bundled Gemini CLI auth plugin".into()),
            }],
        },
        // xAI Group
        ProviderAuthInfo {
            provider_id: "xai".into(),
            label: "xAI (Grok) API key".into(),
            group: "xAI (Grok)".into(),
            hint: "API key".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("XAI_API_KEY".into()),
                hint: None,
            }],
        },
        // OpenRouter Group
        ProviderAuthInfo {
            provider_id: "openrouter".into(),
            label: "OpenRouter API key".into(),
            group: "OpenRouter".into(),
            hint: "API key".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("OPENROUTER_API_KEY".into()),
                hint: None,
            }],
        },
        // Qwen Group
        ProviderAuthInfo {
            provider_id: "qwen".into(),
            label: "Qwen OAuth".into(),
            group: "Qwen".into(),
            hint: "OAuth".into(),
            auth_methods: vec![AuthMethod::OAuth {
                hint: None,
            }],
        },
        // Qianfan Group
        ProviderAuthInfo {
            provider_id: "qianfan".into(),
            label: "Qianfan API key".into(),
            group: "Qianfan".into(),
            hint: "API key".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("QIANFAN_API_KEY".into()),
                hint: None,
            }],
        },
        // Copilot Group
        ProviderAuthInfo {
            provider_id: "github-copilot".into(),
            label: "GitHub Copilot (GitHub device login)".into(),
            group: "Copilot".into(),
            hint: "GitHub + local proxy".into(),
            auth_methods: vec![AuthMethod::OAuth {
                hint: Some("Uses GitHub device flow".into()),
            }],
        },
        // Xiaomi Group
        ProviderAuthInfo {
            provider_id: "xiaomi".into(),
            label: "Xiaomi API key".into(),
            group: "Xiaomi".into(),
            hint: "API key".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("XIAOMI_API_KEY".into()),
                hint: None,
            }],
        },
        // Synthetic Group
        ProviderAuthInfo {
            provider_id: "synthetic".into(),
            label: "Synthetic API key".into(),
            group: "Synthetic".into(),
            hint: "Anthropic-compatible (multi-model)".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: None,
                hint: None,
            }],
        },
        // Together AI Group
        ProviderAuthInfo {
            provider_id: "together".into(),
            label: "Together AI API key".into(),
            group: "Together AI".into(),
            hint: "API key".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("TOGETHER_API_KEY".into()),
                hint: Some("Access to Llama, DeepSeek, Qwen, and more open models".into()),
            }],
        },
        // Hugging Face Group
        ProviderAuthInfo {
            provider_id: "huggingface".into(),
            label: "Hugging Face API key (HF token)".into(),
            group: "Hugging Face".into(),
            hint: "Inference API (HF token)".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("HUGGINGFACE_API_KEY".into()),
                hint: Some("Inference Providers — OpenAI-compatible chat".into()),
            }],
        },
        // Venice AI Group
        ProviderAuthInfo {
            provider_id: "venice".into(),
            label: "Venice AI API key".into(),
            group: "Venice AI".into(),
            hint: "Privacy-focused (uncensored models)".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("VENICE_API_KEY".into()),
                hint: Some("Privacy-focused inference (uncensored models)".into()),
            }],
        },
        // Cloudflare Group
        ProviderAuthInfo {
            provider_id: "cloudflare-ai-gateway".into(),
            label: "Cloudflare AI Gateway".into(),
            group: "Cloudflare AI Gateway".into(),
            hint: "Account ID + Gateway ID + API key".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: None,
                hint: None,
            }],
        },
        // DeepSeek (Custom Addition)
        ProviderAuthInfo {
            provider_id: "deepseek".into(),
            label: "DeepSeek API key".into(),
            group: "DeepSeek".into(),
            hint: "DeepSeek V3, R1".into(),
            auth_methods: vec![AuthMethod::ApiKey {
                env_var: Some("DEEPSEEK_API_KEY".into()),
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
