use ai::{
    ConfigManager,
    auth::{
        self, AuthMethod, Credential, ApiKeyCredential, SetupTokenCredential,
        ProviderAuthInfo,
    },
    models::static_models::static_models_for_provider,
};
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
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::io::{self, stdout};

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

    let result = run_tui_loop(&mut terminal, &config, &groups, &mut screen, &mut group_state, &mut sub_state).await;

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

async fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: &ConfigManager,
    groups: &[(String, Vec<ProviderAuthInfo>)],
    screen: &mut Screen,
    group_state: &mut ListState,
    sub_state: &mut ListState,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| draw(f, config, groups, screen, group_state, sub_state))?;

        if event::poll(std::time::Duration::from_millis(100))? {
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
                                group_state.select(Some(i.saturating_sub(1)));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let i = group_state.selected().unwrap_or(0);
                                if i + 1 < groups.len() {
                                    group_state.select(Some(i + 1));
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(idx) = group_state.selected() {
                                    if idx < groups.len() {
                                        let (_, providers) = &groups[idx];
                                        if providers.len() == 1 {
                                            let prov = &providers[0];
                                            handle_provider_select(config, prov, screen).await?;
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
                                sub_state.select(Some(i.saturating_sub(1)));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let i = sub_state.selected().unwrap_or(0);
                                if i + 1 < providers.len() {
                                    sub_state.select(Some(i + 1));
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(idx) = sub_state.selected() {
                                    if idx < providers.len() {
                                        let prov = &providers[idx];
                                        handle_provider_select(config, prov, screen).await?;
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
                                    let provider_id = state.provider_id.clone();
                                    let input = state.input.clone();

                                    // Save credential
                                    if state.is_oauth {
                                        // For Anthropic OAuth, input is code#state
                                        // For now, store as api key (the OAuth flow will be enhanced)
                                        let cred = Credential::ApiKey(ApiKeyCredential {
                                            key: input,
                                        });
                                        config.set_credential(&provider_id, cred)?;
                                    } else if state.hint.contains("setup-token") {
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

                                    // Move to model selection
                                    let models = get_provider_models(&provider_id);
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
                                        provider_id,
                                        models: model_items,
                                        list_state: ls,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::ModelSelect(state) => {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('q') => {
                                // Save selected models
                                let selected: Vec<String> = state
                                    .models
                                    .iter()
                                    .filter(|(_, s)| *s)
                                    .map(|(id, _)| id.clone())
                                    .collect();

                                // Remove old models for this provider, add new ones
                                let mut all_enabled = config.get_enabled_models().unwrap_or_default();
                                all_enabled.retain(|m| {
                                    !m.starts_with(&format!("{}/", state.provider_id))
                                });
                                all_enabled.extend(selected);
                                config.set_enabled_models(all_enabled)?;

                                *screen = Screen::ProviderGroups;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let i = state.list_state.selected().unwrap_or(0);
                                state.list_state.select(Some(i.saturating_sub(1)));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let i = state.list_state.selected().unwrap_or(0);
                                if i + 1 < state.models.len() {
                                    state.list_state.select(Some(i + 1));
                                }
                            }
                            KeyCode::Char(' ') => {
                                if let Some(idx) = state.list_state.selected() {
                                    if idx < state.models.len() {
                                        state.models[idx].1 = !state.models[idx].1;
                                    }
                                }
                            }
                            KeyCode::Char('a') => {
                                // Toggle all
                                let all_selected = state.models.iter().all(|(_, s)| *s);
                                for item in &mut state.models {
                                    item.1 = !all_selected;
                                }
                            }
                            KeyCode::Enter => {
                                // Confirm and save
                                let selected: Vec<String> = state
                                    .models
                                    .iter()
                                    .filter(|(_, s)| *s)
                                    .map(|(id, _)| id.clone())
                                    .collect();

                                let mut all_enabled = config.get_enabled_models().unwrap_or_default();
                                all_enabled.retain(|m| {
                                    !m.starts_with(&format!("{}/", state.provider_id))
                                });
                                all_enabled.extend(selected);
                                config.set_enabled_models(all_enabled)?;

                                *screen = Screen::ProviderGroups;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Handle provider selection
// ---------------------------------------------------------------------------

async fn handle_provider_select(
    config: &ConfigManager,
    prov: &ProviderAuthInfo,
    screen: &mut Screen,
) -> anyhow::Result<()> {
    let provider_id = &prov.provider_id;

    // Check if already configured
    let has_cred = config.has_credential(provider_id).unwrap_or(false);

    if has_cred {
        // Skip auth, go directly to model selection
        let models = get_provider_models(provider_id);
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
            provider_id: provider_id.clone(),
            models: model_items,
            list_state: ls,
        });
        return Ok(());
    }

    // Try sniffing
    if let Some(cred) = ai::auth::sniff::sniff_external_credential(provider_id) {
        config.set_credential(provider_id, cred)?;
        let models = get_provider_models(provider_id);
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
            provider_id: provider_id.clone(),
            models: model_items,
            list_state: ls,
        });
        return Ok(());
    }

    if let Some(key) = ai::auth::sniff::env_api_key(provider_id) {
        let cred = Credential::ApiKey(ApiKeyCredential { key });
        config.set_credential(provider_id, cred)?;
        let models = get_provider_models(provider_id);
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
            provider_id: provider_id.clone(),
            models: model_items,
            list_state: ls,
        });
        return Ok(());
    }

    // Need auth - determine method
    let method = prov.auth_methods.first().cloned().unwrap_or(AuthMethod::ApiKey {
        env_var: None,
        hint: None,
    });

    match &method {
        AuthMethod::ApiKey { hint, .. } => {
            *screen = Screen::AuthInput(AuthInputState {
                provider_id: provider_id.clone(),
                label: format!("Enter API key for {}", prov.label),
                input: String::new(),
                hint: hint.clone().unwrap_or_default(),
                is_oauth: false,
                oauth_url: None,
            });
        }
        AuthMethod::SetupToken { hint } => {
            *screen = Screen::AuthInput(AuthInputState {
                provider_id: provider_id.clone(),
                label: format!("Enter setup-token for {}", prov.label),
                input: String::new(),
                hint: hint.clone().unwrap_or_else(|| "Run `claude setup-token` to generate".into()),
                is_oauth: false,
                oauth_url: None,
            });
        }
        AuthMethod::OAuth { hint } => {
            *screen = Screen::AuthInput(AuthInputState {
                provider_id: provider_id.clone(),
                label: format!("OAuth for {}", prov.label),
                input: String::new(),
                hint: hint.clone().unwrap_or_else(|| "Paste the authorization response".into()),
                is_oauth: true,
                oauth_url: None, // Will be generated by the OAuth flow
            });
        }
    }

    Ok(())
}

fn get_provider_models(provider_id: &str) -> Vec<String> {
    static_models_for_provider(provider_id)
        .into_iter()
        .map(|m| m.id)
        .collect()
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

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
            let items: Vec<ListItem> = groups
                .iter()
                .map(|(label, providers)| {
                    let has_any_cred = providers.iter().any(|p| {
                        config.has_credential(&p.provider_id).unwrap_or(false)
                    });
                    let marker = if has_any_cred { "●" } else { "○" };
                    let color = if has_any_cred {
                        Color::Green
                    } else {
                        Color::White
                    };
                    let hint = &providers[0].hint;
                    ListItem::new(Line::from(vec![
                        Span::styled(format!(" {} ", marker), Style::default().fg(color)),
                        Span::raw(format!("{} - {}", label, hint)),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().title(" Providers (↑↓ navigate, Enter select, q quit) ").borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            f.render_stateful_widget(list, area, group_state);
        }
        Screen::SubProviders(group_idx) => {
            let (group_label, providers) = &groups[*group_idx];
            let items: Vec<ListItem> = providers
                .iter()
                .map(|p| {
                    let has_cred = config.has_credential(&p.provider_id).unwrap_or(false);
                    let marker = if has_cred { "●" } else { "○" };
                    let color = if has_cred {
                        Color::Green
                    } else {
                        Color::White
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!(" {} ", marker), Style::default().fg(color)),
                        Span::raw(format!("{} - {}", p.label, p.hint)),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().title(format!(" {} (Esc back) ", group_label)).borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            f.render_stateful_widget(list, area, sub_state);
        }
        Screen::AuthInput(state) => {
            let chunks = Layout::vertical([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(1),
            ])
            .split(area);

            let title = Paragraph::new(state.label.as_str())
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(title, chunks[0]);

            if !state.hint.is_empty() {
                let hint = Paragraph::new(state.hint.as_str())
                    .style(Style::default().fg(Color::DarkGray))
                    .block(Block::default().borders(Borders::ALL).title(" Hint "));
                f.render_widget(hint, chunks[1]);
            }

            if let Some(url) = &state.oauth_url {
                let url_widget = Paragraph::new(url.as_str())
                    .style(Style::default().fg(Color::Cyan))
                    .block(Block::default().borders(Borders::ALL).title(" OAuth URL "));
                f.render_widget(url_widget, chunks[2]);
            }

            let masked: String = if state.input.len() > 4 {
                format!("{}****", &state.input[..4])
            } else {
                "*".repeat(state.input.len())
            };
            let input = Paragraph::new(masked)
                .block(Block::default().borders(Borders::ALL).title(" Input (Enter to confirm, Esc to cancel) "));
            f.render_widget(input, chunks[3]);
        }
        Screen::ModelSelect(state) => {
            let items: Vec<ListItem> = state
                .models
                .iter()
                .map(|(id, selected)| {
                    let marker = if *selected { "[x]" } else { "[ ]" };
                    ListItem::new(format!(" {} {}", marker, id))
                })
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .title(" Models (Space toggle, a select all, Enter confirm) ")
                        .borders(Borders::ALL),
                )
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            let mut ls = state.list_state.clone();
            f.render_stateful_widget(list, area, &mut ls);
        }
    }
}
