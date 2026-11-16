//! Watching a subscription CLI work.
//!
//! Run plainly, Claude Code and Codex print an answer when they have one and nothing
//! before it - so a turn through a subscription was a silence and then text, with no word
//! on which model was answering or whether it was thinking at all. Both can be asked to
//! narrate instead: Claude Code with `--output-format stream-json`, which relays the API's
//! own events (the model's name, each piece of text, how much it has reasoned so far), and
//! Codex with `--json`, which reports each finished item and what the turn cost.
//!
//! This reads either narration into the same pieces a streamed API reply is made of. Like
//! the reader in `stream`, it is a state machine over lines with no process attached, so it
//! is tested against output recorded from the real tools.

use serde_json::Value;

use super::stream::Delta;
use super::{ChatReply, Usage};

/// Which tool's narration is being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    Claude,
    Codex,
    /// Something else, or an older version: whatever it prints is the answer.
    Plain,
}

impl Flavour {
    /// Decided by what is being run and how: the program's name, or the arguments that
    /// only one of them takes.
    pub fn of(program: &std::path::Path, args: &[String]) -> Self {
        let name = program
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if name.starts_with("claude") || args.iter().any(|arg| arg == "-p" || arg == "--print") {
            Flavour::Claude
        } else if name.starts_with("codex") || args.first().map(String::as_str) == Some("exec") {
            Flavour::Codex
        } else {
            Flavour::Plain
        }
    }

    /// What to add to the arguments so the tool narrates. Nothing if the connection
    /// already chose an output format of its own.
    pub fn narrate(self, args: &[String]) -> Vec<String> {
        let has = |flag: &str| {
            args.iter()
                .any(|arg| arg == flag || arg.starts_with(&format!("{flag}=")))
        };
        // Codex is left as it is. Its `--json` events never say which model answered,
        // and the plain mode does - in a header on stderr - along with what the turn
        // cost; `aside` reads those. Its events are still understood, for a connection
        // that asks for them itself.
        let flags: &[&str] = match self {
            Flavour::Claude if !has("--output-format") => &[
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
            ],
            _ => &[],
        };
        flags.iter().map(|flag| (*flag).to_string()).collect()
    }
}

/// Reads a CLI's output, line by line, into deltas and in the end a reply.
#[derive(Debug)]
pub struct Reader {
    flavour: Flavour,
    /// The answer so far, tool block and all.
    text: String,
    /// How much of `text` has been shown.
    shown: usize,
    thinking: String,
    model: String,
    usage: Usage,
    error: Option<String>,
    /// The answer as the tool states it at the end, which outranks the pieces.
    stated: Option<String>,
    /// Whether any line was narration. If none was, the output was plain text all along.
    narrated: bool,
    plain: String,
    /// Set by Codex's "tokens used" line: the count is on the line after it.
    tokens_next: bool,
}

impl Reader {
    pub fn new(flavour: Flavour) -> Self {
        Self {
            flavour,
            text: String::new(),
            shown: 0,
            thinking: String::new(),
            model: String::new(),
            usage: Usage::default(),
            error: None,
            stated: None,
            narrated: false,
            plain: String::new(),
            tokens_next: false,
        }
    }

    /// Says which model is answering, when that is known from somewhere other than the
    /// output - Codex names it in its configuration, not in its events.
    pub fn name_model(&mut self, model: &str) -> Vec<Delta> {
        if model.is_empty() || !self.model.is_empty() {
            return Vec::new();
        }
        self.model = model.to_string();
        vec![Delta::Model {
            model: model.to_string(),
        }]
    }

    /// A line from the tool's other stream, where Codex says what it is doing:
    ///
    /// ```text
    /// model: gpt-6.1-sol
    /// ...
    /// tokens used
    /// 2,971
    /// ```
    pub fn aside(&mut self, line: &str) -> Vec<Delta> {
        if self.flavour != Flavour::Codex {
            return Vec::new();
        }
        let line = line.trim();
        if self.tokens_next {
            self.tokens_next = false;
            let digits: String = line.chars().filter(char::is_ascii_digit).collect();
            if let Ok(total) = digits.parse::<u32>() {
                // One figure for the whole turn; filed as output so it is counted once.
                if self.usage.input == 0 && self.usage.output == 0 {
                    self.usage.output = total;
                }
            }
            return Vec::new();
        }
        if line == "tokens used" {
            self.tokens_next = true;
            return Vec::new();
        }
        if let Some(model) = line.strip_prefix("model:") {
            return self.name_model(model.trim());
        }
        // The header is over once the tool starts on the answer.
        if line == "codex" && self.model.is_empty() {
            return vec![Delta::Status {
                text: "working".into(),
            }];
        }
        Vec::new()
    }

