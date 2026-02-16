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
    pub fn resolve_api_key(&self, provider_id: &str) -> anyhow::Result<Option<String>> {
        // 1. Check config
        if let Some(cred) = self.get_credential(provider_id)? {
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
}
