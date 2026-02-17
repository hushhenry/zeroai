use super::{ApiKeyCredential, Credential, OAuthCredential};
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Environment variable sniffing
// ---------------------------------------------------------------------------

/// Known environment variables per provider (expanded to match zeroclaw).
const ENV_VAR_MAP: &[(&str, &str)] = &[
    ("openai", "OPENAI_API_KEY"),
    ("anthropic", "ANTHROPIC_API_KEY"),
    ("google", "GEMINI_API_KEY"),
    ("deepseek", "DEEPSEEK_API_KEY"),
    ("groq", "GROQ_API_KEY"),
    ("together", "TOGETHER_API_KEY"),
    ("together-ai", "TOGETHER_API_KEY"),
    ("siliconflow", "SILICONFLOW_API_KEY"),
    ("zhipuai", "ZHIPUAI_API_KEY"),
    ("fireworks", "FIREWORKS_API_KEY"),
    ("fireworks-ai", "FIREWORKS_API_KEY"),
    ("nebius", "NEBIUS_API_KEY"),
    ("xai", "XAI_API_KEY"),
    ("grok", "XAI_API_KEY"),
    ("openrouter", "OPENROUTER_API_KEY"),
    ("mistral", "MISTRAL_API_KEY"),
    ("huggingface", "HF_TOKEN"),
    ("venice", "VENICE_API_KEY"),
    ("perplexity", "PERPLEXITY_API_KEY"),
    ("cohere", "COHERE_API_KEY"),
    ("moonshot", "MOONSHOT_API_KEY"),
    ("kimi", "MOONSHOT_API_KEY"),
    ("glm", "GLM_API_KEY"),
    ("zhipu", "GLM_API_KEY"),
    ("minimax", "MINIMAX_API_KEY"),
    ("qianfan", "QIANFAN_API_KEY"),
    ("baidu", "QIANFAN_API_KEY"),
    ("qwen", "DASHSCOPE_API_KEY"),
    ("dashscope", "DASHSCOPE_API_KEY"),
    ("qwen-intl", "DASHSCOPE_API_KEY"),
    ("dashscope-intl", "DASHSCOPE_API_KEY"),
    ("qwen-us", "DASHSCOPE_API_KEY"),
    ("dashscope-us", "DASHSCOPE_API_KEY"),
    ("zai", "ZAI_API_KEY"),
    ("nvidia", "NVIDIA_API_KEY"),
    ("nvidia-nim", "NVIDIA_API_KEY"),
    ("build.nvidia.com", "NVIDIA_API_KEY"),
    ("synthetic", "SYNTHETIC_API_KEY"),
    ("opencode", "OPENCODE_API_KEY"),
    ("opencode-zen", "OPENCODE_API_KEY"),
    ("vercel", "VERCEL_API_KEY"),
    ("vercel-ai", "VERCEL_API_KEY"),
    ("cloudflare", "CLOUDFLARE_API_KEY"),
    ("cloudflare-ai", "CLOUDFLARE_API_KEY"),
    ("cloudflare-ai-gateway", "CLOUDFLARE_API_KEY"),
    ("github-copilot", "GITHUB_COPILOT_API_KEY"),
    ("amazon-bedrock", "AWS_ACCESS_KEY_ID"),
];

