use zeroai::{
    AiClient, ConfigManager, StreamEvent, RequestOptions,
    split_model_id,
    providers::retry as retry_helpers,
    types::{
        AssistantMessage, ChatContext, ContentBlock, Message, StopReason, TextContent,
        ThinkingContent, ToolCall, ToolDef, ToolResultMessage, UserMessage,
    },
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub client: RwLock<AiClient>,
    pub config: ConfigManager,
}

impl AppState {
    pub async fn new() -> anyhow::Result<Self> {
        let config = ConfigManager::default_path();
        let client = build_client(&config);

        Ok(Self {
            client: RwLock::new(client),
            config,
        })
    }

    /// Rebuild the AiClient with fresh model data from config.
    pub async fn refresh_models(&self) {
        let new_client = build_client(&self.config);
        *self.client.write().await = new_client;
    }

    /// Resolve an account+api_key for a provider.
    pub async fn resolve_account(&self, provider: &str) -> Option<zeroai::auth::config::AccountSelection> {
        self.config.resolve_account(provider).await.ok().flatten()
    }
}

/// Build an AiClient populated with the enabled models from config.
fn build_client(config: &ConfigManager) -> AiClient {
    let enabled = config.get_enabled_models().unwrap_or_default();
    let all_static = zeroai::models::static_models::all_static_models();

    let mut models = Vec::new();
    for full_id in &enabled {
        if let Some((provider, model_id)) = split_model_id(full_id) {
            if let Some(def) = all_static
                .iter()
                .find(|m| m.provider == provider && m.id == model_id)
            {
                models.push((full_id.clone(), def.clone()));
            }
        }
    }

    AiClient::builder().with_models(models).build()
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

pub async fn run_server(host: &str, port: u16) -> anyhow::Result<()> {
    let state = Arc::new(AppState::new().await?);

    // Start background auto-refresh service (check every 15 minutes, with 20 minute buffer)
    let refresh_config = state.config.clone();
    refresh_config.start_auto_refresh_service(15 * 60, 20 * 60);

    let app = Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(anthropic_messages))
        .with_state(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("AI proxy listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// GET /v1/models - OpenAI compatible
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ModelsResponse {
    object: String,
    data: Vec<ModelObject>,
}

#[derive(Serialize)]
struct ModelObject {
    id: String,
    object: String,
    created: i64,
    owned_by: String,
}

async fn list_models(State(state): State<Arc<AppState>>) -> Json<ModelsResponse> {
    let client = state.client.read().await;
    let data: Vec<ModelObject> = client
        .models()
        .iter()
        .map(|(full_id, def)| ModelObject {
            id: full_id.clone(),
            object: "model".into(),
            created: 0,
            owned_by: def.provider.clone(),
        })
        .collect();

    Json(ModelsResponse {
        object: "list".into(),
        data,
    })
}

// ---------------------------------------------------------------------------
// POST /v1/chat/completions - OpenAI compatible
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_tokens: Option<u64>,
    #[serde(default)]
    tools: Option<Vec<OpenAITool>>,
}

#[derive(Deserialize)]
struct OpenAIMessage {
    role: String,
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAIToolCall>>,
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct OpenAIToolCall {
    id: String,
    function: OpenAIFunction,
}

#[derive(Deserialize)]
struct OpenAIFunction {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct OpenAITool {
    function: OpenAIToolFunction,
}

#[derive(Deserialize)]
struct OpenAIToolFunction {
    name: String,
    description: Option<String>,
    parameters: Option<serde_json::Value>,
}

fn convert_openai_messages(msgs: &[OpenAIMessage]) -> (Option<String>, Vec<Message>) {
    let mut system = None;
    let mut messages = Vec::new();

    for msg in msgs {
        match msg.role.as_str() {
            "system" => {
                if let Some(content) = &msg.content {
                    system = content.as_str().map(String::from);
                }
            }
            "user" => {
                let text = msg
                    .content
                    .as_ref()
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                messages.push(Message::User(UserMessage {
                    content: vec![ContentBlock::Text(TextContent { text })],
                }));
            }
            "assistant" => {
                let mut content = Vec::new();
                if let Some(c) = &msg.content {
                    if let Some(text) = c.as_str() {
                        if !text.is_empty() {
                            content.push(ContentBlock::Text(TextContent {
                                text: text.to_string(),
                            }));
                        }
                    }
                }
                if let Some(tcs) = &msg.tool_calls {
                    for tc in tcs {
                        let args: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                        content.push(ContentBlock::ToolCall(ToolCall {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            arguments: args,
                        }));
                    }
                }
                messages.push(Message::Assistant(AssistantMessage {
                    content,
                    model: String::new(),
                    provider: String::new(),
                    usage: None,
                    stop_reason: StopReason::Stop,
                }));
            }
            "tool" => {
                let text = msg
                    .content
                    .as_ref()
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                messages.push(Message::ToolResult(ToolResultMessage {
                    tool_call_id: msg.tool_call_id.clone().unwrap_or_default(),
                    tool_name: msg.name.clone().unwrap_or_default(),
                    content: vec![ContentBlock::Text(TextContent { text })],
                    is_error: false,
                }));
            }
            _ => {}
        }
    }

    (system, messages)
}

fn convert_openai_tools(tools: &[OpenAITool]) -> Vec<ToolDef> {
    tools
        .iter()
        .map(|t| ToolDef {
            name: t.function.name.clone(),
            description: t.function.description.clone().unwrap_or_default(),
            parameters: t.function.parameters.clone().unwrap_or(json!({})),
        })
        .collect()
}

async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatCompletionRequest>,
) -> Response {
    let provider_name = match split_model_id(&req.model) {
        Some((p, _)) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"message": "Invalid model ID format"}})),
            )
                .into_response();
        }
    };

    let client_arc = {
        let client = state.client.read().await;
        Arc::new((*client).clone())
    };

    if client_arc.get_model(&req.model).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"message": format!("Model not found: {}", req.model)}})),
        )
            .into_response();
    }

    let (system_prompt, messages) = convert_openai_messages(&req.messages);
    let tools = req.tools.as_ref().map(|t| convert_openai_tools(t)).unwrap_or_default();

    let context = ChatContext {
        system_prompt,
        messages,
        tools,
    };

    let base_options = RequestOptions {
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        reasoning: None,
        api_key: None,
        extra_headers: None,
        retry_config: None,
    };

    let is_stream = req.stream.unwrap_or(false);

    if is_stream {
        // Streaming rotation strategy:
        // - pick first healthy account
        // - if the stream fails with 429 BEFORE any content/tool events are emitted, rotate+retry with next account
        // - once anything is emitted, we cannot safely restart; return the error
        let provider_name2 = provider_name.clone();
        let state2 = state.clone();
        let model = req.model.clone();
        let ctx = context.clone();
        let opts0 = base_options.clone();
        let client_arc2 = client_arc.clone();

        let event_stream = async_stream::stream! {
            let mut attempt: usize = 0;
            let max_attempts: usize = state2.config.list_accounts(&provider_name2).map(|v| v.len().max(1)).unwrap_or(1);

            loop {
                let mut emitted_any = false;
                let sel = match state2.resolve_account(&provider_name2).await {
                    Some(s) => s,
                    None => {
                        yield Err(zeroai::ProviderError::AuthRequired(format!("No credentials for provider: {}", provider_name2)));
                        return;
                    }
                };

                let mut opts = opts0.clone();
                opts.api_key = Some(sel.api_key.clone());

                let mut inner = match client_arc2.stream(&model, &ctx, &opts) {
                    Ok(s) => s,
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                };

                while let Some(item) = inner.next().await {
                    match item {
                        Ok(evt) => {
                            match &evt {
                                StreamEvent::TextDelta(_) | StreamEvent::ThinkingDelta(_) | StreamEvent::ToolCallStart {..} | StreamEvent::ToolCallDelta {..} | StreamEvent::ToolCallEnd {..} | StreamEvent::Done {..} => {
                                    emitted_any = true;
                                }
                                _ => {}
                            }
                            yield Ok(evt);
                        }
                        Err(e) => {
                            if !emitted_any && retry_helpers::is_rate_limited(&e) && attempt + 1 < max_attempts {
                                let backoff_ms = retry_helpers::parse_retry_after_ms(&e).unwrap_or(60_000);
                                let _ = state2.config.rate_limit_account(&provider_name2, &sel.account_id, backoff_ms);
                                attempt += 1;
                                // retry outer loop
                                break;
                            }
                            yield Err(e);
                            return;
                        }
                    }
                }

                if attempt + 1 >= max_attempts {
                    return;
                }

                // if inner ended without error, we're done
                if emitted_any {
                    return;
                }
            }
        };

        let event_stream: futures::stream::BoxStream<'static, Result<StreamEvent, zeroai::ProviderError>> = Box::pin(event_stream);

        // Map to OpenAI SSE
        let event_stream = event_stream;


        let model_name = req.model.clone();
        let sse = event_stream.filter_map(move |event| {
            let model_name = model_name.clone();
            async move {
                match event {
                    Ok(StreamEvent::TextDelta(delta)) => {
                        let chunk = json!({
                            "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                            "object": "chat.completion.chunk",
                            "created": chrono::Utc::now().timestamp(),
                            "model": model_name,
                            "choices": [{
                                "index": 0,
                                "delta": {"content": delta},
                                "finish_reason": null
                            }]
                        });
                        Some(Ok::<_, std::convert::Infallible>(
                            Event::default().data(chunk.to_string()),
                        ))
                    }
                    Ok(StreamEvent::ToolCallStart { index, id, name }) => {
                        let chunk = json!({
                            "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                            "object": "chat.completion.chunk",
                            "created": chrono::Utc::now().timestamp(),
                            "model": model_name,
                            "choices": [{
                                "index": 0,
                                "delta": {
                                    "tool_calls": [{
                                        "index": index,
                                        "id": id,
                                        "type": "function",
                                        "function": {"name": name, "arguments": ""}
                                    }]
                                },
                                "finish_reason": null
                            }]
                        });
                        Some(Ok(Event::default().data(chunk.to_string())))
                    }
                    Ok(StreamEvent::ToolCallDelta { index, delta }) => {
                        let chunk = json!({
                            "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                            "object": "chat.completion.chunk",
                            "created": chrono::Utc::now().timestamp(),
                            "model": model_name,
                            "choices": [{
                                "index": 0,
                                "delta": {
                                    "tool_calls": [{
                                        "index": index,
                                        "function": {"arguments": delta}
                                    }]
                                },
                                "finish_reason": null
                            }]
                        });
                        Some(Ok(Event::default().data(chunk.to_string())))
                    }
                    Ok(StreamEvent::Done { message }) => {
                        let reason = match message.stop_reason {
                            StopReason::Stop => "stop",
                            StopReason::Length => "length",
                            StopReason::ToolUse => "tool_calls",
                            _ => "stop",
                        };
                        let chunk = json!({
                            "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                            "object": "chat.completion.chunk",
                            "created": chrono::Utc::now().timestamp(),
                            "model": model_name,
                            "choices": [{
                                "index": 0,
                                "delta": {},
                                "finish_reason": reason
                            }],
                            "usage": message.usage.as_ref().map(|u| json!({
                                "prompt_tokens": u.input_tokens,
                                "completion_tokens": u.output_tokens,
                                "total_tokens": u.total_tokens,
                            }))
                        });
                        Some(Ok(Event::default().data(chunk.to_string())))
                    }
                    Ok(StreamEvent::Error { message }) => {
                        let chunk = json!({
                            "error": {"message": message.content.iter().filter_map(|b| {
                                if let ContentBlock::Text(t) = b { Some(t.text.as_str()) } else { None }
                            }).collect::<Vec<_>>().join("")}
                        });
                        Some(Ok(Event::default().data(chunk.to_string())))
                    }
                    _ => None,
                }
            }
        });

        Sse::new(sse).into_response()
    } else {
        // Non-streaming: rotate accounts on 429.
        let max_attempts: usize = state
            .config
            .list_accounts(&provider_name)
            .map(|v| v.len().max(1))
            .unwrap_or(1);

        let mut last_err: Option<zeroai::ProviderError> = None;
        for attempt in 0..max_attempts {
            let sel = match state.resolve_account(&provider_name).await {
                Some(s) => s,
                None => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error": {"message": format!("No credentials for provider: {}", provider_name)}})),
                    )
                        .into_response();
                }
            };

            let mut options = base_options.clone();
            options.api_key = Some(sel.api_key.clone());

            match client_arc.chat(&req.model, &context, &options).await {
                Ok(msg) => {
                    // Format OpenAI-compatible response below
                    let mut content_text = String::new();
                    let mut tool_calls_json = Vec::new();

                    for block in &msg.content {
                        match block {
                            ContentBlock::Text(t) => content_text.push_str(&t.text),
                            ContentBlock::ToolCall(tc) => {
                                tool_calls_json.push(json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": tc.arguments.to_string()
                                    }
                                }));
                            }
                            _ => {}
                        }
                    }

                    let finish_reason = match msg.stop_reason {
                        StopReason::Stop => "stop",
                        StopReason::Length => "length",
                        StopReason::ToolUse => "tool_calls",
                        _ => "stop",
                    };

                    let response = json!({
                        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                        "object": "chat.completion",
                        "created": chrono::Utc::now().timestamp(),
                        "model": req.model,
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": if content_text.is_empty() { serde_json::Value::Null } else { json!(content_text) },
                                "tool_calls": if tool_calls_json.is_empty() { serde_json::Value::Null } else { json!(tool_calls_json) }
                            },
                            "finish_reason": finish_reason
                        }],
                        "usage": msg.usage.as_ref().map(|u| json!({
                            "prompt_tokens": u.input_tokens,
                            "completion_tokens": u.output_tokens,
                            "total_tokens": u.total_tokens,
                        }))
                    });

                    return Json(response).into_response();
                }
                Err(e) => {
                    if retry_helpers::is_rate_limited(&e) && attempt + 1 < max_attempts {
                        let backoff_ms = retry_helpers::parse_retry_after_ms(&e).unwrap_or(60_000);
                        let _ = state
                            .config
                            .rate_limit_account(&provider_name, &sel.account_id, backoff_ms);
                        last_err = Some(e);
                        continue;
                    }
                    last_err = Some(e);
                    break;
                }
            }
        }

        let msg = last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "No response received".into());
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"message": msg}})),
        )
            .into_response()
    }
}

