use ai::{
    ConfigManager, ModelMapper, StreamEvent, StreamOptions,
    types::{
        ChatContext, ContentBlock, Message, ModelDef, TextContent, ToolDef, ToolResultMessage,
        UserMessage,
    },
};
use futures::StreamExt;
use serde_json::json;

/// Run the doctor check.
pub async fn run_doctor(model_filter: Option<&str>) -> anyhow::Result<()> {
    let config = ConfigManager::default_path();
    let mapper = ModelMapper::new();
    let enabled_models = config.get_enabled_models()?;

    if enabled_models.is_empty() {
        println!("No models configured. Run `ai-proxy config` first.");
        return Ok(());
    }

    let all_static = ai::models::static_models::all_static_models();

    // Determine which models to check
    let models_to_check: Vec<(String, ModelDef)> = if let Some(filter) = model_filter {
        // Check specific model
        match all_static
            .iter()
            .find(|m| {
                let full_id = format!("{}/{}", m.provider, m.id);
                full_id == filter
            })
        {
            Some(def) => vec![(filter.to_string(), def.clone())],
            None => {
                println!("Model not found: {}", filter);
                return Ok(());
            }
        }
    } else {
        // One random model per provider
        let mut providers_seen = std::collections::HashSet::new();
        let mut selected = Vec::new();

        for full_id in &enabled_models {
            if let Some((provider, model_id)) = ModelMapper::parse_model_id(full_id) {
                if providers_seen.contains(provider) {
                    continue;
                }
                if let Some(def) = all_static
                    .iter()
                    .find(|m| m.provider == provider && m.id == model_id)
                {
                    providers_seen.insert(provider.to_string());
                    selected.push((full_id.clone(), def.clone()));
                }
            }
        }

        selected
    };

    if models_to_check.is_empty() {
        println!("No models to check.");
        return Ok(());
    }

    // The test tool
    let tool = ToolDef {
        name: "get_current_time".into(),
        description: "Get the current UTC time.".into(),
        parameters: json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    };

    for (full_id, model_def) in &models_to_check {
        let (provider, _) = ModelMapper::parse_model_id(full_id).unwrap();
        let api_key = config.resolve_api_key(provider)?;

        if api_key.is_none() {
            println!("  {} - No credentials", full_id);
            continue;
        }

        println!("\n📋 Checking {}...", full_id);

        // Test streaming
        let stream_result = check_model(
            &mapper,
            full_id,
            model_def,
            api_key.as_deref().unwrap(),
            &tool,
        )
        .await;

        match stream_result {
            Ok(report) => {
                println!("  Stream:     ✅ {} tokens, stop={:?}", report.total_tokens, report.stop_reason);
                if report.tool_call_received {
                    println!("  Tool call:  ✅ Received");
                    if report.tool_result_ok {
                        println!("  Tool result: ✅ Processed");
                    } else if let Some(err) = report.tool_result_error {
                        println!("  Tool result: ❌ Failed: {}", err);
                    } else {
                        println!("  Tool result: ⚠️  Not triggered by model");
                    }
                } else {
                    println!("  Tool call:  ℹ️  Not triggered");
                }
            }
            Err(e) => {
                println!("  Stream:     ❌ {}", e);
            }
        }
    }

    println!("\nDoctor check complete.");

    Ok(())
}

struct CheckReport {
    total_tokens: u64,
    stop_reason: String,
    tool_call_received: bool,
    tool_result_ok: bool,
    tool_result_error: Option<String>,
}

async fn check_model(
    mapper: &ModelMapper,
    full_id: &str,
    model_def: &ModelDef,
    api_key: &str,
    tool: &ToolDef,
) -> anyhow::Result<CheckReport> {
    let context = ChatContext {
        system_prompt: Some("You are a helpful assistant. When asked for the time, use the get_current_time tool.".into()),
        messages: vec![Message::User(UserMessage {
            content: vec![ContentBlock::Text(TextContent {
                text: "What time is it right now? Please use the tool to check.".into(),
            })],
        })],
        tools: vec![tool.clone()],
    };

    let options = StreamOptions {
        temperature: Some(0.0),
        max_tokens: Some(1024),
        reasoning: None,
        api_key: Some(api_key.to_string()),
        extra_headers: None,
    };

    let mut stream = mapper.stream(full_id, model_def, &context, &options)?;

    let mut report = CheckReport {
        total_tokens: 0,
        stop_reason: "unknown".into(),
        tool_call_received: false,
        tool_result_ok: false,
        tool_result_error: None,
    };

    let mut events: Vec<StreamEvent> = Vec::new();

    while let Some(event) = stream.next().await {
        match event {
            Ok(evt) => events.push(evt),
            Err(e) => return Err(anyhow::anyhow!("{}", e)),
        }
    }

    let mut done_msg = None;

    for evt in &events {
        match evt {
            StreamEvent::Done { message } => {
                report.total_tokens = message.usage.as_ref().map(|u| u.total_tokens).unwrap_or(0);
                report.stop_reason = format!("{:?}", message.stop_reason);
                report.tool_call_received = message
                    .content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolCall(_)));
                done_msg = Some(message.clone());
            }
            StreamEvent::Error { message } => {
                return Err(anyhow::anyhow!(
                    "{}",
                    message
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::Text(t) = b {
                                Some(t.text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("")
                ));
            }
            _ => {}
        }
    }

    // If we got a tool call, do a follow-up with the tool result
    if report.tool_call_received {
        if let Some(msg) = done_msg {
            let tool_call = msg.content.iter().find_map(|b| {
                if let ContentBlock::ToolCall(tc) = b {
                    Some(tc.clone())
                } else {
                    None
                }
            });

            if let Some(tc) = tool_call {
                let follow_up = ChatContext {
                    system_prompt: context.system_prompt.clone(),
                    messages: vec![
                        context.messages[0].clone(),
                        Message::Assistant(msg.clone()),
                        Message::ToolResult(ToolResultMessage {
                            tool_call_id: tc.id,
                            tool_name: tc.name,
                            content: vec![ContentBlock::Text(TextContent {
                                text: chrono::Utc::now().to_rfc3339(),
                            })],
                            is_error: false,
                        }),
                    ],
                    tools: vec![tool.clone()],
                };

                match mapper.stream(full_id, model_def, &follow_up, &options) {
                    Ok(mut s2) => {
                        while let Some(event) = s2.next().await {
                            match event {
                                Ok(StreamEvent::Done { .. }) => {
                                    report.tool_result_ok = true;
                                    break;
                                }
                                Ok(StreamEvent::Error { message }) => {
                                    let err_text = message.content.iter().filter_map(|b| if let ContentBlock::Text(t) = b { Some(t.text.clone()) } else { None }).collect::<Vec<_>>().join("");
                                    report.tool_result_error = Some(format!("Model error in follow-up: {}", err_text));
                                    break;
                                }
                                Err(e) => {
                                    report.tool_result_error = Some(format!("Stream error in follow-up: {}", e));
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        report.tool_result_error = Some(format!("Follow-up start error: {}", e));
                    }
                }
            }
        }
    }

    Ok(report)
}
