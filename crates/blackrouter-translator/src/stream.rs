//! SSE stream translation (Phase 2.3)
//!
//! Translates upstream SSE events from Claude/Gemini format to OpenAI SSE chunk
//! format in real-time, event-by-event, without buffering the full response.

use bytes::Bytes;
use futures_util::stream::{unfold, Stream};
use serde_json::{json, Value};
use std::pin::Pin;

use crate::WireFormat;

/// Error during SSE stream translation
#[derive(Debug, thiserror::Error)]
pub enum SseTranslateError {
    #[error("upstream stream error: {0}")]
    Upstream(String),
    #[error("parse error: {0}")]
    Parse(String),
}

/// State for the SSE translation unfold
struct TranslateState {
    /// The chat completion ID for all chunks
    chatcmpl_id: String,
    /// Model name to include in chunks
    model: String,
    /// Whether we've sent the initial role chunk
    sent_role: bool,
    /// Source wire format
    from: WireFormat,
    /// Accumulated tool call arguments per index (for Claude tool_use)
    tool_call_args: std::collections::HashMap<usize, String>,
    /// Mapping from Claude content_block index → OpenAI tool_calls index
    tool_index_map: std::collections::HashMap<usize, usize>,
    /// Mapping from CommandCode tool-call id → OpenAI tool_calls index
    commandcode_tool_index_map: std::collections::HashMap<String, usize>,
    /// Mapping from Responses item/call id → OpenAI tool_calls index
    responses_tool_index_map: std::collections::HashMap<String, usize>,
    /// Next tool_calls index
    next_tool_index: usize,
    /// Created timestamp
    created: u64,
    /// Total prompt tokens (from usage events)
    prompt_tokens: u64,
    /// Total completion tokens (from usage events)
    completion_tokens: u64,
    /// Finish reason captured from providers that emit a pre-final usage event.
    finish_reason: Option<String>,
    /// Whether we've seen the stop event
    stopped: bool,
}

