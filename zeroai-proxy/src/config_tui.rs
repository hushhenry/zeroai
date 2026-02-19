use zeroai::{
    ConfigManager,
    auth::{
        self, AuthMethod, Credential, ApiKeyCredential, SetupTokenCredential,
        ProviderAuthInfo, config::Account,
    },
    models::{fetch_models_for_provider, is_custom_provider},
    oauth::{
        google_antigravity::AntigravityOAuthProvider,
        google_gemini_cli::GeminiCliOAuthProvider,
        OAuthProvider, OAuthCallbacks, OAuthAuthInfo, OAuthPrompt,
    },
};
use async_trait::async_trait;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io::{self, stdout};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Constants & Colors
// ---------------------------------------------------------------------------

const COLOR_GREEN: Color = Color::Rgb(166, 227, 161);
const COLOR_YELLOW: Color = Color::Rgb(249, 226, 175);
const COLOR_CYAN: Color = Color::Rgb(137, 220, 235);
const COLOR_GRAY: Color = Color::Rgb(108, 112, 134);

// ---------------------------------------------------------------------------
// TUI states
// ---------------------------------------------------------------------------

enum Screen {
    ProviderGroups,
    SubProviders(usize),
    AuthInput(AuthInputState),
    ModelsUrlInput(ModelsUrlInputState),
    ModelSelect(ModelSelectState),
    AccountList(AccountListState),
    AccountLabelInput(AccountLabelInputState),
}

struct ModelsUrlInputState {
    provider_id: String,
    base_url: String,
    input: String,
    cursor_pos: usize,
    /// Auth/test failure message; shown when fetch fails (don't save until fixed).
    auth_error: Option<String>,
}

struct AuthInputState {
    provider_id: String,
    label: String,
    input: String,
    hint: String,
    is_oauth: bool,
    oauth_url: Option<String>,
    is_add: bool,
    cursor_pos: usize,
    initial_account_count: usize,
    oauth_error: Option<String>,
}

struct ModelSelectState {
    provider_id: String,
    models: Vec<(String, bool)>, // (full_model_id, selected)
    list_state: ListState,
    /// Shown when fetch_models_for_provider failed (user can continue with empty list).
    error: Option<String>,
}

struct AccountListState {
    provider_id: String,
    provider_label: String,
    accounts: Vec<Account>,
    list_state: ListState,
}

struct AccountLabelInputState {
    provider_id: String,
    provider_label: String,
    account_id: String,
    input: String,
    cursor_pos: usize,
}

// ---------------------------------------------------------------------------
// OAuth Callbacks for TUI
// ---------------------------------------------------------------------------

struct TuiOAuthCallbacks {
    auth_info: Arc<Mutex<Option<OAuthAuthInfo>>>,
    prompt_result: Arc<Mutex<Option<String>>>,
    _waiting_for_prompt: Arc<Mutex<bool>>,
    _progress: Arc<Mutex<String>>,
}

#[async_trait]
impl OAuthCallbacks for TuiOAuthCallbacks {
    fn on_auth(&self, info: OAuthAuthInfo) {
        let mut lock = self.auth_info.lock().unwrap();
        *lock = Some(info);
    }

    async fn on_prompt(&self, _prompt: OAuthPrompt) -> anyhow::Result<String> {
        {
            let mut waiting = self._waiting_for_prompt.lock().unwrap();
            *waiting = true;
        }
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let mut res = self.prompt_result.lock().unwrap();
            if let Some(val) = res.take() {
                let mut waiting = self._waiting_for_prompt.lock().unwrap();
                *waiting = false;
                return Ok(val);
            }
        }
    }

    fn on_progress(&self, message: &str) {
        let mut lock = self._progress.lock().unwrap();
        *lock = message.to_string();
    }
}

// ---------------------------------------------------------------------------
// Main TUI loop
// ---------------------------------------------------------------------------

pub async fn run_config_tui() -> anyhow::Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let config = ConfigManager::default_path();
    let groups = auth::provider_groups();

    let mut screen = Screen::ProviderGroups;
    let mut group_state = ListState::default();
    group_state.select(Some(0));
    let mut sub_state = ListState::default();

    let result = run_tui_loop(&mut terminal, config, &groups, &mut screen, &mut group_state, &mut sub_state).await;

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

async fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: ConfigManager,
    groups: &[(String, Vec<ProviderAuthInfo>)],
    screen: &mut Screen,
    group_state: &mut ListState,
    sub_state: &mut ListState,
) -> anyhow::Result<()> {
    let oauth_callbacks = Arc::new(TuiOAuthCallbacks {
        auth_info: Arc::new(Mutex::new(None)),
        prompt_result: Arc::new(Mutex::new(None)),
        _waiting_for_prompt: Arc::new(Mutex::new(false)),
        _progress: Arc::new(Mutex::new(String::new())),
    });

    // When there are no accounts at all, auto-enter add-account flow for the first provider (no keypress required).
    let mut auto_entered = false;

    loop {
        terminal.draw(|f| draw(f, &config, groups, screen, group_state, sub_state))?;

        // Once per session: if we're on provider list and no provider has any account, auto-open add flow
        if !auto_entered {
            if let Screen::ProviderGroups = screen {
                let has_any = groups.iter().any(|(_, ps)| {
                    ps.iter().any(|p| config.has_credential(&p.provider_id).unwrap_or(false))
                });
                if !has_any {
                    if let Some((_, providers)) = groups.first() {
                        if let Some(prov) = providers.first() {
                            let no_accounts = enter_account_list(config.clone(), prov, screen)?;
                            if no_accounts {
                                handle_provider_select(config.clone(), prov, screen, oauth_callbacks.clone(), false).await?;
                            }
                            auto_entered = true;
                        }
                    }
                }
            }
        }

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }

                match screen {
                    Screen::ProviderGroups => {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                            KeyCode::Up | KeyCode::Char('k') => {
                                let i = group_state.selected().unwrap_or(0);
                                let next = if i == 0 { groups.len().saturating_sub(1) } else { i - 1 };
                                group_state.select(Some(next));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let i = group_state.selected().unwrap_or(0);
                                let next = if i + 1 >= groups.len() { 0 } else { i + 1 };
                                group_state.select(Some(next));
                            }
                            KeyCode::Enter => {
                                if let Some(idx) = group_state.selected() {
                                    if idx < groups.len() {
                                        let (_, providers) = &groups[idx];
                                        if providers.len() == 1 {
                                            let prov = &providers[0];
                                            let no_accounts = enter_account_list(config.clone(), prov, screen)?;
                                            if no_accounts {
                                                handle_provider_select(config.clone(), prov, screen, oauth_callbacks.clone(), false).await?;
                                            }
                                        } else {
                                            sub_state.select(Some(0));
                                            *screen = Screen::SubProviders(idx);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::SubProviders(group_idx) => {
                        let (_, providers) = &groups[*group_idx];
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('q') => {
                                *screen = Screen::ProviderGroups;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let i = sub_state.selected().unwrap_or(0);
                                let next = if i == 0 { providers.len().saturating_sub(1) } else { i - 1 };
                                sub_state.select(Some(next));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let i = sub_state.selected().unwrap_or(0);
                                let next = if i + 1 >= providers.len() { 0 } else { i + 1 };
                                sub_state.select(Some(next));
                            }
                            KeyCode::Enter => {
                                if let Some(idx) = sub_state.selected() {
                                    if idx < providers.len() {
                                        let prov = &providers[idx];
                                        let no_accounts = enter_account_list(config.clone(), prov, screen)?;
                                        if no_accounts {
                                            handle_provider_select(config.clone(), prov, screen, oauth_callbacks.clone(), false).await?;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::AccountList(state) => {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('q') => {
                                *screen = Screen::ProviderGroups;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                if state.accounts.is_empty() {
                                    continue;
                                }
                                let i = state.list_state.selected().unwrap_or(0);
                                let next = if i == 0 { state.accounts.len().saturating_sub(1) } else { i - 1 };
                                state.list_state.select(Some(next));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if state.accounts.is_empty() {
                                    continue;
                                }
                                let i = state.list_state.selected().unwrap_or(0);
                                let next = if i + 1 >= state.accounts.len() { 0 } else { i + 1 };
                                state.list_state.select(Some(next));
                            }
                            KeyCode::Char('a') => {
                                let prov_info = groups.iter().flat_map(|(_, ps)| ps).find(|p| p.provider_id == state.provider_id);
                                if let Some(prov) = prov_info {
                                    handle_provider_select(config.clone(), prov, screen, oauth_callbacks.clone(), true).await?;
                                }
                            }
                            KeyCode::Char('d') => {
                                if let Some(idx) = state.list_state.selected() {
                                    if idx < state.accounts.len() {
                                        config.remove_account(&state.provider_id, &state.accounts[idx].id)?;
                                        state.accounts = config.list_accounts(&state.provider_id)?;
                                        if state.accounts.is_empty() {
                                            // No accounts left: auto-enter add-account flow
                                            let prov_info = groups.iter().flat_map(|(_, ps)| ps).find(|p| p.provider_id == state.provider_id);
                                            if let Some(prov) = prov_info {
                                                handle_provider_select(config.clone(), prov, screen, oauth_callbacks.clone(), true).await?;
                                            }
                                        } else if idx >= state.accounts.len() {
                                            state.list_state.select(Some(state.accounts.len().saturating_sub(1)));
                                        } else {
                                            state.list_state.select(Some(idx));
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('e') => {
                                if let Some(idx) = state.list_state.selected() {
                                    if idx < state.accounts.len() {
                                        let acc = &state.accounts[idx];
                                        *screen = Screen::AccountLabelInput(AccountLabelInputState {
                                            provider_id: state.provider_id.clone(),
                                            provider_label: state.provider_label.clone(),
                                            account_id: acc.id.clone(),
                                            input: acc.label.clone().unwrap_or_default(),
                                            cursor_pos: acc.label.as_ref().map(|s| s.len()).unwrap_or(0),
                                        });
                                    }
                                }
                            }
                            KeyCode::Char('K') => {
                                // Move account up (swap with previous)
                                if let Some(idx) = state.list_state.selected() {
                                    if idx > 0 && !state.accounts.is_empty() {
                                        config.move_account_up(&state.provider_id, &state.accounts[idx].id)?;
                                        state.accounts = config.list_accounts(&state.provider_id)?;
                                        state.list_state.select(Some(idx - 1));
                                    }
                                }
                            }
                            KeyCode::Char('J') => {
                                // Move account down (swap with next)
                                if let Some(idx) = state.list_state.selected() {
                                    if idx + 1 < state.accounts.len() {
                                        config.move_account_down(&state.provider_id, &state.accounts[idx].id)?;
                                        state.accounts = config.list_accounts(&state.provider_id)?;
                                        state.list_state.select(Some(idx + 1));
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(idx) = state.list_state.selected() {
                                    if idx < state.accounts.len() {
                                        let pid = state.provider_id.clone();
                                        let aid = state.accounts[idx].id.clone();
                                        config.use_account(&pid, &aid)?;
                                        enter_model_selection(&config, &pid, screen).await?;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::AccountLabelInput(state) => {
                        match key.code {
                            KeyCode::Esc => {
                                let accounts = config.list_accounts(&state.provider_id)?;
                                let mut ls = ListState::default();
                                if let Some(pos) = accounts.iter().position(|a| a.id == state.account_id) {
                                    ls.select(Some(pos));
                                }
                                *screen = Screen::AccountList(AccountListState {
                                    provider_id: state.provider_id.clone(),
                                    provider_label: state.provider_label.clone(),
                                    accounts,
                                    list_state: ls,
                                });
                            }
                            KeyCode::Char(c) => {
                                state.input.insert(state.cursor_pos, c);
                                state.cursor_pos += 1;
                            }
                            KeyCode::Backspace => {
                                if state.cursor_pos > 0 {
                                    state.cursor_pos -= 1;
                                    state.input.remove(state.cursor_pos);
                                }
                            }
                            KeyCode::Delete => {
                                if state.cursor_pos < state.input.len() {
                                    state.input.remove(state.cursor_pos);
                                }
                            }
                            KeyCode::Left => {
                                if state.cursor_pos > 0 {
                                    state.cursor_pos -= 1;
                                }
                            }
                            KeyCode::Right => {
                                if state.cursor_pos < state.input.len() {
                                    state.cursor_pos += 1;
                                }
                            }
                            KeyCode::Home => {
                                state.cursor_pos = 0;
                            }
                            KeyCode::End => {
                                state.cursor_pos = state.input.len();
                            }
                            KeyCode::Enter => {
                                // Trim whitespace and only save if non-empty
                                let trimmed = state.input.trim().to_string();
                                let label = if trimmed.is_empty() { None } else { Some(trimmed) };
                                config.set_account_label(&state.provider_id, &state.account_id, label)?;
                                let accounts = config.list_accounts(&state.provider_id)?;
                                let mut ls = ListState::default();
                                if let Some(pos) = accounts.iter().position(|a| a.id == state.account_id) {
                                    ls.select(Some(pos));
                                }
                                *screen = Screen::AccountList(AccountListState {
                                    provider_id: state.provider_id.clone(),
                                    provider_label: state.provider_label.clone(),
                                    accounts,
                                    list_state: ls,
                                });
                            }
                            _ => {}
                        }
                    }
                    Screen::AuthInput(state) => {
                        match key.code {
                            KeyCode::Esc => {
                                *screen = Screen::ProviderGroups;
                            }
                            KeyCode::Char(c) => {
                                state.input.insert(state.cursor_pos, c);
                                state.cursor_pos += 1;
                            }
                            KeyCode::Backspace => {
                                if state.cursor_pos > 0 {
                                    state.cursor_pos -= 1;
                                    state.input.remove(state.cursor_pos);
                                }
                            }
                            KeyCode::Delete => {
                                if state.cursor_pos < state.input.len() {
                                    state.input.remove(state.cursor_pos);
                                }
                            }
                            KeyCode::Left => {
                                if state.cursor_pos > 0 {
                                    state.cursor_pos -= 1;
                                }
                            }
                            KeyCode::Right => {
                                if state.cursor_pos < state.input.len() {
                                    state.cursor_pos += 1;
                                }
                            }
                            KeyCode::Home => {
                                state.cursor_pos = 0;
                            }
                            KeyCode::End => {
                                state.cursor_pos = state.input.len();
                            }
                            KeyCode::Enter => {
                                if !state.input.is_empty() {
                                    if state.is_oauth {
                                        let mut res = oauth_callbacks.prompt_result.lock().unwrap();
                                        *res = Some(state.input.trim().to_string());
                                        state.input.clear();
                                        state.cursor_pos = 0;
                                        state.hint = "Exchanging code for token...".into();
                                    } else {
                                        let provider_id = state.provider_id.clone();
                                        let input = state.input.trim().to_string();
                                        let is_setup = state.hint.contains("setup-token");

                                        let cred = if is_setup {
                                            Credential::SetupToken(SetupTokenCredential {
                                                token: input,
                                            })
                                        } else {
                                            Credential::ApiKey(ApiKeyCredential {
                                                key: input,
                                            })
                                        };

                                        if state.is_add {
                                            config.add_account(&provider_id, None, cred)?;
                                        } else {
                                            config.set_credential(&provider_id, cred)?;
                                        }

                                        if is_custom_provider(&provider_id) {
                                            let base_url = provider_id.strip_prefix("custom:").unwrap_or("").trim().trim_end_matches('/');
                                            let input_url = config.get_models_url(&provider_id).ok().flatten().unwrap_or_default();
                                            let cursor_pos = input_url.len();
                                            *screen = Screen::ModelsUrlInput(ModelsUrlInputState {
                                                provider_id: provider_id.clone(),
                                                base_url: base_url.to_string(),
                                                input: input_url,
                                                cursor_pos,
                                                auth_error: None,
                                            });
                                        } else {
                                            enter_model_selection(&config, &provider_id, screen).await?;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::ModelsUrlInput(state) => {
                        match key.code {
                            KeyCode::Esc => {
                                *screen = Screen::ProviderGroups;
                            }
                            KeyCode::Enter => {
                                let pid = state.provider_id.clone();
                                let url_opt = if state.input.trim().is_empty() {
                                    None
                                } else {
                                    Some(state.input.trim().to_string())
                                };
                                let api_key = config.resolve_api_key(&pid).await.ok().flatten();
                                match fetch_models_for_provider(&pid, api_key.as_deref(), url_opt.as_deref()).await {
                                    Ok(_) => {
                                        let _ = config.set_models_url(&pid, url_opt.as_deref());
                                        *screen = Screen::ProviderGroups;
                                        enter_model_selection(&config, &pid, screen).await?;
                                    }
                                    Err(e) => {
                                        let msg = if e.is_auth_error() {
                                            format!("❌ Failed: {} {}", e.status.unwrap_or(0), e.message)
                                        } else {
                                            format!("❌ Failed: {}", e)
                                        };
                                        state.auth_error = Some(msg);
                                    }
                                }
                            }
                            KeyCode::Backspace => {
                                state.auth_error = None;
                                if state.cursor_pos > 0 {
                                    state.cursor_pos -= 1;
                                    state.input.remove(state.cursor_pos);
                                }
                            }
                            KeyCode::Delete => {
                                state.auth_error = None;
                                if state.cursor_pos < state.input.len() {
                                    state.input.remove(state.cursor_pos);
                                }
                            }
                            KeyCode::Left => {
                                if state.cursor_pos > 0 {
                                    state.cursor_pos -= 1;
                                }
                            }
                            KeyCode::Right => {
                                if state.cursor_pos < state.input.len() {
                                    state.cursor_pos += 1;
                                }
                            }
                            KeyCode::Home => {
                                state.cursor_pos = 0;
                            }
                            KeyCode::End => {
                                state.cursor_pos = state.input.len();
                            }
                            KeyCode::Char(c) => {
                                state.auth_error = None;
                                state.input.insert(state.cursor_pos, c);
                                state.cursor_pos += 1;
                            }
                            _ => {}
                        }
                    }
                    Screen::ModelSelect(state) => {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('q') => {
                                save_models(&config, state)?;
                                *screen = Screen::ProviderGroups;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let i = state.list_state.selected().unwrap_or(0);
                                let next = if i == 0 { state.models.len().saturating_sub(1) } else { i - 1 };
                                state.list_state.select(Some(next));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let i = state.list_state.selected().unwrap_or(0);
                                let next = if i + 1 >= state.models.len() { 0 } else { i + 1 };
                                state.list_state.select(Some(next));
                            }
                            KeyCode::Char(' ') => {
                                if let Some(idx) = state.list_state.selected() {
                                    if idx < state.models.len() {
                                        state.models[idx].1 = !state.models[idx].1;
                                    }
                                }
                            }
                            KeyCode::Char('a') => {
                                let all_selected = state.models.iter().all(|(_, s)| *s);
                                for item in &mut state.models {
                                    item.1 = !all_selected;
                                }
                            }
                            KeyCode::Enter => {
                                save_models(&config, state)?;
                                *screen = Screen::ProviderGroups;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let mut next_provider_id = None;
        let mut oauth_error_msg = None;
        if let Screen::AuthInput(state) = screen {
            if state.is_oauth {
                // Check for OAuth error from the background task
                let progress = oauth_callbacks._progress.lock().unwrap();
                if progress.starts_with("OAuth failed:") {
                    oauth_error_msg = Some(progress.clone());
                }
                drop(progress);

                if oauth_error_msg.is_none() {
                    // No error - check for successful completion
                    if state.is_add {
                        // For adding accounts: transition when account count increases
                        if let Ok(current_accounts) = config.list_accounts(&state.provider_id) {
                            if current_accounts.len() > state.initial_account_count {
                                next_provider_id = Some(state.provider_id.clone());
                            }
                        }
                    } else if config.has_credential(&state.provider_id).unwrap_or(false) {
                        // For first account setup: transition when credential exists
                        next_provider_id = Some(state.provider_id.clone());
                    }

                    // Update OAuth URL, hint and progress
                    if next_provider_id.is_none() {
                        let info = oauth_callbacks.auth_info.lock().unwrap();
                        if let Some(info) = &*info {
                            state.oauth_url = Some(info.url.clone());
                            if let Some(instr) = &info.instructions {
                                if state.hint.is_empty() || state.hint.starts_with("Connecting to") {
                                    state.hint = instr.clone();
                                }
                            }
                        }
                        drop(info);

                        let progress = oauth_callbacks._progress.lock().unwrap();
                        if !progress.is_empty() {
                            state.hint = progress.clone();
                        }
                    }
                }
            }
        }

        // Display OAuth error if present
        if let Some(err_msg) = oauth_error_msg {
            if let Screen::AuthInput(state) = screen {
                state.oauth_error = Some(err_msg);
            }
        }

        if let Some(pid) = next_provider_id {
            let mut is_add_finished = false;
            if let Screen::AuthInput(state) = screen {
                if state.is_add {
                    is_add_finished = true;
                }
            }

            if is_add_finished {
                let prov_info = groups.iter().flat_map(|(_, ps)| ps).find(|p| p.provider_id == pid);
                if let Some(prov) = prov_info {
                    // After adding an account, always go to account list (accounts won't be empty)
                    let _ = enter_account_list(config.clone(), prov, screen)?;
                } else {
                    *screen = Screen::ProviderGroups;
                }
            } else if is_custom_provider(&pid) {
                let base_url = pid.strip_prefix("custom:").unwrap_or("").trim().trim_end_matches('/');
                let input_url = config.get_models_url(&pid).ok().flatten().unwrap_or_default();
                let cursor_pos = input_url.len();
                *screen = Screen::ModelsUrlInput(ModelsUrlInputState {
                    provider_id: pid.clone(),
                    base_url: base_url.to_string(),
                    input: input_url,
                    cursor_pos,
                    auth_error: None,
                });
            } else {
                enter_model_selection(&config, &pid, screen).await?;
            }
        }
    }
}

/// Returns `true` if the provider has zero accounts (caller should trigger add-account flow).
fn enter_account_list(config: ConfigManager, prov: &ProviderAuthInfo, screen: &mut Screen) -> anyhow::Result<bool> {
    let accounts = config.list_accounts(&prov.provider_id)?;
    if accounts.is_empty() {
        return Ok(true);
    }
    let mut ls = ListState::default();
    ls.select(Some(0));
    *screen = Screen::AccountList(AccountListState {
        provider_id: prov.provider_id.clone(),
        provider_label: prov.label.clone(),
        accounts,
        list_state: ls,
    });
    Ok(false)
}

async fn handle_provider_select(
    config: ConfigManager,
    prov: &ProviderAuthInfo,
    screen: &mut Screen,
    callbacks: Arc<TuiOAuthCallbacks>,
    is_add: bool,
) -> anyhow::Result<()> {
    let provider_id = prov.provider_id.clone();

    // Capture initial account count for detecting new accounts after OAuth
    let initial_account_count = config.list_accounts(&provider_id).unwrap_or_default().len();

    if !is_add {
        if config.has_credential(&provider_id).unwrap_or(false) {
            if is_custom_provider(&provider_id) {
                let base_url = provider_id.strip_prefix("custom:").unwrap_or("").trim().trim_end_matches('/');
                let input = config.get_models_url(&provider_id).ok().flatten().unwrap_or_default();
                let cursor_pos = input.len();
                *screen = Screen::ModelsUrlInput(ModelsUrlInputState {
                    provider_id: provider_id.clone(),
                    base_url: base_url.to_string(),
                    input,
                    cursor_pos,
                    auth_error: None,
                });
                return Ok(());
            }
            return enter_model_selection(&config, &provider_id, screen).await;
        }

        if let Some(cred) = auth::sniff::sniff_external_credential(&provider_id) {
            config.set_credential(&provider_id, cred)?;
            return enter_model_selection(&config, &provider_id, screen).await;
        }

        if let Some(key) = auth::sniff::env_api_key(&provider_id) {
            let cred = Credential::ApiKey(ApiKeyCredential { key });
            config.set_credential(&provider_id, cred)?;
            return enter_model_selection(&config, &provider_id, screen).await;
        }
    }

    let method = prov.auth_methods.first().cloned().unwrap_or(AuthMethod::ApiKey {
        env_var: None,
        hint: None,
    });

    match method {
        AuthMethod::ApiKey { hint, .. } => {
            *screen = Screen::AuthInput(AuthInputState {
                provider_id: provider_id.clone(),
                label: format!("Enter API key for {}", prov.label),
                input: String::new(),
                hint: hint.unwrap_or_default(),
                is_oauth: false,
                oauth_url: None,
                is_add,
                cursor_pos: 0,
                initial_account_count,
                oauth_error: None,
            });
        }
        AuthMethod::SetupToken { hint } => {
            *screen = Screen::AuthInput(AuthInputState {
                provider_id: provider_id.clone(),
                label: format!("Enter setup-token for {}", prov.label),
                input: String::new(),
                hint: hint.unwrap_or_else(|| "Run `claude setup-token` to generate".into()),
                is_oauth: false,
                oauth_url: None,
                is_add,
                cursor_pos: 0,
                initial_account_count,
                oauth_error: None,
            });
        }
        AuthMethod::OAuth { hint } => {
            let pid = provider_id.clone();
            let config_mgr = config.clone();
            tokio::spawn(async move {
                let oauth_provider: Box<dyn OAuthProvider + Send> = match pid.as_str() {
                    "gemini-cli" => Box::new(GeminiCliOAuthProvider),
                    "antigravity" => Box::new(AntigravityOAuthProvider),
                    "openai-codex" => Box::new(zeroai::oauth::openai_codex::OpenAiCodexOAuthProvider),
                    "github-copilot" => Box::new(zeroai::oauth::github_copilot::GitHubCopilotOAuthProvider),
                    "qwen-portal" => Box::new(zeroai::oauth::qwen_portal::QwenPortalOAuthProvider),
                    _ => return,
                };
                match oauth_provider.login(&*callbacks).await {
                    Ok(creds) => {
                        let cred = Credential::OAuth(zeroai::auth::OAuthCredential {
                            refresh: creds.refresh,
                            access: creds.access,
                            expires: creds.expires,
                            extra: creds.extra,
                        });
                        if is_add {
                            let _ = config_mgr.add_account(&pid, None, cred);
                        } else {
                            let _ = config_mgr.set_credential(&pid, cred);
                        }
                    }
                    Err(e) => {
                        // Store error in the callbacks progress field for display
                        let mut progress = callbacks._progress.lock().unwrap();
                        *progress = format!("OAuth failed: {}", e);
                    }
                }
            });
            *screen = Screen::AuthInput(AuthInputState {
                provider_id: provider_id.clone(),
                label: format!("OAuth for {}", prov.label),
                input: String::new(),
                hint: hint.unwrap_or_else(|| "Connecting to Google...".into()),
                is_oauth: true,
                oauth_url: None,
                is_add,
                cursor_pos: 0,
                initial_account_count,
                oauth_error: None,
            });
        }
    }
    Ok(())
}

async fn enter_model_selection(config: &ConfigManager, provider_id: &str, screen: &mut Screen) -> anyhow::Result<()> {
    let api_key = config.resolve_api_key(provider_id).await.ok().flatten();
    let models_url = config.get_models_url(provider_id).ok().flatten();
    let models = match fetch_models_for_provider(provider_id, api_key.as_deref(), models_url.as_deref()).await {
        Ok(list) => list.into_iter().map(|m| m.id).collect::<Vec<_>>(),
        Err(e) => {
            let _enabled = config.get_enabled_models().unwrap_or_default();
            let ls = ListState::default();
            *screen = Screen::ModelSelect(ModelSelectState {
                provider_id: provider_id.to_string(),
                models: Vec::new(),
                list_state: ls,
                error: Some(e.to_string()),
            });
            return Ok(());
        }
    };
    let enabled = config.get_enabled_models().unwrap_or_default();
    let model_items: Vec<(String, bool)> = models
        .into_iter()
        .map(|m| {
            let full_id = format!("{}/{}", provider_id, m);
            let selected = enabled.contains(&full_id);
            (full_id, selected)
        })
        .collect();
    let mut ls = ListState::default();
    if !model_items.is_empty() {
        ls.select(Some(0));
    }
    *screen = Screen::ModelSelect(ModelSelectState {
        provider_id: provider_id.to_string(),
        models: model_items,
        list_state: ls,
        error: None,
    });
    Ok(())
}

fn save_models(config: &ConfigManager, state: &ModelSelectState) -> anyhow::Result<()> {
    let selected: Vec<String> = state.models.iter().filter(|(_, s)| *s).map(|(id, _)| id.clone()).collect();
    let mut all_enabled = config.get_enabled_models().unwrap_or_default();
    all_enabled.retain(|m| !m.starts_with(&format!("{}/", state.provider_id)));
    all_enabled.extend(selected);
    config.set_enabled_models(all_enabled)?;
    Ok(())
}

fn draw(
    f: &mut Frame,
    config: &ConfigManager,
    groups: &[(String, Vec<ProviderAuthInfo>)],
    screen: &Screen,
    group_state: &mut ListState,
    sub_state: &mut ListState,
) {
    let area = f.area();
    match screen {
        Screen::ProviderGroups => {
            let items: Vec<ListItem> = groups.iter().map(|(label, providers)| {
                let has_any_cred = providers.iter().any(|p| config.has_credential(&p.provider_id).unwrap_or(false));
                let marker = if has_any_cred { "●" } else { "○" };
                let color = if has_any_cred { COLOR_GREEN } else { Color::White };
                
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", marker), Style::default().fg(color)),
                    Span::styled(format!("{: <15}", label), Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(" - "),
                    Span::styled(providers[0].hint.as_str(), Style::default().fg(COLOR_GRAY)),
                ]))
            }).collect();
            
            let title = Line::from(vec![
                Span::raw(" Providers ("),
                Span::styled("Enter", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" select, "),
                Span::styled("q", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" quit) "),
            ]);
            
            let list = List::new(items)
                .block(Block::default().title(title).borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, area, group_state);
        }
        Screen::SubProviders(group_idx) => {
            let (group_label, providers) = &groups[*group_idx];
            let items: Vec<ListItem> = providers.iter().map(|p| {
                let has_cred = config.has_credential(&p.provider_id).unwrap_or(false);
                let marker = if has_cred { "●" } else { "○" };
                let color = if has_cred { COLOR_GREEN } else { Color::White };
                
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", marker), Style::default().fg(color)),
                    Span::styled(format!("{: <25}", p.label), Style::default().add_modifier(Modifier::BOLD)),
                ]))
            }).collect();
            
            let title = Line::from(vec![
                Span::raw(format!(" {} (", group_label)),
                Span::styled("Esc", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" back, "),
                Span::styled("↑↓/jk", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" navigate) "),
            ]);
            
            let list = List::new(items)
                .block(Block::default().title(title).borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, area, sub_state);
        }
        Screen::AccountList(state) => {
            let items: Vec<ListItem> = state.accounts.iter().enumerate().map(|(i, acc)| {
                let marker = if i == 0 { "★" } else { " " };
                let now = chrono::Utc::now().timestamp_millis();
                let color = if acc.is_healthy_at(now) { COLOR_GREEN } else { Color::Red };

                let id_prefix = acc.id.chars().take(8).collect::<String>();
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", marker), Style::default().fg(COLOR_YELLOW)),
                    Span::styled(acc.display_label(), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                    Span::raw(" - "),
                    Span::styled(format!("ID: {}", id_prefix), Style::default().fg(COLOR_GRAY)),
                ]))
            }).collect();

            let title = Line::from(vec![
                Span::raw(format!(" {} Accounts (", state.provider_label)),
                Span::styled("Enter", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" use, "),
                Span::styled("a", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" add, "),
                Span::styled("e", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" label, "),
                Span::styled("d", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" del, "),
                Span::styled("K/J", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" move) "),
            ]);

            let list = List::new(items)
                .block(Block::default().title(title).borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            
            let mut ls = state.list_state.clone();
            f.render_stateful_widget(list, area, &mut ls);
        }
        Screen::AccountLabelInput(state) => {
            let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(2)]).split(area);
            f.render_widget(
                Paragraph::new("Enter new label for account:").block(Block::default().borders(Borders::ALL)),
                chunks[0],
            );
            // Display input with cursor visualization
            let (before, after) = state.input.split_at(state.cursor_pos);
            let cursor_span = Span::styled(" ", Style::default().bg(COLOR_CYAN));
            let line = Line::from(vec![
                Span::raw(before),
                cursor_span,
                Span::raw(after),
            ]);
            f.render_widget(
                Paragraph::new(line).block(Block::default().borders(Borders::ALL).title("Label (Enter to confirm, Esc to cancel)")),
                chunks[1],
            );
        }
        Screen::AuthInput(state) => {
            let has_info = !state.hint.is_empty() || state.oauth_url.is_some();
            let has_error = state.oauth_error.is_some();
            let mut constraints = vec![
                Constraint::Length(3),
                Constraint::Length(3),
            ];
            if has_error {
                constraints.push(Constraint::Length(3));
            }
            if has_info {
                constraints.push(Constraint::Min(3));
            }
            let chunks = Layout::vertical(constraints).split(area);

            f.render_widget(Paragraph::new(state.label.as_str()).block(Block::default().borders(Borders::ALL)), chunks[0]);

            let input_title = Line::from(vec![
                Span::raw(" Input ("),
                Span::styled("Enter", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" confirm, "),
                Span::styled("Esc", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" cancel) "),
            ]);
            // Display input with cursor visualization
            let (before, after) = state.input.split_at(state.cursor_pos);
            let cursor_span = Span::styled(" ", Style::default().bg(COLOR_CYAN));
            let line = Line::from(vec![
                Span::raw(before),
                cursor_span,
                Span::raw(after),
            ]);
            f.render_widget(Paragraph::new(line).block(Block::default().borders(Borders::ALL).title(input_title)), chunks[1]);

            // Display OAuth error if present
            if let Some(err) = &state.oauth_error {
                let error_idx = if has_error { 2 } else { 1 };
                f.render_widget(
                    Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red)),
                    chunks[error_idx],
                );
            }

            if has_info {
                let info_start_idx = if has_error { 3 } else { 2 };
                if info_start_idx < chunks.len() {
                    let mut info_content = vec![
                        Line::from(Span::styled("Instructions: ", Style::default().fg(COLOR_YELLOW))),
                        Line::from(state.hint.as_str()),
                    ];

                    if let Some(url) = &state.oauth_url {
                        info_content.push(Line::from(""));
                        info_content.push(Line::from(Span::styled("Clean URL (copy below):", Style::default().fg(COLOR_CYAN))));
                        info_content.push(Line::from(url.as_str()));
                    }

                    let info_para = Paragraph::new(info_content)
                        .wrap(Wrap { trim: false })
                        .block(Block::default().borders(Borders::NONE).title(""));
                    f.render_widget(info_para, chunks[info_start_idx]);
                }
            }
        }
        Screen::ModelsUrlInput(state) => {
            let hint = format!(
                "For custom OpenAI-compatible providers, enter models_url (or leave blank for {}/v1/models)",
                state.base_url
            );
            let constraints: Vec<Constraint> = if state.auth_error.is_some() {
                vec![Constraint::Length(3), Constraint::Length(3), Constraint::Min(2), Constraint::Min(2)]
            } else {
                vec![Constraint::Length(3), Constraint::Length(3), Constraint::Min(2)]
            };
            let chunks = Layout::vertical(constraints).split(area);
            f.render_widget(
                Paragraph::new(hint).block(Block::default().borders(Borders::ALL)),
                chunks[0],
            );
            let input_title = Line::from(vec![
                Span::raw(" URL ("),
                Span::styled("Enter", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" confirm, "),
                Span::styled("Esc", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" cancel) "),
            ]);
            // Display input with cursor visualization
            let (before, after) = state.input.split_at(state.cursor_pos);
            let cursor_span = Span::styled(" ", Style::default().bg(COLOR_CYAN));
            let line = Line::from(vec![
                Span::raw(before),
                cursor_span,
                Span::raw(after),
            ]);
            f.render_widget(
                Paragraph::new(line).block(Block::default().borders(Borders::ALL).title(input_title)),
                chunks[1],
            );
            if let Some(err) = &state.auth_error {
                f.render_widget(
                    Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red)),
                    chunks[2],
                );
            }
        }
        Screen::ModelSelect(state) => {
            let items: Vec<ListItem> = state.models.iter().map(|(id, selected)| {
                let (marker, style) = if *selected {
                    ("[x]", Style::default().fg(COLOR_GREEN))
                } else {
                    ("[ ]", Style::default().fg(Color::White))
                };
                ListItem::new(Span::styled(format!(" {} {}", marker, id), style))
            }).collect();
            let title = Line::from(vec![
                Span::raw(" Models ("),
                Span::styled("Space", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" toggle, "),
                Span::styled("a", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" all, "),
                Span::styled("Enter", Style::default().fg(COLOR_YELLOW)),
                Span::raw(" confirm) "),
            ]);
            let list = List::new(items)
                .block(Block::default().title(title).borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            if let Some(err) = &state.error {
                let chunks = Layout::vertical([Constraint::Min(2), Constraint::Min(5)]).split(area);
                f.render_widget(
                    Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red)),
                    chunks[0],
                );
                let mut ls = state.list_state.clone();
                f.render_stateful_widget(list, chunks[1], &mut ls);
            } else {
                let mut ls = state.list_state.clone();
                f.render_stateful_widget(list, area, &mut ls);
            }
        }
    }
}
