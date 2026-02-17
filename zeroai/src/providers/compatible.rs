//! OpenAI-compatible custom provider: configurable base URL, auth, and model listing.
//! Reference: zeroclaw/src/providers/compatible.rs

use super::sanitize;
use super::{Provider, ProviderError};
use crate::types::*;
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// How the API key is sent to the provider.
#[derive(Debug, Clone)]
pub enum AuthStyle {
    /// `Authorization: Bearer <key>`
    Bearer,
    /// `x-api-key: <key>`
    XApiKey,
    /// Custom header name and value pattern (e.g. "Authorization" with "Bearer {key}")
    Custom { header: String, value_prefix: String },
}

/// Provider that speaks OpenAI-compatible `/v1/chat/completions` (and optional GET `/v1/models`).
pub struct OpenAiCompatibleProvider {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub auth_style: AuthStyle,
    /// Custom URL for listing models (GET). If None, uses `{base_url}/models`.
    pub models_url: Option<String>,
    client: Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        name: &str,
        base_url: &str,
        api_key: Option<&str>,
        auth_style: AuthStyle,
    ) -> Self {
        Self {
            name: name.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.map(String::from),
            auth_style,
            models_url: None,
            client: Client::new(),
        }
    }

    pub fn with_models_url(mut self, url: &str) -> Self {
        self.models_url = Some(url.trim_end_matches('/').to_string());
        self
    }

    fn chat_completions_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{}/chat/completions", base)
        }
    }

    fn models_list_url(&self) -> String {
        self.models_url
            .clone()
            .unwrap_or_else(|| format!("{}/models", self.base_url.trim_end_matches('/')))
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder, key: &str) -> reqwest::RequestBuilder {
        match &self.auth_style {
            AuthStyle::Bearer => req.header("Authorization", format!("Bearer {}", key)),
            AuthStyle::XApiKey => req.header("x-api-key", key),
            AuthStyle::Custom { header, value_prefix } => {
                let value = if value_prefix.is_empty() {
                    key.to_string()
                } else if value_prefix.contains("{key}") || value_prefix.contains("{api_key}") {
                    value_prefix.replace("{key}", key).replace("{api_key}", key)
                } else {
                    format!("{}{}", value_prefix, key)
                };
                req.header(header.as_str(), value)
            }
        }
    }
}

// ---- Request/response types (OpenAI wire format) ----
#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMsg>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolSchema>>,
}

#[derive(Serialize)]
struct ChatMsg {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCallReq>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Serialize)]
struct ToolCallReq {
    id: String,
    #[serde(rename = "type")]
    r#type: String,
    function: FunctionCallReq,
}

