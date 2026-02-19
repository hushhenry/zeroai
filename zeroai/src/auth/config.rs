use super::Credential;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A single named credential slot for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub credential: Credential,

    /// When set and in the future, this account should be skipped for selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unhealthy_until_ms: Option<i64>,

    /// Bookkeeping only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rate_limited_ms: Option<i64>,
}

impl Account {
    pub fn is_healthy_at(&self, now_ms: i64) -> bool {
        self.unhealthy_until_ms.unwrap_or(0) <= now_ms
    }

    pub fn display_label(&self) -> String {
        let id_prefix = self.id.chars().take(4).collect::<String>();
        self.label.clone().unwrap_or_else(|| format!("account-{}", id_prefix))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderAccounts {
    #[serde(default)]
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone)]
pub struct AccountSelection {
    pub account_id: String,
    pub api_key: String,
}

/// The main configuration file structure.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// LEGACY: Provider credentials: provider_id -> Credential
    ///
    /// This is kept for backward compatibility with older config.json.
    /// New code should use `provider_accounts`.
    #[serde(default)]
    pub credentials: HashMap<String, Credential>,

    /// Provider accounts: provider_id -> ordered accounts.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub provider_accounts: HashMap<String, ProviderAccounts>,

    /// Enabled models: list of `<provider>/<model>` strings
    #[serde(default)]
    pub enabled_models: Vec<String>,

    /// Custom OpenAI-compatible provider models URL (provider_id -> URL). Blank = use {base_url}/v1/models.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub provider_models_url: HashMap<String, String>,
}

