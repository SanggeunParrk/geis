// The Agents panel: a model that can read this app and change it.
//
// Three connections - a model on this machine, Claude, ChatGPT - chosen under the box you
// type in, and a conversation above it. What makes it more than a chat window is the
// tools: the model can read the graph, the molecules and the selection, and it can open
// entries, select things and restyle them. Every call it makes appears in the conversation
// as a card, so the panel is also a log of what was done to the scene and by whom.
//
// The key never comes here. The core holds it, attaches it at call time, and this panel
// only ever says "ask connection `claude` this". What the agent may do is decided by the
// policy at the top: everything, ask first, or read only - and by the tool list itself,
// which has nothing in it that touches the app, the filesystem or the machine.

import {
  api,
  events,
  type AgentConfig,
  type AgentDelta,
  type AgentConnection,
  type AgentDoc,
  type AgentMessage,
  type AgentSkill,
  type AgentToolCall,
  type EngineState,
  type LocalModelState,
} from "../ipc";
import { compact, splitForSummary, summaryMessage, SUMMARY_MARK, SUMMARY_SYSTEM, transcript } from "../agent/compact";
import { converse, type LoopResult } from "../agent/loop";
import { buildTools, LOCAL_TOOLS, systemPrompt, toolText, type AgentHost, type Tool } from "../agent/tools";
import { clear, composing, el, replace } from "./dom";
import { renderMarkdown } from "./markdown";

export interface AgentsHandle {
  element: HTMLElement;
  /** The panel reloads its connections when the settings screen changes them. */
  refresh: () => Promise<void>;
  /** Opens the settings window - the right column's header has the button now. */
  openSettings: () => Promise<void>;
  /**
   * Says which world is open. The conversation is that world's: the one on screen is put
   * away with the world that was left, and this one's is brought back as it was.
   */
  setWorld: (world: string | null) => Promise<void>;
}


const POLICY_LABELS: Record<string, string> = {
  auto: "Change freely",
  ask: "Ask before changing",
  "read-only": "Read only",
};

/**
 * How many characters of conversation a connection is sent, at most.
 *
 * A model on this machine has the window its engine was started with - chosen from the
 * model's shape and the machine's memory, 8 to 32 thousand tokens - less about five and
 * a half thousand for its instructions and tools and two for its answer; at the two and a half
 * characters a token that Korean and JSON average, the rest is the conversation's. The
 * hosted models hold far more, and are given it.
 */
function roomFor(connection: AgentConnection, engine: EngineState | null): number {
  if (connection.kind !== "local") return 400_000;
  const window = engine?.context && engine.context > 0 ? engine.context : 8192;
  return Math.max(4000, Math.round((window - 7500) * 2.5));
}