    pub fn line(&mut self, line: &str) -> Vec<Delta> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let event = match self.flavour {
            Flavour::Plain => None,
            _ => serde_json::from_str::<Value>(trimmed)
                .ok()
                .filter(|value| value["type"].is_string()),
        };
        let Some(event) = event else {
            // Not narration: kept, in case none of it turns out to be.
            self.plain.push_str(line);
            self.plain.push('\n');
            return Vec::new();
        };
        self.narrated = true;
        match self.flavour {
            Flavour::Claude => self.claude(&event),
            Flavour::Codex => self.codex(&event),
            Flavour::Plain => Vec::new(),
        }
    }

    fn claude(&mut self, event: &Value) -> Vec<Delta> {
        match event["type"].as_str() {
            Some("system") if event["subtype"] == "init" => {
                self.name_model(event["model"].as_str().unwrap_or_default())
            }
            Some("stream_event") => {
                let inner = &event["event"];
                match inner["type"].as_str() {
                    Some("message_start") => {
                        self.name_model(inner["message"]["model"].as_str().unwrap_or_default())
                    }
                    Some("content_block_start") if inner["content_block"]["type"] == "thinking" => {
                        vec![Delta::Status {
                            text: "thinking".into(),
                        }]
                    }
                    Some("content_block_delta") => {
                        let delta = &inner["delta"];
                        match delta["type"].as_str() {
                            Some("text_delta") => {
                                self.text
                                    .push_str(delta["text"].as_str().unwrap_or_default());
                                self.show()
                            }
                            Some("thinking_delta") => {
                                // The reasoning itself where the model gives it; where it
                                // does not - the current models return it empty - how
                                // much there has been, which is still a sign of life.
                                let text = delta["thinking"].as_str().unwrap_or_default();
                                if !text.is_empty() {
                                    self.thinking.push_str(text);
                                    vec![Delta::Thinking {
                                        text: text.to_string(),
                                    }]
                                } else if let Some(tokens) = delta["estimated_tokens"].as_u64() {
                                    vec![Delta::Status {
                                        text: format!("thinking - about {tokens} tokens so far"),
                                    }]
                                } else {
                                    Vec::new()
                                }
                            }
                            _ => Vec::new(),
                        }
                    }
                    _ => Vec::new(),
                }
            }
            Some("result") => {
                let usage = &event["usage"];
                let count = |name: &str| usage[name].as_u64().unwrap_or(0);
                self.usage = Usage {
                    input: (count("input_tokens")
                        + count("cache_creation_input_tokens")
                        + count("cache_read_input_tokens")) as u32,
                    output: count("output_tokens") as u32,
                };
                let said = event["result"].as_str().unwrap_or_default().to_string();
                if event["is_error"].as_bool().unwrap_or(false) {
                    self.error = Some(if said.is_empty() {
                        "the CLI reported an error".into()
                    } else {
                        said
                    });
                    return Vec::new();
                }
                self.stated = Some(said);
                self.catch_up()
            }
            _ => Vec::new(),
        }
    }

    fn codex(&mut self, event: &Value) -> Vec<Delta> {
        match event["type"].as_str() {
            Some("item.completed") => {
                let item = &event["item"];
                let text = item["text"].as_str().unwrap_or_default();
                match item["type"].as_str() {
                    Some("reasoning") if !text.is_empty() => {
                        if !self.thinking.is_empty() {
                            self.thinking.push('\n');
                        }
                        self.thinking.push_str(text);
                        vec![Delta::Thinking {
                            text: format!("{text}\n"),
                        }]
                    }
                    Some("agent_message") => {
                        // Each message replaces the last: the final one is the answer.
                        self.text = text.to_string();
                        self.shown = 0;
                        self.show()
                    }
                    _ => Vec::new(),
                }
            }
            Some("turn.started") => vec![Delta::Status {
                text: "working".into(),
            }],
            Some("turn.completed") => {
                let usage = &event["usage"];
                self.usage = Usage {
                    input: usage["input_tokens"].as_u64().unwrap_or(0) as u32,
                    output: usage["output_tokens"].as_u64().unwrap_or(0) as u32,
                };
                Vec::new()
            }
            Some("turn.failed") | Some("error") => {
                let message = event["error"]["message"]
                    .as_str()
                    .or_else(|| event["message"].as_str())
                    .unwrap_or("the CLI reported an error");
                self.error = Some(message.to_string());
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Shows whatever of the answer is new and is the answer.
    ///
    /// A tool is asked for in a fenced block at the end of the reply, and that block is the
    /// request, not something to read: it becomes a card. So text is shown only up to the
    /// first fence - and a trailing backtick is held back, since it may be one opening.
    fn show(&mut self) -> Vec<Delta> {
        let limit = match self.text.find("```") {
            Some(fence) => fence,
            None => self.text.trim_end_matches('`').len(),
        };
        if limit <= self.shown {
            return Vec::new();
        }
        let piece = self.text[self.shown..limit].to_string();
        self.shown = limit;
        vec![Delta::Text { text: piece }]
    }

    /// At the end: anything the stated answer has that the pieces did not show. A tool
    /// that does not stream its text states it here and nowhere else.
    fn catch_up(&mut self) -> Vec<Delta> {
        let Some(stated) = self.stated.clone() else {
            return Vec::new();
        };
        if self.text.is_empty() {
            self.text = stated;
            self.shown = 0;
            return self.show();
        }
        Vec::new()
    }

    /// The reply, once the tool has exited. `fallback_model` is what the connection calls
    /// its model, for a tool that never said which one answered.
    pub fn finish(self, fallback_model: &str) -> Result<ChatReply, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let output = if self.narrated {
            self.stated
                .filter(|stated| !stated.is_empty())
                .unwrap_or(self.text)
        } else {
            self.plain
        };
        let model = if self.model.is_empty() {
            fallback_model
        } else {
            &self.model
        };
        let mut reply = super::cli_reply(&output, model);
        reply.usage = self.usage;
        if reply.thinking.is_empty() {
            reply.thinking = self.thinking.trim().to_string();
        }
        Ok(reply)
    }
}

/// Where a subscription connection stands: is the tool here, and is it signed in.
///
/// The two questions a person has before the first message, and the two things that can
/// be wrong. An app that is handed to someone else cannot assume either - on the machine
/// it was written on, both tools were installed and signed in, and nothing in the app
/// said that this was what "connected" meant or what to do when it was not.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Account {
    /// Where the tool was found, or nothing if it is not installed.
    pub found: Option<String>,
    pub version: Option<String>,
    /// Whether it is signed in. `None` when it could not be asked.
    pub signed_in: Option<bool>,
    /// What it is signed in as: `Claude subscription (team)`, `ChatGPT`.
    pub account: Option<String>,
    /// The command that installs it, for someone who does not have it.
    pub install: String,
    /// The command that signs it in, as it would be typed in a terminal.
    pub login: String,
}

