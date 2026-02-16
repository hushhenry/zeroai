use crate::providers::anthropic::AnthropicProvider;
use crate::providers::google::GoogleProvider;
use crate::providers::google_gemini_cli::GoogleGeminiCliProvider;
use crate::providers::openai::OpenAiProvider;
use crate::providers::{Provider, ProviderError};
use crate::types::*;
use futures::stream::BoxStream;
use std::collections::HashMap;
use std::sync::Arc;

/// Maps `<provider>/<model>` identifiers to concrete provider implementations.
///
/// Naming convention:
/// - The model naming rule is `<provider>/<model>`.
/// - The `<provider>` prefix is used for routing; only `<model>` is passed to the
///   underlying provider API.
/// - For Google, the first-level "google" is only a group label. The actual providers
///   are `google` (API key), `antigravity` (OAuth), `gemini-cli` (OAuth).
pub struct ModelMapper {
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl ModelMapper {
    /// Create a new mapper with all built-in providers registered.
    pub fn new() -> Self {
        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();

        // OpenAI-compatible providers all share the same OpenAI provider implementation
        let openai = Arc::new(OpenAiProvider::new());
        providers.insert("openai".into(), openai.clone());
        providers.insert("deepseek".into(), openai.clone());
        providers.insert("xai".into(), openai.clone());
        providers.insert("groq".into(), openai.clone());
        providers.insert("together".into(), openai.clone());
        providers.insert("siliconflow".into(), openai.clone());
        providers.insert("zhipuai".into(), openai.clone());
        providers.insert("fireworks".into(), openai.clone());
        providers.insert("nebius".into(), openai.clone());
        providers.insert("openrouter".into(), openai.clone());

        // Anthropic
        providers.insert("anthropic".into(), Arc::new(AnthropicProvider::new()));

        // Google (API key)
        providers.insert("google".into(), Arc::new(GoogleProvider::new()));

        // Gemini CLI (OAuth)
        providers.insert(
            "gemini-cli".into(),
            Arc::new(GoogleGeminiCliProvider::new_gemini_cli()),
        );

        // Antigravity (OAuth)
        providers.insert(
            "antigravity".into(),
            Arc::new(GoogleGeminiCliProvider::new_antigravity()),
        );

        Self { providers }
    }

    /// Register a custom provider.
    pub fn register_provider(&mut self, name: &str, provider: Arc<dyn Provider>) {
        self.providers.insert(name.to_string(), provider);
    }

    /// Parse a `<provider>/<model>` string into (provider, model).
    pub fn parse_model_id(full_id: &str) -> Option<(&str, &str)> {
        let slash = full_id.find('/')?;
        if slash == 0 || slash == full_id.len() - 1 {
            return None;
        }
        Some((&full_id[..slash], &full_id[slash + 1..]))
    }

    /// Get the provider implementation for a given provider name.
    pub fn get_provider(&self, provider: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(provider).cloned()
    }

    /// List all registered provider names.
    pub fn provider_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Stream a chat completion, routing by `<provider>/<model>`.
    ///
    /// The returned `AssistantMessage` in the `Done` event will have the model
    /// field set to `<provider>/<model>` (the full qualified name).
    pub fn stream(
        &self,
        full_model_id: &str,
        model_def: &ModelDef,
        context: &ChatContext,
        options: &StreamOptions,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError> {
        let (provider_name, _model_id) = Self::parse_model_id(full_model_id).ok_or_else(|| {
            ProviderError::Other(format!(
                "Invalid model ID '{}': expected <provider>/<model>",
                full_model_id
            ))
        })?;

        let provider = self.providers.get(provider_name).ok_or_else(|| {
            ProviderError::Other(format!("Unknown provider: {}", provider_name))
        })?;

        let stream = provider.stream(model_def, context, options);

        // Wrap stream to rewrite provider/model in the Done event
        let full_id = full_model_id.to_string();
        let prov_name = provider_name.to_string();

        let mapped = futures::stream::StreamExt::map(stream, move |event| {
            match event {
                Ok(StreamEvent::Done { mut message }) => {
                    // Rewrite model to full qualified name
                    message.model = full_id.clone();
                    message.provider = prov_name.clone();
                    Ok(StreamEvent::Done { message })
                }
                Ok(StreamEvent::Error { mut message }) => {
                    message.model = full_id.clone();
                    message.provider = prov_name.clone();
                    Ok(StreamEvent::Error { message })
                }
                other => other,
            }
        });

        Ok(Box::pin(mapped))
    }

    /// List models for a specific provider.
    pub async fn list_models(
        &self,
        provider: &str,
        api_key: &str,
    ) -> Result<Vec<ModelDef>, ProviderError> {
        let prov = self.providers.get(provider).ok_or_else(|| {
            ProviderError::Other(format!("Unknown provider: {}", provider))
        })?;

        let mut models = prov.list_models(api_key).await?;

        // Ensure all models have the correct provider set
        for model in &mut models {
            model.provider = provider.to_string();
        }

        Ok(models)
    }
}

impl Default for ModelMapper {
    fn default() -> Self {
        Self::new()
    }
}
