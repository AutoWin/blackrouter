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
    /// Next tool_calls index
    next_tool_index: usize,
    /// Created timestamp
    created: u64,
    /// Total prompt tokens (from usage events)
    prompt_tokens: u64,
    /// Total completion tokens (from usage events)
    completion_tokens: u64,
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
            next_tool_index: 0,
            created: blackrouter_common::unix_timestamp(),
            prompt_tokens: 0,
            completion_tokens: 0,
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

    /// Translate a single SSE event data string → list of SSE-formatted output strings
    fn translate_event(&mut self, data: &str) -> Vec<String> {
        match self.from {
            WireFormat::ClaudeMessages => self.translate_claude_event(data),
            WireFormat::Gemini => self.translate_gemini_event(data),
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
                        if let Some(data) = extract_sse_data(&remaining) {
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
