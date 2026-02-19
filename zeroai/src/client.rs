use crate::auth::sniff;
use crate::mapper::{join_model_id, split_model_id};
use crate::providers::compatible::{AuthStyle, OpenAiCompatibleProvider};
use crate::providers::retry::{self, compute_backoff, is_non_retryable};
use crate::providers::{Provider, ProviderError};
use crate::providers::google_gemini_cli::GoogleGeminiCliProvider;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::openai::OpenAiProvider;
use crate::providers::google::GoogleProvider;
use crate::types::*;
use futures::stream::{BoxStream, StreamExt};
use std::sync::Arc;
use std::collections::HashMap;
use std::time::Duration;

/// High-level AI client that coordinates multiple providers and model mapping.
#[derive(Clone)]
pub struct AiClient {
    providers: HashMap<String, Arc<dyn Provider>>,
    models: HashMap<String, ModelDef>,
}

impl AiClient {
    pub fn builder() -> AiClientBuilder {
        AiClientBuilder::new()
    }

    /// Return a reference to the internal models map.
    pub fn models(&self) -> &HashMap<String, ModelDef> {
        &self.models
    }

    /// Look up a model definition by full ID (e.g. "openai/gpt-4o").
    pub fn get_model(&self, full_model_id: &str) -> Option<&ModelDef> {
        self.models.get(full_model_id)
    }

    pub fn stream(
        &self,
        full_model_id: &str,
        context: &ChatContext,
        options: &RequestOptions,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError> {
        let (provider_name, model_def) = self.resolve(full_model_id)?;

        let provider = self.providers.get(provider_name).ok_or_else(|| {
            ProviderError::Other(format!("Unknown provider: {}", provider_name))
        })?;

        let stream: BoxStream<'static, Result<StreamEvent, ProviderError>> = match &options.retry_config {
            Some(config) => {
                let provider = Arc::clone(provider);
                let model_def = model_def.clone();
                let context = context.clone();
                let options = options.clone();
                let config = config.clone();
                retry::retry_stream(provider, model_def, context, options, config)
            }
            None => provider.stream(&model_def, context, options),
        };

        let p_name = provider_name.to_string();
        let mapped = stream.map(move |event| match event {
            Ok(StreamEvent::Done { mut message }) => {
                let short_id = message.model.clone();
                message.model = join_model_id(&p_name, &short_id);
                message.provider = p_name.clone();
                Ok(StreamEvent::Done { message })
            }
            Ok(StreamEvent::Error { mut message }) => {
                let short_id = message.model.clone();
                message.model = join_model_id(&p_name, &short_id);
                message.provider = p_name.clone();
                Ok(StreamEvent::Error { message })
            }
            other => other,
        });

        Ok(Box::pin(mapped))
    }

    pub async fn chat(
        &self,
        full_model_id: &str,
        context: &ChatContext,
        options: &RequestOptions,
    ) -> Result<AssistantMessage, ProviderError> {
        let (provider_name, model_def) = self.resolve(full_model_id)?;

        let provider = self.providers.get(provider_name).ok_or_else(|| {
            ProviderError::Other(format!("Unknown provider: {}", provider_name))
        })?;

        let config = options.retry_config.as_ref();
        let max_retries = config.map(|c| c.max_retries).unwrap_or(0);
        let mut backoff_ms = config.map(|c| c.base_backoff_ms).unwrap_or(1000);

        let mut last_err = None;
        for attempt in 0..=max_retries {
            match provider.chat(&model_def, context, options).await {
                Ok(mut message) => {
                    let p_name = provider_name.to_string();
                    let short_id = message.model.clone();
                    message.model = join_model_id(&p_name, &short_id);
                    message.provider = p_name;
                    return Ok(message);
                }
                Err(e) => {
                    last_err = Some(e);
                    let err = last_err.as_ref().unwrap();
                    if is_non_retryable(err) || attempt >= max_retries {
                        break;
                    }
                    let wait = config
                        .map(|c| compute_backoff(c, backoff_ms, err))
                        .unwrap_or(backoff_ms);
                    tokio::time::sleep(Duration::from_millis(wait)).await;
                    backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| ProviderError::Other("no attempt".into())))
    }

    /// Resolve a full model ID to (provider_name, ModelDef).
    fn resolve<'a>(&'a self, full_model_id: &'a str) -> Result<(&'a str, ModelDef), ProviderError> {
        let (provider_name, _short_id) = split_model_id(full_model_id).ok_or_else(|| {
            ProviderError::Other(format!("Invalid model ID format: {}", full_model_id))
        })?;

        let model_def = self.models.get(full_model_id).ok_or_else(|| {
            ProviderError::Other(format!("Model not registered: {}", full_model_id))
        })?;

        Ok((provider_name, model_def.clone()))
    }
}