/// Manages reading/writing the config file with safe atomic writes + file lock.
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

    fn lock_path(&self) -> PathBuf {
        // A sibling lock file (avoids locking the config file itself during atomic replace).
        self.path.with_extension("json.lock")
    }

    fn with_exclusive_lock<T>(&self, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        let lock_path = self.lock_path();
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;

        lock_file.lock_exclusive()?;
        let out = f();
        let _ = lock_file.unlock();
        out
    }

    fn migrate_legacy(mut cfg: AppConfig) -> AppConfig {
        if cfg.credentials.is_empty() {
            return cfg;
        }

        // For every legacy credential, ensure we have at least one account.
        for (pid, cred) in cfg.credentials.clone() {
            let entry = cfg
                .provider_accounts
                .entry(pid.clone())
                .or_insert_with(ProviderAccounts::default);
            if entry.accounts.is_empty() {
                entry.accounts.push(Account {
                    id: "default".to_string(),
                    label: Some("default".to_string()),
                    credential: cred,
                    unhealthy_until_ms: None,
                    last_rate_limited_ms: None,
                });
            }
        }

        // Keep legacy map for backward compatibility, but we prefer accounts.
        cfg
    }

    /// Load the config from disk. Returns default if file doesn't exist.
    /// Performs legacy migration (single-credential -> accounts).
    pub fn load(&self) -> anyhow::Result<AppConfig> {
        self.with_exclusive_lock(|| {
            if !self.path.exists() {
                return Ok(AppConfig::default());
            }

            let content = fs::read_to_string(&self.path)?;
            let cfg: AppConfig = serde_json::from_str(&content)?;
            Ok(Self::migrate_legacy(cfg))
        })
    }

    /// Save the config to disk atomically (write to temp file, then rename).
    /// This prevents corruption from concurrent writes or crashes.
    pub fn save(&self, config: &AppConfig) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
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
        })
    }

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    fn ensure_accounts<'a>(cfg: &'a mut AppConfig, provider_id: &str) -> &'a mut ProviderAccounts {
        cfg.provider_accounts
            .entry(provider_id.to_string())
            .or_insert_with(ProviderAccounts::default)
    }

    fn mirror_first_to_legacy(cfg: &mut AppConfig, provider_id: &str) {
        if let Some(pa) = cfg.provider_accounts.get(provider_id) {
            if let Some(first) = pa.accounts.first() {
                cfg.credentials.insert(provider_id.to_string(), first.credential.clone());
            } else {
                cfg.credentials.remove(provider_id);
            }
        } else {
            cfg.credentials.remove(provider_id);
        }
    }

    // -----------------------------------------------------------------------
    // Multi-account operations
    // -----------------------------------------------------------------------

    /// Add a new account for a provider (append to end). Returns new account id.
    pub fn add_account(
        &self,
        provider_id: &str,
        label: Option<String>,
        credential: Credential,
    ) -> anyhow::Result<String> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            let id = uuid::Uuid::new_v4().to_string();
            {
                let accs = Self::ensure_accounts(&mut cfg, provider_id);
                // Generate provider-specific label based on existing accounts for this provider
                let next_index = accs.accounts.len() + 1;
                let label = label.and_then(|s| {
                    let t = s.trim().to_string();
                    if t.is_empty() { None } else { Some(t) }
                });
                // Auto-generate label using provider prefix for clarity (e.g., "openai-1", "gemini-cli-2")
                let label = label.or_else(|| {
                    let provider_prefix = provider_id
                        .strip_prefix("custom:")
                        .unwrap_or(provider_id)
                        .split('/')
                        .next()
                        .unwrap_or(provider_id);
                    Some(format!("{}-{}", provider_prefix, next_index))
                });

                accs.accounts.push(Account {
                    id: id.clone(),
                    label,
                    credential,
                    unhealthy_until_ms: None,
                    last_rate_limited_ms: None,
                });
            }

            Self::mirror_first_to_legacy(&mut cfg, provider_id);
            self.save_unlocked(&cfg)?;
            Ok(id)
        })
    }

    /// List accounts for provider (in order).
    pub fn list_accounts(&self, provider_id: &str) -> anyhow::Result<Vec<Account>> {
        let cfg = self.load()?;
        Ok(cfg
            .provider_accounts
            .get(provider_id)
            .map(|p| p.accounts.clone())
            .unwrap_or_default())
    }

    /// Move the given account to the front (index 0).
    pub fn use_account(&self, provider_id: &str, account_id: &str) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            {
                let accs = Self::ensure_accounts(&mut cfg, provider_id);
                if let Some(pos) = accs.accounts.iter().position(|a| a.id == account_id) {
                    if pos != 0 {
                        let a = accs.accounts.remove(pos);
                        accs.accounts.insert(0, a);
                    }
                } else {
                    anyhow::bail!("account not found: {}", account_id);
                }
            }
            Self::mirror_first_to_legacy(&mut cfg, provider_id);
            self.save_unlocked(&cfg)
        })
    }

    /// Remove a specific account.
    pub fn remove_account(&self, provider_id: &str, account_id: &str) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            {
                let accs = Self::ensure_accounts(&mut cfg, provider_id);
                let before = accs.accounts.len();
                accs.accounts.retain(|a| a.id != account_id);
                if accs.accounts.len() == before {
                    anyhow::bail!("account not found: {}", account_id);
                }
            }
            Self::mirror_first_to_legacy(&mut cfg, provider_id);
            self.save_unlocked(&cfg)
        })
    }

    /// Manual rotation: move first account to end.
    pub fn rotate_first(&self, provider_id: &str) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            {
                let accs = Self::ensure_accounts(&mut cfg, provider_id);
                if accs.accounts.len() >= 2 {
                    let first = accs.accounts.remove(0);
                    accs.accounts.push(first);
                }
            }
            Self::mirror_first_to_legacy(&mut cfg, provider_id);
            self.save_unlocked(&cfg)
        })
    }

    pub fn move_account_up(&self, provider_id: &str, account_id: &str) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            {
                let accs = Self::ensure_accounts(&mut cfg, provider_id);
                if let Some(pos) = accs.accounts.iter().position(|a| a.id == account_id) {
                    if pos > 0 {
                        accs.accounts.swap(pos, pos - 1);
                    }
                } else {
                    anyhow::bail!("account not found: {}", account_id);
                }
            }
            Self::mirror_first_to_legacy(&mut cfg, provider_id);
            self.save_unlocked(&cfg)
        })
    }

    pub fn move_account_down(&self, provider_id: &str, account_id: &str) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            {
                let accs = Self::ensure_accounts(&mut cfg, provider_id);
                if let Some(pos) = accs.accounts.iter().position(|a| a.id == account_id) {
                    if pos + 1 < accs.accounts.len() {
                        accs.accounts.swap(pos, pos + 1);
                    }
                } else {
                    anyhow::bail!("account not found: {}", account_id);
                }
            }
            Self::mirror_first_to_legacy(&mut cfg, provider_id);
            self.save_unlocked(&cfg)
        })
    }

    pub fn set_account_label(&self, provider_id: &str, account_id: &str, label: Option<String>) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            {
                let accs = Self::ensure_accounts(&mut cfg, provider_id);
                if let Some(acc) = accs.accounts.iter_mut().find(|a| a.id == account_id) {
                    acc.label = label.filter(|s| !s.trim().is_empty());
                } else {
                    anyhow::bail!("account not found: {}", account_id);
                }
            }
            self.save_unlocked(&cfg)
        })
    }

    /// Mark the account as temporarily unhealthy and move it to the end.
    pub fn rate_limit_account(
        &self,
        provider_id: &str,
        account_id: &str,
        backoff_ms: u64,
    ) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            let now = Self::now_ms();
            let until = now.saturating_add(backoff_ms as i64);

            {
                let accs = Self::ensure_accounts(&mut cfg, provider_id);
                if let Some(pos) = accs.accounts.iter().position(|a| a.id == account_id) {
                    let mut a = accs.accounts.remove(pos);
                    a.unhealthy_until_ms = Some(until);
                    a.last_rate_limited_ms = Some(now);
                    accs.accounts.push(a);
                } else {
                    anyhow::bail!("account not found: {}", account_id);
                }
            }
            Self::mirror_first_to_legacy(&mut cfg, provider_id);
            self.save_unlocked(&cfg)
        })
    }

    /// Resolve API key for provider, preferring the first *healthy* account.
    /// If all accounts are unhealthy, falls back to the first account.
    pub async fn resolve_account(&self, provider_id: &str) -> anyhow::Result<Option<AccountSelection>> {
        // We keep this async because legacy code refreshes OAuth tokens.
        // For multi-account, we select an account first, then refresh that account if needed.
        let mut cfg = self.load()?;

        // Ensure migrated.
        cfg = Self::migrate_legacy(cfg);

        // No accounts? Try env/sniff as before.
        let accs = cfg
            .provider_accounts
            .get(provider_id)
            .map(|p| p.accounts.clone())
            .unwrap_or_default();
        if accs.is_empty() {
            if let Some(key) = super::sniff::env_api_key(provider_id) {
                return Ok(Some(AccountSelection { account_id: "env".into(), api_key: key }));
            }
            if let Some(cred) = super::sniff::sniff_external_credential(provider_id) {
                // Persist as a new account.
                let _id = self.add_account(provider_id, Some("sniffed".into()), cred.clone())?;
                if let Some(k) = cred.api_key() {
                    return Ok(Some(AccountSelection { account_id: _id, api_key: k }));
                }
            }
            return Ok(None);
        }

        let now = Self::now_ms();
        let pick = accs
            .iter()
            .enumerate()
            .find(|(_, a)| a.is_healthy_at(now))
            .map(|(i, _)| i)
            .unwrap_or(0);

        let mut chosen = accs[pick].clone();

        // Refresh OAuth if needed. (We re-use the old single-credential refresh logic.)
        if chosen.credential.is_expired() {
            if let Credential::OAuth(ref mut oauth) = chosen.credential {
                let oauth_provider: Box<dyn crate::oauth::OAuthProvider> = match provider_id {
                    "gemini-cli" => Box::new(crate::oauth::google_gemini_cli::GeminiCliOAuthProvider),
                    "antigravity" => Box::new(crate::oauth::google_antigravity::AntigravityOAuthProvider),
                    "openai-codex" => Box::new(crate::oauth::openai_codex::OpenAiCodexOAuthProvider),
                    "github-copilot" => Box::new(crate::oauth::github_copilot::GitHubCopilotOAuthProvider),
                    "qwen" => Box::new(crate::oauth::qwen_portal::QwenPortalOAuthProvider),
                    _ => {
                        // Unknown provider, can't refresh
                        if let Some(k) = chosen.credential.api_key() {
                            return Ok(Some(AccountSelection { account_id: chosen.id, api_key: k }));
                        }
                        return Ok(None);
                    }
                };

                let old_creds = crate::oauth::OAuthCredentials {
                    refresh: oauth.refresh.clone(),
                    access: oauth.access.clone(),
                    expires: oauth.expires,
                    extra: oauth.extra.clone(),
                };

                if let Ok(new_creds) = oauth_provider.refresh_token(&old_creds).await {
                    oauth.access = new_creds.access;
                    oauth.refresh = new_creds.refresh;
                    oauth.expires = new_creds.expires;
                    oauth.extra = new_creds.extra;

                    // Persist refreshed token to the same account.
                    self.with_exclusive_lock(|| {
                        let mut cfg = self.load_unlocked()?;
                        {
                            let accs = Self::ensure_accounts(&mut cfg, provider_id);
                            if let Some(pos) = accs.accounts.iter().position(|a| a.id == chosen.id) {
                                accs.accounts[pos].credential = chosen.credential.clone();
                            }
                        }
                        Self::mirror_first_to_legacy(&mut cfg, provider_id);
                        self.save_unlocked(&cfg)
                    })?;
                }
            }
        }

        Ok(chosen
            .credential
            .api_key()
            .map(|k| AccountSelection {
                account_id: chosen.id,
                api_key: k,
            }))
    }

    /// Backward-compatible: resolve API key only.
    pub async fn resolve_api_key(&self, provider_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .resolve_account(provider_id)
            .await?
            .map(|s| s.api_key))
    }

    // -----------------------------------------------------------------------
    // Legacy API kept for compatibility with existing TUI code.
    // These operate on the FIRST account.
    // -----------------------------------------------------------------------

    /// Save helpers that assume lock already held.
    fn load_unlocked(&self) -> anyhow::Result<AppConfig> {
        if !self.path.exists() {
            return Ok(AppConfig::default());
        }
        let content = fs::read_to_string(&self.path)?;
        let cfg: AppConfig = serde_json::from_str(&content)?;
        Ok(Self::migrate_legacy(cfg))
    }

    fn save_unlocked(&self, config: &AppConfig) -> anyhow::Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        let json = serde_json::to_string_pretty(config)?;
        let tmp_path = self.path.with_extension("json.tmp");
        {
            let mut file = fs::File::create(&tmp_path)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }

    /// Set a credential for a provider.
    ///
    /// Multi-account semantics: updates the FIRST account if present, otherwise creates one.
    pub fn set_credential(&self, provider_id: &str, credential: Credential) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            {
                let accs = Self::ensure_accounts(&mut cfg, provider_id);
                if let Some(first) = accs.accounts.first_mut() {
                    first.credential = credential.clone();
                } else {
                    accs.accounts.push(Account {
                        id: "default".into(),
                        label: Some("default".into()),
                        credential: credential.clone(),
                        unhealthy_until_ms: None,
                        last_rate_limited_ms: None,
                    });
                }
            }
            Self::mirror_first_to_legacy(&mut cfg, provider_id);
            self.save_unlocked(&cfg)
        })
    }

    /// Remove ALL accounts for a provider (backward compatible behavior).
    pub fn remove_credential(&self, provider_id: &str) -> anyhow::Result<()> {
        self.with_exclusive_lock(|| {
            let mut cfg = self.load_unlocked()?;
            cfg.credentials.remove(provider_id);
            cfg.provider_accounts.remove(provider_id);
            self.save_unlocked(&cfg)
        })
    }

    /// Get the FIRST account credential for a provider.
    pub fn get_credential(&self, provider_id: &str) -> anyhow::Result<Option<Credential>> {
        let cfg = self.load()?;
        if let Some(pa) = cfg.provider_accounts.get(provider_id) {
            if let Some(first) = pa.accounts.first() {
                return Ok(Some(first.credential.clone()));
            }
        }
        Ok(cfg.credentials.get(provider_id).cloned())
    }

    /// Check if credentials exist for a provider.
    pub fn has_credential(&self, provider_id: &str) -> anyhow::Result<bool> {
        let cfg = self.load()?;
        let has_accounts = cfg
            .provider_accounts
            .get(provider_id)
            .map(|p| !p.accounts.is_empty())
            .unwrap_or(false);
        Ok(has_accounts || cfg.credentials.contains_key(provider_id))
    }

    /// List all providers with credentials.
    pub fn list_providers_with_credentials(&self) -> anyhow::Result<Vec<String>> {
        let cfg = self.load()?;
        let mut set = std::collections::BTreeSet::new();
        for k in cfg.credentials.keys() {
            set.insert(k.clone());
        }
        for (k, v) in cfg.provider_accounts.iter() {
            if !v.accounts.is_empty() {
                set.insert(k.clone());
            }
        }
        Ok(set.into_iter().collect())
    }

    /// Set enabled models list.
    pub fn set_enabled_models(&self, models: Vec<String>) -> anyhow::Result<()> {
        let mut cfg = self.load()?;
        cfg.enabled_models = models;
        self.save(&cfg)
    }

    /// Get enabled models list.
    pub fn get_enabled_models(&self) -> anyhow::Result<Vec<String>> {
        let cfg = self.load()?;
        Ok(cfg.enabled_models)
    }

    /// Get custom models URL for a provider (for OpenAI-compatible custom providers).
    pub fn get_models_url(&self, provider_id: &str) -> anyhow::Result<Option<String>> {
        let cfg = self.load()?;
        Ok(cfg.provider_models_url.get(provider_id).cloned())
    }

    /// Set custom models URL for a provider.
    pub fn set_models_url(&self, provider_id: &str, url: Option<&str>) -> anyhow::Result<()> {
        let mut cfg = self.load()?;
        match url {
            Some(u) if !u.trim().is_empty() => {
                cfg.provider_models_url
                    .insert(provider_id.to_string(), u.trim().to_string());
            }
            _ => {
                cfg.provider_models_url.remove(provider_id);
            }
        }
        self.save(&cfg)
    }

    /// Add models to the enabled list (dedup).
    pub fn add_enabled_models(&self, models: &[String]) -> anyhow::Result<()> {
        let mut cfg = self.load()?;
        for m in models {
            if !cfg.enabled_models.contains(m) {
                cfg.enabled_models.push(m.clone());
            }
        }
        self.save(&cfg)
    }

    /// Remove models from the enabled list.
    pub fn remove_enabled_models(&self, models: &[String]) -> anyhow::Result<()> {
        let mut cfg = self.load()?;
        cfg.enabled_models.retain(|m| !models.contains(m));
        self.save(&cfg)
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

    /// Resolve API key with buffer (legacy signature). Uses the selected account.
    pub async fn resolve_api_key_with_buffer(
        &self,
        provider_id: &str,
        _buffer_secs: u64,
    ) -> anyhow::Result<Option<String>> {
        // We keep buffer param to avoid breaking callers; account refresh uses the token expiry itself.
        self.resolve_api_key(provider_id).await
    }

    /// Start a background task that periodically refreshes all OAuth credentials.
    /// buffer_secs should ideally be >= interval_secs to avoid missing tokens.
    pub fn start_auto_refresh_service(
        self,
        interval_secs: u64,
        buffer_secs: u64,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                tracing::debug!(
                    "Running auto-refresh service (interval={}s, buffer={}s)...",
                    interval_secs,
                    buffer_secs
                );
                if let Err(e) = self.refresh_all_credentials(buffer_secs).await {
                    tracing::error!("Auto-refresh service error: {}", e);
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_cfg() -> (tempfile::TempDir, ConfigManager) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        (dir, ConfigManager::new(path))
    }

    fn api_key(k: &str) -> Credential {
        Credential::ApiKey(super::super::ApiKeyCredential { key: k.to_string() })
    }

    #[test]
    fn migration_from_legacy_credentials() {
        let (_dir, mgr) = tmp_cfg();
        let mut cfg = AppConfig::default();
        cfg.credentials.insert("google".into(), api_key("k1"));
        mgr.save(&cfg).unwrap();

        let loaded = mgr.load().unwrap();
        let accs = loaded
            .provider_accounts
            .get("google")
            .unwrap()
            .accounts
            .clone();
        assert_eq!(accs.len(), 1);
        assert_eq!(accs[0].id, "default");
    }

    #[test]
    fn rate_limit_moves_account_to_end_and_sets_unhealthy() {
        let (_dir, mgr) = tmp_cfg();
        let id1 = mgr.add_account("google", Some("a1".into()), api_key("k1")).unwrap();
        let id2 = mgr.add_account("google", Some("a2".into()), api_key("k2")).unwrap();

        // First should be id1
        let list = mgr.list_accounts("google").unwrap();
        assert_eq!(list[0].id, id1);
        assert_eq!(list[1].id, id2);

        mgr.rate_limit_account("google", &id1, 10_000).unwrap();
        let list2 = mgr.list_accounts("google").unwrap();
        assert_eq!(list2[0].id, id2);
        assert_eq!(list2[1].id, id1);
        assert!(list2[1].unhealthy_until_ms.is_some());
    }
}
