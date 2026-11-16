//! Agents: models that can read this app and change it.
//!
//! Three kinds of connection - a model running on this machine, Claude, and ChatGPT - and
//! one neutral shape of request and reply that all three are adapted to. The conversation
//! itself, and the tools an agent calls, live in the UI: the graph, the viewport and the
//! selection are there, and a loop that ran here would have to reach back across the
//! boundary for every step. What lives here is the part that must not live there - the
//! credential, which is read from the OS keychain at call time and never handed to the
//! webview - and the adapters, because the three wire formats differ in every detail while
//! meaning the same thing.
//!
//! What an agent can touch is decided by the tools the UI offers it, and none of them
//! reach the app's own files, settings or code. That is not a permission that can be
//! turned on; there is simply no tool for it.

pub mod cli;
pub mod docs;
pub mod engine;
pub mod stream;
pub mod weights;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

/// Where the list of connections and the agent's settings are kept.
pub const SETTING: &str = "agents";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An OpenAI-compatible endpoint on this machine: Ollama, llama.cpp, LM Studio, vLLM.
    Local,
    Anthropic,
    Openai,
    /// A command-line agent the user has already installed and signed in to - Claude Code,
    /// Codex - run on their own machine against their own subscription.
    ///
    /// This is the only honest way to use a Claude or ChatGPT *subscription* from here. A
    /// subscription is not an API key: the hosted APIs bill separately, and driving the
    /// chat websites or reusing another program's login would breach the terms of both.
    /// Running a first-party CLI that the user installed and signed in to themselves is
    /// just them using their own tool, and it is a supported way to do it.
    Cli,
}

/// One configured connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// The base URL. Kept per connection so a local model can be on another port and a
    /// hosted one behind a proxy or a gateway.
    pub endpoint: String,
    pub model: String,
    /// Name of the keychain entry holding the key. A local model usually needs none, and a
    /// CLI never does - it is already signed in.
    #[serde(default)]
    pub secret_ref: Option<String>,
    /// For `cli`: the program to run, e.g. `claude`.
    #[serde(default)]
    pub command: Option<String>,
    /// For `cli`: the arguments before the prompt, which goes in on stdin.
    #[serde(default)]
    pub args: Vec<String>,
    /// For `local`: the id of a downloaded model that Geis runs itself.
    ///
    /// When this is set the `endpoint` is ignored and the address is worked out at call
    /// time from the engine that is actually running. A port is a fact about a live
    /// process, not a setting: storing one meant that after a restart the panel went on
    /// asking a port nothing was listening on, and said only that it could not reach it.
    #[serde(default)]
    pub engine_model: Option<String>,
    /// For `cli`: which of the provider's models to ask for, by the name its tool takes -
    /// `opus`, `sonnet`. Nothing leaves it to the tool, which is whatever the account
    /// defaults to.
    #[serde(default)]
    pub choice: Option<String>,
    /// For `cli`: how hard the model is to think - `low` to `max` as the tool spells it.
    /// Nothing leaves it to the tool.
    #[serde(default)]
    pub effort: Option<String>,
}

/// The arguments that carry a connection's chosen model and effort to its tool.
///
/// Each tool spells them its own way, and a tool this app does not know is given neither:
/// a flag it does not have would stop it from running at all.
fn cli_choices(flavour: cli::Flavour, connection: &Connection) -> Vec<String> {
    // A name, and nothing that could be read as another argument.
    let plain = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|text| {
                !text.is_empty()
                    && !text.starts_with('-')
                    && text.chars().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']')
                    })
            })
            .map(str::to_string)
    };
    let mut args = Vec::new();
    match flavour {
        cli::Flavour::Claude => {
            if let Some(model) = plain(&connection.choice) {
                args.extend(["--model".to_string(), model]);
            }
            if let Some(effort) = plain(&connection.effort) {
                args.extend(["--effort".to_string(), effort]);
            }
        }
        cli::Flavour::Codex => {
            if let Some(model) = plain(&connection.choice) {
                args.extend(["--model".to_string(), model]);
            }
            if let Some(effort) = plain(&connection.effort) {
                args.extend([
                    "-c".to_string(),
                    format!("model_reasoning_effort=\"{effort}\""),
                ]);
            }
        }
        cli::Flavour::Plain => {}
    }
    args
}

/// How much an agent may do without being asked.
///
/// The default is that it may do everything the tools allow, because the tools allow only
/// what this app is for. Narrowing it is a matter of taste rather than safety.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    /// Read and change, without asking.
    #[default]
    Auto,
    /// Read without asking; ask before anything that changes the scene.
    Ask,
    /// Read only. Every tool that changes something is refused.
    ReadOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub policy: Policy,
    /// The connection the panel opens with.
    #[serde(default)]
    pub active: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            connections: default_connections(),
            policy: Policy::default(),
            active: None,
        }
    }
}

/// The model names the API adapters are tested with. Nothing is seeded with them: the
/// connections this app starts with are the subscription ones.
#[cfg(test)]
pub const DEFAULT_CLAUDE: &str = "claude-opus-5-5";
#[cfg(test)]
pub const DEFAULT_GPT: &str = "gpt-6-luna";

/// How each subscription CLI is run. The prompt goes in on stdin.
///
/// Claude Code is asked to answer and nothing else: none of its own tools, so it cannot
/// read or change files on its own account - what an agent may do here is decided by the
/// tools this app offers, not by the ones the CLI ships with - and no saved session, since
/// the conversation is this app's to keep. Codex likewise: a read-only sandbox, nothing
/// persisted, no colour codes in what comes back, and no complaint about the directory
/// it is run from not being a repository.
///
/// The one thing each keeps of its own is the web: searching and reading a page are not
/// things this app can offer a model, and "find the paper this structure is from" is a
/// thing people ask of it. Claude Code is given those two tools and no others, allowed
/// ahead of time since nobody is at a prompt to approve them; Codex is started with search.
const CLAUDE_CLI_ARGS: &[&str] = &[
    "-p",
    "--tools",
    "WebSearch,WebFetch",
    "--allowedTools",
    "WebSearch,WebFetch",
    "--no-session-persistence",
];
const CODEX_CLI_ARGS: &[&str] = &[
    "--search",
    "exec",
    "--skip-git-repo-check",
    "--sandbox",
    "read-only",
    "--ephemeral",
    "--color",
    "never",
];
/// What the two were seeded with before they could use the web.
const CLAUDE_CLI_ARGS_OFFLINE: &[&str] = &["-p", "--tools", "", "--no-session-persistence"];
const CODEX_CLI_ARGS_OFFLINE: &[&str] = &[
    "exec",
    "--skip-git-repo-check",
    "--sandbox",
    "read-only",
    "--ephemeral",
    "--color",
    "never",
];

impl Settings {
    /// Brings a saved list of connections in line with what this app offers now: Claude and
    /// ChatGPT through the user's own subscription, and nothing that needs an API key.
    ///
    /// The two key-based connections were seeded by an earlier version and are taken out;
    /// the two CLI ones, if still as they were seeded, get the arguments above and lose the
    /// "(subscription)" from their names, there being no other kind to tell them from.
    /// Returns whether anything changed, so the caller knows to save.
    pub fn keep_to_subscriptions(&mut self) -> bool {
        let before = self.connections.len();
        self.connections.retain(|connection| {
            !(matches!(connection.kind, Kind::Anthropic | Kind::Openai)
                && matches!(connection.id.as_str(), "claude" | "chatgpt"))
        });
        let mut changed = self.connections.len() != before;
        if let Some(active) = &self.active {
            if !self
                .connections
                .iter()
                .any(|connection| connection.id == *active)
            {
                self.active = None;
                changed = true;
            }
        }
        for connection in &mut self.connections {
            let (old_args, new_args, old_name, new_name): (&[&str], &[&str], &str, &str) =
                match connection.id.as_str() {
                    "claude-cli" => (&["-p"], CLAUDE_CLI_ARGS, "Claude (subscription)", "Claude"),
                    "codex-cli" => (
                        &["exec"],
                        CODEX_CLI_ARGS,
                        "ChatGPT (subscription)",
                        "ChatGPT",
                    ),
                    _ => continue,
                };
            let offline: &[&str] = if connection.id == "claude-cli" {
                CLAUDE_CLI_ARGS_OFFLINE
            } else {
                CODEX_CLI_ARGS_OFFLINE
            };
            let same = |as_seeded: &[&str]| {
                connection
                    .args
                    .iter()
                    .map(String::as_str)
                    .eq(as_seeded.iter().copied())
            };
            if same(old_args) || same(offline) {
                connection.args = new_args.iter().map(|arg| (*arg).to_string()).collect();
                changed = true;
            }
            if connection.name == old_name {
                connection.name = new_name.to_string();
                changed = true;
            }
        }
        changed
    }
}

/// The three connections the panel starts with: a model on this machine, and Claude and
/// ChatGPT through the subscriptions the user already has.
///
/// Seeded rather than left empty because "add a connection" is a worse first screen than
/// "here are the three". None of them takes an API key - that was a choice: a subscription
/// is what the people this is for already pay for, and a key is a second bill.
pub fn default_connections() -> Vec<Connection> {
    vec![
        Connection {
            id: "local".into(),
            name: "Local".into(),
            kind: Kind::Local,
            endpoint: "http://localhost:11434/v1".into(),
            model: "qwen2.5-coder:14b".into(),
            secret_ref: None,
            command: None,
            args: Vec::new(),
            engine_model: None,
            choice: None,
            effort: None,
        },
        // Claude and ChatGPT, each through the user's own subscription. They need no key:
        // the CLI is already signed in to the user's plan. What they do need is the CLI
        // installed, which Test says - and where it was found.
        Connection {
            id: "claude-cli".into(),
            name: "Claude".into(),
            kind: Kind::Cli,
            endpoint: String::new(),
            model: "claude".into(),
            secret_ref: None,
            command: Some("claude".into()),
            args: CLAUDE_CLI_ARGS
                .iter()
                .map(|arg| (*arg).to_string())
                .collect(),
            engine_model: None,
            choice: None,
            effort: None,
        },
        Connection {
            id: "codex-cli".into(),
            name: "ChatGPT".into(),
            kind: Kind::Cli,
            endpoint: String::new(),
            model: "codex".into(),
            secret_ref: None,
            command: Some("codex".into()),
            args: CODEX_CLI_ARGS
                .iter()
                .map(|arg| (*arg).to_string())
                .collect(),
            engine_model: None,
            choice: None,
            effort: None,
        },
    ]
}