impl TranslateState {
    fn new(from: WireFormat, model: String) -> Self {
        Self {
            chatcmpl_id: format!(
                "chatcmpl-stream-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            ),
            model,
            sent_role: false,
            from,
            tool_call_args: std::collections::HashMap::new(),
            tool_index_map: std::collections::HashMap::new(),
            commandcode_tool_index_map: std::collections::HashMap::new(),
            responses_tool_index_map: std::collections::HashMap::new(),
            next_tool_index: 0,
            created: blackrouter_common::unix_timestamp(),
            prompt_tokens: 0,
            completion_tokens: 0,
            finish_reason: None,
            stopped: false,
        }
    }

    /// Build a base OpenAI chunk with the common fields
    fn base_chunk(&self) -> Value {
        json!({
            "id": self.chatcmpl_id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
        })
    }

    /// Translate a single Claude SSE event → OpenAI chunk(s)
    /// Returns Vec of SSE-formatted byte strings to emit
    fn translate_claude_event(&mut self, data: &str) -> Vec<String> {
        let value: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return vec![],
        };

        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        match event_type {
            "message_start" => {
                // Extract prompt tokens from message.usage
                if let Some(usage) = value.get("message").and_then(|m| m.get("usage")) {
                    self.prompt_tokens = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                }
                // Emit initial role chunk
                if !self.sent_role {
                    self.sent_role = true;
                    let chunk = self.base_chunk();
                    let mut chunk = chunk;
                    chunk["choices"] = json!([{
                        "index": 0,
                        "delta": {"role": "assistant"},
                        "finish_reason": null
                    }]);
                    return vec![format_sse(&chunk)];
                }
                vec![]
            }

            "content_block_start" => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block = value.get("content_block");
                let block_type = block
                    .and_then(|b| b.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("text");

                if block_type == "tool_use" {
                    // Start of a tool call
                    let id = block
                        .and_then(|b| b.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or("tool_call");
                    let name = block
                        .and_then(|b| b.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("");

                    let tool_idx = self.next_tool_index;
                    self.next_tool_index += 1;
                    self.tool_index_map.insert(index, tool_idx);

                    let mut chunk = self.base_chunk();
                    chunk["choices"] = json!([{
                        "index": 0,
                        "delta": {
                            "tool_calls": [{
                                "index": tool_idx,
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": ""
                                }
                            }]
                        },
                        "finish_reason": null
                    }]);
                    vec![format_sse(&chunk)]
                } else {
                    // Text block start — nothing to emit (content comes in deltas)
                    vec![]
                }
            }

            "content_block_delta" => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = value.get("delta");
                let delta_type = delta
                    .and_then(|d| d.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");

                match delta_type {
                    "text_delta" => {
                        let text = delta
                            .and_then(|d| d.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if text.is_empty() {
                            return vec![];
                        }
                        let mut chunk = self.base_chunk();
                        chunk["choices"] = json!([{
                            "index": 0,
                            "delta": {"content": text},
                            "finish_reason": null
                        }]);
                        vec![format_sse(&chunk)]
                    }

                    "input_json_delta" => {
                        // Accumulate partial JSON for tool call arguments
                        let partial = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str)
                            .unwrap_or("");

                        if let Some(&tool_idx) = self.tool_index_map.get(&index) {
                            // Accumulate
                            let args = self.tool_call_args.entry(index).or_default();
                            args.push_str(partial);

                            // Emit the delta as-is (OpenAI also streams partial arguments)
                            let mut chunk = self.base_chunk();
                            chunk["choices"] = json!([{
                                "index": 0,
                                "delta": {
                                    "tool_calls": [{
                                        "index": tool_idx,
                                        "function": {
                                            "arguments": partial
                                        }
                                    }]
                                },
                                "finish_reason": null
                            }]);
                            vec![format_sse(&chunk)]
                        } else {
                            vec![]
                        }
                    }

                    _ => vec![],
                }
            }

            "content_block_stop" => {
                // Nothing special needed for OpenAI format
                vec![]
            }

            "message_delta" => {
                // Extract stop_reason and usage
                let stop_reason = value
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str);

                if let Some(usage) = value.get("usage") {
                    self.completion_tokens = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(self.completion_tokens);
                }

                let finish_reason = match stop_reason {
                    Some("end_turn") | Some("stop_sequence") => "stop",
                    Some("max_tokens") => "length",
                    Some("tool_use") => "tool_calls",
                    _ => "stop",
                };

                self.stopped = true;

                let mut chunk = self.base_chunk();
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": {},
                    "finish_reason": finish_reason
                }]);

                // Include usage in the final chunk
                chunk["usage"] = json!({
                    "prompt_tokens": self.prompt_tokens,
                    "completion_tokens": self.completion_tokens,
                    "total_tokens": self.prompt_tokens + self.completion_tokens
                });