/// How to install each tool, and how to sign it in.
pub fn guidance(
    flavour: Flavour,
) -> (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
) {
    match flavour {
        Flavour::Claude => (
            "npm install -g @anthropic-ai/claude-code",
            &["auth", "status"],
            &["auth", "login"],
        ),
        Flavour::Codex => (
            "npm install -g @openai/codex",
            &["login", "status"],
            &["login"],
        ),
        Flavour::Plain => ("", &[], &[]),
    }
}

/// Reads what a tool's status command said.
///
/// Claude Code answers in JSON - `{"loggedIn": true, "authMethod": "claude.ai",
/// "subscriptionType": "team"}` - and Codex in a sentence, "Logged in using ChatGPT". Both
/// also say it with their exit code, which is what is trusted when the words are not
/// recognised. Nothing that identifies the person is kept: not the address, not the
/// organisation.
pub fn read_status(flavour: Flavour, output: &str, succeeded: bool) -> (bool, Option<String>) {
    match flavour {
        Flavour::Claude => match serde_json::from_str::<Value>(output.trim()) {
            Ok(value) => {
                let signed_in = value["loggedIn"].as_bool().unwrap_or(succeeded);
                let account = signed_in.then(|| {
                    let method = match value["authMethod"].as_str() {
                        Some("claude.ai") => "Claude subscription",
                        Some("console") | Some("api_key") => "API billing",
                        _ => "signed in",
                    };
                    match value["subscriptionType"]
                        .as_str()
                        .filter(|kind| !kind.is_empty())
                    {
                        Some(kind) => format!("{method} ({kind})"),
                        None => method.to_string(),
                    }
                });
                (signed_in, account)
            }
            Err(_) => (succeeded, None),
        },
        Flavour::Codex => {
            let line = output
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or_default()
                .trim();
            let signed_in = succeeded && !line.to_ascii_lowercase().starts_with("not logged in");
            let account = signed_in
                .then(|| line.strip_prefix("Logged in using ").map(str::to_string))
                .flatten();
            (signed_in, account)
        }
        Flavour::Plain => (succeeded, None),
    }
}