// --- the neutral shape ------------------------------------------------------------------

/// One turn of the conversation. `tool` messages carry a call's result back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// `user`, `assistant` or `tool`.
    pub role: String,
    #[serde(default)]
    pub content: String,
    /// What the assistant asked to run, on an assistant turn.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Which call this message answers, on a tool turn.
    #[serde(default)]
    pub tool_call_id: Option<String>,
    /// An assistant turn exactly as its provider wrote it, to be sent back unchanged.
    ///
    /// The neutral shape above keeps the text and the calls and drops the rest, and for
    /// Claude the rest matters: a turn's `thinking` blocks carry a signature, and the API
    /// wants them returned as they came - rebuilt from text and calls alone, the model
    /// loses the reasoning behind its own tool call, and a turn mid-way through a tool
    /// round may be refused. Opaque to everything but the adapter that made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The arguments as JSON. Kept as a value rather than a string so the adapters do not
    /// each have to parse and re-serialise it.
    #[serde(default)]
    pub arguments: Value,
}

/// A tool as the model is told about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// A JSON Schema object for the arguments.
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input: u32,
    pub output: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatReply {
    pub text: String,
    /// What a reasoning model worked through before answering.
    ///
    /// Kept apart from the answer rather than thrown away: it is the most useful thing on
    /// screen when a model does the wrong thing, and it is not the answer, so showing it as
    /// one would be wrong.
    #[serde(default)]
    pub thinking: String,
    pub tool_calls: Vec<ToolCall>,
    /// `end` when the model is done, `tool` when it is waiting for tool results.
    pub stop_reason: String,
    pub usage: Usage,
    /// The model that actually answered, which a gateway may change.
    pub model: String,
    /// The turn as the provider wrote it - see `Message::provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Value>,
}

/// The provider blob for a Claude turn: its content blocks, verbatim.
pub(crate) fn anthropic_provider(content: Value) -> Value {
    json!({ "wire": "anthropic", "content": content })
}

/// The content blocks a message carries from Claude, if it carries any.
fn anthropic_blocks(message: &Message) -> Option<&Vec<Value>> {
    let provider = message.provider.as_ref()?;
    if provider["wire"] != "anthropic" {
        return None;
    }
    provider["content"]
        .as_array()
        .filter(|blocks| !blocks.is_empty())
}

// --- adapters ---------------------------------------------------------------------------

/// Anthropic's Messages API. Tools are top-level objects with an `input_schema`, results
/// come back as `tool_use` content blocks, and a result is sent as a `tool_result` block
/// inside a *user* turn - none of which OpenAI does the same way.
fn anthropic_body(request: &ChatRequest, model: &str) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    for message in &request.messages {
        match message.role.as_str() {
            "tool" => {
                let mut result = json!({
                    "type": "tool_result",
                    "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content,
                });
                // The loop reports a tool that failed or was refused as text beginning
                // "error:"; said in the API's own terms as well, so the model need not
                // infer it from the wording.
                if message.content.starts_with("error:") {
                    result["is_error"] = json!(true);
                }
                // Every result of one assistant turn goes back in one user message. Sent
                // one message each they are still accepted, but a model answered that way
                // learns to stop asking for several things at once.
                match messages.last_mut() {
                    Some(last)
                        if last["role"] == "user"
                            && last["content"].as_array().is_some_and(|blocks| {
                                blocks.iter().all(|block| block["type"] == "tool_result")
                            }) =>
                    {
                        if let Some(blocks) = last["content"].as_array_mut() {
                            blocks.push(result);
                        }
                    }
                    _ => messages.push(json!({ "role": "user", "content": [result] })),
                }
            }
            "assistant" => {
                // A turn Claude wrote goes back as Claude wrote it, thinking blocks and
                // all. Anything else - a turn from another provider, or from a
                // conversation saved before this was kept - is rebuilt from its parts.
                let blocks: Vec<Value> = match anthropic_blocks(message) {
                    Some(blocks) => blocks.clone(),
                    None => {
                        let mut blocks = Vec::new();
                        if !message.content.is_empty() {
                            blocks.push(json!({ "type": "text", "text": message.content }));
                        }
                        for call in &message.tool_calls {
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": call.id,
                                "name": call.name,
                                "input": call.arguments,
                            }));
                        }
                        blocks
                    }
                };
                // An assistant turn with nothing in it - a reply that failed before it
                // said anything - is refused outright by the API, and once one is in the
                // history every later request carries it. Leaving it out costs nothing.
                if !blocks.is_empty() {
                    messages.push(json!({ "role": "assistant", "content": blocks }));
                }
            }
            _ => messages.push(json!({ "role": "user", "content": message.content })),
        }
    }
    let mut body = json!({
        "model": model,
        "max_tokens": request.max_tokens.unwrap_or(4096),
        "system": request.system,
        "messages": messages,
    });
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.parameters,
                    })
                })
                .collect(),
        );
    }
    // No `temperature`: every current Claude model answers a request that sets one with
    // a 400, and nothing here ever asked for one.
    body
}

/// Models that take adaptive thinking with a display setting.
fn thinks_adaptively(model: &str) -> bool {
    [
        "claude-fable-5",
        "claude-mythos-5",
        "claude-opus-5",
        "claude-sonnet-5",
        "claude-haiku-5-5",
        "claude-opus-4-8",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-sonnet-4-6",
    ]
    .iter()
    .any(|prefix| model.starts_with(prefix))
}

/// Models whose requests may name a fallback for a declined turn.
fn takes_fallbacks(model: &str) -> bool {
    [
        "claude-fable-5-2",
        "claude-fable-5-1",
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-sonnet-5-5",
    ]
    .contains(&model)
}

/// The optional parts of a Claude request, added where the model and the endpoint are
/// known to take them. Returns the beta header they need, if any.
///
/// Two things. The reasoning is asked for as a summary: on current models it is otherwise
/// returned empty, and the panel that exists to show a model thinking shows a pause. And
/// a turn the safety classifiers decline is allowed to fall back to another model inside
/// the same request rather than simply stopping - only against Anthropic's own endpoint,
/// since a gateway in between may not know the field.
fn anthropic_extras(connection: &Connection, body: &mut Value) -> Option<&'static str> {
    let model = connection.model.as_str();
    if thinks_adaptively(model) {
        body["thinking"] = json!({ "type": "adaptive", "display": "summarized" });
    }
    let direct = reqwest::Url::parse(&connection.endpoint)
        .ok()
        .and_then(|url| url.host_str().map(|host| host == "api.anthropic.com"))
        .unwrap_or(false);
    if direct && takes_fallbacks(model) {
        body["fallbacks"] = json!("default");
        return Some("server-side-fallback-2026-07-01");
    }
    None
}

/// Takes every thinking block out of the history. True if there were any.
fn strip_thinking(body: &mut Value) -> bool {
    let mut removed = false;
    for message in body["messages"].as_array_mut().into_iter().flatten() {
        if let Some(blocks) = message["content"].as_array_mut() {
            let before = blocks.len();
            blocks.retain(|block| {
                !matches!(
                    block["type"].as_str(),
                    Some("thinking" | "redacted_thinking")
                )
            });
            removed |= blocks.len() != before;
        }
    }
    // A turn that was nothing but thinking is now empty, and an empty turn is refused.
    if let Some(messages) = body["messages"].as_array_mut() {
        messages.retain(|message| {
            message["content"]
                .as_array()
                .map_or(true, |blocks| !blocks.is_empty())
        });
    }
    removed
}

/// Sends a Claude request, and recovers from the two refusals that are about how the
/// request was put rather than what it asked.
///
/// A thinking block is tied to the conversation it was written in, so one replayed after
/// the system prompt or the tools changed - the policy switched mid-conversation, say - is
/// rejected; the documented recovery is to send the history without its thinking blocks.
/// And the optional parts above are optional: an endpoint or a model that will not take
/// them is asked again without, rather than left unusable over a nicety.
async fn anthropic_send(
    client: &reqwest::Client,
    connection: &Connection,
    key: &str,
    request: &ChatRequest,
    stream: bool,
) -> Result<reqwest::Response, String> {
    let url = endpoint(connection, "messages");
    let mut body = anthropic_body(request, &connection.model);
    let mut beta = anthropic_extras(connection, &mut body);
    if stream {
        body["stream"] = json!(true);
    }
    for _ in 0..3 {
        let mut builder = client
            .post(&url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01");
        if let Some(beta) = beta {
            builder = builder.header("anthropic-beta", beta);
        }
        let response = builder
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("could not reach {url}: {error}"))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let text = response.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::BAD_REQUEST {
            if text.contains("`thinking` block") && strip_thinking(&mut body) {
                continue;
            }
            let had_extras =
                beta.is_some() || body.get("thinking").is_some() || body.get("fallbacks").is_some();
            if had_extras {
                if let Some(fields) = body.as_object_mut() {
                    fields.remove("thinking");
                    fields.remove("fallbacks");
                }
                beta = None;
                continue;
            }
        }
        // The body is where these APIs say what was actually wrong, and it is short.
        return Err(format!(
            "{status}: {}",
            text.chars().take(400).collect::<String>()
        ));
    }
    Err(format!("{url} refused the request three times"))
}