                vec![format_sse(&chunk)]
            }

            "message_stop" => {
                // The [DONE] sentinel is emitted by the unfold when stream ends
                vec![]
            }

            _ => vec![],
        }
    }

    /// Translate a single Gemini SSE event → OpenAI chunk(s)
    fn translate_gemini_event(&mut self, data: &str) -> Vec<String> {
        let value: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return vec![],
        };

        // Gemini doesn't have explicit event types — each data line is a full response chunk
        let candidates = value.get("candidates").and_then(Value::as_array);
        let candidate = candidates.and_then(|c| c.first());

        // Send initial role chunk if not sent
        let mut chunks = Vec::new();
        if !self.sent_role {
            self.sent_role = true;
            let mut role_chunk = self.base_chunk();
            role_chunk["choices"] = json!([{
                "index": 0,
                "delta": {"role": "assistant"},
                "finish_reason": null
            }]);
            chunks.push(format_sse(&role_chunk));
        }

        // Extract text from parts
        if let Some(parts) = candidate
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(Value::as_array)
        {
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        let mut chunk = self.base_chunk();
                        chunk["choices"] = json!([{
                            "index": 0,
                            "delta": {"content": text},
                            "finish_reason": null
                        }]);
                        chunks.push(format_sse(&chunk));
                    }
                }

                // Handle functionCall in streaming
                if let Some(fc) = part.get("functionCall") {
                    let name = fc.get("name").and_then(Value::as_str).unwrap_or("");
                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                    let args_str = serde_json::to_string(&args).unwrap_or_default();
                    let tool_idx = self.next_tool_index;
                    self.next_tool_index += 1;

                    // Emit tool call start
                    let mut chunk = self.base_chunk();
                    chunk["choices"] = json!([{
                        "index": 0,
                        "delta": {
                            "tool_calls": [{
                                "index": tool_idx,
                                "id": format!("call_gemini_{}", tool_idx),
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": args_str
                                }
                            }]
                        },
                        "finish_reason": null
                    }]);
                    chunks.push(format_sse(&chunk));
                }
            }
        }

        // Check for finishReason
        let finish_reason = candidate
            .and_then(|c| c.get("finishReason"))
            .and_then(Value::as_str);

        if let Some(reason) = finish_reason {
            let fr = match reason {
                "STOP" => {
                    if self.next_tool_index > 0 {
                        "tool_calls"
                    } else {
                        "stop"
                    }
                }
                "MAX_TOKENS" => "length",
                _ => "stop",
            };

            self.stopped = true;

            // Extract usage metadata
            if let Some(metadata) = value.get("usageMetadata") {
                self.prompt_tokens = metadata
                    .get("promptTokenCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                self.completion_tokens = metadata
                    .get("candidatesTokenCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
            }

            let mut chunk = self.base_chunk();
            chunk["choices"] = json!([{
                "index": 0,
                "delta": {},
                "finish_reason": fr
            }]);

            if self.prompt_tokens > 0 || self.completion_tokens > 0 {
                chunk["usage"] = json!({
                    "prompt_tokens": self.prompt_tokens,
                    "completion_tokens": self.completion_tokens,
                    "total_tokens": self.prompt_tokens + self.completion_tokens
                });
            }

            chunks.push(format_sse(&chunk));
        }

        chunks
    }

    fn translate_commandcode_event(&mut self, data: &str) -> Vec<String> {
        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => return vec![],
        };
        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        let mut chunks = Vec::new();

        match event_type {
            "text-delta" => {
                let text = value
                    .get("text")
                    .or_else(|| value.get("delta"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if text.is_empty() {
                    return vec![];
                }
                let mut chunk = self.base_chunk();
                let delta = if self.sent_role {
                    json!({"content": text})
                } else {
                    self.sent_role = true;
                    json!({"role": "assistant", "content": text})
                };
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": delta,
                    "finish_reason": null
                }]);
                chunks.push(format_sse(&chunk));
            }
            "reasoning-delta" => {
                let text = value.get("text").and_then(Value::as_str).unwrap_or("");
                if text.is_empty() {
                    return vec![];
                }
                let mut chunk = self.base_chunk();
                let delta = if self.sent_role {
                    json!({"reasoning_content": text})
                } else {
                    self.sent_role = true;
                    json!({"role": "assistant", "reasoning_content": text})
                };
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": delta,
                    "finish_reason": null
                }]);
                chunks.push(format_sse(&chunk));
            }
            "tool-input-start" => {
                let id = value
                    .get("id")
                    .or_else(|| value.get("toolCallId"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| format!("call_{}_{}", self.created, self.next_tool_index));
                let tool_idx = self
                    .commandcode_tool_index_map
                    .get(&id)
                    .copied()
                    .unwrap_or_else(|| {
                        let idx = self.next_tool_index;
                        self.next_tool_index += 1;
                        self.commandcode_tool_index_map.insert(id.clone(), idx);
                        idx
                    });
                let name = value.get("toolName").and_then(Value::as_str).unwrap_or("");

                let mut delta = json!({
                    "tool_calls": [{
                        "index": tool_idx,
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": ""
                        }
                    }]
                });
                if !self.sent_role {
                    self.sent_role = true;
                    delta
                        .as_object_mut()
                        .unwrap()
                        .insert("role".to_string(), Value::String("assistant".to_string()));
                }

                let mut chunk = self.base_chunk();
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": delta,
                    "finish_reason": null
                }]);
                chunks.push(format_sse(&chunk));
            }
            "tool-input-delta" => {
                let Some(id) = value
                    .get("id")
                    .or_else(|| value.get("toolCallId"))
                    .and_then(Value::as_str)
                else {
                    return vec![];
                };
                let Some(tool_idx) = self.commandcode_tool_index_map.get(id).copied() else {
                    return vec![];
                };
                let delta = value
                    .get("delta")
                    .or_else(|| value.get("inputTextDelta"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if delta.is_empty() {
                    return vec![];
                }

                let mut chunk = self.base_chunk();
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": tool_idx,
                            "function": {"arguments": delta}
                        }]
                    },
                    "finish_reason": null
                }]);
                chunks.push(format_sse(&chunk));
            }
            "tool-call" => {
                let id = value
                    .get("toolCallId")
                    .or_else(|| value.get("id"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| format!("call_{}_{}", self.created, self.next_tool_index));
                if self.commandcode_tool_index_map.contains_key(&id) {
                    return vec![];
                }
                let tool_idx = self.next_tool_index;
                self.next_tool_index += 1;
                self.commandcode_tool_index_map.insert(id.clone(), tool_idx);
                let name = value.get("toolName").and_then(Value::as_str).unwrap_or("");
                let input = value.get("input").cloned().unwrap_or_else(|| json!({}));
                let arguments = input.as_str().map(ToOwned::to_owned).unwrap_or_else(|| {
                    serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string())
                });

                let mut delta = json!({
                    "tool_calls": [{
                        "index": tool_idx,
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments
                        }
                    }]
                });
                if !self.sent_role {
                    self.sent_role = true;
                    delta
                        .as_object_mut()
                        .unwrap()
                        .insert("role".to_string(), Value::String("assistant".to_string()));
                }

                let mut chunk = self.base_chunk();
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": delta,
                    "finish_reason": null
                }]);
                chunks.push(format_sse(&chunk));
            }
            "finish-step" => {
                if let Some(usage) = value.get("usage") {
                    self.prompt_tokens = usage
                        .get("inputTokens")
                        .or_else(|| usage.get("prompt_tokens"))
                        .and_then(Value::as_u64)
                        .unwrap_or(self.prompt_tokens);
                    self.completion_tokens = usage
                        .get("outputTokens")
                        .or_else(|| usage.get("completion_tokens"))
                        .and_then(Value::as_u64)
                        .unwrap_or(self.completion_tokens);
                }
                self.finish_reason = Some(
                    match value.get("finishReason").and_then(Value::as_str) {
                        Some("length") => "length",
                        Some("tool-calls") | Some("tool_use") => "tool_calls",
                        Some("content-filter") => "content_filter",
                        _ => "stop",
                    }
                    .to_string(),
                );
            }
            "finish" => {
                if !self.stopped {
                    if let Some(usage) = value.get("totalUsage").or_else(|| value.get("usage")) {
                        self.prompt_tokens = usage
                            .get("inputTokens")
                            .or_else(|| usage.get("prompt_tokens"))
                            .and_then(Value::as_u64)
                            .unwrap_or(self.prompt_tokens);
                        self.completion_tokens = usage
                            .get("outputTokens")
                            .or_else(|| usage.get("completion_tokens"))
                            .and_then(Value::as_u64)
                            .unwrap_or(self.completion_tokens);
                    }
                    let finish_reason =
                        self.finish_reason.as_deref().unwrap_or_else(|| {
                            match value.get("finishReason").and_then(Value::as_str) {
                                Some("length") => "length",
                                Some("tool-calls") | Some("tool_use") => "tool_calls",
                                Some("content-filter") => "content_filter",
                                _ => "stop",
                            }
                        });
                    self.stopped = true;
                    let mut chunk = self.base_chunk();
                    chunk["choices"] = json!([{
                        "index": 0,
                        "delta": {},
                        "finish_reason": finish_reason
                    }]);
                    chunk["usage"] = json!({
                        "prompt_tokens": self.prompt_tokens,
                        "completion_tokens": self.completion_tokens,
                        "total_tokens": self.prompt_tokens + self.completion_tokens
                    });
                    chunks.push(format_sse(&chunk));
                }
            }
            "error" => {
                let message = value
                    .get("error")
                    .or_else(|| value.get("message"))
                    .map(|value| {
                        value
                            .as_str()
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| value.to_string())
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                let mut content = self.base_chunk();
                content["choices"] = json!([{
                    "index": 0,
                    "delta": {"content": format!("\n\n[CommandCode error: {message}]")},
                    "finish_reason": null
                }]);
                chunks.push(format_sse(&content));
                let mut done = self.base_chunk();
                done["choices"] = json!([{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }]);
                chunks.push(format_sse(&done));
            }
            _ => {}
        }

        chunks
    }

    fn translate_openai_responses_event(&mut self, data: &str) -> Vec<String> {
        let value: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        match event_type {
            "response.created" | "response.in_progress" => {
                if self.sent_role {
                    return vec![];
                }
                self.sent_role = true;
                let mut chunk = self.base_chunk();
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": {"role": "assistant"},
                    "finish_reason": null
                }]);
                vec![format_sse(&chunk)]
            }
            "response.output_text.delta" | "response.reasoning_text.delta" => {
                let text = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .or_else(|| value.get("text").and_then(Value::as_str))
                    .unwrap_or("");
                if text.is_empty() {
                    return vec![];
                }
                let mut chunk = self.base_chunk();
                let delta = if event_type == "response.reasoning_text.delta" {
                    json!({"reasoning_content": text})
                } else {
                    json!({"content": text})
                };
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": delta,
                    "finish_reason": null
                }]);
                vec![format_sse(&chunk)]
            }
            "response.output_item.added" => {
                let item = value.get("item").unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return vec![];
                }
                let id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("call");
                let tool_idx = self.next_tool_index;
                self.next_tool_index += 1;
                self.responses_tool_index_map
                    .insert(id.to_string(), tool_idx);
                if let Some(item_id) = item.get("id").and_then(Value::as_str) {
                    self.responses_tool_index_map
                        .insert(item_id.to_string(), tool_idx);
                }

                let mut chunk = self.base_chunk();
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": tool_idx,
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": item.get("name").and_then(Value::as_str).unwrap_or(""),
                                "arguments": ""
                            }
                        }]
                    },
                    "finish_reason": null
                }]);
                vec![format_sse(&chunk)]
            }
            "response.function_call_arguments.delta" => {
                let delta = value.get("delta").and_then(Value::as_str).unwrap_or("");
                if delta.is_empty() {
                    return vec![];
                }
                let key = value
                    .get("call_id")
                    .or_else(|| value.get("item_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("call");
                let tool_idx = if let Some(index) = self.responses_tool_index_map.get(key) {
                    *index
                } else {
                    let index = self.next_tool_index;
                    self.next_tool_index += 1;
                    self.responses_tool_index_map.insert(key.to_string(), index);
                    index
                };
                let mut chunk = self.base_chunk();
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": tool_idx,
                            "function": {"arguments": delta}
                        }]
                    },
                    "finish_reason": null
                }]);
                vec![format_sse(&chunk)]
            }
            "response.completed" => {
                if let Some(usage) = value
                    .get("response")
                    .and_then(|response| response.get("usage"))
                {
                    self.prompt_tokens = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(self.prompt_tokens);
                    self.completion_tokens = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(self.completion_tokens);
                }
                self.stopped = true;
                let finish_reason = if self.next_tool_index > 0 {
                    "tool_calls"
                } else {
                    "stop"
                };
                let mut chunk = self.base_chunk();
                chunk["choices"] = json!([{
                    "index": 0,
                    "delta": {},
                    "finish_reason": finish_reason
                }]);
                chunk["usage"] = json!({
                    "prompt_tokens": self.prompt_tokens,
                    "completion_tokens": self.completion_tokens,
                    "total_tokens": self.prompt_tokens + self.completion_tokens
                });
                vec![format_sse(&chunk)]
            }
            "response.failed" | "error" => {
                let message = value
                    .get("error")
                    .and_then(|error| error.get("message").or_else(|| error.get("error")))
                    .and_then(Value::as_str)
                    .or_else(|| value.get("message").and_then(Value::as_str))
                    .unwrap_or("Responses stream error");
                let mut content = self.base_chunk();
                content["choices"] = json!([{
                    "index": 0,
                    "delta": {"content": format!("\n\n[Responses error: {message}]")},
                    "finish_reason": null
                }]);
                let mut done = self.base_chunk();
                done["choices"] = json!([{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }]);
                vec![format_sse(&content), format_sse(&done)]
            }
            _ => vec![],
        }
    }

    /// Translate a single SSE event data string → list of SSE-formatted output strings
    fn translate_event(&mut self, data: &str) -> Vec<String> {
        match self.from {
            WireFormat::ClaudeMessages => self.translate_claude_event(data),
            WireFormat::Gemini | WireFormat::Antigravity => self.translate_gemini_event(data),
            WireFormat::CommandCode => self.translate_commandcode_event(data),
            WireFormat::OpenAiResponses => self.translate_openai_responses_event(data),
            _ => {
                // For OpenAI passthrough, just forward as-is
                vec![format!("data: {}\n\n", data)]
            }
        }
    }
}

