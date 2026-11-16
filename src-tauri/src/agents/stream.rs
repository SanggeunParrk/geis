//! Watching a model think.
//!
//! Without this, a turn is a wait and then a wall of text: the panel asked, sat silent for
//! however long the model took, and then showed everything at once. What Claude Code and
//! Codex do instead is show the work as it happens - the reasoning as it is reasoned, the
//! answer as it is written, the tool calls as they are decided - and that is most of what
//! makes an agent feel like something you are working *with* rather than something you
//! submitted a job to.
//!
//! Both APIs stream server-sent events, and both do it differently: OpenAI sends one JSON
//! object per `data:` line with the new characters in `delta`, and Anthropic sends typed
//! events that open, fill and close each block. This module turns either into the same
//! small set of pieces. The parsing is a state machine over lines, apart from the network,
//! so it can be tested against recorded streams rather than against a live model.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ChatReply, ToolCall, Usage};

/// A piece of an answer as it arrives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Delta {
    /// More of the answer.
    Text { text: String },
    /// More of the model's reasoning.
    Thinking { text: String },
    /// A tool call has been decided on - named before its arguments are complete, because
    /// "it is about to read the graph" is worth showing at once.
    Tool { id: String, name: String },
    /// Which model is answering - the one that actually is, which a gateway, a fallback
    /// or a CLI's own default may make different from the one that was asked for.
    Model { model: String },
    /// What it is doing when there is nothing to show for it yet: "thinking".
    Status { text: String },
}

/// What a stream has built so far, and what it becomes at the end.
#[derive(Debug, Default, Clone)]
pub struct Assembled {
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<ToolCall>,
    /// Arguments arrive as fragments of JSON text and are only parsed at the end.
    pub arguments: Vec<String>,
    pub usage: Usage,
    pub stop_reason: String,
    pub model: String,
    /// Claude's content blocks as they were sent, rebuilt from the stream: what goes back
    /// to it on the next request in place of anything reconstructed.
    pub blocks: Vec<Value>,
    /// The JSON of the tool call being written, which arrives in pieces.
    pub block_json: String,
    /// Why a turn was declined, when it was.
    pub refusal: Option<String>,
}

/// The four ways a turn ends, in one vocabulary for both wire formats: `tool` when the
/// model is waiting on results, `length` when it ran out of tokens, `refusal` when it
/// declined, `end` otherwise.
pub fn neutral_stop(reason: &str) -> &'static str {
    match reason {
        "tool_use" | "tool_calls" => "tool",
        "max_tokens" | "length" => "length",
        "refusal" | "content_filter" => "refusal",
        _ => "end",
    }
}

/// What to show for a turn the model declined, which has no text of its own.
pub fn refusal_note(category: Option<&str>) -> String {
    match category.filter(|category| !category.is_empty()) {
        Some(category) => format!("The model declined this request ({category})."),
        None => "The model declined this request.".to_string(),
    }
}

impl Assembled {
    pub fn finish(mut self) -> Result<ChatReply, String> {
        for (call, raw) in self.tool_calls.iter_mut().zip(self.arguments.iter()) {
            // No arguments at all is a call that takes none. Arguments that do not parse
            // are a reply cut off part-way - the token limit landing inside the JSON - and
            // running the tool with nothing in their place would do something the model
            // never asked for.
            if raw.trim().is_empty() {
                call.arguments = serde_json::json!({});
                continue;
            }
            call.arguments = serde_json::from_str(raw).map_err(|_| {
                format!(
                    "The reply was cut off inside a call to {}, so nothing was run. Ask again, or raise the token limit.",
                    if call.name.is_empty() { "a tool" } else { &call.name }
                )
            })?;
        }
        // A model that asked for a tool is waiting for it, whatever it said about stopping.
        let stop = if self.tool_calls.is_empty() {
            if self.stop_reason.is_empty() {
                "end".to_string()
            } else {
                self.stop_reason.clone()
            }
        } else {
            "tool".to_string()
        };
        let mut text = self.text.trim().to_string();
        if stop == "refusal" && text.is_empty() {
            text = refusal_note(self.refusal.as_deref());
        }
        // Out of tokens before a word of the answer: thinking counts against the limit,
        // and a reply that spent all of it there is not an answer that happens to be empty.
        if stop == "length" && text.is_empty() {
            return Err(
                "The reply ran out of tokens before it reached an answer. Ask again, or raise the token limit."
                    .into(),
            );
        }
        Ok(ChatReply {
            text,
            thinking: self.thinking.trim().to_string(),
            tool_calls: self.tool_calls,
            stop_reason: stop,
            usage: self.usage,
            model: self.model,
            provider: (!self.blocks.is_empty())
                .then(|| super::anthropic_provider(Value::Array(self.blocks))),
        })
    }
}

