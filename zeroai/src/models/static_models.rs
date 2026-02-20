use crate::auth;
use crate::providers::anthropic::{static_anthropic_models, static_anthropic_setup_token_models};
use crate::providers::google_gemini_cli::{static_antigravity_models, static_gemini_cli_models};
use crate::types::*;

/// Base URL for a provider (single source: auth::provider_base_url).
fn base_url(provider: &str) -> &'static str {
    auth::provider_base_url(provider).unwrap_or("https://api.example.com/v1")
}

pub fn all_static_models() -> Vec<ModelDef> {
    let mut models = Vec::new();
    models.extend(static_openai_models());
    models.extend(static_openai_codex_models());
    models.extend(static_anthropic_models());
    models.extend(static_anthropic_setup_token_models());
    models.extend(static_google_models());
    models.extend(static_gemini_cli_models());
    models.extend(static_antigravity_models());
    models.extend(static_deepseek_models());
    models.extend(static_xai_models());
    models.extend(static_groq_models());
    models.extend(static_together_models());
    models.extend(static_siliconflow_models());
    models.extend(static_zhipuai_models());
    models.extend(static_fireworks_models());
    models.extend(static_nebius_models());
    models.extend(static_openrouter_models());
    models.extend(static_minimax_models());
    models.extend(static_xiaomi_models());
    models.extend(static_moonshot_models());
    models.extend(static_qwen_portal_models());
    models.extend(static_qianfan_models());
    models.extend(static_synthetic_models());
    models.extend(static_cloudflare_models());
    models.extend(static_ollama_models());
    models.extend(static_vllm_models());
    models.extend(static_huggingface_models());
    models.extend(static_copilot_models());
    models.extend(static_bedrock_models());
    models
}

pub fn static_models_for_provider(provider: &str) -> Vec<ModelDef> {
    match provider {
        "openai" => static_openai_models(),
        "openai-codex" => static_openai_codex_models(),
        "anthropic" => static_anthropic_models(),
        "anthropic-setup-token" => static_anthropic_setup_token_models(),
        "google" => static_google_models(),
        "gemini-cli" => static_gemini_cli_models(),
        "antigravity" => static_antigravity_models(),
        "deepseek" => static_deepseek_models(),
        "xai" => static_xai_models(),
        "groq" => static_groq_models(),
        "together" => static_together_models(),
        "siliconflow" => static_siliconflow_models(),
        "zhipuai" => static_zhipuai_models(),
        "fireworks" => static_fireworks_models(),
        "nebius" => static_nebius_models(),
        "openrouter" => static_openrouter_models(),
        "minimax" => static_minimax_models(),
        "xiaomi" => static_xiaomi_models(),
        "moonshot" => static_moonshot_models(),
        "qwen-portal" => static_qwen_portal_models(),
        "qianfan" => static_qianfan_models(),
        "synthetic" => static_synthetic_models(),
        "cloudflare-ai-gateway" => static_cloudflare_models(),
        "ollama" => static_ollama_models(),
        "vllm" => static_vllm_models(),
        "huggingface" => static_huggingface_models(),
        "github-copilot" => static_copilot_models(),
        "amazon-bedrock" => static_bedrock_models(),
        _ => Vec::new(),
    }
}

fn oai(provider: &str, base_url: &str, id: &str, name: &str, reasoning: bool, ctx: u64, max_tok: u64) -> ModelDef {
    ModelDef {
        id: id.into(),
        name: name.into(),
        api: Api::OpenaiCompletions,
        provider: provider.into(),
        base_url: base_url.into(),
        reasoning,
        input: vec![InputModality::Text, InputModality::Image],
        cost: ModelCost::default(),
        context_window: ctx,
        max_tokens: max_tok,
        headers: None,
    }
}

fn oai_responses(provider: &str, base_url: &str, id: &str, name: &str, reasoning: bool, ctx: u64, max_tok: u64) -> ModelDef {
    ModelDef {
        id: id.into(),
        name: name.into(),
        api: Api::OpenaiResponses,
        provider: provider.into(),
        base_url: base_url.into(),
        reasoning,
        input: vec![InputModality::Text, InputModality::Image],
        cost: ModelCost::default(),
        context_window: ctx,
        max_tokens: max_tok,
        headers: None,
    }
}