#[derive(Serialize)]
struct FunctionCallReq {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct ToolSchema {
    #[serde(rename = "type")]
    r#type: String,
    function: FunctionSchema,
}

#[derive(Serialize)]
struct FunctionSchema {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Option<Vec<StreamChoice>>,
    usage: Option<UsageResp>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: Option<DeltaContent>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct DeltaContent {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCallDelta>>,
    #[allow(dead_code)]
    role: Option<String>,
}

#[derive(Deserialize)]
struct ToolCallDelta {
    index: Option<usize>,
    id: Option<String>,
    function: Option<FunctionDelta>,
}

#[derive(Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct UsageResp {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: Option<UsageResp>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResp,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessageResp {
    #[allow(dead_code)]
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<ToolCallResp>>,
}

#[derive(Deserialize)]
struct ToolCallResp {
    id: String,
    #[allow(dead_code)]
    r#type: String,
    function: FunctionResp,
}

#[derive(Deserialize)]
struct FunctionResp {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
    #[allow(dead_code)]
    owned_by: Option<String>,
}

fn convert_messages(context: &ChatContext) -> Vec<ChatMsg> {
    let mut msgs = Vec::new();
    if let Some(sys) = &context.system_prompt {
        msgs.push(ChatMsg {
            role: "system".into(),
            content: Some(json!(sys)),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
    }
    for msg in &context.messages {
        match msg {
            Message::User(u) => {
                let content = user_content_to_json(&u.content);
                msgs.push(ChatMsg {
                    role: "user".into(),
                    content: Some(content),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                });
            }
            Message::Assistant(a) => {
                let mut text_parts = String::new();
                let mut tool_calls = Vec::new();
                for block in &a.content {
                    match block {
                        ContentBlock::Text(t) => text_parts.push_str(&t.text),
                        ContentBlock::ToolCall(tc) => {
                            tool_calls.push(ToolCallReq {
                                id: tc.id.clone(),
                                r#type: "function".into(),
                                function: FunctionCallReq {
                                    name: tc.name.clone(),
                                    arguments: tc.arguments.to_string(),
                                },
                            });
                        }
                        _ => {}
                    }
                }
                msgs.push(ChatMsg {
                    role: "assistant".into(),
                    content: if text_parts.is_empty() {
                        None
                    } else {
                        Some(json!(text_parts))
                    },
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                    name: None,
                });
            }
            Message::ToolResult(tr) => {
                let text = tr
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
                    .join("\n");
                msgs.push(ChatMsg {
                    role: "tool".into(),
                    content: Some(json!(text)),
                    tool_calls: None,
                    tool_call_id: Some(tr.tool_call_id.clone()),
                    name: Some(tr.tool_name.clone()),
                });
            }
        }
    }
    msgs
}

fn user_content_to_json(blocks: &[ContentBlock]) -> serde_json::Value {
    if blocks.len() == 1 {
        if let ContentBlock::Text(t) = &blocks[0] {
            return json!(t.text);
        }
    }
    let parts: Vec<serde_json::Value> = blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(json!({"type": "text", "text": t.text})),
            ContentBlock::Image(img) => Some(json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{};base64,{}", img.mime_type, img.data)
                }
            })),
            _ => None,
        })
        .collect();
    json!(parts)
}

fn convert_tools(tools: &[ToolDef]) -> Vec<ToolSchema> {
    tools
        .iter()
        .map(|t| ToolSchema {
            r#type: "function".into(),
            function: FunctionSchema {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            },
        })
        .collect()
}