fn anthropic_reply(value: &Value) -> ChatReply {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in value["content"].as_array().unwrap_or(&Vec::new()) {
        match block["type"].as_str() {
            Some("text") => text.push_str(block["text"].as_str().unwrap_or_default()),
            Some("tool_use") => tool_calls.push(ToolCall {
                id: block["id"].as_str().unwrap_or_default().to_string(),
                name: block["name"].as_str().unwrap_or_default().to_string(),
                arguments: block["input"].clone(),
            }),
            _ => {}
        }
    }
    let thinking: String = value["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "thinking")
        .filter_map(|block| block["thinking"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let stop = value["stop_reason"].as_str().unwrap_or("end_turn");
    // A declined turn has no answer in it; what it has is a reason, and that is the reply.
    if stop == "refusal" && text.trim().is_empty() {
        text = stream::refusal_note(value["stop_details"]["category"].as_str());
    }
    ChatReply {
        text,
        thinking,
        stop_reason: stream::neutral_stop(stop).into(),
        provider: value
            .get("content")
            .filter(|content| content.is_array())
            .cloned()
            .map(anthropic_provider),
        tool_calls,
        usage: Usage {
            input: value["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            output: value["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
        },
        model: value["model"].as_str().unwrap_or_default().to_string(),
    }
}

/// OpenAI's Chat Completions API, which the local servers speak too - that is the whole
/// reason a local model needs no adapter of its own.
fn openai_body(request: &ChatRequest, model: &str, kind: Kind) -> Value {
    let mut messages: Vec<Value> = vec![json!({ "role": "system", "content": request.system })];
    for message in &request.messages {
        match message.role.as_str() {
            "tool" => messages.push(json!({
                "role": "tool",
                "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
                "content": message.content,
            })),
            "assistant" => {
                let mut turn = json!({ "role": "assistant", "content": message.content });
                if !message.tool_calls.is_empty() {
                    turn["tool_calls"] = Value::Array(
                        message
                            .tool_calls
                            .iter()
                            .map(|call| {
                                json!({
                                    "id": call.id,
                                    "type": "function",
                                    "function": {
                                        "name": call.name,
                                        "arguments": call.arguments.to_string(),
                                    },
                                })
                            })
                            .collect(),
                    );
                }
                messages.push(turn);
            }
            _ => messages.push(json!({ "role": "user", "content": message.content })),
        }
    }
    let mut body = json!({ "model": model, "messages": messages });
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        },
                    })
                })
                .collect(),
        );
    }
    // The same limit under two names. OpenAI deprecated `max_tokens` for
    // `max_completion_tokens` - which also counts reasoning - and its reasoning models
    // refuse the old one outright; the local servers that borrowed this API mostly know
    // only the old one. So each is spoken to in the dialect it has.
    if let Some(max) = request.max_tokens {
        let name = if kind == Kind::Openai {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        body[name] = json!(max);
    }
    // A sampling temperature is a local model's to take; the hosted reasoning models
    // reject any but their default.
    if let (Some(temperature), Kind::Local) = (request.temperature, kind) {
        body["temperature"] = json!(temperature);
    }
    body
}

/// Splits a reasoning model's `<think>` block out of its answer.
///
/// llama.cpp puts the reasoning in `reasoning_content` when it can, and leaves it inline in
/// the content when it cannot; both happen, depending on the model's template.
fn split_thinking(content: &str) -> (String, String) {
    let Some(start) = content.find("<think>") else {
        return (content.trim().to_string(), String::new());
    };
    let rest = &content[start + 7..];
    match rest.find("</think>") {
        Some(end) => (
            format!("{}{}", &content[..start], &rest[end + 8..])
                .trim()
                .to_string(),
            rest[..end].trim().to_string(),
        ),
        // Still thinking when it ran out of tokens: there is no answer, only the thought.
        None => (String::new(), rest.trim().to_string()),
    }
}

fn openai_reply(value: &Value) -> ChatReply {
    let choice = &value["choices"][0];
    let message = &choice["message"];
    let tool_calls = message["tool_calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .map(|call| ToolCall {
                    id: call["id"].as_str().unwrap_or_default().to_string(),
                    name: call["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    // Arguments arrive as a JSON string here, and as an object from
                    // Anthropic; the neutral shape is the object.
                    arguments: call["function"]["arguments"]
                        .as_str()
                        .and_then(|text| serde_json::from_str(text).ok())
                        .unwrap_or_else(|| json!({})),
                })
                .collect()
        })
        .unwrap_or_default();
    let (text, inline) = split_thinking(message["content"].as_str().unwrap_or_default());
    let thinking = message["reasoning_content"]
        .as_str()
        .map(|reasoning| reasoning.trim().to_string())
        .filter(|reasoning| !reasoning.is_empty())
        .unwrap_or(inline);
    ChatReply {
        text,
        thinking,
        stop_reason: stream::neutral_stop(choice["finish_reason"].as_str().unwrap_or("stop"))
            .into(),
        tool_calls,
        usage: Usage {
            input: value["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output: value["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
        },
        model: value["model"].as_str().unwrap_or_default().to_string(),
        provider: None,
    }
}

/// What a CLI agent is asked, and how it is asked to answer.
///
/// A command-line agent has no tool-calling protocol we can drive - it has its own tools
/// and its own loop - so the conversation is flattened into one prompt and the reply is
/// read back. Tools are offered as a convention rather than a protocol: one fenced JSON
/// block if it wants to call something, prose otherwise. Less exact than a real tool API,
/// and the reason an API key remains the better route when there is one.
const TOOL_CONVENTION: &str = concat!(
    "\n\n# Tools\n\n",
    "You act through the tools listed below, and they are the only way you have of seeing or ",
    "changing anything in the app. You have no other tools here: no shell, no files, no web.\n\n",
    "To use one, reply with a single fenced json block and nothing else:\n\n",
    "```json\n{\"tool\": \"name\", \"arguments\": {}}\n```\n\n",
    "You will then be given its result, and can call another or answer. One call per reply.\n\n",
    "If answering needs something you have not been told - which molecules are open, what a ",
    "chain is called, what is selected - call the tool that says, rather than answering that ",
    "you do not know. Only when you have what you need, write the answer, with no json block.\n\n",
);

/// What a CLI is told it is and how it is to work: the app's system prompt, the way a
/// tool is asked for, and the tools.
fn cli_system(request: &ChatRequest) -> String {
    let mut prompt = String::new();
    prompt.push_str(&request.system);
    if !request.tools.is_empty() {
        prompt.push_str(TOOL_CONVENTION);
        for tool in &request.tools {
            prompt.push_str(&format!(
                "- `{}` - {}\n  arguments: {}\n",
                tool.name, tool.description, tool.parameters
            ));
        }
    }
    prompt
}

/// The conversation so far, as text.
fn cli_conversation(request: &ChatRequest) -> String {
    let mut prompt = String::from("# Conversation\n\n");
    for message in &request.messages {
        match message.role.as_str() {
            "tool" => prompt.push_str(&format!("Tool result: {}\n\n", message.content)),
            "assistant" => {
                if !message.content.is_empty() {
                    prompt.push_str(&format!("You: {}\n\n", message.content));
                }
                for call in &message.tool_calls {
                    prompt.push_str(&format!(
                        "You called `{}` with {}\n\n",
                        call.name, call.arguments
                    ));
                }
            }
            _ => prompt.push_str(&format!("User: {}\n\n", message.content)),
        }
    }
    prompt.push_str("Answer now.");
    prompt
}

/// Everything, as one text: for a tool that has nowhere else to put a system prompt.
fn cli_prompt(request: &ChatRequest) -> String {
    format!("{}\n\n{}", cli_system(request), cli_conversation(request))
}

/// How a request is handed to a CLI: extra arguments, and what goes in on stdin.
///
/// Claude Code takes a system prompt as one - and it has to be given that way, because
/// the one it has by default is for a coding agent in a repository: left in place, the
/// app's instructions arrive as something the user said, underneath a prompt that says
/// the model is there to edit code. Codex has no such argument, so it gets the
/// instructions at the head of what it reads.
fn cli_handoff(
    flavour: cli::Flavour,
    args: &[String],
    request: &ChatRequest,
) -> (Vec<String>, String) {
    let already = args
        .iter()
        .any(|arg| arg.starts_with("--system-prompt") || arg.starts_with("--append-system-prompt"));
    if flavour == cli::Flavour::Claude && !already {
        (
            vec!["--system-prompt".to_string(), cli_system(request)],
            cli_conversation(request),
        )
    } else {
        (Vec::new(), cli_prompt(request))
    }
}

/// Reads a CLI's output: the last fenced json block naming a tool, or the whole thing as
/// an answer. The last, because an agentic CLI narrates before it concludes.
fn cli_reply(output: &str, model: &str) -> ChatReply {
    let mut tool_calls = Vec::new();
    let mut text = output.trim().to_string();
    if let Some(block) = last_json_block(output) {
        if let Ok(value) = serde_json::from_str::<Value>(block.trim()) {
            if let Some(name) = value["tool"].as_str() {
                tool_calls.push(ToolCall {
                    // Unique, not "cli-1" every time: the panel files each result under
                    // its call's id, and a second call with the first one's id had its
                    // result written onto the first one's card.
                    id: format!("cli-{}", &crate::util::new_id()[..8]),
                    name: name.to_string(),
                    arguments: value.get("arguments").cloned().unwrap_or_else(|| json!({})),
                });
                // Whatever it said around the block is narration; keep that and drop the
                // block, so the panel does not show raw json as an answer.
                text = output
                    .replace(&format!("```json{block}```"), "")
                    .trim()
                    .to_string();
            }
        }
    }
    let (answer, thinking) = split_thinking(&text);
    ChatReply {
        stop_reason: if tool_calls.is_empty() {
            "end".into()
        } else {
            "tool".into()
        },
        text: if answer.is_empty() { text } else { answer },
        thinking,
        tool_calls,
        usage: Usage::default(),
        model: model.to_string(),
        provider: None,
    }
}

