use super::Credential;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The main configuration file structure.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Provider credentials: provider_id -> Credential
    #[serde(default)]
    pub credentials: HashMap<String, Credential>,

    /// Enabled models: list of `<provider>/<model>` strings
    #[serde(default)]
    pub enabled_models: Vec<String>,
}

/// Manages reading/writing the config file with safe atomic writes.
#[derive(Clone)]
pub struct ConfigManager {
    path: PathBuf,
}

impl ConfigManager {
    /// Create a config manager with a custom path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Create a config manager with the default path (~/.ai-rs/config.json).
    pub fn default_path() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::new(home.join(".ai-rs").join("config.json"))
    }

    /// Get the config file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the config from disk. Returns default if file doesn't exist.
    pub fn load(&self) -> anyhow::Result<AppConfig> {
        if !self.path.exists() {
            return Ok(AppConfig::default());
        }

        let content = fs::read_to_string(&self.path)?;
        let config: AppConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Save the config to disk atomically (write to temp file, then rename).
    /// This prevents corruption from concurrent writes or crashes.
    pub fn save(&self, config: &AppConfig) -> anyhow::Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
            // Set directory permissions to 700 on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        let json = serde_json::to_string_pretty(config)?;

        // Write to a temp file in the same directory, then rename for atomicity
        let tmp_path = self.path.with_extension("json.tmp");
        {
            let mut file = fs::File::create(&tmp_path)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }

        // Set file permissions to 600 on Unix (before rename)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600));
        }

        fs::rename(&tmp_path, &self.path)?;

        Ok(())
    }

    /// Set a credential for a provider (read-modify-write, atomic).
    pub fn set_credential(
        &self,
        provider_id: &str,
        credential: Credential,
    ) -> anyhow::Result<()> {
        let mut config = self.load()?;
        config.credentials.insert(provider_id.to_string(), credential);
        self.save(&config)
    }

    /// Remove a credential for a provider.
    pub fn remove_credential(&self, provider_id: &str) -> anyhow::Result<()> {
        let mut config = self.load()?;
        config.credentials.remove(provider_id);
        self.save(&config)
    }

    /// Get a credential for a provider.
    pub fn get_credential(&self, provider_id: &str) -> anyhow::Result<Option<Credential>> {
        let config = self.load()?;
        Ok(config.credentials.get(provider_id).cloned())
    }

    /// Check if credentials exist for a provider.
    pub fn has_credential(&self, provider_id: &str) -> anyhow::Result<bool> {
        let config = self.load()?;
        Ok(config.credentials.contains_key(provider_id))
    }

    /// List all providers with credentials.
    pub fn list_providers_with_credentials(&self) -> anyhow::Result<Vec<String>> {
        let config = self.load()?;
        Ok(config.credentials.keys().cloned().collect())
    }

    /// Set enabled models list.
    pub fn set_enabled_models(&self, models: Vec<String>) -> anyhow::Result<()> {
        let mut config = self.load()?;
        config.enabled_models = models;
        self.save(&config)
    }

    /// Get enabled models list.
    pub fn get_enabled_models(&self) -> anyhow::Result<Vec<String>> {
        let config = self.load()?;
        Ok(config.enabled_models)
    }

    /// Add models to the enabled list (dedup).
    pub fn add_enabled_models(&self, models: &[String]) -> anyhow::Result<()> {
        let mut config = self.load()?;
        for m in models {
            if !config.enabled_models.contains(m) {
                config.enabled_models.push(m.clone());
            }
        }
        self.save(&config)
    }

    /// Remove models from the enabled list.
    pub fn remove_enabled_models(&self, models: &[String]) -> anyhow::Result<()> {
        let mut config = self.load()?;
        config.enabled_models.retain(|m| !models.contains(m));
        self.save(&config)
    }

    /// Get the API key for a provider. Checks config, then env vars, then sniffed files.
    /// Automatically refreshes OAuth tokens if expired or near expiry.
    pub async fn resolve_api_key_with_buffer(&self, provider_id: &str, buffer_secs: u64) -> anyhow::Result<Option<String>> {
        // 1. Check config
        if let Some(mut cred) = self.get_credential(provider_id)? {
            // Handle OAuth refresh if needed
            if let super::Credential::OAuth(ref mut oauth) = cred {
                let now = chrono::Utc::now().timestamp_millis();
                // If expired or expiring within buffer
                if now + (buffer_secs as i64 * 1000) >= oauth.expires {
                    let oauth_provider: Box<dyn crate::oauth::OAuthProvider> = match provider_id {
                        "anthropic" => Box::new(crate::oauth::anthropic::AnthropicOAuthProvider),
                        "gemini-cli" => Box::new(crate::oauth::google_gemini_cli::GeminiCliOAuthProvider),
                        "antigravity" => Box::new(crate::oauth::google_antigravity::AntigravityOAuthProvider),
                        "openai-codex" => Box::new(crate::oauth::openai_codex::OpenAiCodexOAuthProvider),
                        "github-copilot" => Box::new(crate::oauth::github_copilot::GitHubCopilotOAuthProvider),
                        "qwen" => Box::new(crate::oauth::qwen_portal::QwenPortalOAuthProvider),
                        _ => return Ok(cred.api_key()), // Unknown provider, can't refresh
                    };

                    let old_creds = crate::oauth::OAuthCredentials {
                        refresh: oauth.refresh.clone(),
                        access: oauth.access.clone(),
                        expires: oauth.expires,
                        extra: oauth.extra.clone(),
                    };

                    match oauth_provider.refresh_token(&old_creds).await {
                        Ok(new_creds) => {
                            oauth.access = new_creds.access;
                            oauth.refresh = new_creds.refresh;
                            oauth.expires = new_creds.expires;
                            oauth.extra = new_creds.extra;
                            // Save refreshed token back to config
                            self.set_credential(provider_id, cred.clone())?;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to refresh OAuth token for {}: {}", provider_id, e);
                            // Continue with old token, might fail with 401
                        }
                    }
                }
            }

            if let Some(key) = cred.api_key() {
                return Ok(Some(key));
            }
        }

        // 2. Check environment variables
        if let Some(key) = super::sniff::env_api_key(provider_id) {
            return Ok(Some(key));
        }

        // 3. Check external credential files
        if let Some(cred) = super::sniff::sniff_external_credential(provider_id) {
            // Persist the sniffed credential
            self.set_credential(provider_id, cred.clone())?;
            return Ok(cred.api_key());
        }

        Ok(None)
    }

    /// Resolve API key with a default buffer of 5 minutes.
    pub async fn resolve_api_key(&self, provider_id: &str) -> anyhow::Result<Option<String>> {
        self.resolve_api_key_with_buffer(provider_id, 5 * 60).await
    }

    /// Refresh all OAuth credentials in the config if they are near expiry.
    pub async fn refresh_all_credentials(&self, buffer_secs: u64) -> anyhow::Result<()> {
        let providers = self.list_providers_with_credentials()?;
        for pid in providers {
            // resolve_api_key handles the logic of checking expiry and refreshing
            let _ = self.resolve_api_key_with_buffer(&pid, buffer_secs).await?;
        }
        Ok(())
    }

    /// Start a background task that periodically refreshes all OAuth credentials.
    /// buffer_secs should ideally be >= interval_secs to avoid missing tokens.
    pub fn start_auto_refresh_service(self, interval_secs: u64, buffer_secs: u64) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                tracing::debug!("Running auto-refresh service (interval={}s, buffer={}s)...", interval_secs, buffer_secs);
                if let Err(e) = self.refresh_all_credentials(buffer_secs).await {
                    tracing::error!("Auto-refresh service error: {}", e);
                }
            }
        })
    }
}