// ---------------------------------------------------------------------------
// POST /v1/messages - Anthropic compatible
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[allow(dead_code)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u64,
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    tools: Option<Vec<AnthropicToolReq>>,
}

#[derive(Deserialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Deserialize)]
struct AnthropicToolReq {
    name: String,
    description: Option<String>,
    input_schema: Option<serde_json::Value>,
}

fn convert_anthropic_messages(
    msgs: &[AnthropicMessage],
) -> Vec<Message> {
    let mut messages = Vec::new();

    for msg in msgs {
        match msg.role.as_str() {
            "user" => {
                let text = msg.content.as_str().unwrap_or("").to_string();
                messages.push(Message::User(UserMessage {
                    content: vec![ContentBlock::Text(TextContent { text })],
                }));
            }
            "assistant" => {
                let mut content = Vec::new();
                if let Some(text) = msg.content.as_str() {
                    content.push(ContentBlock::Text(TextContent {
                        text: text.to_string(),
                    }));
                } else if let Some(blocks) = msg.content.as_array() {
                    for block in blocks {
                        if let Some(block_type) = block.get("type").and_then(|v| v.as_str()) {
                            match block_type {
                                "text" => {
                                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                        content.push(ContentBlock::Text(TextContent {
                                            text: text.to_string(),
                                        }));
                                    }
                                }
                                "thinking" => {
                                    if let Some(text) = block.get("thinking").and_then(|v| v.as_str()) {
                                        content.push(ContentBlock::Thinking(ThinkingContent {
                                            thinking: text.to_string(),
                                            signature: None,
                                        }));
                                    }
                                }
                                "tool_use" => {
                                    let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let input = block.get("input").cloned().unwrap_or(json!({}));
                                    content.push(ContentBlock::ToolCall(ToolCall {
                                        id,
                                        name,
                                        arguments: input,
                                    }));
                                }
                                _ => {}
                            }
                        }
                    }
                }
                messages.push(Message::Assistant(AssistantMessage {
                    content,
                    model: String::new(),
                    provider: String::new(),
                    usage: None,
                    stop_reason: StopReason::Stop,
                }));
            }
            _ => {}
        }
    }

    messages
}