/// The contents of the last ```json fence, if there is one.
fn last_json_block(text: &str) -> Option<String> {
    let start = text.rfind("```json")?;
    let rest = &text[start + 7..];
    let end = rest.find("```")?;
    Some(rest[..end].to_string())
}

/// The key for a connection, read at call time and never cached or logged.
fn credential(connection: &Connection) -> Option<String> {
    let reference = connection.secret_ref.as_ref()?;
    crate::legacy::read_secret(crate::SECRET_SERVICE, reference)
}

fn endpoint(connection: &Connection, path: &str) -> String {
    format!(
        "{}/{}",
        connection.endpoint.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Sends one turn and returns what came back, adapted.
pub async fn complete(connection: &Connection, request: &ChatRequest) -> Result<ChatReply, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|error| error.to_string())?;
    let key = credential(connection);
    if key.is_none() && !matches!(connection.kind, Kind::Local | Kind::Cli) {
        return Err(format!("{} has no API key yet.", connection.name));
    }

    if connection.kind == Kind::Cli {
        return run_cli(connection, request).await;
    }

    if connection.kind == Kind::Anthropic {
        let response = anthropic_send(
            &client,
            connection,
            key.as_deref().unwrap_or_default(),
            request,
            false,
        )
        .await?;
        let text = response.text().await.map_err(|error| error.to_string())?;
        let value: Value =
            serde_json::from_str(&text).map_err(|error| format!("unreadable reply: {error}"))?;
        return Ok(anthropic_reply(&value));
    }

    let url = endpoint(connection, "chat/completions");
    let body = openai_body(request, &connection.model, connection.kind);
    let mut builder = client.post(&url);
    if let Some(key) = key.clone() {
        builder = builder.bearer_auth(key);
    }

    let response = builder
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("could not reach {url}: {error}"))?;
    let status = response.status();
    let text = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        // The body is where these APIs say what was actually wrong, and it is short.
        return Err(format!(
            "{status}: {}",
            text.chars().take(400).collect::<String>()
        ));
    }
    let value: Value =
        serde_json::from_str(&text).map_err(|error| format!("unreadable reply: {error}"))?;
    Ok(openai_reply(&value))
}

/// Where a CLI named by a bare command actually is.
///
/// "On the PATH" is not an answer for an app started from the Dock, whose PATH is the
/// system's four directories and none of the user's - and it is not an answer at all for
/// someone who has Claude Code or Codex only as an editor extension, which ships the same
/// CLI inside itself and puts it on no PATH. Both were the case on the machine this was
/// written on, and the connection said only "No such file or directory". So, in order:
/// the PATH, the directories these tools install themselves into, and the newest copy
/// inside an editor's extensions.
pub fn find_cli(
    command: &str,
    path: Option<&std::ffi::OsStr>,
    home: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    // A path is taken as given; only a bare name is looked for.
    if command.contains('/') {
        let given = PathBuf::from(command);
        return given.is_file().then_some(given);
    }

    let mut directories: Vec<PathBuf> = path
        .map(|path| std::env::split_paths(path).collect())
        .unwrap_or_default();
    if let Some(home) = home {
        for relative in [
            ".local/bin",
            ".claude/local",
            ".npm-global/bin",
            ".bun/bin",
            ".volta/bin",
            ".cargo/bin",
        ] {
            directories.push(home.join(relative));
        }
    }
    directories.push(PathBuf::from("/opt/homebrew/bin"));
    directories.push(PathBuf::from("/usr/local/bin"));
    if let Some(found) = directories
        .iter()
        .map(|directory| directory.join(command))
        .find(|candidate| candidate.is_file())
    {
        return Some(found);
    }

    // Inside an editor extension: which extension, and where in it the binary sits.
    let (prefix, inside): (&str, &[&str]) = match command {
        "claude" => ("anthropic.claude-code-", &["resources/native-binary"]),
        "codex" => ("openai.chatgpt-", &["bin"]),
        _ => return None,
    };
    let home = home?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut consider = |candidate: PathBuf| {
        if !candidate.is_file() {
            return;
        }
        let modified = std::fs::metadata(&candidate)
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if newest.as_ref().map_or(true, |(at, _)| modified > *at) {
            newest = Some((modified, candidate));
        }
    };
    let children = |directory: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(directory)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default()
    };
    for editor in [".vscode", ".vscode-insiders", ".cursor", ".windsurf"] {
        for extension in children(&home.join(editor).join("extensions")) {
            let named = extension
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix));
            if !named {
                continue;
            }
            for relative in inside {
                let base = extension.join(relative);
                consider(base.join(command));
                // Codex keeps one directory per platform under `bin`.
                for platform in children(&base) {
                    consider(platform.join(command));
                }
            }
        }
    }
    newest.map(|(_, found)| found)
}

/// The program a CLI connection runs, found - or why it could not be.
fn cli_program(connection: &Connection) -> Result<std::path::PathBuf, String> {
    let command = connection
        .command
        .clone()
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| format!("{} has no command set.", connection.name))?;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    find_cli(&command, std::env::var_os("PATH").as_deref(), home.as_deref()).ok_or_else(|| {
        format!(
            "`{command}` was not found - not on the PATH, not where it usually installs, and not inside an              editor extension. Install it and sign in, or give its full path in Settings."
        )
    })
}

/// Where a subscription connection stands - see `cli::Account`.
pub async fn cli_account(connection: &Connection) -> cli::Account {
    let command = connection.command.clone().unwrap_or_default();
    let flavour = cli::Flavour::of(std::path::Path::new(&command), &connection.args);
    let (install, status_args, login_args) = cli::guidance(flavour);
    let mut account = cli::Account {
        found: None,
        version: None,
        signed_in: None,
        account: None,
        install: install.to_string(),
        login: format!("{} {}", command, login_args.join(" "))
            .trim()
            .to_string(),
    };
    let Ok(program) = cli_program(connection) else {
        return account;
    };
    account.found = Some(program.display().to_string());
    let run = |args: &'static [&'static str]| {
        let program = program.clone();
        async move {
            tokio::time::timeout(
                Duration::from_secs(20),
                tokio::process::Command::new(&program)
                    .args(args)
                    .current_dir(std::env::temp_dir())
                    .stdin(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .ok()
            .and_then(Result::ok)
        }
    };
    if let Some(output) = run(&["--version"]).await {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        account.version = (!version.is_empty()).then_some(version);
    }
    if !status_args.is_empty() {
        if let Some(output) = run(status_args).await {
            let said = String::from_utf8_lossy(&output.stdout);
            let (signed_in, who) = cli::read_status(flavour, &said, output.status.success());
            account.signed_in = Some(signed_in);
            account.account = who;
        }
    }
    account
}

/// Starts the tool's own sign-in, which opens the browser, and waits for it to finish.
///
/// The tool's, not this app's: there is no login screen here and there must not be one.
/// The person signs in to Anthropic or OpenAI in their browser, with the tool they
/// installed, and this only starts that and then asks how it went.
pub async fn cli_login(connection: &Connection) -> Result<cli::Account, String> {
    let program = cli_program(connection)?;
    let flavour = cli::Flavour::of(&program, &connection.args);
    let (_, _, login_args) = cli::guidance(flavour);
    if login_args.is_empty() {
        return Err(format!(
            "{} has no sign-in this app knows how to start.",
            connection.name
        ));
    }
    let finished = tokio::time::timeout(
        Duration::from_secs(300),
        tokio::process::Command::new(&program)
            .args(login_args)
            .current_dir(std::env::temp_dir())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "the sign-in was not finished within five minutes".to_string())?
    .map_err(|error| format!("could not start the sign-in: {error}"))?;
    let account = cli_account(connection).await;
    if account.signed_in != Some(true) {
        let said = String::from_utf8_lossy(&finished.stderr);
        let said = if said.trim().is_empty() {
            String::from_utf8_lossy(&finished.stdout)
        } else {
            said
        };
        return Err(format!(
            "the sign-in did not complete{}",
            said.lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .map(|line| format!(": {}", line.trim()))
                .unwrap_or_default()
        ));
    }
    Ok(account)
}

/// Runs the user's own agent CLI with the prompt on stdin.
async fn run_cli(connection: &Connection, request: &ChatRequest) -> Result<ChatReply, String> {
    use tokio::io::AsyncWriteExt;
    let program = cli_program(connection)?;
    let command = program.display().to_string();
    let (system_args, stdin_text) = cli_handoff(
        cli::Flavour::of(&program, &connection.args),
        &connection.args,
        request,
    );
    let mut child = tokio::process::Command::new(&program)
        .args(&connection.args)
        .args(cli_choices(cli::Flavour::of(&program, &connection.args), connection))
        .args(&system_args)
        // Somewhere that is nobody's project. Run from wherever the app happened to
        // start, an agentic CLI picks up that directory's instructions and settings - in
        // development, this repository's - and answers as if it were working on it.
        .current_dir(std::env::temp_dir())
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            format!("could not run `{command}`: {error}. Install it, or give the full path in Settings.")
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        // As in `stream_cli`: a tool that quit without reading says why on its way out.
        if let Err(error) = stdin.write_all(stdin_text.as_bytes()).await {
            if error.kind() != std::io::ErrorKind::BrokenPipe {
                return Err(format!("could not send the prompt to `{command}`: {error}"));
            }
        }
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|error| format!("`{command}` failed: {error}"))?;
    if !output.status.success() {
        // Whichever stream it used to say why. Claude Code reports "Not logged in" on
        // stdout, so reading stderr alone showed an exit code and nothing after the colon.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let said = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        let name = program
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&command);
        return Err(format!(
            "`{name}` exited with {}: {}",
            output.status,
            if said.is_empty() {
                "it gave no reason".to_string()
            } else {
                said.chars().take(300).collect()
            }
        ));
    }
    Ok(cli_reply(
        &String::from_utf8_lossy(&output.stdout),
        &connection.model,
    ))
}