/// Return provider-specific env var names for resolution (zeroclaw order).
fn provider_env_candidates(name: &str) -> &'static [&'static str] {
    match name {
        "anthropic" => &["ANTHROPIC_API_KEY"],
        "openrouter" => &["OPENROUTER_API_KEY"],
        "openai" => &["OPENAI_API_KEY"],
        "venice" => &["VENICE_API_KEY"],
        "groq" => &["GROQ_API_KEY"],
        "mistral" => &["MISTRAL_API_KEY"],
        "deepseek" => &["DEEPSEEK_API_KEY"],
        "xai" | "grok" => &["XAI_API_KEY"],
        "together" | "together-ai" => &["TOGETHER_API_KEY"],
        "fireworks" | "fireworks-ai" => &["FIREWORKS_API_KEY"],
        "perplexity" => &["PERPLEXITY_API_KEY"],
        "cohere" => &["COHERE_API_KEY"],
        "moonshot" | "kimi" => &["MOONSHOT_API_KEY"],
        "glm" | "zhipu" | "zhipuai" => &["GLM_API_KEY", "ZHIPUAI_API_KEY"],
        "minimax" => &["MINIMAX_API_KEY"],
        "qianfan" | "baidu" => &["QIANFAN_API_KEY"],
        "qwen" | "dashscope" | "qwen-intl" | "dashscope-intl" | "qwen-us" | "dashscope-us" => {
            &["DASHSCOPE_API_KEY"]
        }
        "zai" | "z.ai" => &["ZAI_API_KEY"],
        "nvidia" | "nvidia-nim" | "build.nvidia.com" => &["NVIDIA_API_KEY"],
        "synthetic" => &["SYNTHETIC_API_KEY"],
        "opencode" | "opencode-zen" => &["OPENCODE_API_KEY"],
        "vercel" | "vercel-ai" => &["VERCEL_API_KEY"],
        "cloudflare" | "cloudflare-ai" | "cloudflare-ai-gateway" => &["CLOUDFLARE_API_KEY"],
        "google" => &["GEMINI_API_KEY"],
        "huggingface" => &["HF_TOKEN"],
        "siliconflow" => &["SILICONFLOW_API_KEY"],
        "nebius" => &["NEBIUS_API_KEY"],
        "github-copilot" => &["GITHUB_COPILOT_API_KEY"],
        "amazon-bedrock" => &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"],
        _ => &[],
    }
}

/// Resolve API key for a provider. Resolution order (same as zeroclaw):
/// 1. Explicit override (trimmed, ignored if empty)
/// 2. Provider-specific environment variable(s)
/// 3. Generic fallback: ZEROAI_API_KEY, API_KEY
///
/// Does not include file-based credentials; use `sniff_external_credential` for that.
pub fn resolve_credential(provider_name: &str, override_key: Option<&str>) -> Option<String> {
    if let Some(raw) = override_key {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }

    for &env_var in provider_env_candidates(provider_name) {
        if let Ok(val) = std::env::var(env_var) {
            let val = val.trim();
            if !val.is_empty() {
                return Some(val.to_owned());
            }
        }
    }

    for &env_var in &["ZEROAI_API_KEY", "API_KEY"] {
        if let Ok(val) = std::env::var(env_var) {
            let val = val.trim();
            if !val.is_empty() {
                return Some(val.to_owned());
            }
        }
    }

    None
}

/// Try to get an API key from environment variables for the given provider.
pub fn env_api_key(provider_id: &str) -> Option<String> {
    resolve_credential(provider_id, None)
}

/// Returns all environment variable mappings: (provider_id, env_var_name).
pub fn all_env_var_mappings() -> Vec<(String, String)> {
    ENV_VAR_MAP
        .iter()
        .map(|(p, e)| (p.to_string(), e.to_string()))
        .collect()
}

/// Sniff all environment variables and return found credentials.
pub fn sniff_all_env_vars() -> HashMap<String, String> {
    let mut found = HashMap::new();
    for (provider, env_var) in ENV_VAR_MAP {
        if let Ok(val) = std::env::var(env_var) {
            if !val.is_empty() {
                found.insert(provider.to_string(), val);
            }
        }
    }
    found
}

// ---------------------------------------------------------------------------
// External credential file sniffing
// ---------------------------------------------------------------------------

/// Known external credential file paths.
fn external_credential_paths() -> Vec<ExternalCredFile> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

    vec![
        ExternalCredFile {
            provider: "gemini-cli".into(),
            path: home.join(".gemini").join("oauth_creds.json"),
            kind: CredFileKind::GeminiOAuth,
        },
        ExternalCredFile {
            provider: "gemini-cli".into(),
            path: home
                .join(".config")
                .join("gcloud")
                .join("application_default_credentials.json"),
            kind: CredFileKind::GCloudADC,
        },
        ExternalCredFile {
            provider: "anthropic".into(),
            path: home.join(".anthropic").join("config.json"),
            kind: CredFileKind::AnthropicConfig,
        },
        ExternalCredFile {
            provider: "openai".into(),
            path: home.join(".openai").join("auth.json"),
            kind: CredFileKind::OpenAiAuth,
        },
    ]
}

struct ExternalCredFile {
    provider: String,
    path: PathBuf,
    kind: CredFileKind,
}

enum CredFileKind {
    GeminiOAuth,
    GCloudADC,
    AnthropicConfig,
    OpenAiAuth,
}