/// Format a JSON value as an SSE data line
fn format_sse(value: &Value) -> String {
    format!("data: {}\n\n", value)
}

/// Extract the `data:` payload from a raw SSE event block
fn extract_sse_data(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("data:") {
            let data = rest.trim();
            if data == "[DONE]" {
                return None; // Signal end
            }
            return Some(data.to_string());
        }
    }
    None
}

fn extract_commandcode_line(raw: &str) -> Option<String> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }
    let data = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
    if data.is_empty() || data == "[DONE]" {
        None
    } else {
        Some(data.to_string())
    }
}

/// Type alias for the translated stream
pub type TranslatedSseStream = Pin<Box<dyn Stream<Item = Result<Bytes, SseTranslateError>> + Send>>;

/// Internal stream type after error mapping
type BoxedByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>;

/// Create a translated SSE stream from an upstream byte stream.
///
/// This reads SSE events from the upstream, translates each event from the
/// source format to OpenAI SSE chunk format, and yields the translated bytes.
/// When the upstream stream ends, it emits `data: [DONE]\n\n`.
///
/// Generic over the upstream error type — converts to `SseTranslateError::Upstream`.
pub fn translate_sse_stream<S, E>(
    upstream: S,
    from: WireFormat,
    model: String,
) -> TranslatedSseStream
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    use futures_util::StreamExt as _;

    // Map upstream errors to String so we can type-erase the stream
    let upstream: BoxedByteStream =
        Box::pin(upstream.map(|result| result.map_err(|e| e.to_string())));

    let state = TranslateState::new(from, model);

    Box::pin(unfold(
        SseUnfoldState {
            upstream,
            buffer: String::new(),
            translate: state,
            upstream_done: false,
            emitted_done: false,
        },
        |mut st| async move {
            if st.emitted_done {
                return None;
            }

            loop {
                if st.translate.from == WireFormat::CommandCode {
                    if let Some(pos) = st.buffer.find('\n') {
                        let raw_line = st.buffer[..pos].to_string();
                        st.buffer = st.buffer[pos + 1..].to_string();
                        if let Some(data) = extract_commandcode_line(&raw_line) {
                            let chunks = st.translate.translate_event(&data);
                            if !chunks.is_empty() {
                                return Some((Ok(Bytes::from(chunks.concat())), st));
                            }
                        }
                        continue;
                    }
                }

                // Try to extract a complete SSE event from the buffer
                // SSE events are separated by \n\n
                if let Some(pos) = st.buffer.find("\n\n") {
                    let raw_event = st.buffer[..pos].to_string();
                    st.buffer = st.buffer[pos + 2..].to_string();

                    // Extract data: line from the event
                    if let Some(data) = extract_sse_data(&raw_event) {
                        let chunks = st.translate.translate_event(&data);
                        if !chunks.is_empty() {
                            // Concatenate all translated chunks into one Bytes
                            let combined = chunks.concat();
                            return Some((Ok(Bytes::from(combined)), st));
                        }
                        // Skip non-translatable events, continue loop
                    }
                    // Skip events without data: line
                    continue;
                }

                // Need more data from upstream
                if st.upstream_done {
                    // Process any remaining buffer
                    if !st.buffer.is_empty() {
                        let remaining = std::mem::take(&mut st.buffer);
                        let data = if st.translate.from == WireFormat::CommandCode {
                            extract_commandcode_line(&remaining)
                        } else {
                            extract_sse_data(&remaining)
                        };
                        if let Some(data) = data {
                            let chunks = st.translate.translate_event(&data);
                            if !chunks.is_empty() {
                                let combined = chunks.concat();
                                // Emit remaining + [DONE]
                                st.emitted_done = true;
                                let done = format!("{}data: [DONE]\n\n", combined);
                                return Some((Ok(Bytes::from(done)), st));
                            }
                        }
                    }

                    // Emit [DONE]
                    st.emitted_done = true;
                    return Some((Ok(Bytes::from("data: [DONE]\n\n")), st));
                }

                // Fetch next chunk from upstream
                match st.upstream.next().await {
                    Some(Ok(bytes)) => {
                        st.buffer.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    Some(Err(e)) => {
                        return Some((Err(SseTranslateError::Upstream(e.to_string())), st));
                    }
                    None => {
                        st.upstream_done = true;
                    }
                }
            }
        },
    ))
}