export function agentsPanel(host: AgentHost): AgentsHandle {
  const log = el("div", { class: "agent-log" });
  /** The line that says something is happening, and for how long it has been. */
  const working = el("div", { class: "agent-working", hidden: "true" });
  const input = el("textarea", {
    class: "agent-input",
    rows: "2",
    placeholder: "Ask for something. ⏎ to send, ⇧⏎ for a new line.",
  });
  const sendButton = el("button", { class: "agent-send", title: "Send (⏎)", text: "↑" });
  const picks = el("div", { class: "agent-picks" });
  const bar = el("div", { class: "agent-bar" });
  // Laid out the way an editor's assistant panel is: who is answering across the top, the
  // conversation, and a box at the bottom with its own controls inside it - the model and
  // how much it may do are choices about the next message, so they sit with it.
  const element = el(
    "div",
    { class: "agents" },
    bar,
    log,
    working,
    el(
      "div",
      { class: "agent-compose" },
      input,
      el("div", { class: "agent-compose-row" }, picks, sendButton),
    ),
  );

  let config: AgentConfig | null = null;
  /** The local catalogue, for naming what is answering. */
  let localModels: LocalModelState[] = [];
  /** The engine that runs the weights: whether it is here, and what it is running. */
  let engine: EngineState | null = null;
  let ready: string[] = [];
  let docs: AgentDoc[] = [];
  const tools: Tool[] = buildTools(host);
  /**
   * The ceiling on one reply. Reasoning counts against it on every current hosted model,
   * so the four thousand this started at could be spent before the answer began - an
   * empty turn, or a tool call cut off half-way through its arguments.
   */
  const MAX_TOKENS = 16000;

  /**
   * A conversation, and the world it belongs to.
   *
   * It was one conversation for the whole app, which made it nobody's: ask about one
   * structure, open another world, and the panel still held the first - with the model
   * still believing chain A was the chain it had just coloured, in a world that has no
   * such molecule. A conversation is about what is on screen, so it lives and is saved
   * with the world whose screen that is.
   */
  interface Chat {
    world: string | null;
    /** As the model sees it. */
    messages: AgentMessage[];
    usage: { input: number; output: number };
  }
  let chat: Chat = { world: null, messages: [], usage: { input: 0, output: 0 } };
  let busy = false;

  /**
   * The turn that is running, if one is: the conversation it belongs to, what it has added
   * so far, and how to stop it. Kept so that coming back to its world shows that turn
   * rather than an older copy read from disk, and so a turn that never answers can be let go.
   */
  interface Turn {
    chat: Chat;
    collected: AgentMessage[];
    stopped: boolean;
    stop: () => void;
  }
  let turn: Turn | null = null;
  /** Settles the Run/Refuse question on screen, if one is waiting. */
  let pendingApproval: ((value: boolean) => void) | null = null;

  function persist(which: Chat): void {
    if (!which.world) return;
    void api.worldChatSave(which.world, { messages: which.messages, usage: which.usage }).catch(() => {
      // The world was deleted while its turn was finishing; there is nowhere to keep it.
    });
  }

  async function setWorld(world: string | null): Promise<void> {
    if (chat.world === world) return;
    // A turn still running belongs to the conversation it was started in, which it keeps
    // hold of; from here on it draws nothing and runs no tools - see `send`. A question
    // waiting on Run/Refuse loses its buttons with the log, so it is answered now: left
    // open, the turn would wait for a press that can no longer be made.
    pendingApproval?.(false);
    canUndoClear = false;
    live = null;
    cards.clear();
    pendingCards.clear();
    clear(log);
    // Back to the world whose turn is still running: that turn's own conversation, with
    // what it has added so far, rather than the copy on disk - which has neither, and
    // would be saved over the answer the next time something was sent.
    if (turn && !turn.stopped && world !== null && turn.chat.world === world) {
      chat = turn.chat;
      replay([...turn.chat.messages, ...turn.collected]);
      drawBar();
      drawStatus();
      return;
    }
    const next: Chat = { world, messages: [], usage: { input: 0, output: 0 } };
    chat = next;
    drawBar();
    drawStatus();
    if (!world) return;
    const saved = await api.worldChat(world).catch(() => null);
    // Asked for one world and another was opened before the answer came.
    if (chat !== next || !saved) return;
    next.messages = Array.isArray(saved.messages) ? saved.messages : [];
    if (saved.usage) next.usage = saved.usage;
    replay(next.messages);
    drawStatus();
  }

  /** Draws a conversation that was had earlier, results and all. */
  function replay(history: AgentMessage[]): void {
    for (const message of history) {
      if (message.role !== "tool") {
        drawMessage(message);
        continue;
      }
      const body = message.tool_call_id ? cards.get(message.tool_call_id) : undefined;
      if (!body) continue;
      setText(body, message.content);
      const card = body.closest<HTMLElement>(".agent-call");
      card?.classList.remove("running");
      card?.classList.add(/^(error|refused)/.test(message.content) ? "failed" : "done");
    }
    // A call with no result was cut off - the app closed mid-turn. It is not still running.
    for (const card of log.querySelectorAll(".agent-call.running")) card.classList.remove("running");
    follow();
  }
  /**
   * The model each connection last answered with. A connection is configured with a name
   * - "claude" - and the model behind it is whatever the subscription or the server makes
   * it; the only way to know is to be told, and once told it is worth showing.
   */
  const answeredWith = new Map<string, string>();

  function connection(): AgentConnection | null {
    if (!config) return null;
    return config.connections.find((entry) => entry.id === config!.active) ?? config.connections[0] ?? null;
  }

  /** What a connection's model is called on screen: the one it answered with, if known. */
  function modelOf(entry: AgentConnection): string {
    const answered = answeredWith.get(entry.id);
    if (answered) return answered;
    if (entry.engine_model) {
      return localModels.find((model) => model.id === entry.engine_model)?.name ?? entry.engine_model;
    }
    // A CLI's "model" is only the command's name until it has answered once.
    return entry.kind === "cli" ? "subscription" : entry.model;
  }

  async function refresh(): Promise<void> {
    config = await api.agentConfig();
    localModels = await api.localModels().catch(() => []);
    engine = await api.engineState().catch(() => null);
    if (!config.active && config.connections.length > 0) config.active = config.connections[0].id;
    [ready, docs] = await Promise.all([api.agentReady(), api.agentDocs()]);
    drawBar();
    drawStatus();
  }

  async function save(): Promise<void> {
    if (!config) return;
    // The core decides which model the local connection runs when an engine is started,
    // and this panel's copy of the settings may predate that; saving it whole would undo it.
    const stored = await api.agentConfig().catch(() => null);
    const mine = config.connections.find((entry) => entry.kind === "local");
    const theirs = stored?.connections.find((entry) => entry.kind === "local");
    if (mine && theirs && theirs.engine_model) {
      mine.engine_model = theirs.engine_model;
      mine.model = theirs.model;
    }
    try {
      config = await api.agentSaveConfig(config);
      ready = await api.agentReady();
    } catch (error) {
      // Said rather than dropped: the picker shows the new choice either way, and a choice
      // that was not kept should not look as if it was.
      note(`The setting could not be saved: ${String(error)}`, true);
    }
    drawBar();
    drawStatus();
  }

  // --- the bar and the pickers ----------------------------------------------------------

  function drawBar(): void {
    if (!config) return;
    const active = connection();
    // A CLI is signed in already, and a local model needs nothing.
    const needsKey = active !== null && active.kind !== "cli" && active.secret_ref !== null && !ready.includes(active.id);
    replace(
      bar,
      // Who is answering, said outright rather than left inside a drop-down.
      el(
        "div",
        { class: "agent-who", title: "The connection, and the model it last answered with" },
        el("span", { class: `agent-dot${busy ? " live" : ""}` }),
        el("span", { class: "agent-who-name", text: active?.name ?? "No connection" }),
        active ? el("span", { class: "agent-who-model", text: modelOf(active) }) : null,
      ),
      needsKey ? el("span", { class: "agent-warn", text: "needs a key" }) : null,
      el("span", { class: "spacer" }),
      canUndoClear
        ? el("button", {
            class: "agent-btn",
            text: "Undo clear",
            title: "Bring back the conversation that was just cleared",
            disabled: busy,
            onclick: () => void undoClear(),
          })
        : null,
      el("button", {
        class: "agent-btn",
        text: "Clear",
        title: busy ? "Wait for the answer to finish" : "Clear this conversation and start again. It can be brought back with Undo clear.",
        // Not while a turn is running. The turn would finish into the emptied history -
        // an answer with no question before it, which the next request is refused for -
        // and clearing the log takes an approval's buttons with it, leaving the turn
        // waiting for a press that can no longer be made.
        disabled: busy,
        onclick: () => void clearChat(),
      }),
    );
    replace(
      picks,
      el(
        "select",
        {
          class: "agent-pick",
          title: "Which model answers",
          onchange: (event: Event) => {
            config!.active = (event.target as HTMLSelectElement).value;
            void save();
          },
        },
        ...config.connections.map((entry) =>
          el("option", {
            value: entry.id,
            text: `${entry.name} · ${modelOf(entry)}`,
            selected: entry.id === active?.id ? "" : undefined,
          }),
        ),
      ),
      ...cliPickers(active),
      el(
        "select",
        {
          class: "agent-pick",
          title: "How much it may do without asking",
          onchange: (event: Event) => {
            config!.policy = (event.target as HTMLSelectElement).value as AgentConfig["policy"];
            void save();
          },
        },
        ...(Object.entries(POLICY_LABELS) as [AgentConfig["policy"], string][]).map(([value, label]) =>
          el("option", { value, text: label, selected: value === config!.policy ? "" : undefined }),
        ),
      ),
    );
  }

  /**
   * Which model and how much thinking, for a subscription connection.
   *
   * Asked of the provider's own tool by the names it takes, so the list is the tool's
   * aliases rather than model ids that go stale; "Default" passes nothing and gets
   * whatever the account is set to.
   */
  const CLI_MODELS: Record<string, [string, string][]> = {
    claude: [["", "Default model"], ["fable", "Fable"], ["opus", "Opus"], ["sonnet", "Sonnet"], ["haiku", "Haiku"]],
  };
  const CLI_EFFORTS: Record<string, [string, string][]> = {
    claude: [["", "Default effort"], ["low", "Low"], ["medium", "Medium"], ["high", "High"], ["xhigh", "Extra high"], ["max", "Max"]],
    codex: [["", "Default effort"], ["low", "Low"], ["medium", "Medium"], ["high", "High"]],
  };

  function cliPickers(connection: AgentConnection | null | undefined): HTMLElement[] {
    if (!connection || connection.kind !== "cli") return [];
    const tool = /codex/i.test(connection.command ?? "") ? "codex" : /claude/i.test(connection.command ?? "") ? "claude" : "";
    const pick = (field: "choice" | "effort", options: [string, string][] | undefined, title: string) =>
      options
        ? [
            el(
              "select",
              {
                class: "agent-pick",
                title,
                onchange: (event: Event) => {
                  connection[field] = (event.target as HTMLSelectElement).value || null;
                  void save();
                },
              },
              ...options.map(([value, label]) =>
                el("option", { value, text: label, selected: (connection[field] ?? "") === value ? "" : undefined }),
              ),
            ),
          ]
        : [];
    return [
      ...pick("choice", CLI_MODELS[tool], "Which of its models answers"),
      ...pick("effort", CLI_EFFORTS[tool], "How hard it thinks before answering"),
    ];
  }

  /**
   * The settings, in a window of their own.
   *
   * Downloading four gigabytes and choosing an endpoint are not things anyone does *while*
   * talking to an agent, and as a card in the conversation they pushed it off the screen
   * while they did it.
   */
  async function openSettings(): Promise<void> {
    // Over the world, not in a window of its own: the same settings, a click away, and
    // closed with Esc back to where the conversation was.
    window.dispatchEvent(new CustomEvent("geis:settings", { detail: "agents" }));
  }

  // --- the conversation ------------------------------------------------------------

  /** Keeps the newest line in view, unless the reader has scrolled up to read. */
  function follow(wasAtBottom = true): void {
    if (wasAtBottom) log.scrollTop = log.scrollHeight;
  }
  const atBottom = () => log.scrollHeight - log.scrollTop - log.clientHeight < 48;

  function note(text: string, bad = false): void {
    log.append(el("div", { class: `agent-note${bad ? " bad" : ""}`, text }));
    follow();
  }

  /** Says that no assistant is set up yet, with the button that leads to setting one up. */
  function setupNote(): void {
    log.append(
      el(
        "div",
        { class: "agent-note agent-setup" },
        el("div", { text: "No assistant is set up yet." }),
        el("div", {
          class: "agent-setup-detail",
          text: "Use Claude or ChatGPT through the subscription you already have, or download a model that runs on this machine.",
        }),
        el("button", { class: "btn primary", text: "Set up an assistant...", onclick: () => void openSettings() }),
      ),
    );
    follow();
  }

  function drawStatus(): void {
    // While a turn runs the button stops it - the one way out of a request that never
    // answers. A summary in progress cannot be stopped, so the button waits for it.
    sendButton.disabled = busy && turn === null;
    sendButton.textContent = turn ? "■" : "↑";
    sendButton.title = turn ? "Stop this answer" : "Send (⏎)";
    const active = connection();
    if (!busy && active?.engine_model && !engine?.running && chat.messages.length === 0) {
      working.hidden = false;
      working.textContent = "The local engine starts on your first question.";
      working.classList.remove("live");
    } else if (!busy) {
      working.hidden = true;
    }
  }

  // --- while it works -------------------------------------------------------------------
  //
  // The one thing a panel like this must never do is sit still. A subscription CLI takes
  // five seconds to say its first word and a local model can take thirty; with nothing
  // moving, that is indistinguishable from broken. So there is always a line saying what
  // is going on and counting the seconds.

  let workingSince = 0;
  let workingWhat = "Working";
  let ticker: number | null = null;

  function drawWorking(): void {
    const seconds = Math.max(0, Math.round((performance.now() - workingSince) / 1000));
    working.textContent = `${workingWhat}… ${seconds}s`;
  }

  function startWorking(): void {
    workingSince = performance.now();
    workingWhat = "Working";
    working.hidden = false;
    working.classList.add("live");
    drawWorking();
    if (ticker !== null) clearInterval(ticker);
    ticker = window.setInterval(drawWorking, 1000);
  }

  function stopWorking(): void {
    if (ticker !== null) clearInterval(ticker);
    ticker = null;
    working.classList.remove("live");
    working.hidden = true;
  }

  // --- one step of an answer ------------------------------------------------------------

  /**
   * One request to the model and what came back: who answered, what it thought, what it
   * said, and what it asked to run. A turn that uses tools is several of these.
   *
   * Drawn as it arrives. Claude Code and Codex both show the work rather than the wait -
   * the reasoning as it is reasoned, the answer as it is written, each tool call the
   * moment it is decided - and this is where those pieces land.
   */
  interface Step {
    root: HTMLElement;
    model: HTMLElement;
    thought: { wrap: HTMLElement; head: HTMLElement; body: HTMLElement; text: string; since: number; note: string } | null;
    answer: HTMLElement | null;
    answerText: string;
    filled: boolean;
  }
  let live: Step | null = null;

  function newStep(): Step {
    const active = connection();
    const model = el("span", { class: "agent-step-model", text: active ? modelOf(active) : "" });
    const root = el(
      "div",
      { class: "agent-step" },
      el(
        "div",
        { class: "agent-step-head" },
        el("span", { class: "agent-dot" }),
        el("span", { class: "agent-step-name", text: active?.name ?? "Agent" }),
        model,
      ),
    );
    return { root, model, thought: null, answer: null, answerText: "", filled: false };
  }

  function startLiveTurn(): void {
    endLiveTurn();
    live = newStep();
    log.append(live.root);
    follow();
  }

  /** How a thought is titled once it is over: how long it took, and how much there was. */
  function thoughtTitle(step: Step, open: boolean): string {
    const thought = step.thought!;
    const seconds = Math.max(1, Math.round((performance.now() - thought.since) / 1000));
    const arrow = thought.text ? (open ? "▾ " : "▸ ") : "";
    return `${arrow}Thought for ${seconds}s${thought.note ? ` · ${thought.note}` : ""}`;
  }

  function ensureThought(step: Step): NonNullable<Step["thought"]> {
    if (step.thought) return step.thought;
    const body = el("div", { class: "agent-thought-body", hidden: "true" });
    const head = el("button", { class: "agent-thought-head", text: "Thinking…" });
    const wrap = el("div", { class: "agent-entry agent-thought live" }, head, body);
    head.addEventListener("click", () => {
      if (!step.thought?.text) return;
      body.hidden = !body.hidden;
      if (!wrap.classList.contains("live")) head.textContent = thoughtTitle(step, !body.hidden);
    });
    step.thought = { wrap, head, body, text: "", since: performance.now(), note: "" };
    step.root.append(wrap);
    step.filled = true;
    return step.thought;
  }

  /** The thought is over: it stops looking like one in progress, and says how long it was. */
  function settleThought(step: Step): void {
    const thought = step.thought;
    if (!thought || !thought.wrap.classList.contains("live")) return;
    thought.wrap.classList.remove("live");
    thought.head.textContent = thoughtTitle(step, !thought.body.hidden);
  }

  /** Ends the live step. One that received nothing at all - a failed request - is removed. */
  function endLiveTurn(): void {
    if (!live) return;
    settleThought(live);
    if (!live.filled) live.root.remove();
    live = null;
  }

  function onDelta(delta: AgentDelta): void {
    if (!live) return;
    const step = live;
    const pinned = atBottom();
    if (delta.kind === "model") {
      step.model.textContent = delta.model;
      const active = connection();
      if (active && answeredWith.get(active.id) !== delta.model) {
        answeredWith.set(active.id, delta.model);
        drawBar();
      }
    } else if (delta.kind === "status") {
      // "thinking", or "thinking - about 370 tokens so far": the first opens the thought,
      // the rest is kept as its footnote - on models that do not return their reasoning,
      // how much of it there was is all there is to show.
      if (delta.text.startsWith("thinking")) {
        const thought = ensureThought(step);
        workingWhat = "Thinking";
        const detail = delta.text.replace(/^thinking\s*-?\s*/, "").replace(/ so far$/, "");
        if (detail) thought.note = detail;
      } else {
        workingWhat = delta.text.charAt(0).toUpperCase() + delta.text.slice(1);
      }
      drawWorking();
    } else if (delta.kind === "thinking") {
      const thought = ensureThought(step);
      workingWhat = "Thinking";
      thought.text += delta.text;
      thought.body.textContent = thought.text;
      // Open while it is being had: this is the part people watch.
      thought.body.hidden = false;
      // The thought scrolls itself, so a long one does not push the answer off screen.
      thought.body.scrollTop = thought.body.scrollHeight;
    } else if (delta.kind === "text") {
      settleThought(step);
      workingWhat = "Writing";
      if (!step.answer) {
        step.answer = el("div", { class: "agent-entry agent-answer" });
        step.root.append(step.answer);
        step.filled = true;
      }
      step.answerText += delta.text;
      renderMarkdown(step.answer, step.answerText);
    } else {
      // A call, named before its arguments are complete: "it is about to read the graph".
      settleThought(step);
      workingWhat = `Running ${delta.name}`;
      step.root.append(callCard({ id: delta.id, name: delta.name, arguments: {} }, true));
      step.filled = true;
    }
    follow(pinned);
  }

  function drawMessage(message: AgentMessage): void {
    if (message.role === "tool") return; // drawn on the call's own card
    if (message.role === "user" && message.content.startsWith(SUMMARY_MARK)) {
      // What stands in for the conversation before it: folded, and marked as not typed.
      log.append(
        el(
          "details",
          { class: "agent-summary" },
          el("summary", { text: "Earlier conversation, summarised" }),
          el("div", { class: "agent-summary-text", text: message.content.slice(SUMMARY_MARK.length).trim() }),
        ),
      );
      follow();
      return;
    }
    if (message.role === "user") {
      log.append(el("div", { class: "agent-user", text: message.content }));
      follow();
      return;
    }
    // A streamed step is already on screen; what is left is what only the finished message
    // knows - the arguments each call was made with. One that was not streamed is drawn
    // whole.
    const step = live ?? newStep();
    if (!live) {
      log.append(step.root);
      if (message.thinking?.trim()) {
        const thought = ensureThought(step);
        thought.text = message.thinking.trim();
        thought.body.textContent = thought.text;
        settleThought(step);
      }
    }
    if (!step.answer && message.content.trim()) {
      step.answer = el("div", { class: "agent-entry agent-answer" });
      renderMarkdown(step.answer, message.content.trim());
      step.root.append(step.answer);
      step.filled = true;
    }
    for (const call of message.tool_calls) {
      const drawn = pendingCards.get(call.id);
      if (!drawn) {
        step.root.append(callCard(call));
        step.filled = true;
        continue;
      }
      pendingCards.delete(call.id);
      const shown = drawn.querySelector<HTMLElement>(".agent-call-args");
      if (shown) shown.textContent = summarise(call.arguments);
      fillInput(drawn, call.arguments);
    }
    if (live) endLiveTurn();
    else if (!step.filled) step.root.remove();
    follow();
  }

  /** A call's arguments on one line: `chain: A, colour: #ff0000`. */
  function summarise(args: Record<string, unknown> | undefined): string {
    const entries = Object.entries(args ?? {});
    if (entries.length === 0) return "";
    const text = entries
      .map(([key, value]) => `${key}: ${typeof value === "string" ? value : JSON.stringify(value)}`)
      .join(", ");
    return text.length > 140 ? `${text.slice(0, 140)}…` : text;
  }

  /** A result as it is easiest to read: JSON laid out, anything else as it came. */
  function pretty(text: string): string {
    if (!/^[[{]/.test(text.trim())) return text;
    try {
      return JSON.stringify(JSON.parse(text), null, 2);
    } catch {
      return text;
    }
  }

  /** Result elements by call id, so a result can be written into the row that asked for it. */
  const cards = new Map<string, HTMLElement>();
  /** Rows drawn from a stream, whose arguments arrive after the row does. */
  const pendingCards = new Map<string, HTMLElement>();

  /**
   * One tool call: a dot for how it went, its name, what it was given, and what came back.
   * The result is a line until it is clicked, since most of them are read once if at all.
   */
  function callCard(call: AgentToolCall, pending = false): HTMLElement {
    const writes = tools.find((tool) => tool.spec.name === call.name)?.writes ?? false;
    const result = el("div", { class: "agent-call-text" });
    // What went in and what came out, in a box under the call - folded to a few lines,
    // opened by a click, since most results are read once if at all.
    const box = el(
      "div",
      { class: "agent-call-box", title: "Click to see all of it" },
      el("div", { class: "agent-call-row in", hidden: "true" }, el("span", { class: "agent-call-tag", text: "IN" }), el("div", { class: "agent-call-text agent-call-in" })),
      el("div", { class: "agent-call-row out" }, el("span", { class: "agent-call-tag", text: "OUT" }), result),
    );
    const card = el(
      "div",
      { class: `agent-entry agent-call running${writes ? " writes" : ""}` },
      el(
        "div",
        { class: "agent-call-head" },
        el("span", { class: "agent-call-name", text: call.name }),
        el("span", { class: "agent-call-args", text: pending ? "" : summarise(call.arguments) }),
      ),
      box,
    );
    box.addEventListener("click", () => {
      // Not when the click was the end of selecting some of it to copy.
      if (box.classList.contains("open") && String(window.getSelection() ?? "") !== "") return;
      box.classList.toggle("open");
      for (const text of box.querySelectorAll<HTMLElement>(".agent-call-text")) showText(text);
    });
    if (!pending) fillInput(card, call.arguments);
    cards.set(call.id, result);
    if (pending) pendingCards.set(call.id, card);
    return card;
  }

  /** Writes a call's arguments into its box, once they are known. */
  function fillInput(card: HTMLElement, args: Record<string, unknown> | undefined): void {
    const row = card.querySelector<HTMLElement>(".agent-call-row.in");
    const text = card.querySelector<HTMLElement>(".agent-call-in");
    if (!row || !text) return;
    const has = Object.keys(args ?? {}).length > 0;
    row.hidden = !has;
    setText(text, has ? JSON.stringify(args) : "");
  }

  /**
   * What a box's text is, and how it is shown: on one line while the box is folded, laid
   * out when it is open. Folded JSON laid out is a brace on a line of its own and nothing
   * else in view.
   */
  const texts = new WeakMap<HTMLElement, string>();
  function setText(target: HTMLElement, text: string): void {
    texts.set(target, text);
    showText(target);
  }
  function showText(target: HTMLElement): void {
    const text = texts.get(target) ?? "";
    const open = target.closest(".agent-call-box")?.classList.contains("open") ?? false;
    target.textContent = open ? pretty(text) : text.replace(/\s+/g, " ");
  }

  /** What the panel does with a call: the policy first, then the tool. */
  async function runTool(call: AgentToolCall): Promise<string> {
    const tool = tools.find((candidate) => candidate.spec.name === call.name);
    const write = tool?.writes ?? true;
    const body = cards.get(call.id);
    const card = body?.closest<HTMLElement>(".agent-call") ?? null;
    workingWhat = `Running ${call.name}`;
    drawWorking();
    const say = (text: string) => {
      const pinned = atBottom();
      if (body) setText(body, text);
      card?.classList.remove("running");
      card?.classList.add(/^(error|refused)/.test(text) ? "failed" : "done");
      follow(pinned);
      workingWhat = "Working";
      return text;
    };
    if (!tool) return say(`error: there is no tool called ${call.name}`);
    if (write && config?.policy === "read-only") {
      return say("refused: the user has this agent on read-only. Say what you would have done.");
    }
    if (write && config?.policy === "ask") {
      const approved = await approve(call, body);
      if (!approved) return say("refused by the user. Do not try another way; ask them what they would prefer.");
    }
    try {
      const result = await tool.run((call.arguments ?? {}));
      return say(toolText(result));
    } catch (error) {
      return say(`error: ${String(error)}`);
    }
  }

  /** Two buttons on the call's own card, and a promise that settles when one is pressed. */
  function approve(call: AgentToolCall, body: HTMLElement | undefined): Promise<boolean> {
    return new Promise((resolve) => {
      const choose = (value: boolean) => {
        if (pendingApproval === choose) pendingApproval = null;
        row.remove();
        resolve(value);
      };
      pendingApproval = choose;
      const row = el(
        "div",
        { class: "agent-approve" },
        el("span", { text: `Run ${call.name}?` }),
        el("button", { class: "agent-btn go", text: "Run", onclick: () => choose(true) }),
        el("button", { class: "agent-btn", text: "Refuse", onclick: () => choose(false) }),
      );
      (body?.closest(".agent-call") ?? log).append(row);
      workingWhat = "Waiting for you";
      drawWorking();
      follow();
    });
  }

  // --- clearing, and taking it back -----------------------------------------------------

  /** Whether this world has a cleared conversation that can still be brought back. */
  let canUndoClear = false;

  /**
   * Clears the conversation - into a bin, not away. The bin is a folder of the app's that
   * is emptied oldest-first once it passes its size, so a cleared conversation can be had
   * back for as long as it is among the recent ones.
   */
  async function clearChat(): Promise<void> {
    if (busy || chat.messages.length === 0) return;
    const world = chat.world;
    if (world) {
      try {
        await api.chatBinPut(world, { messages: chat.messages, usage: chat.usage });
        canUndoClear = true;
      } catch (error) {
        // Not cleared if it could not be kept: an undo that is promised has to be there.
        note(`The conversation could not be set aside, so it was not cleared: ${String(error)}`, true);
        return;
      }
    }
    chat.messages = [];
    chat.usage = { input: 0, output: 0 };
    persist(chat);
    clear(log);
    cards.clear();
    drawBar();
    drawStatus();
    note(world ? "Cleared. Undo clear brings it back." : "Cleared.");
  }

  async function undoClear(): Promise<void> {
    const world = chat.world;
    if (busy || !world) return;
    const back = await api.chatBinTake(world).catch(() => null);
    canUndoClear = false;
    if (!back || !Array.isArray(back.messages)) {
      drawBar();
    drawStatus();
      note("That conversation is no longer in the bin.", true);
      return;
    }
    // What was said since the clear is kept, after what comes back.
    chat.messages = [...back.messages, ...chat.messages];
    chat.usage = { input: (back.usage?.input ?? 0) + chat.usage.input, output: (back.usage?.output ?? 0) + chat.usage.output };
    persist(chat);
    clear(log);
    cards.clear();
    replay(chat.messages);
    drawBar();
    drawStatus();
    note("The conversation is back.");
  }

  // --- skills ----------------------------------------------------------------------------
  //
  // `/name` in the box calls on a skill: a named set of instructions, sent as the message
  // with whatever was typed after the name. Some come with the app; the rest are the
  // user's own, one `.md` file each in the app's `skills` folder. Two are not instructions
  // at all but things the panel does: /clear and /compact.

  const BUILT_IN: AgentSkill[] = [
    { name: "describe", description: "Say what is open: molecules, chains, ligands, what it is", body: "Describe what is open in this world: each molecule, its chains and what they are, its ligands and ions, and in one or two sentences what the structure is. Use list_molecules and list_ligands; do not change anything." },
    { name: "figure", description: "Style the scene for a clean publication figure", body: "Style the scene for a clean publication figure: polymer chains as cartoon coloured by chain, ligands as sticks coloured by element, solvent removed, white background with outlines on, and zoom to everything. Say in one line what you set." },
    { name: "site", description: "Show the binding site around the ligands", body: "Show the binding site: find the ligands, select the residues around them, show those residues as sticks on top of the cartoon, find the polar contacts if you can, and zoom to the site. If there is more than one ligand, ask which." },
    { name: "compare", description: "Compare the molecules that are open", body: "Compare the molecules open in this world: what each is, how their chains and sequences differ, and anything notable about their ligands. Read, do not change." },
  ];
  const ACTIONS: { name: string; description: string; run: () => void }[] = [
    { name: "clear", description: "Clear this conversation (Undo clear brings it back)", run: () => void clearChat() },
    { name: "compact", description: "Summarise the conversation so far to make room", run: () => void compactNow() },
  ];
  let ownSkills: AgentSkill[] = [];
  void api.agentSkills().then((list) => (ownSkills = list)).catch(() => {});
  const allSkills = () => [...ownSkills, ...BUILT_IN.filter((skill) => !ownSkills.some((own) => own.name === skill.name))];

  const skillList = el("div", { class: "agent-skills", hidden: "true" });
  // Over the box, where a completion list goes.
  input.parentElement?.prepend(skillList);
  let skillAt = 0;

  /** The skills and actions whose names start with what is typed after the slash. */
  function skillMatches(): { name: string; description: string }[] {
    const typed = /^\/([a-z0-9-]*)$/i.exec(input.value);
    if (!typed) return [];
    const start = typed[1].toLowerCase();
    return [...ACTIONS, ...allSkills()].filter((entry) => entry.name.startsWith(start));
  }

  function drawSkills(): void {
    const matches = skillMatches();
    skillList.hidden = matches.length === 0;
    if (matches.length === 0) return;
    skillAt = Math.min(skillAt, matches.length - 1);
    replace(
      skillList,
      ...matches.map((entry, index) =>
        el(
          "button",
          {
            class: `agent-skill${index === skillAt ? " active" : ""}`,
            // On press, not click: a click would first take the focus from the box.
            onmousedown: (event: Event) => {
              event.preventDefault();
              pickSkill(entry.name);
            },
          },
          el("span", { class: "agent-skill-name", text: `/${entry.name}` }),
          el("span", { class: "agent-skill-about", text: entry.description }),
        ),
      ),
      el("div", { class: "agent-skill-foot", text: "Your own: .md files in the app's skills folder" }),
    );
  }

  function pickSkill(name: string): void {
    skillList.hidden = true;
    const action = ACTIONS.find((entry) => entry.name === name);
    if (action) {
      input.value = "";
      action.run();
      return;
    }
    input.value = `/${name} `;
    input.focus();
  }

  /** `/name rest` as the message a skill makes of it, or null when it is not a skill. */
  function expandSkill(text: string): string | null {
    const called = /^\/([a-z0-9-]+)(?:\s+([\s\S]*))?$/i.exec(text);
    if (!called) return null;
    const skill = allSkills().find((entry) => entry.name === called[1].toLowerCase());
    if (!skill) return null;
    const rest = (called[2] ?? "").trim();
    return rest ? `${skill.body}\n\nThe user adds: ${rest}` : skill.body;
  }

  /** Summarises the conversation now, whether or not it has run out of room. */
  async function compactNow(): Promise<void> {
    const active = connection();
    if (busy) {
      note("Wait for the answer to finish, then summarise.", true);
      return;
    }
    if (!active || chat.messages.length < 4) {
      note("There is not enough conversation to summarise yet.");
      return;
    }
    const mine = chat;
    // Everything but the last exchange.
    let cut = -1;
    mine.messages.forEach((message, index) => {
      if (message.role === "user") cut = index;
    });
    if (cut <= 0) {
      note("There is not enough conversation to summarise yet.");
      return;
    }
    note("Summarising the conversation…");
    // Busy for as long as it takes: a message sent meanwhile would be cut by an index taken
    // before it arrived, and the replay below would take its streaming answer off the screen.
    busy = true;
    drawBar();
    drawStatus();
    try {
      const reply = await api.agentStream(
        active.id,
        {
          system: SUMMARY_SYSTEM,
          messages: [{ role: "user", content: transcript(mine.messages.slice(0, cut), Math.round(roomFor(active, engine) * 0.7)), tool_calls: [], tool_call_id: null }],
          tools: [],
          max_tokens: 1200,
        },
        () => {},
      );
      if (!reply.text.trim()) throw new Error("the model wrote nothing");
      mine.messages.splice(0, cut, summaryMessage(reply.text));
      persist(mine);
      if (chat === mine) {
        clear(log);
        cards.clear();
        replay(mine.messages);
      }
      if (chat === mine) note("The earlier conversation is summarised.");
    } catch (error) {
      if (chat === mine) note(`Could not summarise: ${String(error)}`, true);
    } finally {
      busy = false;
      drawBar();
      drawStatus();
    }
  }

  /**
   * What a stopped turn had finished: everything up to the last point where every call it
   * made has its result. A call with no result after it is a request every provider refuses
   * to be sent again, so it is left out rather than kept half-done.
   */
  function settled(messages: AgentMessage[]): AgentMessage[] {
    const open = new Set<string>();
    let end = 0;
    messages.forEach((message, index) => {
      if (message.role === "assistant") for (const call of message.tool_calls) open.add(call.id);
      if (message.role === "tool" && message.tool_call_id) open.delete(message.tool_call_id);
      if (open.size === 0) end = index + 1;
    });
    return messages.slice(0, end);
  }

  async function send(): Promise<void> {
    const typed = input.value.trim();
    // An action is done, not sent; a skill is sent as what it says.
    const action = /^\/([a-z0-9-]+)$/i.exec(typed);
    const named = action ? ACTIONS.find((entry) => entry.name === action[1].toLowerCase()) : undefined;
    if (named) {
      input.value = "";
      skillList.hidden = true;
      named.run();
      return;
    }
    if (typed.startsWith("/") && !expandSkill(typed) && !busy) {
      note(`There is no skill called ${typed.split(/\s/)[0]}. Type / to see them.`, true);
      return;
    }
    const prompt = expandSkill(typed) ?? typed;
    const shown = typed;
    const active = connection();
    if (!prompt || busy) return;
    if (!active) {
      note("No connection is configured.", true);
      return;
    }
    input.value = "";
    busy = true;
    drawBar();
    drawStatus();
    // The conversation this turn belongs to, held for as long as the turn runs. If another
    // world is opened meanwhile, `chat` moves on and this does not: what the turn adds
    // still goes into the conversation it was asked in, and is saved with that world -
    // but it draws nothing on a screen that is no longer its own, and it runs no tools,
    // since a tool acts on whatever world is open and that is now a different one.
    const mine = chat;
    // A turn can be stopped - there is no way to call a request back, so stopping is letting
    // go of it: whatever it does after that is not listened to, and runs nothing.
    let release = () => {};
    const halted = new Promise<null>((resolve) => (release = () => resolve(null)));
    const current: Turn = {
      chat: mine,
      collected: [],
      stopped: false,
      stop: () => {
        if (current.stopped) return;
        current.stopped = true;
        pendingApproval?.(false);
        release();
      },
    };
    turn = current;
    /** On screen and still wanted: what may draw and run tools. */
    const here = () => !current.stopped && chat === mine;
    const onScreen = () => chat === mine;
    const user: AgentMessage = { role: "user", content: prompt, tool_calls: [], tool_call_id: null };
    mine.messages.push(user);
    // A skill is shown as it was called, not as the paragraph it stands for.
    drawMessage(shown === prompt ? user : { ...user, content: shown });
    canUndoClear = false;
    drawBar();
    drawStatus();
    startWorking();
    if (active.engine_model && !engine?.running) workingWhat = "Starting the local engine";

    // A model on this machine gets the tools most requests are made of, and a prompt that
    // names only those; the hosted ones get everything.
    const offered = active.kind === "local" ? tools.filter((tool) => LOCAL_TOOLS.has(tool.spec.name)) : tools;
    const system = systemPrompt(
      docs,
      config?.policy ?? "auto",
      active.kind === "local" ? new Set(offered.map((tool) => tool.spec.name)) : undefined,
    );
    // A conversation that has nearly filled its room is summarised first: the early part
    // is put to the model to be written down, and the summary takes its place - kept, so
    // it is done once and not on every request. If that fails the turn goes ahead anyway;
    // what is sent is still shortened, by dropping, as it always was.
    const room = roomFor(active, engine);
    const split = splitForSummary(mine.messages, room);
    if (split) {
      workingWhat = "Summarising the earlier conversation";
      drawWorking();
      try {
        const reply = await Promise.race([
          api.agentStream(
            active.id,
            {
              system: SUMMARY_SYSTEM,
              messages: [{ role: "user", content: transcript(split.early, Math.round(room * 0.7)), tool_calls: [], tool_call_id: null }],
              tools: [],
              max_tokens: 1200,
            },
            () => {},
          ),
          halted,
        ]);
        if (reply && !current.stopped && reply.text.trim()) {
          const summary = summaryMessage(reply.text);
          mine.messages.splice(0, mine.messages.length, summary, ...split.recent);
          mine.usage.input += reply.usage.input;
          mine.usage.output += reply.usage.output;
          persist(mine);
          if (here()) {
            clear(log);
            cards.clear();
            replay(mine.messages);
          }
        }
      } catch {
        // Left as it was; `compact` below keeps the request inside the window regardless.
      }
      workingWhat = "";
    }

    const began = performance.now();
    const empty = { input: 0, output: 0 };
    let result: LoopResult;
    try {
      const finished = current.stopped ? null : await Promise.race([converse(mine.messages, {
        // A stopped turn asks the model nothing more; the loop ends at its next request.
        complete: (history, onPiece) =>
          current.stopped
            ? Promise.reject(new Error("stopped"))
            : api.agentStream(
            active.id,
            // What is sent is shortened to what this model can hold; what is kept is not.
            { system, messages: compact(history, room), tools: offered.map((tool) => tool.spec), max_tokens: MAX_TOKENS },
            onPiece,
          ),
        onDelta: (delta) => {
          if (here()) onDelta(delta);
        },
        // Each time round the loop is a fresh answer, with its own thought and text.
        onStep: () => {
          if (!here()) return;
          workingWhat = "Working";
          startLiveTurn();
        },
        runTool: (call) =>
          current.stopped
            ? Promise.resolve("error: the user stopped this turn.")
            : here()
              ? runTool(call)
              : Promise.resolve("error: the user has opened a different world. Stop here and do not call any more tools."),
        // Drawn as they arrive; kept only once the turn is over, since the loop is already
        // carrying them and a message in both places is a message sent twice. Noted here as
        // well, so a world come back to mid-turn can show them, and a stopped turn keep them.
        onMessage: (message) => {
          if (current.stopped) return;
          current.collected.push(message);
          if (here()) drawMessage(message);
        },
      }), halted]);
      result = finished ?? { added: settled(current.collected), stopped: "error", error: "stopped", usage: empty };
      mine.messages.push(...result.added);
    } finally {
      busy = false;
      if (turn === current) turn = null;
      if (onScreen()) endLiveTurn();
      stopWorking();
    }
    mine.usage = { input: mine.usage.input + result.usage.input, output: mine.usage.output + result.usage.output };
    persist(mine);
    if (!onScreen()) {
      drawBar();
      drawStatus();
      return;
    }
    const usage = mine.usage;
    if (current.stopped) {
      // A call that was under way when the turn was let go is not running any more.
      for (const card of log.querySelectorAll(".agent-call.running")) card.classList.remove("running");
      note("Stopped. What it had finished is kept.");
    } else if (result.stopped === "error") {
      const raw = result.error ?? "the model could not be reached";
      // A local connection with no model behind it is not a network failure, it is an
      // assistant that has not been set up - which is where a first question usually
      // lands. That is said, with the way to fix it, rather than as an address that
      // could not be reached.
      const unset =
        active.kind === "local" &&
        !localModels.some((model) => model.installed) &&
        /could not reach|error sending request|connection refused/i.test(raw);
      if (unset) {
        setupNote();
      } else {
        // The other failure with something to do about it, said as that rather than as JSON.
        note(
          /exceed_context_size|context size|context length|too many tokens/i.test(raw)
            ? "This conversation no longer fits in the model's memory. Type /compact to summarise it, or press Clear to start again."
            : raw,
          true,
        );
      }
    }
    if (result.stopped === "steps") note("Stopped after too many steps. Ask again, more specifically.", true);
    // What the turn was: which model, how long, what it cost. Small, and under the answer
    // it belongs to, so it can be found again after the conversation has moved on.
    const seconds = ((performance.now() - began) / 1000).toFixed(1);
    const spent = result.usage.input + result.usage.output;
    const total = usage.input + usage.output;
    log.append(
      el("div", {
        class: "agent-meta",
        title: total > 0 ? `${usage.input.toLocaleString()} in, ${usage.output.toLocaleString()} out, this conversation` : "",
        text: [
          modelOf(active),
          `${seconds}s`,
          spent > 0 ? `${spent.toLocaleString()} tokens` : "",
          total > spent ? `${total.toLocaleString()} so far` : "",
        ]
          .filter(Boolean)
          .join(" · "),
      }),
    );
    follow();
    engine = await api.engineState().catch(() => engine);
    drawBar();
    drawStatus();
  }

  input.addEventListener("input", () => {
    skillAt = 0;
    drawSkills();
  });
  input.addEventListener("blur", () => (skillList.hidden = true));
  input.addEventListener("keydown", (event) => {
    // With the list of skills up, the arrows move in it and Tab or Enter takes one.
    const matches = skillList.hidden ? [] : skillMatches();
    if (matches.length > 0 && !composing(event)) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        skillAt = (skillAt + (event.key === "ArrowDown" ? 1 : matches.length - 1)) % matches.length;
        drawSkills();
        return;
      }
      if (event.key === "Tab" || (event.key === "Enter" && !event.shiftKey)) {
        event.preventDefault();
        pickSkill(matches[skillAt].name);
        return;
      }
      if (event.key === "Escape") {
        skillList.hidden = true;
        return;
      }
    }
    // Not while an input method is composing: the Enter that picks a Hangul or Kana
    // candidate is not the Enter that sends.
    if (event.key === "Enter" && !event.shiftKey && !composing(event)) {
      event.preventDefault();
      void send();
    }
  });
  sendButton.addEventListener("click", () => {
    if (turn) turn.stop();
    else void send();
  });

  // The settings are another window, so what changed there has to arrive as an event;
  // without this, pressing Use looked like nothing had happened.
  void events.onAgentsChanged(() => void refresh());
  void refresh();
  return { element, refresh, openSettings, setWorld };
}