fn parse_sse_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with("data: ") {
        let data = &trimmed[6..];
        if data == "[DONE]" {
            return None;
        }
        Some(data)
    } else {
        None
    }
}

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    fn stream(
        &self,
        model: &ModelDef,
        context: &ChatContext,
        options: &RequestOptions,
    ) -> BoxStream<'static, Result<StreamEvent, ProviderError>> {
        let name = self.name.clone();
        let api_key = match options.api_key.as_deref().or(self.api_key.as_deref()) {
            Some(k) => k.to_string(),
            None => {
                return Box::pin(stream::once(async move {
                    Err(ProviderError::AuthRequired(format!(
                        "API key required for {}",
                        name
                    )))
                }));
            }
        };

        let url = self.chat_completions_url();
        let messages = convert_messages(context);
        let tools = if context.tools.is_empty() {
            None
        } else {
            Some(convert_tools(&context.tools))
        };
        let body = ChatRequest {
            model: model.id.clone(),
            messages,
            temperature: options.temperature,
            max_tokens: options.max_tokens,
            stream: true,
            tools,
        };

        let client = self.client.clone();
        let auth_style = self.auth_style.clone();
        let model_id = model.id.clone();
        let provider_id = model.provider.clone();
        let extra_headers = options.extra_headers.clone();
        let model_headers = model.headers.clone();

        let s = async_stream::stream! {
            let mut req = client.post(&url).header("Content-Type", "application/json");
            req = match &auth_style {
                AuthStyle::Bearer => req.header("Authorization", format!("Bearer {}", api_key)),
                AuthStyle::XApiKey => req.header("x-api-key", &api_key),
                AuthStyle::Custom { header, value_prefix } => {
                    let v = value_prefix.replace("{key}", &api_key).replace("{api_key}", &api_key);
                    req.header(header, v)
                }
            };
            if let Some(ref extra) = extra_headers {
                for (k, v) in extra {
                    req = req.header(k.as_str(), v.as_str());
                }
            }
            if let Some(ref h) = model_headers {
                for (k, v) in h {
                    req = req.header(k.as_str(), v.as_str());
                }
            }

            let resp = match req.json(&body).send().await {
                Ok(r) => r,
                Err(e) => { yield Err(ProviderError::Network(e)); return; }
            };
            let status = resp.status();
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                yield Err(ProviderError::Http {
                    status: status.as_u16(),
                    body: sanitize::sanitize_api_error(&body_text),
                });
                return;
            }
            yield Ok(StreamEvent::Start);

            let mut text_buf = String::new();
            let mut tool_calls: Vec<(String, String, String)> = Vec::new();
            let mut usage = Usage::default();
            let mut stop_reason = StopReason::Stop;
            let mut line_buf = String::new();
            let mut byte_stream = resp.bytes_stream();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk_bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => { yield Err(ProviderError::Network(e)); return; }
                };
                line_buf.push_str(&String::from_utf8_lossy(&chunk_bytes));
                while let Some(newline_pos) = line_buf.find('\n') {
                    let line: String = line_buf.drain(..=newline_pos).collect();
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let data = match parse_sse_line(&line) {
                        Some(d) => d,
                        None => continue,
                    };
                    let chunk: StreamChunk = match serde_json::from_str(data) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    if let Some(u) = chunk.usage {
                        usage.input_tokens = u.prompt_tokens.unwrap_or(0);
                        usage.output_tokens = u.completion_tokens.unwrap_or(0);
                        usage.total_tokens = u.total_tokens.unwrap_or(0);
                    }
                    if let Some(choices) = chunk.choices {
                        for choice in choices {
                            if let Some(reason) = &choice.finish_reason {
                                stop_reason = match reason.as_str() {
                                    "stop" => StopReason::Stop,
                                    "length" => StopReason::Length,
                                    "tool_calls" => StopReason::ToolUse,
                                    _ => StopReason::Stop,
                                };
                            }
                            if let Some(delta) = &choice.delta {
                                if let Some(content) = &delta.content {
                                    text_buf.push_str(content);
                                    yield Ok(StreamEvent::TextDelta(content.clone()));
                                }
                                if let Some(tc_deltas) = &delta.tool_calls {
                                    for tc_delta in tc_deltas {
                                        let idx = tc_delta.index.unwrap_or(tool_calls.len());
                                        while tool_calls.len() <= idx {
                                            tool_calls.push((String::new(), String::new(), String::new()));
                                        }
                                        if let Some(id) = &tc_delta.id {
                                            tool_calls[idx].0 = id.clone();
                                        }
                                        if let Some(func) = &tc_delta.function {
                                            if let Some(name) = &func.name {
                                                if tool_calls[idx].1.is_empty() {
                                                    tool_calls[idx].1 = name.clone();
                                                    yield Ok(StreamEvent::ToolCallStart {
                                                        index: idx,
                                                        id: tool_calls[idx].0.clone(),
                                                        name: name.clone(),
                                                    });
                                                }
                                            }
                                            if let Some(args) = &func.arguments {
                                                tool_calls[idx].2.push_str(args);
                                                yield Ok(StreamEvent::ToolCallDelta {
                                                    index: idx,
                                                    delta: args.clone(),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            for (idx, (id, name, args_str)) in tool_calls.iter().enumerate() {
                let arguments: serde_json::Value =
                    serde_json::from_str(args_str).unwrap_or(json!({}));
                yield Ok(StreamEvent::ToolCallEnd {
                    index: idx,
                    tool_call: ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments,
                    },
                });
            }

            let mut content = Vec::new();
            if !text_buf.is_empty() {
                content.push(ContentBlock::Text(TextContent { text: text_buf }));
            }
            for (id, name, args_str) in tool_calls {
                let arguments: serde_json::Value =
                    serde_json::from_str(&args_str).unwrap_or(json!({}));
                content.push(ContentBlock::ToolCall(ToolCall {
                    id,
                    name,
                    arguments,
                }));
            }
            usage.total_tokens = usage.input_tokens + usage.output_tokens;
            yield Ok(StreamEvent::Done {
                message: AssistantMessage {
                    content,
                    model: model_id,
                    provider: provider_id,
                    usage: Some(usage),
                    stop_reason,
                },
            });
        };
        Box::pin(s)
    }

    async fn chat(
        &self,
        model: &ModelDef,
        context: &ChatContext,
        options: &RequestOptions,
    ) -> Result<AssistantMessage, ProviderError> {
        let api_key = options
            .api_key
            .as_deref()
            .or(self.api_key.as_deref())
            .ok_or_else(|| {
                ProviderError::AuthRequired(format!("API key required for {}", self.name))
            })?;

        let url = self.chat_completions_url();
        let messages = convert_messages(context);
        let tools = if context.tools.is_empty() {
            None
        } else {
            Some(convert_tools(&context.tools))
        };
        let body = ChatRequest {
            model: model.id.clone(),
            messages,
            temperature: options.temperature,
            max_tokens: options.max_tokens,
            stream: false,
            tools,
        };

        let mut req = self.client.post(&url).header("Content-Type", "application/json");
        req = self.apply_auth(req, api_key);
        if let Some(extra) = &options.extra_headers {
            for (k, v) in extra {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        if let Some(ref h) = model.headers {
            for (k, v) in h {
                req = req.header(k.as_str(), v.as_str());
            }
        }

        let resp = req.json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: sanitize::sanitize_api_error(&body_text),
            });
        }

        let chat_resp: ChatResponse = resp.json().await?;
        let mut usage = Usage::default();
        if let Some(u) = chat_resp.usage {
            usage.input_tokens = u.prompt_tokens.unwrap_or(0);
            usage.output_tokens = u.completion_tokens.unwrap_or(0);
            usage.total_tokens = u.total_tokens.unwrap_or(0);
        }

        if let Some(choice) = chat_resp.choices.first() {
            let mut content = Vec::new();
            if let Some(text) = &choice.message.content {
                content.push(ContentBlock::Text(TextContent { text: text.clone() }));
            }
            if let Some(tc_resps) = &choice.message.tool_calls {
                for tc in tc_resps {
                    let arguments: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                    content.push(ContentBlock::ToolCall(ToolCall {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        arguments,
                    }));
                }
            }
            let stop_reason = match choice.finish_reason.as_deref() {
                Some("stop") => StopReason::Stop,
                Some("length") => StopReason::Length,
                Some("tool_calls") => StopReason::ToolUse,
                _ => StopReason::Stop,
            };
            Ok(AssistantMessage {
                content,
                model: model.id.clone(),
                provider: model.provider.clone(),
                usage: Some(usage),
                stop_reason,
            })
        } else {
            Err(ProviderError::Other("Empty response".into()))
        }
    }

    async fn list_models(&self, api_key: &str) -> Result<Vec<ModelDef>, ProviderError> {
        let url = self.models_list_url();
        let mut req = self.client.get(&url);
        req = self.apply_auth(req, api_key);

        let resp = req.send().await?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status,
                body: sanitize::sanitize_api_error(&body),
            });
        }

        let models_resp: ModelsResponse = resp.json().await?;
        let base_url = self.base_url.clone();
        let provider = self.name.clone();

        let models = models_resp
            .data
            .into_iter()
            .map(|entry| ModelDef {
                id: entry.id.clone(),
                name: entry.id.clone(),
                api: Api::OpenaiCompletions,
                provider: provider.clone(),
                base_url: base_url.clone(),
                reasoning: false,
                input: vec![InputModality::Text],
                cost: ModelCost::default(),
                context_window: 128000,
                max_tokens: 16384,
                headers: None,
            })
            .collect();

        Ok(models)
    }
}
