pub mod auth;
pub mod client;
pub mod mapper;
pub mod models;
pub mod oauth;
pub mod providers;
pub mod types;

// Re-exports for convenience
pub use auth::config::ConfigManager;
pub use auth::{
    all_provider_auth_info, provider_groups, AuthMethod, Credential, ProviderAuthInfo,
};
pub use client::{AiClient, AiClientBuilder};
pub use mapper::{join_model_id, split_model_id};
pub use models::static_models;
pub use oauth::{OAuthAuthInfo, OAuthCallbacks, OAuthCredentials, OAuthPrompt, OAuthProvider};
pub use providers::{Provider, ProviderError};
pub use types::*;
