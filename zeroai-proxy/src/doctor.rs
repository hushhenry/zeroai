use zeroai::{
    AiClient, ConfigManager, StreamEvent, RequestOptions,
    split_model_id,
    types::{
        ChatContext, ContentBlock, Message, ModelDef, TextContent, ToolDef, ToolResultMessage,
        UserMessage,
    },
};
use futures::StreamExt;
use rand::seq::IndexedRandom;
use serde_json::json;

/// Run the doctor check.
pub async fn run_doctor(model_filter: Option<&str>) -> anyhow::Result<()> {
    let config = ConfigManager::default_path();
    let enabled_models = config.get_enabled_models()?;

    if enabled_models.is_empty() {
        println!("No models configured. Run `ai-proxy config` first.");
        return Ok(());
    }

    let all_static = zeroai::models::static_models::all_static_models();

    // Build the set of models to register with the client
    let mut registered_models: Vec<(String, ModelDef)> = Vec::new();
    for full_id in &enabled_models {
        if let Some((provider, model_id)) = split_model_id(full_id) {
            if let Some(def) = all_static
                .iter()
                .find(|m| m.provider == provider && m.id == model_id)
            {
                registered_models.push((full_id.clone(), def.clone()));
            }
        }
    }

    let client = AiClient::builder()
        .with_models(registered_models.clone())
        .build();

    // Determine which models to check
    let models_to_check: Vec<(String, ModelDef)> = if let Some(filter) = model_filter {
        match registered_models
            .iter()
            .find(|(id, _)| id == filter)
        {
            Some((id, def)) => vec![(id.clone(), def.clone())],
            None => {
                println!("Model not found: {}", filter);
                return Ok(());
            }
        }
    } else {
        // One random model per provider from the enabled list
        let mut provider_models: std::collections::HashMap<String, Vec<(String, ModelDef)>> =
            std::collections::HashMap::new();
        for (full_id, def) in &registered_models {
            if let Some((provider, _)) = split_model_id(full_id) {
                provider_models
                    .entry(provider.to_string())
                    .or_default()
                    .push((full_id.clone(), def.clone()));
            }
        }

        let mut rng = rand::rng();
        let mut selected: Vec<(String, ModelDef)> = Vec::new();

        for (_provider, models) in provider_models {
            if let Some((full_id, def)) = models.choose(&mut rng) {
                selected.push((full_id.clone(), def.clone()));
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

    for (full_id, _model_def) in &models_to_check {
        let (provider, _) = split_model_id(full_id).unwrap();
        let api_key = config.resolve_api_key(provider).await?;

        if api_key.is_none() {
            println!("  {} - No credentials", full_id);
            continue;
        }

        println!("\n📋 Checking {}...", full_id);

        let stream_result = check_model(
            &client,
            full_id,
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
    client: &AiClient,
    full_id: &str,
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

    let options = RequestOptions {
        temperature: Some(0.0),
        max_tokens: Some(1024),
        reasoning: None,
        api_key: Some(api_key.to_string()),
        extra_headers: None,
    };

    let mut stream = client.stream(full_id, &context, &options)?;

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

                match client.stream(full_id, &follow_up, &options) {
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