/// Runs the user's agent CLI and reports what it does as it does it.
///
/// The same run as `run_cli`, with the tool asked to narrate - see `cli` - and its output
/// read a line at a time, so the panel can say which model is answering, that it is
/// thinking, and show the answer as it is written rather than after.
async fn stream_cli(
    connection: &Connection,
    request: &ChatRequest,
    mut on_delta: impl FnMut(stream::Delta),
) -> Result<ChatReply, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let program = cli_program(connection)?;
    let name = program
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("the CLI")
        .to_string();
    let flavour = cli::Flavour::of(&program, &connection.args);
    let (system_args, stdin_text) = cli_handoff(flavour, &connection.args, request);
    let mut child = tokio::process::Command::new(&program)
        .args(&connection.args)
        .args(cli_choices(flavour, connection))
        .args(&system_args)
        .args(flavour.narrate(&connection.args))
        .current_dir(std::env::temp_dir())
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run `{name}`: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A tool that exits before reading its input - not signed in, a flag it does not
        // know - closes the pipe, and the reason it gives on its way out is worth more
        // than "broken pipe". So that one failure is let through to the reading below.
        if let Err(error) = stdin.write_all(stdin_text.as_bytes()).await {
            if error.kind() != std::io::ErrorKind::BrokenPipe {
                return Err(format!("could not send the prompt to `{name}`: {error}"));
            }
        }
        // Dropped here, which closes it: the tool waits for the end of its input.
    }

    let mut reader = cli::Reader::new(flavour);
    if flavour == cli::Flavour::Codex {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        for delta in reader.name_model(&cli::codex_model(home.as_deref()).unwrap_or_default()) {
            on_delta(delta);
        }
    }
    // Both streams, a line at a time and as they come: the answer is on one, and on the
    // other Codex says which model it is using and what the turn cost. Reading them
    // together also keeps a tool that fills its error pipe from being left blocked on it.
    let mut said = String::new();
    let mut complaints = String::new();
    //
    // Split on bytes and decoded a line at a time, not read with `lines()`: that fails on
    // the first line that is not valid UTF-8 - a tool echoing a file, a progress bar cut
    // mid-character - and the failure was taken for the end of the output, so the rest of
    // the answer was dropped and the pipe closed under a tool still writing to it.
    let mut answers = child
        .stdout
        .take()
        .map(|pipe| tokio::io::BufReader::new(pipe).split(b'\n'));
    let mut asides = child
        .stderr
        .take()
        .map(|pipe| tokio::io::BufReader::new(pipe).split(b'\n'));
    let text = |bytes: Vec<u8>| {
        let line = String::from_utf8_lossy(&bytes).into_owned();
        match line.strip_suffix('\r') {
            Some(line) => line.to_string(),
            None => line,
        }
    };
    while answers.is_some() || asides.is_some() {
        tokio::select! {
            line = async { answers.as_mut().unwrap().next_segment().await }, if answers.is_some() => match line.map(|line| line.map(text)) {
                Ok(Some(line)) => {
                    said.push_str(&line);
                    said.push('\n');
                    for delta in reader.line(&line) {
                        on_delta(delta);
                    }
                }
                _ => answers = None,
            },
            line = async { asides.as_mut().unwrap().next_segment().await }, if asides.is_some() => match line.map(|line| line.map(text)) {
                Ok(Some(line)) => {
                    complaints.push_str(&line);
                    complaints.push('\n');
                    for delta in reader.aside(&line) {
                        on_delta(delta);
                    }
                }
                _ => asides = None,
            },
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|error| format!("`{name}` failed: {error}"))?;

    let reply = reader.finish(&connection.model);
    if !status.success() {
        // What the tool itself said went wrong, if it said so in its narration; else
        // whichever stream it used - Claude Code reports "Not logged in" on stdout.
        if let Err(error) = &reply {
            return Err(format!("`{name}`: {error}"));
        }
        let reason = if complaints.trim().is_empty() {
            said.trim()
        } else {
            complaints.trim()
        };
        return Err(format!(
            "`{name}` exited with {status}: {}",
            if reason.is_empty() {
                "it gave no reason".to_string()
            } else {
                reason.chars().take(300).collect()
            }
        ));
    }
    let reply = reply.map_err(|error| format!("`{name}`: {error}"))?;
    for call in &reply.tool_calls {
        on_delta(stream::Delta::Tool {
            id: call.id.clone(),
            name: call.name.clone(),
        });
    }
    Ok(reply)
}

/// Sends one turn and reports the answer as it arrives.
///
/// The same request as `complete`, with `stream` set, read as server-sent events. A model
/// that cannot stream - a command-line agent, which has its own loop - falls back to the
/// whole answer at once, and the caller sees one delta at the end rather than none.
pub async fn complete_streaming(
    connection: &Connection,
    request: &ChatRequest,
    on_delta: impl FnMut(stream::Delta),
) -> Result<ChatReply, String> {
    if connection.kind == Kind::Cli {
        return stream_cli(connection, request, on_delta).await;
    }

    let key = credential(connection);
    if key.is_none() && connection.kind != Kind::Local {
        return Err(format!("{} has no API key yet.", connection.name));
    }
    stream_with_key(connection, key, request, on_delta).await
}

/// The streaming request itself, with the credential handed in rather than looked up -
/// which is what lets the whole exchange be run against a stand-in server in a test,
/// where there is no keychain to ask.
async fn stream_with_key(
    connection: &Connection,
    key: Option<String>,
    request: &ChatRequest,
    mut on_delta: impl FnMut(stream::Delta),
) -> Result<ChatReply, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(900))
        .build()
        .map_err(|error| error.to_string())?;

    let (mut response, wire) = if connection.kind == Kind::Anthropic {
        let response = anthropic_send(
            &client,
            connection,
            key.as_deref().unwrap_or_default(),
            request,
            true,
        )
        .await?;
        (response, stream::Wire::Anthropic)
    } else {
        let url = endpoint(connection, "chat/completions");
        let mut body = openai_body(request, &connection.model, connection.kind);
        // Usage arrives in a final frame only if it is asked for.
        body["stream_options"] = json!({ "include_usage": true });
        body["stream"] = json!(true);
        let mut builder = client.post(&url);
        if let Some(key) = key.clone() {
            builder = builder.bearer_auth(key);
        }
        let response = builder
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("could not reach {url}: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(format!(
                "{status}: {}",
                text.chars().take(400).collect::<String>()
            ));
        }
        (response, stream::Wire::Openai)
    };

    let mut reader = stream::Reader::new(wire);
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                for delta in reader.push(&chunk) {
                    on_delta(delta);
                }
                // Nothing after an error is part of an answer.
                if reader.error().is_some() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    reader.finish()
}

