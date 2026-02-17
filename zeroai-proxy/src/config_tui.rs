use zeroai::{
    ConfigManager,
    auth::{
        self, AuthMethod, Credential, ApiKeyCredential, SetupTokenCredential,
        ProviderAuthInfo,
    },
    models::static_models::static_models_for_provider,
    oauth::{
        anthropic::AnthropicOAuthProvider,
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
    ModelSelect(ModelSelectState),
}

struct AuthInputState {
    provider_id: String,
    label: String,
    input: String,
    hint: String,
    is_oauth: bool,
    oauth_url: Option<String>,
}

struct ModelSelectState {
    provider_id: String,
    models: Vec<(String, bool)>, // (full_model_id, selected)
    list_state: ListState,
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

    loop {
        terminal.draw(|f| draw(f, &config, groups, screen, group_state, sub_state))?;

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
                                            handle_provider_select(config.clone(), prov, screen, oauth_callbacks.clone()).await?;
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
                                        handle_provider_select(config.clone(), prov, screen, oauth_callbacks.clone()).await?;
                                    }
                                }
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
                                state.input.push(c);
                            }
                            KeyCode::Backspace => {
                                state.input.pop();
                            }
                            KeyCode::Enter => {
                                if !state.input.is_empty() {
                                    if state.is_oauth {
                                        let mut res = oauth_callbacks.prompt_result.lock().unwrap();
                                        *res = Some(state.input.trim().to_string());
                                        state.input.clear();
                                        state.hint = "Exchanging code for token...".into();
                                    } else {
                                        let provider_id = state.provider_id.clone();
                                        let input = state.input.trim().to_string();
                                        let is_setup = state.hint.contains("setup-token");

                                        if is_setup {
                                            let cred = Credential::SetupToken(SetupTokenCredential {
                                                token: input,
                                            });
                                            config.set_credential(&provider_id, cred)?;
                                        } else {
                                            let cred = Credential::ApiKey(ApiKeyCredential {
                                                key: input,
                                            });
                                            config.set_credential(&provider_id, cred)?;
                                        }
                                        enter_model_selection(&config, &provider_id, screen).await?;
                                    }
                                }
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
        if let Screen::AuthInput(state) = screen {
            if state.is_oauth {
                if config.has_credential(&state.provider_id).unwrap_or(false) {
                    next_provider_id = Some(state.provider_id.clone());
                } else {
                    let info = oauth_callbacks.auth_info.lock().unwrap();
                    if let Some(info) = &*info {
                        state.oauth_url = Some(info.url.clone());
                        if let Some(instr) = &info.instructions {
                            state.hint = instr.clone();
                        }
                    }
                }
            }
        }
        if let Some(pid) = next_provider_id {
            enter_model_selection(&config, &pid, screen).await?;
        }
    }
}

async fn handle_provider_select(
    config: ConfigManager,
    prov: &ProviderAuthInfo,
    screen: &mut Screen,
    callbacks: Arc<TuiOAuthCallbacks>,
) -> anyhow::Result<()> {
    let provider_id = prov.provider_id.clone();

    if config.has_credential(&provider_id).unwrap_or(false) {
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
            });
        }
        AuthMethod::OAuth { hint } => {
            let pid = provider_id.clone();
            let config_mgr = config.clone();
            tokio::spawn(async move {
                let oauth_provider: Box<dyn OAuthProvider + Send> = match pid.as_str() {
                    "anthropic" => Box::new(AnthropicOAuthProvider),
                    "gemini-cli" => Box::new(GeminiCliOAuthProvider),
                    "antigravity" => Box::new(AntigravityOAuthProvider),
                    "openai-codex" => Box::new(zeroai::oauth::openai_codex::OpenAiCodexOAuthProvider),
                    "github-copilot" => Box::new(zeroai::oauth::github_copilot::GitHubCopilotOAuthProvider),
                    "qwen" => Box::new(zeroai::oauth::qwen_portal::QwenPortalOAuthProvider),
                    _ => return,
                };
                if let Ok(creds) = oauth_provider.login(&*callbacks).await {
                    let _ = config_mgr.set_credential(&pid, Credential::OAuth(zeroai::auth::OAuthCredential {
                        refresh: creds.refresh,
                        access: creds.access,
                        expires: creds.expires,
                        extra: creds.extra,
                    }));
                }
            });
            *screen = Screen::AuthInput(AuthInputState {
                provider_id: provider_id.clone(),
                label: format!("OAuth for {}", prov.label),
                input: String::new(),
                hint: hint.unwrap_or_else(|| "Connecting to Google...".into()),
                is_oauth: true,
                oauth_url: None,
            });
        }
    }
    Ok(())
}

async fn enter_model_selection(config: &ConfigManager, provider_id: &str, screen: &mut Screen) -> anyhow::Result<()> {
    let models = static_models_for_provider(provider_id).into_iter().map(|m| m.id).collect::<Vec<_>>();
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
    if !model_items.is_empty() { ls.select(Some(0)); }
    *screen = Screen::ModelSelect(ModelSelectState {
        provider_id: provider_id.to_string(),
        models: model_items,
        list_state: ls,
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
                    Span::raw(" - "),
                    Span::styled(p.hint.as_str(), Style::default().fg(COLOR_GRAY)),
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
        Screen::AuthInput(state) => {
            let has_info = !state.hint.is_empty() || state.oauth_url.is_some();
            let mut constraints = vec![
                Constraint::Length(3), 
                Constraint::Length(3), 
            ];
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
            f.render_widget(Paragraph::new(state.input.clone()).block(Block::default().borders(Borders::ALL).title(input_title)), chunks[1]);

            if has_info {
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
                f.render_widget(info_para, chunks[2]);
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
            let mut ls = state.list_state.clone();
            f.render_stateful_widget(list, area, &mut ls);
        }
    }
}