/// Which wire format a stream is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    Openai,
    Anthropic,
}

/// Reads a stream of server-sent events into deltas and, in the end, a reply.
#[derive(Debug)]
pub struct Reader {
    wire: Wire,
    pub assembled: Assembled,
    /// Anthropic names the kind of the block that is currently open.
    block: Option<String>,
    /// Bytes not yet a whole line. Bytes, not text: the network cuts a chunk wherever it
    /// likes, including through the middle of a character, and decoding each chunk on its
    /// own turns both halves of that character into U+FFFD.
    pending: Vec<u8>,
    /// Characters held back because they might be the start of a `<think>` tag.
    carry: String,
    /// What the server said went wrong, if it said so mid-stream.
    error: Option<String>,
    /// Where each of OpenAI's tool-call indices is being assembled.
    slots: std::collections::HashMap<u64, usize>,
}

impl Reader {
    pub fn new(wire: Wire) -> Self {
        Self {
            wire,
            assembled: Assembled::default(),
            block: None,
            pending: Vec::new(),
            carry: String::new(),
            error: None,
            slots: std::collections::HashMap::new(),
        }
    }

    /// The reply, once the stream is over. Anything held back is part of the answer after
    /// all: a run of characters that looked like the start of a tag and never became one.
    ///
    /// An error the server reported part-way is the result, not the half answer before
    /// it: handing that back as a finished reply is how an overloaded server turns into an
    /// empty turn in the conversation.
    pub fn finish(mut self) -> Result<ChatReply, String> {
        // A last line with no newline after it is still a line.
        if !self.pending.is_empty() {
            let tail = std::mem::take(&mut self.pending);
            let line = String::from_utf8_lossy(&tail).into_owned();
            self.line(line.trim_end_matches(['\r', '\n']));
        }
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        if !self.carry.is_empty() {
            let carry = std::mem::take(&mut self.carry);
            if self.block.as_deref() == Some("think") {
                self.assembled.thinking.push_str(&carry);
            } else {
                self.assembled.text.push_str(&carry);
            }
        }
        self.assembled.finish()
    }

