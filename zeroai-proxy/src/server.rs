use zeroai::{
    AiClient, ConfigManager, ModelMapper, StreamEvent, StreamOptions,
    types::{
        AssistantMessage, ChatContext, ContentBlock, Message, ModelDef, StopReason, TextContent,
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
    pub client: AiClient,
    pub config: ConfigManager,
    /// Cache of model definitions keyed by `<provider>/<model>`.
    pub models_cache: RwLock<Vec<(String, ModelDef)>>,
}

impl AppState {
    pub async fn new() -> anyhow::Result<Self> {
        let config = ConfigManager::default_path();
        let client = AiClient::builder().build();

        let state = Self {
            client,
            config,
            models_cache: RwLock::new(Vec::new()),
        };

        state.refresh_models_cache().await;

        Ok(state)
    }

    /// Rebuild the models cache from enabled models in config.
    pub async fn refresh_models_cache(&self) {
        let enabled = self.config.get_enabled_models().unwrap_or_default();

        let mut cache = Vec::new();

        // Build model defs from static lists
        let all_static = zeroai::models::static_models::all_static_models();

        for full_id in &enabled {
            if let Some((provider, model_id)) = ModelMapper::default().split_id(full_id) {
                // Look up in static models
                if let Some(def) = all_static
                    .iter()
                    .find(|m| m.provider == provider && m.id == model_id)
                {
                    cache.push((full_id.clone(), def.clone()));
                }
            }
        }

        *self.models_cache.write().await = cache;
    }

    /// Find a model definition by full ID.
    pub async fn find_model(&self, full_id: &str) -> Option<ModelDef> {
        let cache = self.models_cache.read().await;
        cache
            .iter()
            .find(|(id, _)| id == full_id)
            .map(|(_, def)| def.clone())
    }

    /// Resolve API key for a provider.
    pub async fn resolve_api_key(&self, provider: &str) -> Option<String> {
        self.config.resolve_api_key(provider).await.ok().flatten()
    }
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
    let cache = state.models_cache.read().await;
    let data: Vec<ModelObject> = cache
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
    let model_def = match state.find_model(&req.model).await {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": {"message": format!("Model not found: {}", req.model)}})),
            )
                .into_response();
        }
    };

    let (provider_name, _) = match ModelMapper::default().split_id(&req.model) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"message": "Invalid model ID format"}})),
            )
                .into_response();
        }
    };

    let api_key = match state.resolve_api_key(provider_name).await {
        Some(k) => k,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": {"message": format!("No credentials for provider: {}", provider_name)}})),
            )
                .into_response();
        }
    };

    let (system_prompt, messages) = convert_openai_messages(&req.messages);
    let tools = req.tools.as_ref().map(|t| convert_openai_tools(t)).unwrap_or_default();

    let context = ChatContext {
        system_prompt,
        messages,
        tools,
    };

    let options = StreamOptions {
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        reasoning: None,
        api_key: Some(api_key),
        extra_headers: None,
    };

    let is_stream = req.stream.unwrap_or(false);

    if is_stream {
        let event_stream = match state.client.stream(&req.model, &model_def, &context, &options) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"message": e.to_string()}})),
                )
                    .into_response();
            }
        };

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
        // Non-streaming: collect the full response
        let event_stream = match state.client.stream(&req.model, &model_def, &context, &options) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"message": e.to_string()}})),
                )
                    .into_response();
            }
        };

        let mut final_message: Option<AssistantMessage> = None;
        let mut stream = event_stream;

        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::Done { message }) => {
                    final_message = Some(message);
                    break;
                }
                Ok(StreamEvent::Error { message }) => {
                    final_message = Some(message);
                    break;
                }
                _ => {}
            }
        }

        let msg = match final_message {
            Some(m) => m,
            None => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"message": "No response received"}})),
                )
                    .into_response();
            }
        };

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

        Json(response).into_response()
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
    let model_def = match state.find_model(&req.model).await {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"type": "error", "error": {"type": "not_found_error", "message": format!("Model not found: {}", req.model)}})),
            )
                .into_response();
        }
    };

    let (provider_name, _) = match ModelMapper::default().split_id(&req.model) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"type": "error", "error": {"type": "invalid_request_error", "message": "Invalid model ID format"}})),
            )
                .into_response();
        }
    };

    let api_key = match state.resolve_api_key(provider_name).await {
        Some(k) => k,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"type": "error", "error": {"type": "authentication_error", "message": format!("No credentials for: {}", provider_name)}})),
            )
                .into_response();
        }
    };

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

    let options = StreamOptions {
        temperature: req.temperature,
        max_tokens: Some(req.max_tokens),
        reasoning: None,
        api_key: Some(api_key),
        extra_headers: None,
    };

    // Non-streaming Anthropic response
    let event_stream = match state.client.stream(&req.model, &model_def, &context, &options) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"type": "error", "error": {"type": "api_error", "message": e.to_string()}})),
            )
                .into_response();
        }
    };

    let mut final_message: Option<AssistantMessage> = None;
    let mut stream = event_stream;

    while let Some(event) = stream.next().await {
        match event {
            Ok(StreamEvent::Done { message }) => {
                final_message = Some(message);
                break;
            }
            Ok(StreamEvent::Error { message }) => {
                final_message = Some(message);
                break;
            }
            _ => {}
        }
    }

    let msg = match final_message {
        Some(m) => m,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"type": "error", "error": {"type": "api_error", "message": "No response"}})),
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
