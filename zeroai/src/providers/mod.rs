pub mod anthropic;
pub mod google;
pub mod google_gemini_cli;
pub mod openai;
pub mod retry;

use crate::types::{AssistantMessage, ChatContext, ModelDef, RequestOptions, StreamEvent};
use async_trait::async_trait;
use futures::stream::BoxStream;

/// Errors from provider operations.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error {status}: {body}")]
    Http { status: u16, body: String },

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Authentication required: {0}")]
    AuthRequired(String),

    #[error("Rate limited, retry after {retry_after_ms:?}ms")]
    RateLimited { retry_after_ms: Option<u64> },

    #[error("{0}")]
    Other(String),
}

/// Trait for AI provider implementations.
///
/// Each provider (OpenAI, Anthropic, Google, etc.) implements this trait
/// to handle the actual API calls.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stream a chat completion.
    fn stream(
        &self,
        model: &ModelDef,
        context: &ChatContext,
        options: &RequestOptions,
    ) -> BoxStream<'static, Result<StreamEvent, ProviderError>>;

    /// Execute a chat completion (non-streaming).
    async fn chat(
        &self,
        model: &ModelDef,
        context: &ChatContext,
        options: &RequestOptions,
    ) -> Result<AssistantMessage, ProviderError>;

    /// List models available from this provider.
    /// Some providers support dynamic model listing via API; others return a static list.
    async fn list_models(&self, api_key: &str) -> Result<Vec<ModelDef>, ProviderError>;
}
