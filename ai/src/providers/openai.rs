use super::{Provider, ProviderError};
use crate::types::*;
use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

/// OpenAI-compatible provider (also used by xAI, Groq, DeepSeek, etc.).
pub struct OpenAiProvider {
    client: Client,
}

impl OpenAiProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for OpenAiProvider {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolSchema>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptionsReq>,
}

#[derive(Serialize)]
struct StreamOptionsReq {
    include_usage: bool,
}

#[derive(Serialize)]
struct ChatMessage {
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

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Models list response
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn convert_messages(context: &ChatContext) -> Vec<ChatMessage> {
    let mut msgs = Vec::new();

    if let Some(sys) = &context.system_prompt {
        msgs.push(ChatMessage {
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
                msgs.push(ChatMessage {
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

                msgs.push(ChatMessage {
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

                msgs.push(ChatMessage {
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

// ---------------------------------------------------------------------------
// SSE parsing
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Provider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for OpenAiProvider {
    fn stream(
        &self,
        model: &ModelDef,
        context: &ChatContext,
        options: &StreamOptions,
    ) -> BoxStream<'static, Result<StreamEvent, ProviderError>> {
        let api_key = match &options.api_key {
            Some(k) => k.clone(),
            None => {
                return Box::pin(stream::once(async {
                    Err(ProviderError::AuthRequired(
                        "API key required for OpenAI".into(),
                    ))
                }));
            }
        };

        let base_url = model.base_url.trim_end_matches('/').to_string();
        let url = format!("{}/chat/completions", base_url);

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
            stream_options: Some(StreamOptionsReq {
                include_usage: true,
            }),
        };

        let mut headers_map = HashMap::new();
        if let Some(model_headers) = &model.headers {
            headers_map.extend(model_headers.clone());
        }
        if let Some(extra) = &options.extra_headers {
            headers_map.extend(extra.clone());
        }

        let client = self.client.clone();
        let model_id = model.id.clone();
        let provider_id = model.provider.clone();

        let s = async_stream::stream! {
            let mut req = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json");

            for (k, v) in &headers_map {
                req = req.header(k.as_str(), v.as_str());
            }

            let resp = match req.json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    yield Err(ProviderError::Network(e));
                    return;
                }
            };

            let status = resp.status();
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                yield Err(ProviderError::Http {
                    status: status.as_u16(),
                    body: body_text,
                });
                return;
            }

            yield Ok(StreamEvent::Start);

            let mut text_buf = String::new();
            let mut tool_calls: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
            let mut usage = Usage::default();
            let mut stop_reason = StopReason::Stop;
            let mut line_buf = String::new();

            let mut byte_stream = resp.bytes_stream();
            use futures::StreamExt;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk_bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        yield Err(ProviderError::Network(e));
                        return;
                    }
                };

                let chunk_str = String::from_utf8_lossy(&chunk_bytes);
                line_buf.push_str(&chunk_str);

                while let Some(newline_pos) = line_buf.find('\n') {
                    let line: String = line_buf.drain(..=newline_pos).collect();
                    let line = line.trim();

                    if line.is_empty() {
                        continue;
                    }

                    let data = match parse_sse_line(line) {
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

            // Emit tool call end events
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

            let msg = AssistantMessage {
                content,
                model: model_id,
                provider: provider_id,
                usage: Some(usage),
                stop_reason,
            };

            yield Ok(StreamEvent::Done { message: msg });
        };

        Box::pin(s)
    }

    async fn list_models(&self, api_key: &str) -> Result<Vec<ModelDef>, ProviderError> {
        // OpenAI supports GET /v1/models
        let url = "https://api.openai.com/v1/models";
        let resp = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status: 400,
                body,
            });
        }

        let models_resp: ModelsResponse = resp.json().await?;

        let models = models_resp
            .data
            .into_iter()
            .map(|entry| ModelDef {
                id: entry.id.clone(),
                name: entry.id,
                api: Api::OpenaiCompletions,
                provider: "openai".into(),
                base_url: "https://api.openai.com/v1".into(),
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