/// Asks a connection what models it has, which is also the cheapest test that it works.
pub async fn probe(connection: &Connection) -> Result<Vec<String>, String> {
    if connection.kind == Kind::Cli {
        // For a CLI, "is it installed and does it run" is the whole question.
        let program = cli_program(connection)?;
        let command = program.display().to_string();
        let output = tokio::process::Command::new(&program)
            .arg("--version")
            .output()
            .await
            .map_err(|error| {
                format!("could not run `{command}`: {error}. Install it, or give the full path.")
            })?;
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // The version, and where it was found - which matters when it was not the PATH.
        return Ok(vec![if version.is_empty() {
            command
        } else {
            format!("{version} - {command}")
        }]);
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let key = credential(connection);
    let url = endpoint(connection, "models");
    let mut builder = client.get(&url);
    builder = match connection.kind {
        Kind::Anthropic => builder
            .header("x-api-key", key.unwrap_or_default())
            .header("anthropic-version", "2023-06-01"),
        _ => {
            if let Some(key) = key {
                builder.bearer_auth(key)
            } else {
                builder
            }
        }
    };
    let response = builder
        .send()
        .await
        .map_err(|error| format!("could not reach {url}: {error}"))?;
    let status = response.status();
    let text = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "{status}: {}",
            text.chars().take(200).collect::<String>()
        ));
    }
    let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    // All three list models under `data`, with the id in `id`.
    Ok(value["data"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ChatRequest {
        ChatRequest {
            system: "be brief".into(),
            messages: vec![
                Message {
                    role: "user".into(),
                    content: "colour chain A red".into(),
                    tool_calls: vec![],
                    tool_call_id: None,
                    provider: None,
                },
                Message {
                    role: "assistant".into(),
                    content: "looking".into(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read_graph".into(),
                        arguments: json!({"depth": 1}),
                    }],
                    tool_call_id: None,
                    provider: None,
                },
                Message {
                    role: "tool".into(),
                    content: "{\"nodes\":[]}".into(),
                    tool_calls: vec![],
                    tool_call_id: Some("c1".into()),
                    provider: None,
                },
            ],
            tools: vec![ToolSpec {
                name: "read_graph".into(),
                description: "the node graph".into(),
                parameters: json!({"type": "object", "properties": {}}),
            }],
            max_tokens: Some(1024),
            temperature: None,
        }
    }

    #[test]
    fn anthropic_gets_blocks_and_a_tool_result_as_a_user_turn() {
        let body = anthropic_body(&request(), "claude-x");
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["max_tokens"], 1024);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[1]["content"][0]["type"], "text");
        assert_eq!(messages[1]["content"][1]["type"], "tool_use");
        assert_eq!(messages[1]["content"][1]["input"]["depth"], 1);
        // A result goes back as a user turn holding a tool_result block.
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "c1");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn openai_gets_a_system_message_and_stringified_arguments() {
        let body = openai_body(&request(), "gpt-x", Kind::Openai);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["name"],
            "read_graph"
        );
        // OpenAI wants the arguments as a string, Anthropic as an object.
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["arguments"],
            "{\"depth\":1}"
        );
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "c1");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn both_replies_read_as_the_same_thing() {
        let anthropic = anthropic_reply(&json!({
            "model": "claude-x",
            "stop_reason": "tool_use",
            "content": [
                {"type": "text", "text": "opening it"},
                {"type": "tool_use", "id": "t1", "name": "open_pdb", "input": {"id": "4hhb"}},
            ],
            "usage": {"input_tokens": 12, "output_tokens": 3},
        }));
        assert_eq!(anthropic.text, "opening it");
        assert_eq!(anthropic.stop_reason, "tool");
        assert_eq!(anthropic.tool_calls[0].arguments["id"], "4hhb");
        assert_eq!(anthropic.usage.input, 12);

        let openai = openai_reply(&json!({
            "model": "gpt-x",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "opening it",
                    "tool_calls": [{"id": "t1", "function": {"name": "open_pdb", "arguments": "{\"id\":\"4hhb\"}"}}],
                },
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 3},
        }));
        assert_eq!(openai.text, "opening it");
        assert_eq!(openai.stop_reason, "tool");
        assert_eq!(openai.tool_calls[0].name, "open_pdb");
        // The string of arguments becomes the same object the other adapter produced.
        assert_eq!(
            openai.tool_calls[0].arguments,
            anthropic.tool_calls[0].arguments
        );
        assert_eq!(openai.usage.output, 3);
    }

    #[test]
    fn a_reasoning_models_thought_is_kept_apart_from_its_answer() {
        // Inline, which is what llama.cpp does when the template leaves it in the content.
        let reply = openai_reply(&json!({
            "choices": [{"finish_reason": "stop", "message": {
                "content": "<think>The chain is A_1, so a slot is needed.</think>Chain A is magenta now.",
            }}],
        }));
        assert_eq!(reply.text, "Chain A is magenta now.");
        assert_eq!(reply.thinking, "The chain is A_1, so a slot is needed.");

        // And in its own field, which is what it does when the template allows.
        let separate = openai_reply(&json!({
            "choices": [{"finish_reason": "stop", "message": {
                "content": "Done.",
                "reasoning_content": "Read the graph first.",
            }}],
        }));
        assert_eq!(separate.text, "Done.");
        assert_eq!(separate.thinking, "Read the graph first.");

        // Cut off mid-thought: there is no answer yet, and pretending otherwise would put
        // half a thought on screen as if it were one.
        let cut = openai_reply(&json!({
            "choices": [{"finish_reason": "length", "message": {"content": "<think>Still working"}}],
        }));
        assert_eq!(cut.text, "");
        assert_eq!(cut.thinking, "Still working");
    }

    #[test]
    fn a_plain_answer_stops() {
        let reply = openai_reply(&json!({
            "choices": [{"finish_reason": "stop", "message": {"content": "done"}}],
        }));
        assert_eq!(reply.stop_reason, "end");
        assert!(reply.tool_calls.is_empty());
    }

    #[test]
    fn the_connections_are_seeded_ready_to_configure() {
        let settings = Settings::default();
        assert_eq!(settings.policy, Policy::Auto);
        let kinds: Vec<Kind> = settings.connections.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, vec![Kind::Local, Kind::Cli, Kind::Cli]);
        // Nothing here takes a key: a local model needs none, and the CLIs are already
        // signed in to the user's own subscription.
        assert!(settings
            .connections
            .iter()
            .all(|connection| connection.secret_ref.is_none()));
        assert_eq!(settings.connections[1].command.as_deref(), Some("claude"));
        assert_eq!(settings.connections[2].command.as_deref(), Some("codex"));
        // Claude Code is run with the web and nothing else of its own: no shell, no files.
        assert!(settings.connections[1]
            .args
            .windows(2)
            .any(|pair| pair[0] == "--tools" && pair[1] == "WebSearch,WebFetch"));
        // And a connection saved before that is brought up to it, unless it was edited.
        let mut saved = Settings::default();
        saved.connections[1].args = CLAUDE_CLI_ARGS_OFFLINE
            .iter()
            .map(|a| (*a).to_string())
            .collect();
        saved.connections[2].args = vec!["exec".into(), "--mine".into()];
        assert!(saved.keep_to_subscriptions());
        assert_eq!(saved.connections[1].args, CLAUDE_CLI_ARGS);
        assert_eq!(saved.connections[2].args, vec!["exec", "--mine"]);
    }

    #[test]
    fn a_connection_mini_runs_itself_remembers_the_model_not_the_port() {
        // The port belongs to a process, so it is not part of the settings at all: a
        // connection says which model, and the address is worked out when it is used.
        let json = r#"{"id":"local","name":"Local","kind":"local","endpoint":"http://127.0.0.1:62765/v1","model":"qwen2.5-0.5b-instruct-q4_k_m.gguf","secret_ref":null}"#;
        let connection: Connection = serde_json::from_str(json).unwrap();
        // Read from a version that did not have the field, it is simply absent.
        assert!(connection.engine_model.is_none());
        assert!(connection.command.is_none());

        let written = serde_json::to_string(&Connection {
            engine_model: Some("qwen2.5-7b-instruct-q4".into()),
            ..connection
        })
        .unwrap();
        assert!(written.contains("\"engine_model\":\"qwen2.5-7b-instruct-q4\""));
    }

    #[test]
    fn a_cli_is_given_the_conversation_and_a_json_convention() {
        let prompt = cli_prompt(&request());
        assert!(prompt.starts_with("be brief"));
        assert!(prompt.contains("- `read_graph` - the node graph"));
        assert!(prompt.contains("User: colour chain A red"));
        assert!(prompt.contains("You called `read_graph`"));
        assert!(prompt.contains("Tool result: {\"nodes\":[]}"));
    }

    #[test]
    fn a_cli_answer_is_read_as_prose_or_as_a_call() {
        let plain = cli_reply("Chain A is red now.", "claude");
        assert_eq!(plain.stop_reason, "end");
        assert_eq!(plain.text, "Chain A is red now.");

        // An agentic CLI narrates first; the last block is the one that counts.
        let calling = cli_reply(
            "Let me look.\n```json\n{\"tool\": \"set_style\", \"arguments\": {\"colour\": \"#ff00ff\"}}\n```",
            "claude",
        );
        assert_eq!(calling.stop_reason, "tool");
        assert_eq!(calling.tool_calls[0].name, "set_style");
        assert_eq!(calling.tool_calls[0].arguments["colour"], "#ff00ff");
        assert!(calling.text.starts_with("Let me look."));

        // A block that is not a tool call is left alone.
        let data = cli_reply("Here:\n```json\n{\"chains\": 4}\n```", "claude");
        assert_eq!(data.stop_reason, "end");
        assert!(data.tool_calls.is_empty());
    }

    fn turn(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            provider: None,
        }
    }

    #[test]
    fn a_claude_turn_goes_back_exactly_as_it_came() {
        let blocks = json!([
            { "type": "thinking", "thinking": "read first", "signature": "sig-1" },
            { "type": "text", "text": "Reading the graph." },
            { "type": "tool_use", "id": "t1", "name": "read_graph", "input": {} },
        ]);
        let mut assistant = turn("assistant", "Reading the graph.");
        assistant.tool_calls = vec![ToolCall {
            id: "t1".into(),
            name: "read_graph".into(),
            arguments: json!({}),
        }];
        assistant.provider = Some(anthropic_provider(blocks.clone()));
        let mut first = turn("tool", "{\"nodes\":3}");
        first.tool_call_id = Some("t1".into());

        let request = ChatRequest {
            system: "s".into(),
            messages: vec![turn("user", "what is here?"), assistant, first],
            tools: Vec::new(),
            max_tokens: Some(100),
            temperature: Some(0.2),
        };
        let body = anthropic_body(&request, "claude-opus-5-5");
        assert_eq!(
            body["messages"][1]["content"], blocks,
            "thinking and signature intact"
        );
        assert!(
            body.get("temperature").is_none(),
            "current models refuse a temperature"
        );
    }

    #[test]
    fn the_results_of_one_turn_share_one_message() {
        let mut assistant = turn("assistant", "");
        assistant.tool_calls = vec![
            ToolCall {
                id: "a".into(),
                name: "read_graph".into(),
                arguments: json!({}),
            },
            ToolCall {
                id: "b".into(),
                name: "list_molecules".into(),
                arguments: json!({}),
            },
        ];
        let mut one = turn("tool", "ok");
        one.tool_call_id = Some("a".into());
        let mut two = turn("tool", "error: no such molecule");
        two.tool_call_id = Some("b".into());
        let request = ChatRequest {
            system: "s".into(),
            messages: vec![
                turn("user", "go"),
                assistant,
                one,
                two,
                turn("user", "and now?"),
            ],
            tools: Vec::new(),
            max_tokens: None,
            temperature: None,
        };
        let body = anthropic_body(&request, "claude-opus-5-5");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "user, assistant, both results, user");
        let results = messages[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].get("is_error").is_none());
        assert_eq!(results[1]["is_error"], true);
    }

    #[test]
    fn thinking_can_be_taken_out_of_a_history_that_is_refused() {
        let mut body = json!({ "messages": [
            { "role": "user", "content": "hi" },
            { "role": "assistant", "content": [
                { "type": "thinking", "thinking": "", "signature": "x" },
                { "type": "text", "text": "hello" },
            ]},
            { "role": "assistant", "content": [{ "type": "redacted_thinking", "data": "x" }] },
        ]});
        assert!(strip_thinking(&mut body));
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages.len(),
            2,
            "a turn that was only thinking is gone, not left empty"
        );
        assert_eq!(
            messages[1]["content"],
            json!([{ "type": "text", "text": "hello" }])
        );
        assert!(!strip_thinking(&mut body));
    }

    #[test]
    fn the_extras_go_only_where_they_are_known_to_be_taken() {
        let mut connection = claude_at("https://api.anthropic.com/v1");
        let mut body = json!({});
        assert_eq!(
            anthropic_extras(&connection, &mut body),
            Some("server-side-fallback-2026-07-01")
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["fallbacks"], "default");

        // Through a gateway: the reasoning summary still, the fallback not.
        connection.endpoint = "https://gateway.example/anthropic/v1".into();
        let mut body = json!({});
        assert_eq!(anthropic_extras(&connection, &mut body), None);
        assert!(body.get("fallbacks").is_none());

        // A model from before either existed gets neither.
        connection.model = "claude-3-haiku-20240307".into();
        let mut body = json!({});
        assert_eq!(anthropic_extras(&connection, &mut body), None);
        assert_eq!(body, json!({}));
    }

    #[test]
    fn each_dialect_gets_its_own_name_for_the_token_limit() {
        let request = ChatRequest {
            system: "s".into(),
            messages: vec![turn("user", "hi")],
            tools: Vec::new(),
            max_tokens: Some(512),
            temperature: Some(0.3),
        };
        let hosted = openai_body(&request, "gpt-6-luna", Kind::Openai);
        assert_eq!(hosted["max_completion_tokens"], 512);
        assert!(hosted.get("max_tokens").is_none() && hosted.get("temperature").is_none());
        let local = openai_body(&request, "qwen", Kind::Local);
        assert_eq!(local["max_tokens"], 512);
        assert!(local.get("max_completion_tokens").is_none());
    }

    #[test]
    fn a_saved_list_is_brought_down_to_the_subscriptions() {
        let api = |id: &str, kind: Kind| Connection {
            id: id.into(),
            name: id.into(),
            kind,
            endpoint: "https://api.example/v1".into(),
            model: "m".into(),
            secret_ref: Some(format!("agents/{id}")),
            command: None,
            args: Vec::new(),
            engine_model: None,
            choice: None,
            effort: None,
        };
        let cli = |id: &str, name: &str, command: &str, args: &[&str]| Connection {
            id: id.into(),
            name: name.into(),
            kind: Kind::Cli,
            endpoint: String::new(),
            model: command.into(),
            secret_ref: None,
            command: Some(command.into()),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            engine_model: None,
            choice: None,
            effort: None,
        };
        let mut settings = Settings {
            connections: vec![
                api("claude", Kind::Anthropic),
                api("chatgpt", Kind::Openai),
                cli("claude-cli", "Claude (subscription)", "claude", &["-p"]),
                cli("codex-cli", "My Codex", "codex", &["exec", "--model", "x"]),
            ],
            policy: Policy::default(),
            active: Some("claude".into()),
        };
        assert!(settings.keep_to_subscriptions());
        let ids: Vec<&str> = settings.connections.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["claude-cli", "codex-cli"]);
        assert_eq!(
            settings.active, None,
            "the open connection was one that is gone"
        );
        assert_eq!(settings.connections[0].name, "Claude");
        assert_eq!(settings.connections[0].args, CLAUDE_CLI_ARGS);
        // Arguments and a name somebody changed are theirs.
        assert_eq!(settings.connections[1].name, "My Codex");
        assert_eq!(settings.connections[1].args, vec!["exec", "--model", "x"]);
        assert!(!settings.keep_to_subscriptions());

        // And what is seeded today is already in that shape.
        let mut fresh = Settings::default();
        assert!(!fresh.keep_to_subscriptions());
        assert!(fresh
            .connections
            .iter()
            .all(|c| matches!(c.kind, Kind::Local | Kind::Cli)));
    }

    #[test]
    fn a_cli_is_found_where_it_actually_lives() {
        let root = std::env::temp_dir().join(format!("geis-cli-{}", crate::util::new_id()));
        let touch = |relative: &str| {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            path
        };
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let nowhere = std::ffi::OsString::from(root.join("empty").as_os_str());

        // Nothing anywhere.
        assert_eq!(
            find_cli("claude", Some(nowhere.as_os_str()), Some(&home)),
            None
        );

        // Only inside an editor extension - two versions of it; the newer wins.
        let old = touch("home/.vscode/extensions/anthropic.claude-code-2.1.1-darwin-arm64/resources/native-binary/claude");
        let new = touch("home/.vscode/extensions/anthropic.claude-code-2.1.9-darwin-arm64/resources/native-binary/claude");
        let earlier = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        assert_eq!(
            find_cli("claude", Some(nowhere.as_os_str()), Some(&home)),
            Some(new)
        );
        let codex = touch(
            "home/.cursor/extensions/openai.chatgpt-26.1-darwin-arm64/bin/macos-aarch64/codex",
        );
        assert_eq!(
            find_cli("codex", Some(nowhere.as_os_str()), Some(&home)),
            Some(codex)
        );

        // Installed properly as well: the PATH comes first, then the usual directories.
        let local = touch("home/.local/bin/claude");
        assert_eq!(
            find_cli("claude", Some(nowhere.as_os_str()), Some(&home)),
            Some(local)
        );
        let on_path = touch("bin/claude");
        let path = std::ffi::OsString::from(root.join("bin").as_os_str());
        assert_eq!(
            find_cli("claude", Some(path.as_os_str()), Some(&home)),
            Some(on_path.clone())
        );

        // A full path is taken at its word, and only if it is there.
        assert_eq!(
            find_cli(on_path.to_str().unwrap(), None, None),
            Some(on_path)
        );
        assert_eq!(find_cli("/no/such/claude", None, None), None);
        assert_eq!(
            find_cli("some-other-tool", Some(nowhere.as_os_str()), Some(&home)),
            None
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// A one-connection-at-a-time HTTP server that answers each request with the next
    /// canned reply and records what it was sent.
    async fn canned(
        replies: Vec<(u16, &'static str)>,
    ) -> (String, tokio::task::JoinHandle<Vec<(String, Value)>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            for (status, reply) in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut chunk = [0u8; 4096];
                let (head, body) = loop {
                    let read = socket.read(&mut chunk).await.unwrap();
                    raw.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&raw).into_owned();
                    if let Some(at) = text.find("\r\n\r\n") {
                        let head = text[..at].to_string();
                        let wanted: usize = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        if raw.len() >= at + 4 + wanted {
                            break (head, raw[at + 4..at + 4 + wanted].to_vec());
                        }
                    }
                    if read == 0 {
                        panic!("the request ended early");
                    }
                };
                seen.push((
                    head.to_ascii_lowercase(),
                    serde_json::from_slice(&body).unwrap(),
                ));
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.ok();
            }
            seen
        });
        (address, handle)
    }

    fn claude_at(endpoint: &str) -> Connection {
        Connection {
            id: "claude".into(),
            name: "Claude".into(),
            kind: Kind::Anthropic,
            endpoint: endpoint.into(),
            model: DEFAULT_CLAUDE.into(),
            secret_ref: None,
            command: None,
            args: Vec::new(),
            engine_model: None,
            choice: None,
            effort: None,
        }
    }

    const OK: &str = r#"{"model":"claude-opus-5-5","stop_reason":"end_turn","content":[{"type":"text","text":"done"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;

    #[tokio::test]
    async fn a_history_refused_for_its_thinking_is_sent_again_without_it() {
        let refused = r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages.1.content.0: Invalid `signature` in `thinking` block. The block is bound to a different conversation."}}"#;
        let (address, server) = canned(vec![(400, refused), (200, OK)]).await;
        let mut assistant = turn("assistant", "hello");
        assistant.provider = Some(anthropic_provider(json!([
            { "type": "thinking", "thinking": "", "signature": "old" },
            { "type": "text", "text": "hello" },
        ])));
        let request = ChatRequest {
            system: "s".into(),
            messages: vec![turn("user", "hi"), assistant, turn("user", "again")],
            tools: Vec::new(),
            max_tokens: Some(64),
            temperature: None,
        };
        let client = reqwest::Client::new();
        let response = anthropic_send(&client, &claude_at(&address), "k", &request, false)
            .await
            .unwrap();
        assert_eq!(
            anthropic_reply(&response.json::<Value>().await.unwrap()).text,
            "done"
        );

        let seen = server.await.unwrap();
        assert_eq!(seen[0].1["messages"][1]["content"][0]["type"], "thinking");
        assert_eq!(
            seen[1].1["messages"][1]["content"],
            json!([{ "type": "text", "text": "hello" }])
        );
        assert!(
            seen[0].0.contains("x-api-key: k")
                && seen[0].0.contains("anthropic-version: 2023-06-01")
        );
    }

    #[tokio::test]
    async fn an_endpoint_that_refuses_the_optional_parts_is_asked_again_without_them() {
        let refused = r#"{"type":"error","error":{"type":"invalid_request_error","message":"thinking.display: Extra inputs are not permitted"}}"#;
        let (address, server) = canned(vec![(400, refused), (200, OK)]).await;
        let request = ChatRequest {
            system: "s".into(),
            messages: vec![turn("user", "hi")],
            tools: Vec::new(),
            max_tokens: Some(64),
            temperature: None,
        };
        let client = reqwest::Client::new();
        anthropic_send(&client, &claude_at(&address), "k", &request, false)
            .await
            .unwrap();
        let seen = server.await.unwrap();
        assert_eq!(seen[0].1["thinking"]["display"], "summarized");
        assert!(seen[1].1.get("thinking").is_none());
        // And a refusal that is about neither is simply reported.
        let (address, server) =
            canned(vec![(401, r#"{"error":{"message":"invalid x-api-key"}}"#)]).await;
        let mut plain = claude_at(&address);
        plain.model = "claude-3-haiku-20240307".into();
        let error = anthropic_send(&client, &plain, "bad", &request, false)
            .await
            .unwrap_err();
        assert!(
            error.contains("401") && error.contains("invalid x-api-key"),
            "{error}"
        );
        server.await.unwrap();
    }

    /// Claude's side of a tool round, as server-sent events: think, say a word, ask for a
    /// tool - and then, once it has the result, answer.
    const ASKS: &str = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":20}}}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"체인 A를 찾는다\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig-777\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"colour_chain\",\"input\":{}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"chain\\\":\\\"A\\\",\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"colour\\\":\\\"#ff0000\\\"}\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":30}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    const ANSWERS: &str = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":60}}}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"체인 A를 빨간색으로 칠했습니다.\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":12}}\n\n",
    );

    /// The whole of a tool round over the wire: what the loop in the UI does, driven from
    /// here, against a server that answers as the API does. What is being checked is the
    /// second request - that the turn Claude wrote goes back with its thinking block and
    /// signature as they came, and the tool's result in the shape the API takes.
    #[tokio::test]
    async fn a_streamed_tool_round_with_claude_holds_together() {
        let (address, server) = canned(vec![(200, ASKS), (200, ANSWERS)]).await;
        let connection = claude_at(&address);
        let tools = vec![ToolSpec {
            name: "colour_chain".into(),
            description: "Colours one chain.".into(),
            parameters: json!({ "type": "object", "properties": { "chain": { "type": "string" }, "colour": { "type": "string" } } }),
        }];
        let mut history = vec![turn("user", "체인 A를 빨간색으로")];
        let ask = |messages: Vec<Message>| ChatRequest {
            system: "be brief".into(),
            messages,
            tools: tools.clone(),
            max_tokens: Some(16000),
            temperature: None,
        };

        let mut seen = Vec::new();
        let first = stream_with_key(
            &connection,
            Some("k".into()),
            &ask(history.clone()),
            |delta| seen.push(delta),
        )
        .await
        .unwrap();
        assert_eq!(first.stop_reason, "tool");
        assert_eq!(first.thinking, "체인 A를 찾는다");
        assert_eq!(
            first.tool_calls[0].arguments,
            json!({ "chain": "A", "colour": "#ff0000" })
        );
        assert!(seen.iter().any(
            |delta| matches!(delta, stream::Delta::Tool { name, .. } if name == "colour_chain")
        ));

        // What the UI's loop does with a reply: keep it as an assistant turn, run the
        // tool, and append the result.
        history.push(Message {
            role: "assistant".into(),
            content: first.text.clone(),
            tool_calls: first.tool_calls.clone(),
            tool_call_id: None,
            provider: first.provider.clone(),
        });
        let mut result = turn("tool", "{\"chain\":\"A_1\",\"colour\":\"#ff0000\"}");
        result.tool_call_id = Some("toolu_1".into());
        history.push(result);

        let second = stream_with_key(&connection, Some("k".into()), &ask(history), |_| {})
            .await
            .unwrap();
        assert_eq!(second.stop_reason, "end");
        assert_eq!(second.text, "체인 A를 빨간색으로 칠했습니다.");
        assert_eq!((second.usage.input, second.usage.output), (60, 12));

        let requests = server.await.unwrap();
        let (head, body) = &requests[1];
        assert!(head.contains("post /v1/messages"));
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["max_tokens"], 16000);
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["tools"][0]["name"], "colour_chain");
        assert!(body["tools"][0]["input_schema"].is_object());
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1],
            json!({ "role": "assistant", "content": [
                { "type": "thinking", "thinking": "체인 A를 찾는다", "signature": "sig-777" },
                { "type": "tool_use", "id": "toolu_1", "name": "colour_chain", "input": { "chain": "A", "colour": "#ff0000" } },
            ]})
        );
        assert_eq!(
            messages[2],
            json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "content": "{\"chain\":\"A_1\",\"colour\":\"#ff0000\"}" },
            ]})
        );
    }

    /// The same round with ChatGPT's wire format, for the same reason.
    #[tokio::test]
    async fn a_streamed_tool_round_with_chatgpt_holds_together() {
        const ASKS: &str = concat!(
            "data: {\"model\":\"gpt-6-luna\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":null,\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"colour_chain\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"chain\\\":\\\"A\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":15,\"completion_tokens\":9}}\n\n",
            "data: [DONE]\n\n",
        );
        const ANSWERS: &str = concat!(
            "data: {\"model\":\"gpt-6-luna\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Done.\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (address, server) = canned(vec![(200, ASKS), (200, ANSWERS)]).await;
        let mut connection = claude_at(&address);
        connection.kind = Kind::Openai;
        connection.model = DEFAULT_GPT.into();
        let ask = |messages: Vec<Message>| ChatRequest {
            system: "be brief".into(),
            messages,
            tools: vec![ToolSpec {
                name: "colour_chain".into(),
                description: "d".into(),
                parameters: json!({ "type": "object" }),
            }],
            max_tokens: Some(16000),
            temperature: None,
        };
        let mut history = vec![turn("user", "colour chain A")];
        let first = stream_with_key(
            &connection,
            Some("sk".into()),
            &ask(history.clone()),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(first.stop_reason, "tool");
        assert_eq!(first.tool_calls[0].id, "call_1");
        assert_eq!(first.tool_calls[0].arguments, json!({ "chain": "A" }));
        assert_eq!((first.usage.input, first.usage.output), (15, 9));

        history.push(Message {
            role: "assistant".into(),
            content: first.text.clone(),
            tool_calls: first.tool_calls.clone(),
            tool_call_id: None,
            provider: first.provider.clone(),
        });
        let mut result = turn("tool", "ok");
        result.tool_call_id = Some("call_1".into());
        history.push(result);
        let second = stream_with_key(&connection, Some("sk".into()), &ask(history), |_| {})
            .await
            .unwrap();
        assert_eq!(second.text, "Done.");

        let requests = server.await.unwrap();
        let (head, body) = &requests[1];
        assert!(
            head.contains("post /v1/chat/completions") && head.contains("authorization: bearer sk")
        );
        assert_eq!(body["max_completion_tokens"], 16000);
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["stream_options"]["include_usage"], true);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["arguments"],
            "{\"chain\":\"A\"}"
        );
        assert_eq!(
            messages[3],
            json!({ "role": "tool", "tool_call_id": "call_1", "content": "ok" })
        );
    }

    /// A stand-in for a CLI: a shell script in a scratch directory, run by its full path.
    /// Nothing real is started, and nothing is looked up on the PATH.
    fn fake_cli(body: &str) -> (std::path::PathBuf, Connection) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("geis-fake-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-tool");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let connection = Connection {
            id: "fake".into(),
            name: "Fake".into(),
            kind: Kind::Cli,
            endpoint: String::new(),
            model: "fake".into(),
            secret_ref: None,
            command: Some(script.to_string_lossy().to_string()),
            args: Vec::new(),
            engine_model: None,
            choice: None,
            effort: None,
        };
        (dir, connection)
    }

    fn plain_request(prompt: &str) -> ChatRequest {
        ChatRequest {
            system: String::new(),
            messages: vec![turn("user", prompt)],
            tools: Vec::new(),
            max_tokens: None,
            temperature: None,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_line_that_is_not_utf8_does_not_end_the_answer() {
        let (dir, connection) = fake_cli("cat > /dev/null\nprintf 'first \\377 line\\r\\n'\nprintf 'second line\\n'\nprintf 'no newline'");
        let reply = complete_streaming(&connection, &plain_request("hi"), |_| {})
            .await
            .unwrap();
        assert_eq!(reply.text, "first \u{fffd} line\nsecond line\nno newline");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_tool_that_quits_without_reading_says_why() {
        // It exits before reading a prompt far larger than a pipe holds, so the write
        // fails with a broken pipe - and what the tool said is still what is reported.
        let (dir, connection) = fake_cli("echo 'Not logged in - Please run /login' >&2\nexit 1");
        let prompt = "x".repeat(1 << 20);
        let error = complete_streaming(&connection, &plain_request(&prompt), |_| {})
            .await
            .unwrap_err();
        assert!(error.contains("Not logged in"), "{error}");
        let error = complete(&connection, &plain_request(&prompt))
            .await
            .unwrap_err();
        assert!(error.contains("Not logged in"), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_system_prompt_reaches_each_cli_the_way_it_takes_one() {
        let request = ChatRequest {
            system: "You are the assistant inside Geis.".into(),
            messages: vec![turn("user", "make chain A red")],
            tools: vec![ToolSpec {
                name: "colour_chain".into(),
                description: "Colours a chain.".into(),
                parameters: json!({}),
            }],
            max_tokens: None,
            temperature: None,
        };
        let args = |list: &[&str]| {
            list.iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>()
        };

        // Claude Code: as its system prompt, in place of the coding agent's.
        let (extra, stdin) = cli_handoff(cli::Flavour::Claude, &args(&["-p"]), &request);
        assert_eq!(extra[0], "--system-prompt");
        assert!(extra[1].starts_with("You are the assistant inside Geis."));
        assert!(
            extra[1].contains("colour_chain"),
            "the tools are part of it"
        );
        assert!(stdin.starts_with("# Conversation") && stdin.contains("User: make chain A red"));
        assert!(
            !stdin.contains("You are the assistant"),
            "said once, not twice"
        );

        // Codex, which has no such argument: at the head of what it reads.
        let (extra, stdin) = cli_handoff(cli::Flavour::Codex, &args(&["exec"]), &request);
        assert!(extra.is_empty());
        assert!(
            stdin.starts_with("You are the assistant inside Geis.")
                && stdin.contains("User: make chain A red")
        );

        // A connection that sets its own is left to it.
        let (extra, _) = cli_handoff(
            cli::Flavour::Claude,
            &args(&["-p", "--system-prompt", "x"]),
            &request,
        );
        assert!(extra.is_empty());
    }
}
