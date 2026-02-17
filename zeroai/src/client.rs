use crate::mapper::{join_model_id, split_model_id};
use crate::providers::{Provider, ProviderError};
use crate::providers::google_gemini_cli::GoogleGeminiCliProvider;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::openai::OpenAiProvider;
use crate::providers::google::GoogleProvider;
use crate::types::*;
use futures::stream::{BoxStream, StreamExt};
use std::sync::Arc;
use std::collections::HashMap;

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

        let stream = provider.stream(&model_def, context, options);

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

        let mut message = provider.chat(&model_def, context, options).await?;

        let p_name = provider_name.to_string();
        let short_id = message.model.clone();
        message.model = join_model_id(&p_name, &short_id);
        message.provider = p_name;

        Ok(message)
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

pub struct AiClientBuilder {
    models: HashMap<String, ModelDef>,
}

impl AiClientBuilder {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
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
        providers.insert("qianfan".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("ollama".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("vllm".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("huggingface".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("github-copilot".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("amazon-bedrock".into(), openai.clone() as Arc<dyn Provider>);
        providers.insert("openai-codex".into(), openai.clone() as Arc<dyn Provider>);

        let anthropic = Arc::new(AnthropicProvider::new());
        providers.insert("anthropic".into(), anthropic.clone() as Arc<dyn Provider>);
        providers.insert("xiaomi".into(), anthropic.clone() as Arc<dyn Provider>);
        providers.insert("synthetic".into(), anthropic.clone() as Arc<dyn Provider>);
        providers.insert("cloudflare-ai-gateway".into(), anthropic.clone() as Arc<dyn Provider>);

        providers.insert("google".into(), Arc::new(GoogleProvider::new()) as Arc<dyn Provider>);
        providers.insert("gemini-cli".into(), Arc::new(GoogleGeminiCliProvider::new_gemini_cli()) as Arc<dyn Provider>);
        providers.insert("antigravity".into(), Arc::new(GoogleGeminiCliProvider::new_antigravity()) as Arc<dyn Provider>);

        AiClient {
            providers,
            models: self.models,
        }
    }
}