/// The model Codex is set to use, from its own configuration.
pub fn codex_model(home: Option<&std::path::Path>) -> Option<String> {
    let text = std::fs::read_to_string(home?.join(".codex/config.toml")).ok()?;
    // The top-level `model = "..."`, which comes before any table.
    text.lines()
        .take_while(|line| !line.trim_start().starts_with('['))
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "model").then(|| value.trim().trim_matches('"').to_string())
        })
        .filter(|model| !model.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(flavour: Flavour, lines: &[&str]) -> (Vec<Delta>, Result<ChatReply, String>) {
        let mut reader = Reader::new(flavour);
        let mut seen = Vec::new();
        for line in lines {
            seen.extend(reader.line(line));
        }
        (seen, reader.finish("fallback"))
    }

    /// Trimmed from a real run of Claude Code 2.1.322.
    const CLAUDE: &[&str] = &[
        r#"{"type":"system","subtype":"init","cwd":"/tmp","tools":[],"model":"claude-fable-5-2"}"#,
        r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-fable-5-2","usage":{"input_tokens":2}}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"","estimated_tokens":50}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Colouring chain A"}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":" red.\n\n``"}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"`json\n{\"tool\": \"colour_chain\", \"arguments\": {\"chain\": \"A\"}}\n```"}}}"#,
        r#"{"type":"result","is_error":false,"result":"Colouring chain A red.\n\n```json\n{\"tool\": \"colour_chain\", \"arguments\": {\"chain\": \"A\"}}\n```","usage":{"input_tokens":2,"cache_creation_input_tokens":860,"cache_read_input_tokens":3609,"output_tokens":460}}"#,
    ];

    #[test]
    fn claude_code_names_its_model_and_streams_its_answer_but_not_its_tool_block() {
        let (seen, reply) = read(Flavour::Claude, CLAUDE);
        let reply = reply.unwrap();
        assert_eq!(
            seen[0],
            Delta::Model {
                model: "claude-fable-5-2".into()
            }
        );
        assert!(seen.contains(&Delta::Status {
            text: "thinking".into()
        }));
        assert!(seen.contains(&Delta::Status {
            text: "thinking - about 50 tokens so far".into()
        }));
        let shown: String = seen
            .iter()
            .filter_map(|delta| match delta {
                Delta::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            shown, "Colouring chain A red.\n\n",
            "nothing of the fence, not even its first backticks"
        );
        assert_eq!(reply.model, "claude-fable-5-2");
        assert_eq!(reply.text, "Colouring chain A red.");
        assert_eq!(reply.tool_calls[0].name, "colour_chain");
        assert_eq!(reply.stop_reason, "tool");
        assert_eq!((reply.usage.input, reply.usage.output), (4471, 460));
    }

    #[test]
    fn codex_reports_whole_items() {
        let (seen, reply) = read(
            Flavour::Codex,
            &[
                r#"{"type":"thread.started","thread_id":"t"}"#,
                r#"{"type":"turn.started"}"#,
                r#"{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"Find the chain first."}}"#,
                r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"Chain C_1 is now cyan."}}"#,
                r#"{"type":"turn.completed","usage":{"input_tokens":15163,"cached_input_tokens":12288,"output_tokens":63}}"#,
            ],
        );
        let reply = reply.unwrap();
        assert!(seen.contains(&Delta::Thinking {
            text: "Find the chain first.\n".into()
        }));
        assert!(seen.contains(&Delta::Text {
            text: "Chain C_1 is now cyan.".into()
        }));
        assert_eq!(reply.text, "Chain C_1 is now cyan.");
        assert_eq!(reply.thinking, "Find the chain first.");
        assert_eq!(
            reply.model, "fallback",
            "Codex does not say which model in its events"
        );
        assert_eq!((reply.usage.input, reply.usage.output), (15163, 63));
    }

    #[test]
    fn a_reported_failure_is_the_result() {
        let (_, reply) = read(
            Flavour::Claude,
            &[r#"{"type":"result","is_error":true,"result":"Not logged in - Please run /login"}"#],
        );
        assert_eq!(reply.unwrap_err(), "Not logged in - Please run /login");
        let (_, reply) = read(
            Flavour::Codex,
            &[r#"{"type":"turn.failed","error":{"message":"usage limit reached"}}"#],
        );
        assert_eq!(reply.unwrap_err(), "usage limit reached");
    }

    #[test]
    fn output_that_is_not_narration_is_the_answer() {
        // An older version, or a different tool altogether.
        let (seen, reply) = read(Flavour::Claude, &["Hello.", "", "Second line."]);
        assert!(seen.is_empty());
        assert_eq!(reply.unwrap().text, "Hello.\nSecond line.");
        let (_, reply) = read(Flavour::Plain, &[r#"{"type":"looks like an event"}"#]);
        assert_eq!(reply.unwrap().text, r#"{"type":"looks like an event"}"#);
    }

    #[test]
    fn each_tool_is_recognised_and_asked_to_narrate_once() {
        use std::path::Path;
        let args = |list: &[&str]| {
            list.iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(Flavour::of(Path::new("/x/claude"), &[]), Flavour::Claude);
        assert_eq!(
            Flavour::of(Path::new("/x/wrapper.sh"), &args(&["-p"])),
            Flavour::Claude
        );
        assert_eq!(Flavour::of(Path::new("/x/codex"), &[]), Flavour::Codex);
        assert_eq!(
            Flavour::of(Path::new("/x/run"), &args(&["exec"])),
            Flavour::Codex
        );
        assert_eq!(
            Flavour::of(Path::new("/x/ollama"), &args(&["run"])),
            Flavour::Plain
        );

        assert_eq!(Flavour::Claude.narrate(&args(&["-p"])).len(), 4);
        assert!(Flavour::Claude
            .narrate(&args(&["-p", "--output-format", "json"]))
            .is_empty());
        assert!(
            Flavour::Codex.narrate(&args(&["exec"])).is_empty(),
            "its plain mode says more"
        );
        assert!(Flavour::Plain.narrate(&[]).is_empty());
    }

    #[test]
    fn codex_says_its_model_and_its_cost_on_the_side() {
        // stderr and stdout of a real `codex exec`, trimmed.
        let mut reader = Reader::new(Flavour::Codex);
        let mut seen = Vec::new();
        for line in [
            "Reading prompt from stdin...",
            "OpenAI Codex v0.167.0-alpha.4.1",
            "--------",
            "model: gpt-6.1-sol",
            "provider: openai",
            "--------",
            "codex",
            "tokens used",
            "2,971",
        ] {
            seen.extend(reader.aside(line));
        }
        seen.extend(reader.line("Chain C_1 is now cyan."));
        let reply = reader.finish("codex").unwrap();
        assert_eq!(
            seen,
            vec![Delta::Model {
                model: "gpt-6.1-sol".into()
            }]
        );
        assert_eq!(reply.model, "gpt-6.1-sol");
        assert_eq!(reply.text, "Chain C_1 is now cyan.");
        assert_eq!(reply.usage.output, 2971);
        // Claude Code's stderr is nothing of the kind.
        assert!(Reader::new(Flavour::Claude)
            .aside("model: something")
            .is_empty());
    }

    #[test]
    fn each_tool_says_whether_it_is_signed_in() {
        let claude = r#"{"loggedIn": true, "authMethod": "claude.ai", "email": "someone@example.com", "subscriptionType": "team"}"#;
        assert_eq!(
            read_status(Flavour::Claude, claude, true),
            (true, Some("Claude subscription (team)".into()))
        );
        let out = r#"{"loggedIn": false, "authMethod": "none"}"#;
        assert_eq!(read_status(Flavour::Claude, out, false), (false, None));
        // Words it does not know: the exit code decides.
        assert_eq!(
            read_status(Flavour::Claude, "something new", true),
            (true, None)
        );

        assert_eq!(
            read_status(Flavour::Codex, "Logged in using ChatGPT\n", true),
            (true, Some("ChatGPT".into()))
        );
        assert_eq!(
            read_status(Flavour::Codex, "Not logged in\n", false),
            (false, None)
        );
    }
}