/// Returns all known external credential file scan paths: (provider_id, path).
pub fn all_external_credential_paths() -> Vec<(String, PathBuf)> {
    external_credential_paths()
        .into_iter()
        .map(|f| (f.provider, f.path))
        .collect()
}

/// Try to sniff an external credential file for the given provider.
pub fn sniff_external_credential(provider_id: &str) -> Option<Credential> {
    for file in external_credential_paths() {
        if file.provider != provider_id {
            continue;
        }
        if !file.path.exists() {
            continue;
        }

        let content = std::fs::read_to_string(&file.path).ok()?;

        match file.kind {
            CredFileKind::GeminiOAuth => {
                return parse_gemini_oauth_creds(&content);
            }
            CredFileKind::GCloudADC => {
                return parse_gcloud_adc(&content);
            }
            CredFileKind::AnthropicConfig => {
                return parse_anthropic_config(&content);
            }
            CredFileKind::OpenAiAuth => {
                return parse_openai_auth(&content);
            }
        }
    }
    None
}

/// Sniff all external credential files and return found credentials.
pub fn sniff_all_external_credentials() -> HashMap<String, Credential> {
    let mut found = HashMap::new();
    for file in external_credential_paths() {
        if !file.path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(&file.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let cred = match file.kind {
            CredFileKind::GeminiOAuth => parse_gemini_oauth_creds(&content),
            CredFileKind::GCloudADC => parse_gcloud_adc(&content),
            CredFileKind::AnthropicConfig => parse_anthropic_config(&content),
            CredFileKind::OpenAiAuth => parse_openai_auth(&content),
        };

        if let Some(c) = cred {
            found.insert(file.provider.clone(), c);
        }
    }
    found
}

// ---------------------------------------------------------------------------
// File parsers
// ---------------------------------------------------------------------------

/// Parse ~/.gemini/oauth_creds.json
fn parse_gemini_oauth_creds(content: &str) -> Option<Credential> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct GeminiOAuth {
        refresh_token: Option<String>,
        access_token: Option<String>,
        #[serde(default)]
        expiry: Option<String>,
        client_id: Option<String>,
        client_secret: Option<String>,
    }

    let creds: GeminiOAuth = serde_json::from_str(content).ok()?;
    let refresh = creds.refresh_token?;
    let access = creds.access_token.unwrap_or_default();

    // Parse expiry
    let expires = creds
        .expiry
        .and_then(|e| {
            chrono::DateTime::parse_from_rfc3339(&e)
                .ok()
                .map(|dt| dt.timestamp_millis())
        })
        .unwrap_or(0);

    Some(Credential::OAuth(OAuthCredential {
        refresh,
        access,
        expires,
        extra: HashMap::new(),
    }))
}

/// Parse ~/.config/gcloud/application_default_credentials.json
fn parse_gcloud_adc(content: &str) -> Option<Credential> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct ADC {
        client_id: Option<String>,
        client_secret: Option<String>,
        refresh_token: Option<String>,
        #[serde(rename = "type")]
        cred_type: Option<String>,
    }

    let creds: ADC = serde_json::from_str(content).ok()?;
    let refresh = creds.refresh_token?;

    Some(Credential::OAuth(OAuthCredential {
        refresh,
        access: String::new(),
        expires: 0,
        extra: HashMap::new(),
    }))
}

/// Parse ~/.anthropic/config.json
fn parse_anthropic_config(content: &str) -> Option<Credential> {
    #[derive(serde::Deserialize)]
    struct AnthropicConfig {
        api_key: Option<String>,
        oauth_token: Option<String>,
    }

    let config: AnthropicConfig = serde_json::from_str(content).ok()?;

    if let Some(key) = config.api_key {
        if !key.is_empty() {
            return Some(Credential::ApiKey(ApiKeyCredential { key }));
        }
    }
    if let Some(token) = config.oauth_token {
        if !token.is_empty() {
            return Some(Credential::ApiKey(ApiKeyCredential { key: token }));
        }
    }
    None
}

/// Parse ~/.openai/auth.json
fn parse_openai_auth(content: &str) -> Option<Credential> {
    #[derive(serde::Deserialize)]
    struct OpenAiAuth {
        api_key: Option<String>,
    }

    let auth: OpenAiAuth = serde_json::from_str(content).ok()?;
    let key = auth.api_key?;
    if key.is_empty() {
        return None;
    }
    Some(Credential::ApiKey(ApiKeyCredential { key }))
}