fn ant(provider: &str, base_url: &str, id: &str, name: &str, reasoning: bool, ctx: u64, max_tok: u64) -> ModelDef {
    ModelDef {
        id: id.into(),
        name: name.into(),
        api: Api::AnthropicMessages,
        provider: provider.into(),
        base_url: base_url.into(),
        reasoning,
        input: vec![InputModality::Text, InputModality::Image],
        cost: ModelCost::default(),
        context_window: ctx,
        max_tokens: max_tok,
        headers: None,
    }
}

pub fn static_openai_models() -> Vec<ModelDef> {
    let p = "openai";
    let url = base_url(p);
    vec![
        oai(p, url, "gpt-4o", "GPT-4o", false, 128000, 16384),
        oai(p, url, "gpt-4o-mini", "GPT-4o Mini", false, 128000, 16384),
        oai(p, url, "o1", "o1", true, 200000, 100000),
        oai(p, url, "o3-mini", "o3-mini", true, 200000, 65536),

        // API-key Codex models: use OpenAI Responses API on api.openai.com.
        oai_responses(p, url, "gpt-5.2-codex", "GPT-5.2 Codex", true, 200000, 65536),
        oai_responses(p, url, "gpt-5.3-codex", "GPT-5.3 Codex", true, 200000, 65536),
    ]
}

/// OpenAI Codex (ChatGPT OAuth): token has no api.model.read; use static list only.
pub fn static_openai_codex_models() -> Vec<ModelDef> {
    let p = "openai-codex";
    let url = base_url(p);
    vec![
        // Codex OAuth runs on the ChatGPT backend "responses" API.
        oai_responses(p, url, "gpt-5.2", "GPT-5.2", true, 200000, 65536),
        oai_responses(p, url, "gpt-5.2-codex", "GPT-5.2 Codex", true, 200000, 65536),
        oai_responses(p, url, "gpt-5.3-codex", "GPT-5.3 Codex", true, 200000, 65536),
        // Keep a few non-codex IDs for convenience; still routed via the same backend for this provider.
        oai_responses(p, url, "gpt-4o", "GPT-4o", false, 128000, 16384),
        oai_responses(p, url, "gpt-4o-mini", "GPT-4o Mini", false, 128000, 16384),
        oai_responses(p, url, "o1", "o1", true, 200000, 100000),
        oai_responses(p, url, "o3-mini", "o3-mini", true, 200000, 65536),
    ]
}

pub fn static_google_models() -> Vec<ModelDef> {
    let provider = "google";
    let base_url = base_url(provider);
    let api = Api::GoogleGenerativeAi;
    vec![
        ModelDef {
            id: "gemini-2.0-flash".into(),
            name: "Gemini 2.0 Flash".into(),
            api: api.clone(), provider: provider.into(), base_url: base_url.into(),
            reasoning: false, input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost::default(),
            context_window: 1048576, max_tokens: 8192, headers: None,
        },
    ]
}

pub fn static_deepseek_models() -> Vec<ModelDef> {
    let p = "deepseek";
    let url = base_url(p);
    vec![
        oai(p, url, "deepseek-chat", "DeepSeek V3", false, 128000, 8192),
        oai(p, url, "deepseek-reasoner", "DeepSeek R1", true, 128000, 8192),
    ]
}

pub fn static_xai_models() -> Vec<ModelDef> {
    let p = "xai";
    let url = base_url(p);
    vec![
        oai(p, url, "grok-3", "Grok 3", true, 131072, 16384),
        oai(p, url, "grok-3-mini", "Grok 3 Mini", true, 131072, 16384),
    ]
}

pub fn static_groq_models() -> Vec<ModelDef> {
    let p = "groq";
    let url = base_url(p);
    vec![
        oai(p, url, "llama-3.3-70b-versatile", "Llama 3.3 70B", false, 128000, 32768),
    ]
}

pub fn static_together_models() -> Vec<ModelDef> {
    let p = "together";
    let url = base_url(p);
    vec![
        oai(p, url, "deepseek-ai/DeepSeek-R1", "DeepSeek R1", true, 128000, 8192),
    ]
}

pub fn static_siliconflow_models() -> Vec<ModelDef> {
    let p = "siliconflow";
    let url = base_url(p);
    vec![
        oai(p, url, "deepseek-ai/DeepSeek-V3", "DeepSeek V3", false, 128000, 8192),
    ]
}