/// Custom provider registration for build().
struct CustomProviderReg {
    name: String,
    base_url: String,
    api_key: Option<String>,
    models_url: Option<String>,
}

pub struct AiClientBuilder {
    models: HashMap<String, ModelDef>,
    custom_providers: Vec<CustomProviderReg>,
}

impl AiClientBuilder {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            custom_providers: Vec::new(),
        }
    }

    /// Register a single model under its full ID (`provider/model`).
    pub fn with_model(mut self, full_id: String, def: ModelDef) -> Self {
        self.models.insert(full_id, def);
        self
    }

    /// Register multiple models at once. Each model's full ID is derived from
    /// `provider/id` fields on the `ModelDef`.
    pub fn with_models(mut self, models: impl IntoIterator<Item = (String, ModelDef)>) -> Self {
        self.models.extend(models);
        self
    }

    /// Add an OpenAI-compatible custom provider with a fixed list of models.
    pub fn with_custom_provider(
        mut self,
        name: &str,
        base_url: &str,
        api_key: Option<&str>,
        models: Vec<ModelDef>,
    ) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        for mut def in models {
            def.provider = name.to_string();
            def.base_url = base_url.clone();
            let full_id = format!("{}/{}", name, def.id);
            self.models.insert(full_id, def);
        }
        self.custom_providers.push(CustomProviderReg {
            name: name.to_string(),
            base_url,
            api_key: api_key.map(String::from),
            models_url: None,
        });
        self
    }

    /// Add an OpenAI-compatible custom provider with dynamic model discovery via GET models_url.
    pub fn with_custom_provider_with_models_url(
        mut self,
        name: &str,
        base_url: &str,
        api_key: Option<&str>,
        models_url: &str,
    ) -> Self {
        self.custom_providers.push(CustomProviderReg {
            name: name.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.map(String::from),
            models_url: Some(models_url.to_string()),
        });
        self
    }

    pub fn build(self) -> AiClient {
        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();

        let openai = Arc::new(OpenAiProvider::new());
        providers.insert("openai".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("deepseek".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("xai".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("groq".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("together".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("siliconflow".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("zhipuai".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("fireworks".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("nebius".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("openrouter".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("minimax".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("moonshot".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("qwen".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("qwen-portal".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("qianfan".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("ollama".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("vllm".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("huggingface".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("github-copilot".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("amazon-bedrock".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("openai-codex".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("xiaomi".into(), openai.clone() as Arc<dyn Provider>);

        let anthropic = Arc::new(AnthropicProvider::new());
        providers.insert("anthropic".into(), anthropic.clone() as Arc<dyn Provider>);
        providers.insert("anthropic-setup-token".into(), anthropic.clone() as Arc<dyn Provider>);
        providers.insert("synthetic".into(), anthropic.clone() as Arc<dyn Provider>);
        providers.insert("cloudflare-ai-gateway".into(), anthropic.clone() as Arc<dyn Provider>);

        providers.insert("google".into(), Arc::new(GoogleProvider::new()) as Arc<dyn Provider>);
        providers.insert("gemini-cli".into(), Arc::new(GoogleGeminiCliProvider::new_gemini_cli()) as Arc<dyn Provider>);
        providers.insert("antigravity".into(), Arc::new(GoogleGeminiCliProvider::new_antigravity()) as Arc<dyn Provider>);

        // Register custom providers (with_custom_provider / with_custom_provider_with_models_url)
        for reg in &self.custom_providers {
            let mut p = OpenAiCompatibleProvider::new(
                &reg.name,
                &reg.base_url,
                reg.api_key.as_deref(),
                AuthStyle::Bearer,
            );
            if let Some(ref url) = reg.models_url {
                p = p.with_models_url(url);
            }
            providers.insert(reg.name.clone(), Arc::new(p) as Arc<dyn Provider>);
        }

        // Auto-create provider for "custom:https://..." model IDs
        for full_id in self.models.keys() {
            if let Some((provider_name, _)) = split_model_id(full_id) {
                if provider_name.starts_with("custom:") && !providers.contains_key(provider_name) {
                    let base_url = provider_name.strip_prefix("custom:").unwrap_or("").trim();
                    if !base_url.is_empty() && (base_url.starts_with("http://") || base_url.starts_with("https://")) {
                        let api_key = sniff::resolve_credential(provider_name, None);
                        let p = OpenAiCompatibleProvider::new(
                            provider_name,
                            base_url,
                            api_key.as_deref(),
                            AuthStyle::Bearer,
                        );
                        providers.insert(provider_name.to_string(), Arc::new(p) as Arc<dyn Provider>);
                    }
                }
            }
        }

        AiClient {
            providers,
            models: self.models,
        }
    }
}
