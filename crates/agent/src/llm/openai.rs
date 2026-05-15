//! OpenAI-compatible chat-completions backend.
//!
//! Used for OpenAI, Kimi (Moonshot), and Qwen (DashScope). Each provider
//! supplies a different `base_url` via `ProviderConfig`. Some providers
//! expose reasoning tokens via `reasoning_content` or need an
//! `extra_body`-style toggle — those quirks live here.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::api_errors::format_http_error;
use crate::llm::{
    Backend, Block, LLMResponse, Message, StopReason, ToolCall, ToolResultContent, ToolSchema,
};
use crate::provider::{config_for, load_api_key, Provider, ProviderConfig};

#[derive(Debug, Clone)]
pub struct OpenAICompatBackend {
    provider: Provider,
    model: String,
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAICompatBackend {
    pub fn new(provider: Provider, model: impl Into<String>) -> anyhow::Result<Self> {
        let cfg: &'static ProviderConfig = config_for(provider);
        let api_key = load_api_key(provider).ok_or_else(|| {
            anyhow::anyhow!(
                "no {} API key found. Set {} or add {}.api_key to ~/.socai/auth.json.",
                cfg.display_name,
                cfg.env_keys.join(" or "),
                provider.as_str(),
            )
        })?;
        let base_url = cfg
            .base_url
            .ok_or_else(|| anyhow::anyhow!("provider {:?} is not OpenAI-compatible", provider))?
            .trim_end_matches('/')
            .to_string();
        let model = model.into();
        let resolved_model = if model.trim().is_empty() {
            cfg.default_model.to_string()
        } else {
            model
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()?;
        Ok(Self {
            provider,
            model: resolved_model,
            api_key,
            base_url,
            client,
        })
    }

    fn url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// Provider-specific extra fields merged into the request body.
    fn extra_body(&self, has_tools: bool) -> Map<String, Value> {
        let mut extra = Map::new();
        if !has_tools {
            return extra;
        }
        match self.provider {
            Provider::Kimi if self.model.starts_with("kimi-k2.6") => {
                extra.insert("thinking".into(), json!({"type": "disabled"}));
            }
            Provider::Qwen => {
                extra.insert("enable_thinking".into(), Value::Bool(false));
            }
            _ => {}
        }
        extra
    }

    /// Some providers (Kimi, Qwen) want `reasoning_content` round-tripped
    /// in the assistant message when tool_calls are present. Others
    /// (OpenAI proper) ignore it. Toggle is per-provider.
    fn preserve_reasoning_content(&self) -> bool {
        matches!(self.provider, Provider::Kimi | Provider::Qwen)
    }
}

fn flatten_tool_result_content(blocks: &[ToolResultContent]) -> Value {
    // OpenAI Chat Completions tool messages support content as a string or
    // as an array of {type: text} / {type: image_url} parts when the model
    // can read vision. We emit an array whenever there's at least one image;
    // otherwise fall back to a plain string for older models.
    let has_image = blocks
        .iter()
        .any(|b| matches!(b, ToolResultContent::Image { .. }));
    if has_image {
        let parts: Vec<Value> = blocks
            .iter()
            .map(|b| match b {
                ToolResultContent::Text { text } => json!({"type": "text", "text": text}),
                ToolResultContent::Image { data, media_type } => json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{media_type};base64,{data}"),
                    },
                }),
            })
            .collect();
        Value::Array(parts)
    } else {
        let combined = blocks
            .iter()
            .filter_map(|b| match b {
                ToolResultContent::Text { text } => Some(text.clone()),
                ToolResultContent::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        Value::String(combined)
    }
}

/// Translate Anthropic-shaped history into chat-completion messages.
fn build_chat_messages(system: &str, messages: &[Message], preserve_reasoning: bool) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": system})];

    for msg in messages {
        let blocks = msg.content.as_blocks();
        match msg.role {
            crate::llm::MessageRole::Assistant => {
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                let mut reasoning: Option<String> = None;
                for block in blocks {
                    match block {
                        Block::Text { text } => text_parts.push(text),
                        Block::Image { .. } => {}
                        Block::ReasoningContent { text } => {
                            reasoning = Some(text);
                        }
                        Block::ToolUse { id, name, input } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".into()),
                                },
                            }));
                        }
                        Block::ToolResult { .. } => {}
                    }
                }
                let content_str = text_parts.join("\n").trim().to_string();
                let mut assistant_msg = Map::new();
                assistant_msg.insert("role".into(), json!("assistant"));
                if content_str.is_empty() {
                    assistant_msg.insert("content".into(), Value::Null);
                } else {
                    assistant_msg.insert("content".into(), json!(content_str));
                }
                if !tool_calls.is_empty() {
                    assistant_msg.insert("tool_calls".into(), Value::Array(tool_calls.clone()));
                }
                if preserve_reasoning && !tool_calls.is_empty() {
                    assistant_msg.insert(
                        "reasoning_content".into(),
                        json!(reasoning.unwrap_or_default()),
                    );
                }
                out.push(Value::Object(assistant_msg));
            }
            crate::llm::MessageRole::User => {
                let mut user_text_parts: Vec<String> = Vec::new();
                let mut user_image_parts: Vec<Value> = Vec::new();
                for block in blocks {
                    match block {
                        Block::Text { text } => user_text_parts.push(text),
                        Block::Image { data, media_type } => {
                            user_image_parts.push(json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{media_type};base64,{data}"),
                                },
                            }));
                        }
                        Block::ToolResult {
                            tool_use_id,
                            content,
                        } => {
                            out.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": flatten_tool_result_content(&content),
                            }));
                        }
                        Block::ReasoningContent { .. } | Block::ToolUse { .. } => {}
                    }
                }
                let joined = user_text_parts.join("\n").trim().to_string();
                if !user_image_parts.is_empty() {
                    let mut content = Vec::new();
                    if !joined.is_empty() {
                        content.push(json!({"type": "text", "text": joined}));
                    }
                    content.extend(user_image_parts);
                    out.push(json!({"role": "user", "content": content}));
                } else if !joined.is_empty() {
                    out.push(json!({"role": "user", "content": joined}));
                }
            }
        }
    }

    out
}