async fn anthropic_messages(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AnthropicRequest>,
) -> Response {
    let provider_name = match split_model_id(&req.model) {
        Some((p, _)) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"type": "error", "error": {"type": "invalid_request_error", "message": "Invalid model ID format"}})),
            )
                .into_response();
        }
    };

    let client = state.client.read().await;
    if client.get_model(&req.model).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"type": "error", "error": {"type": "not_found_error", "message": format!("Model not found: {}", req.model)}})),
        )
            .into_response();
    }

    let messages = convert_anthropic_messages(&req.messages);
    let tools = req
        .tools
        .as_ref()
        .map(|t| {
            t.iter()
                .map(|tool| ToolDef {
                    name: tool.name.clone(),
                    description: tool.description.clone().unwrap_or_default(),
                    parameters: tool.input_schema.clone().unwrap_or(json!({})),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let context = ChatContext {
        system_prompt: req.system.clone(),
        messages,
        tools,
    };

    let base_options = RequestOptions {
        temperature: req.temperature,
        max_tokens: Some(req.max_tokens),
        reasoning: None,
        api_key: None,
        extra_headers: None,
        retry_config: None,
    };

    let max_attempts: usize = state
        .config
        .list_accounts(&provider_name)
        .map(|v| v.len().max(1))
        .unwrap_or(1);

    let mut last_err: Option<zeroai::ProviderError> = None;
    let mut msg_opt: Option<AssistantMessage> = None;

    for attempt in 0..max_attempts {
        let sel = match state.resolve_account(&provider_name).await {
            Some(s) => s,
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"type": "error", "error": {"type": "authentication_error", "message": format!("No credentials for: {}", provider_name)}})),
                )
                    .into_response();
            }
        };

        let mut options = base_options.clone();
        options.api_key = Some(sel.api_key.clone());

        match client.chat(&req.model, &context, &options).await {
            Ok(m) => {
                msg_opt = Some(m);
                break;
            }
            Err(e) => {
                if retry_helpers::is_rate_limited(&e) && attempt + 1 < max_attempts {
                    let backoff_ms = retry_helpers::parse_retry_after_ms(&e).unwrap_or(60_000);
                    let _ = state
                        .config
                        .rate_limit_account(&provider_name, &sel.account_id, backoff_ms);
                    last_err = Some(e);
                    continue;
                }
                last_err = Some(e);
                break;
            }
        }
    }

    let msg = match msg_opt {
        Some(m) => m,
        None => {
            let message = last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "No response".into());
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"type": "error", "error": {"type": "api_error", "message": message}})),
            )
                .into_response();
        }
    };

    let mut content_blocks = Vec::new();
    for block in &msg.content {
        match block {
            ContentBlock::Text(t) => {
                content_blocks.push(json!({"type": "text", "text": t.text}));
            }
            ContentBlock::Thinking(th) => {
                content_blocks.push(json!({"type": "thinking", "thinking": th.thinking}));
            }
            ContentBlock::ToolCall(tc) => {
                content_blocks.push(json!({
                    "type": "tool_use",
                    "id": tc.id,
                    "name": tc.name,
                    "input": tc.arguments
                }));
            }
            _ => {}
        }
    }

    let stop_reason = match msg.stop_reason {
        StopReason::Stop => "end_turn",
        StopReason::Length => "max_tokens",
        StopReason::ToolUse => "tool_use",
        _ => "end_turn",
    };

    let response = json!({
        "id": format!("msg_{}", uuid::Uuid::new_v4()),
        "type": "message",
        "role": "assistant",
        "content": content_blocks,
        "model": req.model,
        "stop_reason": stop_reason,
        "usage": msg.usage.as_ref().map(|u| json!({
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "cache_read_input_tokens": u.cache_read_tokens,
            "cache_creation_input_tokens": u.cache_write_tokens,
        }))
    });

    Json(response).into_response()
}