/// Internal state for the unfold
struct SseUnfoldState {
    upstream: BoxedByteStream,
    buffer: String,
    translate: TranslateState,
    upstream_done: bool,
    emitted_done: bool,
}

#[cfg(test)]
use futures_util::StreamExt;

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test]
    async fn test_claude_sse_to_openai_stream() {
        // Simulate Claude SSE stream
        let events: Vec<Result<Bytes, String>> = vec![
            Ok(Bytes::from("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":10}}}\n\n")),
            Ok(Bytes::from("event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n")),
            Ok(Bytes::from("event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\n")),
            Ok(Bytes::from("event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n")),
            Ok(Bytes::from("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n")),
        ];

        let upstream = stream::iter(events);
        let mut translated =
            translate_sse_stream(upstream, WireFormat::ClaudeMessages, "claude-3".to_string());

        let mut all_output = String::new();
        while let Some(chunk) = translated.next().await {
            if let Ok(bytes) = chunk {
                all_output.push_str(&String::from_utf8_lossy(&bytes));
            }
        }

        // Should contain role chunk, text chunks, finish_reason chunk, and [DONE]
        assert!(all_output.contains("\"role\":\"assistant\""));
        assert!(all_output.contains("Hello"));
        assert!(all_output.contains(" world"));
        assert!(all_output.contains("\"finish_reason\":\"stop\""));
        assert!(all_output.contains("\"prompt_tokens\":10"));
        assert!(all_output.contains("\"completion_tokens\":5"));
        assert!(all_output.contains("[DONE]"));
    }

    #[tokio::test]
    async fn test_gemini_sse_to_openai_stream() {
        let events: Vec<Result<Bytes, String>> = vec![
            Ok(Bytes::from("data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}],\"role\":\"model\"}}]}\n\n")),
            Ok(Bytes::from("data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" from Gemini\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":8,\"candidatesTokenCount\":3}}\n\n")),
        ];

        let upstream = stream::iter(events);
        let mut translated =
            translate_sse_stream(upstream, WireFormat::Gemini, "gemini-2.0".to_string());

        let mut all_output = String::new();
        while let Some(chunk) = translated.next().await {
            if let Ok(bytes) = chunk {
                all_output.push_str(&String::from_utf8_lossy(&bytes));
            }
        }

        assert!(all_output.contains("\"role\":\"assistant\""));
        assert!(all_output.contains("Hello"));
        assert!(all_output.contains(" from Gemini"));
        assert!(all_output.contains("\"finish_reason\":\"stop\""));
        assert!(all_output.contains("\"prompt_tokens\":8"));
        assert!(all_output.contains("[DONE]"));
    }

    #[tokio::test]
    async fn test_commandcode_finish_step_does_not_end_stream() {
        let events: Vec<Result<Bytes, String>> = vec![
            Ok(Bytes::from("{\"type\":\"text-delta\",\"text\":\"Hello \"}\n")),
            Ok(Bytes::from("{\"type\":\"finish-step\",\"finishReason\":\"stop\",\"usage\":{\"inputTokens\":4,\"outputTokens\":2}}\n")),
            Ok(Bytes::from("{\"type\":\"text-delta\",\"text\":\"world\"}\n")),
            Ok(Bytes::from("{\"type\":\"finish\",\"totalUsage\":{\"inputTokens\":4,\"outputTokens\":3,\"totalTokens\":7}}\n")),
        ];

        let upstream = stream::iter(events);
        let mut translated =
            translate_sse_stream(upstream, WireFormat::CommandCode, "commandcode".to_string());

        let mut all_output = String::new();
        while let Some(chunk) = translated.next().await {
            if let Ok(bytes) = chunk {
                all_output.push_str(&String::from_utf8_lossy(&bytes));
            }
        }

        let world = all_output.find("world").expect("world delta emitted");
        let finish = all_output
            .find("\"finish_reason\":\"stop\"")
            .expect("finish emitted");
        assert!(world < finish);
        assert!(all_output.contains("\"completion_tokens\":3"));
        assert!(all_output.contains("[DONE]"));
    }

    #[tokio::test]
    async fn test_commandcode_tool_input_streams_openai_tool_calls() {
        let events: Vec<Result<Bytes, String>> = vec![
            Ok(Bytes::from("{\"type\":\"tool-input-start\",\"id\":\"tool_1\",\"toolName\":\"read_file\"}\n")),
            Ok(Bytes::from("{\"type\":\"tool-input-delta\",\"id\":\"tool_1\",\"delta\":\"{\\\"path\\\":\"}\n")),
            Ok(Bytes::from("{\"type\":\"tool-input-delta\",\"id\":\"tool_1\",\"delta\":\"\\\"Cargo.toml\\\"}\"}\n")),
            Ok(Bytes::from("{\"type\":\"finish-step\",\"finishReason\":\"tool-calls\",\"usage\":{\"inputTokens\":4,\"outputTokens\":3}}\n")),
            Ok(Bytes::from("{\"type\":\"finish\"}\n")),
        ];

        let upstream = stream::iter(events);
        let mut translated =
            translate_sse_stream(upstream, WireFormat::CommandCode, "commandcode".to_string());

        let mut all_output = String::new();
        while let Some(chunk) = translated.next().await {
            if let Ok(bytes) = chunk {
                all_output.push_str(&String::from_utf8_lossy(&bytes));
            }
        }

        assert!(all_output.contains("\"tool_calls\""));
        assert!(all_output.contains("\"read_file\""));
        assert!(all_output.contains("\"tool_1\""));
        assert!(all_output.contains("{\\\"path\\\":"));
        assert!(all_output.contains("\\\"Cargo.toml\\\"}"));
        assert!(all_output.contains("\"finish_reason\":\"tool_calls\""));
        assert!(all_output.contains("[DONE]"));
    }

    #[tokio::test]
    async fn test_claude_sse_with_tool_use() {
        // Build test data using json! to avoid escaping issues
        let events: Vec<Result<Bytes, String>> = vec![
            Ok(Bytes::from(format!(
                "event: message_start\ndata: {}\n\n",
                json!({"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":10}}})
            ))),
            Ok(Bytes::from(format!(
                "event: content_block_start\ndata: {}\n\n",
                json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_123","name":"get_weather"}})
            ))),
            Ok(Bytes::from(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}})
            ))),
            Ok(Bytes::from(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"SF\"}"}})
            ))),
            Ok(Bytes::from(format!(
                "event: message_delta\ndata: {}\n\n",
                json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":15}})
            ))),
            Ok(Bytes::from(
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            )),
        ];

        let upstream = stream::iter(events);
        let mut translated =
            translate_sse_stream(upstream, WireFormat::ClaudeMessages, "claude-3".to_string());

        let mut all_output = String::new();
        while let Some(chunk) = translated.next().await {
            if let Ok(bytes) = chunk {
                all_output.push_str(&String::from_utf8_lossy(&bytes));
            }
        }

        // Should contain tool_calls with name and arguments
        assert!(all_output.contains("\"tool_calls\""));
        assert!(all_output.contains("\"get_weather\""));
        assert!(all_output.contains("\"toolu_123\""));
        assert!(all_output.contains("\"finish_reason\":\"tool_calls\""));
        assert!(all_output.contains("[DONE]"));
    }
}