fn tools_to_wire(tools: &[ToolSchema]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct WireResponse {
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: WireUsage,
}

#[derive(Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(Deserialize)]
struct WireToolCall {
    id: String,
    function: WireFunction,
}

#[derive(Deserialize)]
struct WireFunction {
    name: String,
    #[serde(default)]
    arguments: Option<String>,
}

fn parse_stop_reason(s: Option<&str>) -> StopReason {
    match s {
        Some("stop") => StopReason::EndTurn,
        Some("tool_calls") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        _ => StopReason::Other,
    }
}

#[derive(Serialize)]
struct OutgoingRequest {
    model: String,
    messages: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[async_trait]
impl Backend for OpenAICompatBackend {
    fn label(&self) -> String {
        format!("{}:{}", self.provider.as_str(), self.model)
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn send(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolSchema],
        max_tokens: u32,
    ) -> anyhow::Result<LLMResponse> {
        let chat_messages =
            build_chat_messages(system, messages, self.preserve_reasoning_content());
        let chat_tools = tools_to_wire(tools);
        let has_tools = !chat_tools.is_empty();

        let (max_tokens_field, max_completion_tokens_field) =
            if matches!(self.provider, Provider::OpenAI) {
                (None, Some(max_tokens))
            } else {
                (Some(max_tokens), None)
            };

        let body = OutgoingRequest {
            model: self.model.clone(),
            messages: chat_messages,
            max_tokens: max_tokens_field,
            max_completion_tokens: max_completion_tokens_field,
            tools: chat_tools,
            tool_choice: if has_tools { Some("auto") } else { None },
            extra: self.extra_body(has_tools),
        };

        let response = self
            .client
            .post(self.url())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!(format_http_error(
                self.provider.as_str(),
                status.as_u16(),
                &text
            ));
        }

        let parsed: WireResponse = response.json().await?;
        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("response had no choices"))?;

        let text = choice.message.content.unwrap_or_default();
        let text_blocks = if text.trim().is_empty() {
            Vec::new()
        } else {
            vec![text]
        };

        let mut tool_calls = Vec::new();
        for tc in choice.message.tool_calls.unwrap_or_default() {
            let args_raw = tc.function.arguments.unwrap_or_else(|| "{}".into());
            let input: Value = serde_json::from_str(&args_raw).unwrap_or(Value::Object(Map::new()));
            tool_calls.push(ToolCall {
                id: tc.id,
                name: tc.function.name,
                input,
            });
        }

        Ok(LLMResponse {
            text_blocks,
            tool_calls,
            stop_reason: parse_stop_reason(choice.finish_reason.as_deref()),
            input_tokens: parsed.usage.prompt_tokens,
            output_tokens: parsed.usage.completion_tokens,
            reasoning_content: choice.message.reasoning_content.unwrap_or_default(),
        })
    }
}
