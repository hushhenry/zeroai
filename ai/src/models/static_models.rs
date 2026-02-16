use crate::providers::anthropic::static_anthropic_models;
use crate::providers::google_gemini_cli::{static_antigravity_models, static_gemini_cli_models};
use crate::types::*;

/// Returns the full static model catalog for providers that don't support
/// dynamic model listing (or as a fallback).
pub fn all_static_models() -> Vec<ModelDef> {
    let mut models = Vec::new();
    models.extend(static_openai_models());
    models.extend(static_anthropic_models());
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
    models
}

/// Get static models for a given provider.
pub fn static_models_for_provider(provider: &str) -> Vec<ModelDef> {
    match provider {
        "openai" => static_openai_models(),
        "anthropic" => static_anthropic_models(),
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
        _ => Vec::new(),
    }
}

fn oai(id: &str, name: &str, reasoning: bool, ctx: u64, max_tok: u64) -> ModelDef {
    ModelDef {
        id: id.into(),
        name: name.into(),
        api: Api::OpenaiCompletions,
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning,
        input: vec![InputModality::Text, InputModality::Image],
        cost: ModelCost::default(),
        context_window: ctx,
        max_tokens: max_tok,
        headers: None,
    }
}

pub fn static_openai_models() -> Vec<ModelDef> {
    vec![
        oai("gpt-4o", "GPT-4o", false, 128000, 16384),
        oai("gpt-4o-mini", "GPT-4o Mini", false, 128000, 16384),
        oai("gpt-4-turbo", "GPT-4 Turbo", false, 128000, 4096),
        oai("o1", "o1", true, 200000, 100000),
        oai("o1-mini", "o1-mini", true, 128000, 65536),
        oai("o1-pro", "o1-pro", true, 200000, 100000),
        oai("o3", "o3", true, 200000, 100000),
        oai("o3-mini", "o3-mini", true, 200000, 65536),
        oai("o4-mini", "o4-mini", true, 200000, 100000),
        oai("gpt-4.1", "GPT-4.1", false, 1047576, 32768),
        oai("gpt-4.1-mini", "GPT-4.1 Mini", false, 1047576, 32768),
        oai("gpt-4.1-nano", "GPT-4.1 Nano", false, 1047576, 32768),
        oai("gpt-5.2-turbo", "GPT-5.2 Turbo", true, 1047576, 65536),
    ]
}

pub fn static_google_models() -> Vec<ModelDef> {
    let provider = "google";
    let base_url = "https://generativelanguage.googleapis.com/v1beta";
    let api = Api::GoogleGenerativeAi;

    vec![
        ModelDef {
            id: "gemini-2.5-pro-preview-06-05".into(),
            name: "Gemini 2.5 Pro".into(),
            api: api.clone(), provider: provider.into(), base_url: base_url.into(),
            reasoning: true, input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 1.25, output: 10.0, cache_read: 0.31, cache_write: 0.0 },
            context_window: 1048576, max_tokens: 65536, headers: None,
        },
        ModelDef {
            id: "gemini-2.5-flash-preview-05-20".into(),
            name: "Gemini 2.5 Flash".into(),
            api: api.clone(), provider: provider.into(), base_url: base_url.into(),
            reasoning: true, input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 0.15, output: 0.6, cache_read: 0.0375, cache_write: 0.0 },
            context_window: 1048576, max_tokens: 65536, headers: None,
        },
        ModelDef {
            id: "gemini-2.0-flash".into(),
            name: "Gemini 2.0 Flash".into(),
            api: api.clone(), provider: provider.into(), base_url: base_url.into(),
            reasoning: false, input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 0.1, output: 0.4, cache_read: 0.025, cache_write: 0.0 },
            context_window: 1048576, max_tokens: 8192, headers: None,
        },
        ModelDef {
            id: "gemini-2.0-flash-lite".into(),
            name: "Gemini 2.0 Flash Lite".into(),
            api: api.clone(), provider: provider.into(), base_url: base_url.into(),
            reasoning: false, input: vec![InputModality::Text, InputModality::Image],
            cost: ModelCost { input: 0.075, output: 0.3, cache_read: 0.0, cache_write: 0.0 },
            context_window: 1048576, max_tokens: 8192, headers: None,
        },
    ]
}

fn oai_compat(provider: &str, base_url: &str, id: &str, name: &str, reasoning: bool, ctx: u64, max_tok: u64) -> ModelDef {
    ModelDef {
        id: id.into(),
        name: name.into(),
        api: Api::OpenaiCompletions,
        provider: provider.into(),
        base_url: base_url.into(),
        reasoning,
        input: vec![InputModality::Text],
        cost: ModelCost::default(),
        context_window: ctx,
        max_tokens: max_tok,
        headers: None,
    }
}

pub fn static_deepseek_models() -> Vec<ModelDef> {
    let p = "deepseek";
    let url = "https://api.deepseek.com/v1";
    vec![
        oai_compat(p, url, "deepseek-chat", "DeepSeek V3", false, 128000, 8192),
        oai_compat(p, url, "deepseek-reasoner", "DeepSeek R1", true, 128000, 8192),
    ]
}

