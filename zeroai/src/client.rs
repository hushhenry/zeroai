use crate::mapper::ModelMapper;
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
    mapper: ModelMapper,
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl AiClient {
    pub fn builder() -> AiClientBuilder {
        AiClientBuilder::new()
    }

    pub fn stream(
        &self,
        full_model_id: &str,
        model_def: &ModelDef,
        context: &ChatContext,
        options: &StreamOptions,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError> {
        let (provider_name, _short_model_id) = self.mapper.split_id(full_model_id).ok_or_else(|| {
            ProviderError::Other(format!("Invalid model ID format: {}", full_model_id))
        })?;

        // Resolve provider
        let provider = self.providers.get(provider_name).ok_or_else(|| {
            ProviderError::Other(format!("Unknown provider: {}", provider_name))
        })?;

        // Call the provider
        let stream = provider.stream(model_def, context, options);
        
        // Hook the response to add provider prefix back to the model ID
        let p_name = provider_name.to_string();
        let mapper = self.mapper.clone();
        
        let mapped = stream.map(move |event| match event {
            Ok(StreamEvent::Done { mut message }) => {
                let short_id = message.model.clone();
                message.model = mapper.join_id(&p_name, &short_id);
                message.provider = p_name.clone();
                Ok(StreamEvent::Done { message })
            }
            Ok(StreamEvent::Error { mut message }) => {
                let short_id = message.model.clone();
                message.model = mapper.join_id(&p_name, &short_id);
                message.provider = p_name.clone();
                Ok(StreamEvent::Error { message })
            }
            other => other,
        });
        
        Ok(Box::pin(mapped))
    }
}

pub struct AiClientBuilder {
    mapper: Option<ModelMapper>,
}

impl AiClientBuilder {
    pub fn new() -> Self {
        Self { mapper: None }
    }

    pub fn with_mapper(mut self, mapper: ModelMapper) -> Self {
        self.mapper = Some(mapper);
        self
    }

    pub fn build(self) -> AiClient {
        let mapper = self.mapper.unwrap_or_default();
        let mut providers = HashMap::new();
        
        // Register all providers
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
            mapper,
            providers,
        }
    }
}