    /// Feeds raw bytes from the response and returns whatever became readable.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Delta> {
        self.pending.extend_from_slice(chunk);
        let mut deltas = Vec::new();
        while let Some(at) = self.pending.iter().position(|byte| *byte == b'\n') {
            let bytes: Vec<u8> = self.pending.drain(..=at).collect();
            // A newline is never part of a longer character, so a whole line is whole text.
            let line = String::from_utf8_lossy(&bytes);
            deltas.extend(self.line(line.trim_end_matches(['\r', '\n'])));
        }
        deltas
    }

    /// What the server reported going wrong, once it has.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn line(&mut self, line: &str) -> Vec<Delta> {
        let Some(data) = line.strip_prefix("data:") else {
            // `event:` lines name the type, which the payload repeats, and blank lines only
            // separate events. Neither carries anything this needs.
            return Vec::new();
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Vec::new();
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        // Both formats report a failure as an `error` object - Anthropic inside an event
        // of that type, OpenAI and llama.cpp bare.
        if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
            let message = error["message"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            let kind = error["type"].as_str().unwrap_or_default();
            self.error.get_or_insert(if kind.is_empty() {
                message
            } else {
                format!("{kind}: {message}")
            });
            return Vec::new();
        }
        match self.wire {
            Wire::Openai => self.openai(&value),
            Wire::Anthropic => self.anthropic(&value),
        }
    }

    fn openai(&mut self, value: &Value) -> Vec<Delta> {
        let mut deltas = Vec::new();
        if let Some(model) = value["model"].as_str() {
            if self.assembled.model.is_empty() && !model.is_empty() {
                self.assembled.model = model.to_string();
                deltas.push(Delta::Model {
                    model: model.to_string(),
                });
            }
        }
        if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
            self.assembled.usage = Usage {
                input: usage["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                output: usage["completion_tokens"].as_u64().unwrap_or(0) as u32,
            };
        }
        let choice = &value["choices"][0];
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.assembled.stop_reason = neutral_stop(reason).into();
        }
        let delta = &choice["delta"];
        if let Some(text) = delta["reasoning_content"]
            .as_str()
            .filter(|text| !text.is_empty())
        {
            self.assembled.thinking.push_str(text);
            deltas.push(Delta::Thinking {
                text: text.to_string(),
            });
        }
        if let Some(text) = delta["content"].as_str().filter(|text| !text.is_empty()) {
            // Some servers leave the reasoning inline in the content, fenced in <think>.
            for piece in self.split_inline(text) {
                deltas.push(piece);
            }
        }
        for call in delta["tool_calls"].as_array().unwrap_or(&Vec::new()) {
            let wire_index = call["index"].as_u64().unwrap_or(0);
            let id = call["id"].as_str().filter(|id| !id.is_empty());
            // The index says which call a fragment belongs to - but some compatible
            // servers send every call as index 0 with an id of its own, and going by the
            // index alone welds their names and arguments into one call nobody made.
            let started_elsewhere = self
                .slots
                .get(&wire_index)
                .and_then(|slot| self.assembled.tool_calls.get(*slot))
                .is_some_and(|entry| id.is_some_and(|id| !entry.id.is_empty() && entry.id != id));
            let index = match self.slots.get(&wire_index) {
                Some(slot) if !started_elsewhere => *slot,
                _ => {
                    self.assembled.tool_calls.push(ToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: serde_json::json!({}),
                    });
                    self.assembled.arguments.push(String::new());
                    let slot = self.assembled.tool_calls.len() - 1;
                    self.slots.insert(wire_index, slot);
                    slot
                }
            };
            let entry = &mut self.assembled.tool_calls[index];
            if let Some(id) = call["id"].as_str() {
                entry.id = id.to_string();
            }
            if let Some(name) = call["function"]["name"]
                .as_str()
                .filter(|name| !name.is_empty())
            {
                entry.name.push_str(name);
                deltas.push(Delta::Tool {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                });
            }
            if let Some(arguments) = call["function"]["arguments"].as_str() {
                self.assembled.arguments[index].push_str(arguments);
            }
        }
        deltas
    }

    /// Splits `<think>` out of content that carries it inline, keeping the state across
    /// chunks - the tags themselves are often split down the middle by the stream.
    fn split_inline(&mut self, text: &str) -> Vec<Delta> {
        let mut deltas = Vec::new();
        // Whatever was held back last time goes in front: the stream cuts tags in half.
        let joined = format!("{}{text}", std::mem::take(&mut self.carry));
        let mut rest = joined.as_str();
        loop {
            let thinking = self.block.as_deref() == Some("think");
            let marker = if thinking { "</think>" } else { "<think>" };
            match rest.find(marker) {
                Some(at) => {
                    let (before, after) = rest.split_at(at);
                    if !before.is_empty() {
                        if thinking {
                            self.assembled.thinking.push_str(before);
                            deltas.push(Delta::Thinking {
                                text: before.to_string(),
                            });
                        } else {
                            self.assembled.text.push_str(before);
                            deltas.push(Delta::Text {
                                text: before.to_string(),
                            });
                        }
                    }
                    self.block = if thinking {
                        None
                    } else {
                        Some("think".to_string())
                    };
                    rest = &after[marker.len()..];
                }
                None => {
                    // A tail that could still become the marker waits for the next chunk.
                    // Only at a character boundary: the tail of "안녕" is three bytes of one
                    // character, and slicing one byte off it is a panic, not a mismatch.
                    let keep = (1..marker.len().min(rest.len() + 1))
                        .rev()
                        .filter(|take| rest.is_char_boundary(rest.len() - take))
                        .find(|take| marker.starts_with(&rest[rest.len() - take..]))
                        .unwrap_or(0);
                    let (emit, held) = rest.split_at(rest.len() - keep);
                    self.carry = held.to_string();
                    if !emit.is_empty() {
                        if thinking {
                            self.assembled.thinking.push_str(emit);
                            deltas.push(Delta::Thinking {
                                text: emit.to_string(),
                            });
                        } else {
                            self.assembled.text.push_str(emit);
                            deltas.push(Delta::Text {
                                text: emit.to_string(),
                            });
                        }
                    }
                    return deltas;
                }
            }
        }
    }

    /// Appends to a text field of the block now being written.
    ///
    /// Only into an object. `value[key] = ...` on a string or a number panics, and the
    /// block is whatever the server sent as `content_block` - a proxy that mangles it
    /// would otherwise take down the turn, and the app's async runtime thread with it.
    fn grow(&mut self, field: &str, piece: &str) {
        if let Some(block) = self
            .assembled
            .blocks
            .last_mut()
            .filter(|block| block.is_object())
        {
            let so_far = block[field].as_str().unwrap_or_default().to_string();
            block[field] = Value::String(so_far + piece);
        }
    }

    fn anthropic(&mut self, value: &Value) -> Vec<Delta> {
        let mut deltas = Vec::new();
        match value["type"].as_str() {
            Some("message_start") => {
                self.assembled.model = value["message"]["model"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if !self.assembled.model.is_empty() {
                    deltas.push(Delta::Model {
                        model: self.assembled.model.clone(),
                    });
                }
                self.assembled.usage.input = value["message"]["usage"]["input_tokens"]
                    .as_u64()
                    .unwrap_or(0) as u32;
            }
            Some("content_block_start") => {
                let block = &value["content_block"];
                self.block = block["type"].as_str().map(str::to_string);
                if block["type"] == "thinking" {
                    deltas.push(Delta::Status {
                        text: "thinking".into(),
                    });
                }
                self.assembled.blocks.push(block.clone());
                self.assembled.block_json.clear();
                if block["type"] == "tool_use" {
                    let call = ToolCall {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name: block["name"].as_str().unwrap_or_default().to_string(),
                        arguments: serde_json::json!({}),
                    };
                    deltas.push(Delta::Tool {
                        id: call.id.clone(),
                        name: call.name.clone(),
                    });
                    self.assembled.tool_calls.push(call);
                    self.assembled.arguments.push(String::new());
                }
            }
            Some("content_block_delta") => {
                let delta = &value["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => {
                        let text = delta["text"].as_str().unwrap_or_default();
                        self.assembled.text.push_str(text);
                        self.grow("text", text);
                        deltas.push(Delta::Text {
                            text: text.to_string(),
                        });
                    }
                    Some("thinking_delta") => {
                        let text = delta["thinking"].as_str().unwrap_or_default();
                        self.assembled.thinking.push_str(text);
                        self.grow("thinking", text);
                        if !text.is_empty() {
                            deltas.push(Delta::Thinking {
                                text: text.to_string(),
                            });
                        }
                    }
                    // The signature is what makes a thinking block acceptable when it is
                    // sent back; a block rebuilt without it is refused.
                    Some("signature_delta") => {
                        self.grow("signature", delta["signature"].as_str().unwrap_or_default());
                    }
                    Some("input_json_delta") => {
                        let piece = delta["partial_json"].as_str().unwrap_or_default();
                        self.assembled.block_json.push_str(piece);
                        if self.block.as_deref() == Some("tool_use") {
                            if let Some(last) = self.assembled.arguments.last_mut() {
                                last.push_str(piece);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some("content_block_stop") => {
                // A block that took its input as streamed JSON has it whole now.
                let raw = std::mem::take(&mut self.assembled.block_json);
                if !raw.trim().is_empty() {
                    if let (Some(block), Ok(input)) = (
                        self.assembled
                            .blocks
                            .last_mut()
                            .filter(|block| block.is_object()),
                        serde_json::from_str::<Value>(&raw),
                    ) {
                        block["input"] = input;
                    }
                }
                self.block = None;
            }
            Some("message_delta") => {
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    self.assembled.stop_reason = neutral_stop(reason).into();
                }
                let details = if value["delta"]["stop_details"].is_object() {
                    &value["delta"]["stop_details"]
                } else {
                    &value["stop_details"]
                };
                if let Some(category) = details["category"].as_str() {
                    self.assembled.refusal = Some(category.to_string());
                }
                self.assembled.usage.output =
                    value["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
            }
            _ => {}
        }
        deltas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deltas(wire: Wire, lines: &[&str]) -> (Vec<Delta>, ChatReply) {
        let mut reader = Reader::new(wire);
        let mut all = Vec::new();
        for line in lines {
            all.extend(reader.push(format!("{line}\n").as_bytes()));
        }
        (all, reader.finish().unwrap())
    }

    #[test]
    fn a_malformed_block_from_the_server_is_survived() {
        // A content block that is not an object, then deltas and JSON that would be
        // written into it.
        let mut reader = Reader::new(Wire::Anthropic);
        for line in [
            r#"data: {"type":"content_block_start","index":0,"content_block":"text"}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"s"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":42}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"x"}}"#,
            r#"data: {"type":"content_block_start","index":2}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"!"}}"#,
        ] {
            reader.push(format!("{line}\n").as_bytes());
        }
        assert_eq!(reader.finish().unwrap().text, "hi!");
    }

    #[test]
    fn bytes_cut_anywhere_read_the_same_as_whole_lines() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"단백질 \"}}]}\r\n",
            "\n",
            "data: not json at all\n",
            "event: ping\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"🧬 구조\"}}]}\n",
            "data: [DONE]"
        )
        .as_bytes();
        // Every way of cutting the stream in two, including through each character.
        for cut in 0..=stream.len() {
            let mut reader = Reader::new(Wire::Openai);
            reader.push(&stream[..cut]);
            reader.push(&stream[cut..]);
            assert_eq!(
                reader.finish().unwrap().text,
                "단백질 🧬 구조",
                "cut at {cut}"
            );
        }
        // And one byte at a time.
        let mut reader = Reader::new(Wire::Openai);
        for byte in stream {
            reader.push(std::slice::from_ref(byte));
        }
        assert_eq!(reader.finish().unwrap().text, "단백질 🧬 구조");
    }

    #[test]
    fn a_very_long_line_and_odd_values_do_not_break_the_reader() {
        let long = "x".repeat(2_000_000);
        let mut reader = Reader::new(Wire::Openai);
        reader.push(
            format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{long}\"}}}}]}}\n")
                .as_bytes(),
        );
        // Fields of the wrong type are ignored rather than trusted.
        reader.push(br#"data: {"choices":"none","usage":{"prompt_tokens":"many"},"model":7}"#);
        reader.push(b"\n");
        reader.push(br#"data: {"choices":[{"delta":{"tool_calls":[{"index":"zero","function":{"name":5}}]}}]}"#);
        reader.push(b"\n");
        reader.push(b"data: [1,2,3]\ndata: null\ndata: \"text\"\n");
        let reply = reader.finish().unwrap();
        assert_eq!(reply.text.len(), long.len());
    }

    #[test]
    fn an_error_mid_stream_is_the_result() {
        let mut reader = Reader::new(Wire::Anthropic);
        reader.push(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"half\"}}\n");
        reader.push(b"data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n");
        assert!(reader.error().is_some());
        assert_eq!(reader.finish().unwrap_err(), "overloaded_error: Overloaded");
    }

    #[test]
    fn openai_deltas_become_text_thinking_and_calls() {
        let (seen, reply) = deltas(
            Wire::Openai,
            &[
                r#"data: {"model":"m","choices":[{"delta":{"reasoning_content":"the chain "}}]}"#,
                r#"data: {"choices":[{"delta":{"reasoning_content":"is A_1"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"Colouring"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":" it."}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"colour_chain","arguments":"{\"chain\""}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"A\"}"}}]}}]}"#,
                r#"data: {"choices":[{"finish_reason":"tool_calls","delta":{}}],"usage":{"prompt_tokens":10,"completion_tokens":4}}"#,
                "data: [DONE]",
            ],
        );
        assert_eq!(
            seen,
            vec![
                Delta::Model { model: "m".into() },
                Delta::Thinking {
                    text: "the chain ".into()
                },
                Delta::Thinking {
                    text: "is A_1".into()
                },
                Delta::Text {
                    text: "Colouring".into()
                },
                Delta::Text {
                    text: " it.".into()
                },
                Delta::Tool {
                    id: "c1".into(),
                    name: "colour_chain".into()
                },
            ]
        );
        assert_eq!(reply.text, "Colouring it.");
        assert_eq!(reply.thinking, "the chain is A_1");
        assert_eq!(reply.stop_reason, "tool");
        // The arguments arrived in fragments and are only JSON once the stream is over.
        assert_eq!(reply.tool_calls[0].arguments["chain"], "A");
        assert_eq!(reply.usage.input, 10);
        assert_eq!(reply.model, "m");
    }

    #[test]
    fn a_think_tag_split_across_chunks_is_still_thinking() {
        // llama.cpp leaves the reasoning inline, and the stream cuts the tag in half.
        let (seen, reply) = deltas(
            Wire::Openai,
            &[
                r#"data: {"choices":[{"delta":{"content":"<th"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"ink>working"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":" on it</think>Done."}}]}"#,
            ],
        );
        assert_eq!(reply.thinking, "working on it");
        assert_eq!(reply.text, "Done.");
        assert!(seen
            .iter()
            .any(|delta| matches!(delta, Delta::Thinking { .. })));
    }

    #[test]
    fn anthropic_events_become_the_same_pieces() {
        let (seen, reply) = deltas(
            Wire::Anthropic,
            &[
                r#"event: message_start"#,
                r#"data: {"type":"message_start","message":{"model":"claude","usage":{"input_tokens":12}}}"#,
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#,
                r#"data: {"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"read first"}}"#,
                r#"data: {"type":"content_block_stop"}"#,
                r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text"}}"#,
                r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Opening it."}}"#,
                r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"open_pdb"}}"#,
                r#"data: {"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{\"id\":"}}"#,
                r#"data: {"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"\"4hhb\"}"}}"#,
                r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#,
            ],
        );
        assert_eq!(
            seen,
            vec![
                Delta::Model {
                    model: "claude".into()
                },
                Delta::Status {
                    text: "thinking".into()
                },
                Delta::Thinking {
                    text: "read first".into()
                },
                Delta::Text {
                    text: "Opening it.".into()
                },
                Delta::Tool {
                    id: "t1".into(),
                    name: "open_pdb".into()
                },
            ]
        );
        assert_eq!(reply.text, "Opening it.");
        assert_eq!(reply.thinking, "read first");
        assert_eq!(reply.tool_calls[0].arguments["id"], "4hhb");
        assert_eq!(reply.stop_reason, "tool");
        assert_eq!((reply.usage.input, reply.usage.output), (12, 7));
    }

    #[test]
    fn a_line_split_across_two_chunks_is_read_once_it_is_whole() {
        let mut reader = Reader::new(Wire::Openai);
        assert!(reader
            .push(b"data: {\"choices\":[{\"delta\":{\"con")
            .is_empty());
        let deltas = reader.push(b"tent\":\"hi\"}}]}\n");
        assert_eq!(deltas, vec![Delta::Text { text: "hi".into() }]);
    }

    #[test]
    fn text_that_is_not_ascii_does_not_panic_or_tear() {
        // The hold-back for a split <think> tag slices the tail of each delta.
        let (_, reply) = deltas(
            Wire::Openai,
            &[
                r#"data: {"choices":[{"delta":{"content":"안녕"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"하세요 —"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":" café"}}]}"#,
            ],
        );
        assert_eq!(reply.text, "안녕하세요 — café");

        // And the network may cut a chunk through the middle of a character.
        let line = "data: {\"choices\":[{\"delta\":{\"content\":\"단백질\"}}]}\n".as_bytes();
        let cut = line.len() - 8; // inside the last character
        assert!(std::str::from_utf8(&line[..cut]).is_err());
        let mut reader = Reader::new(Wire::Openai);
        assert!(reader.push(&line[..cut]).is_empty());
        reader.push(&line[cut..]);
        assert_eq!(reader.finish().unwrap().text, "단백질");
    }

    #[test]
    fn an_error_sent_mid_stream_is_the_result() {
        let mut reader = Reader::new(Wire::Anthropic);
        reader.push(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Half\"}}\n");
        reader.push(b"data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n");
        assert_eq!(reader.finish().unwrap_err(), "overloaded_error: Overloaded");

        let mut reader = Reader::new(Wire::Openai);
        reader.push(b"data: {\"error\":{\"message\":\"context length exceeded\"}}");
        // No newline after it: the last line still counts.
        assert_eq!(reader.finish().unwrap_err(), "context length exceeded");
    }

    #[test]
    fn a_call_cut_off_mid_arguments_is_not_run_with_none() {
        let mut reader = Reader::new(Wire::Openai);
        reader.push(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"set_style\",\"arguments\":\"{\\\"molecule\\\":\"}}]}}]}\n");
        let error = reader.finish().unwrap_err();
        assert!(error.contains("set_style"), "{error}");

        // A call with no arguments at all is a call that takes none.
        let mut reader = Reader::new(Wire::Openai);
        reader.push(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"read_graph\"}}]}}]}\n");
        let reply = reader.finish().unwrap();
        assert_eq!(reply.tool_calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn two_calls_sharing_an_index_stay_two_calls() {
        let mut reader = Reader::new(Wire::Openai);
        reader.push(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"read_graph\",\"arguments\":\"{}\"}}]}}]}\n");
        reader.push(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"b\",\"function\":{\"name\":\"list_molecules\",\"arguments\":\"{}\"}}]}}]}\n");
        let reply = reader.finish().unwrap();
        let names: Vec<&str> = reply
            .tool_calls
            .iter()
            .map(|call| call.name.as_str())
            .collect();
        assert_eq!(names, vec!["read_graph", "list_molecules"]);
    }

    #[test]
    fn a_claude_stream_is_rebuilt_into_the_blocks_it_was() {
        let (_, reply) = deltas(
            Wire::Anthropic,
            &[
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"read "}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"first"}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-abc"}}"#,
                r#"data: {"type":"content_block_stop","index":0}"#,
                r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
                r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Opening it."}}"#,
                r#"data: {"type":"content_block_stop","index":1}"#,
                r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"open_pdb","input":{}}}"#,
                r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"id\":"}}"#,
                r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"4hhb\"}"}}"#,
                r#"data: {"type":"content_block_stop","index":2}"#,
                r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#,
            ],
        );
        let provider = reply.provider.expect("a Claude reply carries its blocks");
        assert_eq!(provider["wire"], "anthropic");
        assert_eq!(
            provider["content"],
            serde_json::json!([
                { "type": "thinking", "thinking": "read first", "signature": "sig-abc" },
                { "type": "text", "text": "Opening it." },
                { "type": "tool_use", "id": "t1", "name": "open_pdb", "input": { "id": "4hhb" } },
            ])
        );
        assert_eq!(reply.stop_reason, "tool");
    }

    #[test]
    fn a_declined_turn_says_so_and_an_empty_one_out_of_tokens_is_an_error() {
        let (_, reply) = deltas(
            Wire::Anthropic,
            &[
                r#"data: {"type":"message_delta","delta":{"stop_reason":"refusal","stop_details":{"type":"refusal","category":"bio"}},"usage":{"output_tokens":0}}"#,
            ],
        );
        assert_eq!(reply.stop_reason, "refusal");
        assert_eq!(reply.text, "The model declined this request (bio).");

        let mut reader = Reader::new(Wire::Openai);
        reader.push(b"data: {\"choices\":[{\"finish_reason\":\"length\",\"delta\":{}}]}\n");
        assert!(reader.finish().unwrap_err().contains("ran out of tokens"));
    }

    #[test]
    fn an_openai_reply_carries_no_provider_blocks() {
        let (_, reply) = deltas(
            Wire::Openai,
            &[r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#],
        );
        assert!(reply.provider.is_none());
    }
}