pub fn static_zhipuai_models() -> Vec<ModelDef> {
    let p = "zhipuai";
    let url = base_url(p);
    vec![
        oai(p, url, "glm-4-plus", "GLM-4 Plus", false, 128000, 4096),
    ]
}

pub fn static_fireworks_models() -> Vec<ModelDef> {
    let p = "fireworks";
    let url = base_url(p);
    vec![
        oai(p, url, "accounts/fireworks/models/deepseek-r1", "DeepSeek R1", true, 128000, 8192),
    ]
}

pub fn static_nebius_models() -> Vec<ModelDef> {
    let p = "nebius";
    let url = base_url(p);
    vec![
        oai(p, url, "deepseek-ai/DeepSeek-R1", "DeepSeek R1", true, 128000, 8192),
    ]
}

pub fn static_openrouter_models() -> Vec<ModelDef> {
    let p = "openrouter";
    let url = base_url(p);
    vec![
        oai(p, url, "google/gemini-2.5-pro-preview", "Gemini 2.5 Pro", true, 1048576, 65536),
    ]
}

pub fn static_minimax_models() -> Vec<ModelDef> {
    let p = "minimax";
    let url = base_url(p);
    vec![
        oai(p, url, "MiniMax-M2.1", "MiniMax M2.1", false, 200000, 8192),
        oai(p, url, "MiniMax-M2.5", "MiniMax M2.5", true, 200000, 8192),
    ]
}

pub fn static_xiaomi_models() -> Vec<ModelDef> {
    let p = "xiaomi";
    let url = base_url(p);
    vec![
        oai(p, url, "mimo-v2-flash", "Xiaomi MiMo V2 Flash", false, 262144, 8192),
    ]
}

pub fn static_moonshot_models() -> Vec<ModelDef> {
    let p = "moonshot";
    let url = base_url(p);
    vec![
        oai(p, url, "kimi-k2.5", "Kimi K2.5", false, 256000, 8192),
    ]
}

pub fn static_qianfan_models() -> Vec<ModelDef> {
    let p = "qianfan";
    let url = base_url(p);
    vec![
        oai(p, url, "deepseek-v3.2", "DEEPSEEK V3.2", true, 98304, 32768),
    ]
}

/// Qwen Portal (OAuth): token is for portal.qwen.ai only; static list per openclaw.
pub fn static_qwen_portal_models() -> Vec<ModelDef> {
    let p = "qwen-portal";
    let url = base_url(p);
    vec![
        oai(p, url, "coder-model", "Qwen Coder", false, 128000, 8192),
        oai(p, url, "vision-model", "Qwen Vision", false, 128000, 8192),
    ]
}

pub fn static_synthetic_models() -> Vec<ModelDef> {
    let p = "synthetic";
    let url = base_url(p);
    vec![
        ant(p, url, "synthetic-model", "Synthetic Model", false, 128000, 8192),
    ]
}

pub fn static_cloudflare_models() -> Vec<ModelDef> {
    let p = "cloudflare-ai-gateway";
    let url = base_url(p);
    vec![
        ant(p, url, "cloudflare-model", "Cloudflare AI Gateway", false, 128000, 8192),
    ]
}

pub fn static_ollama_models() -> Vec<ModelDef> {
    let p = "ollama";
    let url = base_url(p);
    vec![
        oai(p, url, "llama3", "Llama 3 (Ollama)", false, 128000, 8192),
    ]
}

pub fn static_vllm_models() -> Vec<ModelDef> {
    let p = "vllm";
    let url = base_url(p);
    vec![
        oai(p, url, "vllm-model", "vLLM Model", false, 128000, 8192),
    ]
}

pub fn static_huggingface_models() -> Vec<ModelDef> {
    let p = "huggingface";
    let url = base_url(p);
    vec![
        oai(p, url, "hf-model", "HuggingFace Model", false, 128000, 8192),
    ]
}

pub fn static_copilot_models() -> Vec<ModelDef> {
    let p = "github-copilot";
    let url = base_url(p);
    vec![
        oai(p, url, "gpt-4o", "Copilot GPT-4o", false, 128000, 8192),
    ]
}

pub fn static_bedrock_models() -> Vec<ModelDef> {
    let p = "amazon-bedrock";
    let url = base_url(p);
    vec![
        oai(p, url, "anthropic.claude-3-5-sonnet-20241022-v2:0", "Bedrock Claude 3.5 Sonnet", false, 200000, 8192),
    ]
}