pub fn static_xai_models() -> Vec<ModelDef> {
    let p = "xai";
    let url = "https://api.x.ai/v1";
    vec![
        oai_compat(p, url, "grok-3", "Grok 3", true, 131072, 16384),
        oai_compat(p, url, "grok-3-mini", "Grok 3 Mini", true, 131072, 16384),
        oai_compat(p, url, "grok-2", "Grok 2", false, 131072, 8192),
    ]
}

pub fn static_groq_models() -> Vec<ModelDef> {
    let p = "groq";
    let url = "https://api.groq.com/openai/v1";
    vec![
        oai_compat(p, url, "llama-3.3-70b-versatile", "Llama 3.3 70B", false, 128000, 32768),
        oai_compat(p, url, "llama-3.1-8b-instant", "Llama 3.1 8B", false, 128000, 8192),
        oai_compat(p, url, "mixtral-8x7b-32768", "Mixtral 8x7B", false, 32768, 32768),
        oai_compat(p, url, "gemma2-9b-it", "Gemma 2 9B", false, 8192, 8192),
    ]
}

pub fn static_together_models() -> Vec<ModelDef> {
    let p = "together";
    let url = "https://api.together.xyz/v1";
    vec![
        oai_compat(p, url, "meta-llama/Llama-3.3-70B-Instruct-Turbo", "Llama 3.3 70B Turbo", false, 128000, 8192),
        oai_compat(p, url, "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo", "Llama 3.1 405B Turbo", false, 128000, 4096),
        oai_compat(p, url, "deepseek-ai/DeepSeek-R1", "DeepSeek R1", true, 128000, 8192),
        oai_compat(p, url, "Qwen/Qwen2.5-72B-Instruct-Turbo", "Qwen 2.5 72B Turbo", false, 128000, 8192),
    ]
}

pub fn static_siliconflow_models() -> Vec<ModelDef> {
    let p = "siliconflow";
    let url = "https://api.siliconflow.cn/v1";
    vec![
        oai_compat(p, url, "deepseek-ai/DeepSeek-V3", "DeepSeek V3", false, 128000, 8192),
        oai_compat(p, url, "deepseek-ai/DeepSeek-R1", "DeepSeek R1", true, 128000, 8192),
        oai_compat(p, url, "Qwen/Qwen2.5-72B-Instruct", "Qwen 2.5 72B", false, 128000, 8192),
    ]
}

pub fn static_zhipuai_models() -> Vec<ModelDef> {
    let p = "zhipuai";
    let url = "https://open.bigmodel.cn/api/paas/v4";
    vec![
        oai_compat(p, url, "glm-4-plus", "GLM-4 Plus", false, 128000, 4096),
        oai_compat(p, url, "glm-4-flash", "GLM-4 Flash", false, 128000, 4096),
        oai_compat(p, url, "glm-4-air", "GLM-4 Air", false, 128000, 4096),
    ]
}

pub fn static_fireworks_models() -> Vec<ModelDef> {
    let p = "fireworks";
    let url = "https://api.fireworks.ai/inference/v1";
    vec![
        oai_compat(p, url, "accounts/fireworks/models/llama-v3p3-70b-instruct", "Llama 3.3 70B", false, 128000, 16384),
        oai_compat(p, url, "accounts/fireworks/models/deepseek-r1", "DeepSeek R1", true, 128000, 8192),
        oai_compat(p, url, "accounts/fireworks/models/qwen2p5-72b-instruct", "Qwen 2.5 72B", false, 128000, 8192),
    ]
}

pub fn static_nebius_models() -> Vec<ModelDef> {
    let p = "nebius";
    let url = "https://api.studio.nebius.com/v1";
    vec![
        oai_compat(p, url, "meta-llama/Llama-3.3-70B-Instruct", "Llama 3.3 70B", false, 128000, 8192),
        oai_compat(p, url, "deepseek-ai/DeepSeek-R1", "DeepSeek R1", true, 128000, 8192),
        oai_compat(p, url, "Qwen/Qwen2.5-72B-Instruct", "Qwen 2.5 72B", false, 128000, 8192),
    ]
}

pub fn static_openrouter_models() -> Vec<ModelDef> {
    let p = "openrouter";
    let url = "https://openrouter.ai/api/v1";
    vec![
        oai_compat(p, url, "anthropic/claude-sonnet-4-0-20250514", "Claude Sonnet 4", true, 200000, 16384),
        oai_compat(p, url, "openai/gpt-4o", "GPT-4o", false, 128000, 16384),
        oai_compat(p, url, "google/gemini-2.5-pro-preview", "Gemini 2.5 Pro", true, 1048576, 65536),
        oai_compat(p, url, "deepseek/deepseek-r1", "DeepSeek R1", true, 128000, 8192),
    ]
}
