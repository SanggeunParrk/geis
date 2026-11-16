// The Agents settings: connections, local models, and the engine that runs them.
//
// Its own window, not a card inside the conversation. Downloading four gigabytes and
// choosing an endpoint are not things you do *while* talking to an agent, and they were
// pushing the conversation off the top of the panel while they did it. A window can be put
// beside the main one, left open while a download runs, and closed when it is done.
//
// Rendered into whatever element it is given, so the same code is the window's content and
// could be a pane if that ever reads better.

import { askConfirm } from "./ask";
import {
  api,
  events,
  type AgentAccount,
  type AgentConfig,
  type AgentConnection,
  type EngineState,
  type LocalModelProgress,
  type LocalModelState,
} from "../ipc";
import { el, replace } from "./dom";

export interface AgentSettingsHandle {
  element: HTMLElement;
  refresh: () => Promise<void>;
}

const KIND_LABELS: Record<string, string> = {
  local: "on this machine",
  anthropic: "Anthropic, by API key",
  openai: "OpenAI, by API key",
  cli: "your own subscription, through its command-line tool",
};

/** The same, in the two or three words a list row has room for. */
const KIND_SHORT: Record<string, string> = {
  local: "local model",
  anthropic: "API key",
  openai: "API key",
  cli: "subscription",
};

const TOOL_USE_LABELS: Record<string, string> = {
  good: "calls tools reliably",
  fair: "usually gets a tool call right",
  poor: "not to be trusted with a tool call",
};

export function bytes(value: number): string {
  if (value >= 1e9) return `${(value / 1e9).toFixed(1)} GB`;
  if (value >= 1e6) return `${Math.round(value / 1e6)} MB`;
  return `${Math.round(value / 1e3)} kB`;
}

export function agentSettingsPanel(): AgentSettingsHandle {
  const body = el("div", { class: "agent-settings-body" });
  const status = el("div", { class: "agent-settings-status" });
  const element = el("div", { class: "agent-settings-screen" }, body, status);

  let config: AgentConfig | null = null;
  let ready: string[] = [];
  let models: LocalModelState[] = [];
  let engine: EngineState | null = null;
  /** Downloads in flight, by model id - and `engine` for the engine itself. */
  const progress = new Map<string, LocalModelProgress>();

  function active(): AgentConnection | null {
    if (!config) return null;
    return config.connections.find((entry) => entry.id === config!.active) ?? config.connections[0] ?? null;
  }

  function note(text: string, bad = false): void {
    status.className = `agent-settings-status${bad ? " bad" : ""}`;
    status.textContent = text;
  }

  async function refresh(): Promise<void> {
    config = await api.agentConfig();
    if (!config.active && config.connections.length > 0) config.active = config.connections[0].id;
    [ready, models, engine] = await Promise.all([
      api.agentReady().catch(() => []),
      api.localModels().catch(() => []),
      api.engineState().catch(() => null),
    ]);
    draw();
  }

  /** Writes the settings; says why if they were not kept, and answers whether they were. */
  async function save(): Promise<boolean> {
    if (!config) return false;
    // The core decides which model the local connection runs; this copy may predate that.
    const stored = await api.agentConfig().catch(() => null);
    const mine = config.connections.find((entry) => entry.kind === "local");
    const theirs = stored?.connections.find((entry) => entry.kind === "local");
    if (mine && theirs && theirs.engine_model) {
      mine.engine_model = theirs.engine_model;
      mine.model = theirs.model;
    }
    let kept = true;
    try {
      config = await api.agentSaveConfig(config);
      ready = await api.agentReady().catch(() => ready);
    } catch (error) {
      kept = false;
      note(`The settings could not be saved: ${String(error)}`, true);
    }
    draw();
    return kept;
  }

  // --- drawing ----------------------------------------------------------------------

  function field(label: string, value: string, onchange: (value: string) => void, type = "text"): HTMLElement {
    return el(
      "label",
      { class: "agent-field" },
      el("span", { text: label }),
      el("input", {
        type,
        value,
        spellcheck: "false",
        onchange: (event: Event) => onchange((event.target as HTMLInputElement).value.trim()),
      }),
    );
  }

  /**
   * The connection whose details are open. Looking at one is not choosing it: the list
   * can be read through, and each signed in to, without changing who answers.
   */
  let viewing: string | null = null;

  function viewed(): AgentConnection | null {
    return config?.connections.find((entry) => entry.id === viewing) ?? active();
  }

  /** Whether a connection could answer right now, and in a few words why not. */
  function standing(entry: AgentConnection): { state: "ok" | "bad" | "wait"; text: string } {
    if (entry.kind === "cli") {
      askAccount(entry);
      const account = accounts.get(entry.id);
      if (!account) return { state: "wait", text: "checking…" };
      if (!account.found) return { state: "bad", text: "not installed" };
      if (account.signed_in === true) return { state: "ok", text: account.account ? `signed in · ${account.account}` : "signed in" };
      return { state: "bad", text: account.signed_in === false ? "not signed in" : "sign-in unknown" };
    }
    if (entry.kind === "local" && entry.engine_model !== undefined && entry.engine_model !== null) {
      if (engine?.running) return { state: "ok", text: "running on this machine" };
      const have = models.some((model) => model.installed);
      if (!engine?.installed) return { state: "bad", text: "engine not installed" };
      return have ? { state: "ok", text: "ready on this machine" } : { state: "bad", text: "no model downloaded" };
    }
    if (entry.kind === "local") {
      const have = engine?.installed && models.some((model) => model.installed);
      return have ? { state: "ok", text: "ready on this machine" } : { state: "bad", text: "nothing downloaded yet" };
    }
    return ready.includes(entry.id) ? { state: "ok", text: "key saved" } : { state: "bad", text: "needs a key" };
  }

  function rosterRow(entry: AgentConnection, open: AgentConnection): HTMLElement {
    const inUse = entry.id === active()?.id;
    const where = standing(entry);
    return el(
      "div",
      {
        class: `agent-roster-row ${where.state}${entry.id === open.id ? " open" : ""}`,
        role: "option",
        tabindex: "0",
        "aria-selected": String(entry.id === open.id),
        onclick: () => {
          viewing = entry.id;
          draw();
        },
        onkeydown: (event: KeyboardEvent) => {
          if (event.key !== "Enter" && event.key !== " ") return;
          event.preventDefault();
          viewing = entry.id;
          draw();
        },
      },
      el("span", { class: "agent-account-dot" }),
      el(
        "div",
        { class: "agent-roster-who" },
        el("span", { class: "agent-roster-name", text: entry.name }),
        el("span", { class: "agent-roster-sub", text: `${KIND_SHORT[entry.kind] ?? entry.kind} · ${where.text}` }),
      ),
      inUse
        ? el("span", { class: "agent-roster-inuse", text: "In use" })
        : el("button", {
            class: "agent-btn",
            text: "Use",
            title: "Make this the one that answers in the agent panel",
            onclick: (event: Event) => {
              event.stopPropagation();
              config!.active = entry.id;
              viewing = entry.id;
              void save().then((kept) => {
                if (kept) note(`${entry.name} now answers in the agent panel.`);
              });
            },
          }),
    );
  }

  /**
   * Connections by API key: not seeded, because a subscription is what most people have,
   * but there for someone who has none, or whose plan meters this kind of use separately.
   */
  const KEYED: { id: string; name: string; kind: AgentConnection["kind"]; endpoint: string; model: string }[] = [
    { id: "claude-api", name: "Claude (API key)", kind: "anthropic", endpoint: "https://api.anthropic.com/v1", model: "claude-opus-5-5" },
    { id: "openai-api", name: "ChatGPT (API key)", kind: "openai", endpoint: "https://api.openai.com/v1", model: "gpt-6-luna" },
  ];
  const isKeyed = (entry: AgentConnection) => entry.kind === "anthropic" || entry.kind === "openai";

  function addRow(): HTMLElement | null {
    const missing = KEYED.filter((keyed) => !config!.connections.some((entry) => entry.id === keyed.id));
    if (missing.length === 0) return null;
    return el(
      "div",
      { class: "agent-add" },
      el("span", { text: "Add, by API key:" }),
      ...missing.map((keyed) =>
        el("button", {
          class: "agent-btn",
          text: `+ ${keyed.name.replace(" (API key)", "")}`,
          onclick: () => {
            config!.connections.push({
              ...keyed,
              secret_ref: `agents/${keyed.id}`,
              command: null,
              args: [],
              engine_model: null,
            });
            viewing = keyed.id;
            void save().then((kept) => {
                if (kept) note(`${keyed.name} added. Paste its key below; it is kept in the system keychain.`);
              });
          },
        }),
      ),
    );
  }

  async function removeConnection(entry: AgentConnection): Promise<void> {
    if (!config) return;
    config.connections = config.connections.filter((other) => other.id !== entry.id);
    if (config.active === entry.id) config.active = config.connections.find((other) => other.kind === "cli")?.id ?? config.connections[0]?.id ?? null;
    viewing = null;
    // The key goes with the connection: left behind, it is a secret nothing points at.
    if (entry.secret_ref) await api.deleteSecret(entry.secret_ref).catch(() => {});
    if (await save()) note(`${entry.name} removed, and its key with it.`);
  }

  /**
   * Whether a redraw is waiting for a text field to be let go of.
   *
   * Everything here redraws whole, and redraws come from outside - a download changing
   * state, the agent panel saving, an account check answering. Done while a field had the
   * keyboard, it threw away what was half typed there, a pasted key included. So it waits
   * until the focus has left the fields, and happens once then.
   */
  let drawWaiting = false;
  const typingHere = () => {
    const focused = document.activeElement;
    return focused instanceof HTMLInputElement && focused.type !== "checkbox" && body.contains(focused);
  };
  body.addEventListener("focusout", () => {
    if (!drawWaiting) return;
    // After the focus has landed: moving from one field to the next is still typing.
    setTimeout(() => {
      if (drawWaiting && !typingHere()) draw();
    }, 0);
  });

  function draw(): void {
    if (typingHere()) {
      drawWaiting = true;
      return;
    }
    drawWaiting = false;
    const connection = viewed();
    if (!config || !connection) {
      replace(body, el("div", { class: "agent-note", text: "No connections." }));
      return;
    }
    const needsKey = connection.kind !== "cli" && connection.secret_ref !== null && !ready.includes(connection.id);
    replace(
      body,
      el(
        "section",
        { class: "agent-block" },
        el("h2", { text: "Agents" }),
        el("div", { class: "agent-roster", role: "listbox" }, ...config.connections.map((entry) => rosterRow(entry, connection))),
        addRow(),
        el("div", {
          class: "agent-note",
          text: "Whichever you use, it is your own account: Geis never sees a password or a sign-in token, and usage is billed to you by the provider, under your agreement with them.",
        }),
      ),
      el(
        "section",
        { class: "agent-block" },
        el(
          "div",
          { class: "agent-row" },
          el("h2", { text: connection.name }),
          el("span", { class: "agent-kind", text: KIND_LABELS[connection.kind] ?? connection.kind }),
          el("span", { class: "spacer" }),
          needsKey ? el("span", { class: "agent-warn", text: "needs a key" }) : null,
          isKeyed(connection)
            ? el("button", { class: "agent-btn", text: "Remove", onclick: () => void removeConnection(connection) })
            : null,
          el("button", {
            class: "agent-btn",
            text: "Test",
            title: "Sends one short question to see that it answers",
            onclick: () => {
              note(`${connection.name}: asking…`);
              void api
                .agentProbe(connection.id)
                .then((found) =>
                  note(
                    found.length > 1
                      ? `${connection.name}: reachable, ${found.length} models.`
                      : `${connection.name}: ${found[0] ?? "reachable"}.`,
                  ),
                )
                .catch((error) => note(`${connection.name}: ${String(error)}`, true));
            },
          }),
        ),
        ...connectionFields(connection),
      ),
      ...(connection.kind === "local"
        ? [
            el(
              "section",
              { class: "agent-block" },
              el("h2", { text: "On this machine" }),
              engineRow(),
              ...models.map((model) => modelRow(model)),
            ),
          ]
        : []),
      el(
        "section",
        { class: "agent-block" },
        el("h2", { text: "What it may do" }),
        el(
          "div",
          { class: "agent-row" },
          el(
            "select",
            {
              class: "agent-pick",
              onchange: (event: Event) => {
                config!.policy = (event.target as HTMLSelectElement).value as AgentConfig["policy"];
                void save();
              },
            },
            ...(
              [
                ["auto", "Change freely"],
                ["ask", "Ask before changing"],
                ["read-only", "Read only"],
              ] as [AgentConfig["policy"], string][]
            ).map(([value, label]) =>
              el("option", { value, text: label, selected: value === config!.policy ? "" : undefined }),
            ),
          ),
        ),
        el("div", {
          class: "agent-note",
          text: "An agent can read and change what is on screen. It has no tool that touches the app's own files, settings or code, and none that runs anything on this machine.",
        }),
      ),
    );
  }

  /**
   * Where each subscription connection stands, asked once per connection and again after
   * anything that could have changed it. `undefined` is "not asked yet", `null` "asking".
   */
  const accounts = new Map<string, AgentAccount | null>();
  /** The connection whose sign-in is under way in the browser, if one is. */
  let signingIn: string | null = null;

  function askAccount(connection: AgentConnection, again = false): void {
    if (!again && accounts.has(connection.id)) return;
    accounts.set(connection.id, null);
    void api
      .agentAccount(connection.id)
      .then((account) => accounts.set(connection.id, account))
      .catch(() => accounts.delete(connection.id))
      .finally(draw);
  }

  /**
   * The two things a subscription connection needs, each with what to do about it: the
   * tool installed, and the tool signed in. This is the whole of "connecting" - there is
   * no key to paste and no login in this app; the person signs in to their own account,
   * in their own browser, with the provider's own tool.
   */
  function accountRows(connection: AgentConnection): HTMLElement[] {
    askAccount(connection);
    const account = accounts.get(connection.id);
    if (!account) {
      return [el("div", { class: "agent-account" }, el("span", { class: "agent-account-dot" }), el("span", { text: "Checking…" }))];
    }
    const copy = (text: string) =>
      el("button", {
        class: "agent-btn",
        text: "Copy",
        onclick: () =>
          void navigator.clipboard
            .writeText(text)
            .then(() => note("Copied."))
            .catch((error) => note(`Could not copy: ${String(error)}`, true)),
      });
    const recheck = el("button", { class: "agent-btn", text: "Check again", onclick: () => askAccount(connection, true) });

    if (!account.found) {
      return [
        el(
          "div",
          { class: "agent-account bad" },
          el("span", { class: "agent-account-dot" }),
          el("span", { class: "agent-account-what", text: `${connection.name}'s command-line tool is not installed.` }),
          recheck,
        ),
        account.install
          ? el(
              "div",
              { class: "agent-account-step" },
              el("span", { text: "1. Install it, in a terminal:" }),
              el("code", { text: account.install }),
              copy(account.install),
            )
          : null,
        el("div", { class: "agent-account-step" }, el("span", { text: "2. Come back and press Check again, then Sign in." })),
      ].filter((node) => node !== null);
    }

    const installed = el(
      "div",
      { class: "agent-account ok", title: account.found },
      el("span", { class: "agent-account-dot" }),
      el("span", { class: "agent-account-what", text: `Installed${account.version ? ` · ${account.version}` : ""}` }),
    );
    if (account.signed_in === true) {
      return [
        installed,
        el(
          "div",
          { class: "agent-account ok" },
          el("span", { class: "agent-account-dot" }),
          el("span", { class: "agent-account-what", text: `Signed in${account.account ? ` · ${account.account}` : ""}` }),
          recheck,
        ),
      ];
    }
    const waiting = signingIn === connection.id;
    return [
      installed,
      el(
        "div",
        { class: "agent-account bad" },
        el("span", { class: "agent-account-dot" }),
        el("span", {
          class: "agent-account-what",
          text: waiting ? "Finish signing in in your browser…" : account.signed_in === false ? "Not signed in." : "Could not tell whether it is signed in.",
        }),
        el("button", {
          class: "agent-btn go",
          text: waiting ? "Waiting…" : "Sign in",
          disabled: waiting,
          onclick: () => {
            signingIn = connection.id;
            draw();
            note(`${connection.name}: your browser will open to sign in.`);
            void api
              .agentSignIn(connection.id)
              .then((signed) => {
                accounts.set(connection.id, signed);
                note(`${connection.name}: signed in.`);
              })
              .catch((error) => {
                note(`${connection.name}: ${String(error)}`, true);
                accounts.delete(connection.id);
              })
              .finally(() => {
                signingIn = null;
                draw();
              });
          },
        }),
        recheck,
      ),
      el(
        "div",
        { class: "agent-account-step" },
        el("span", { text: "Or in a terminal:" }),
        el("code", { text: account.login }),
        copy(account.login),
      ),
    ];
  }

  /** Arguments as one line that can be read back: an empty one is written `""`. */
  const joinArgs = (args: string[]) => args.map((arg) => (arg === "" ? '""' : /\s/.test(arg) ? `"${arg}"` : arg)).join(" ");
  const splitArgs = (line: string) =>
    [...line.matchAll(/"([^"]*)"|(\S+)/g)].map((match) => (match[1] !== undefined ? match[1] : match[2]));

  function connectionFields(connection: AgentConnection): HTMLElement[] {
    if (connection.kind === "cli") {
      return [
        el("div", {
          class: "agent-note",
          text: "Uses your own subscription, through the provider's command-line tool on this machine. There is no key to enter: install the tool, sign in to it, and it is connected.",
        }),
        ...accountRows(connection),
        el(
          "details",
          { class: "agent-advanced" },
          el("summary", { text: "Advanced" }),
          field("Command", connection.command ?? "", (value) => {
            connection.command = value || null;
            accounts.delete(connection.id);
            void save();
          }),
          // Split the way a shell would, so an argument that is deliberately empty -
          // `--tools ""`, which is what turns the tool's own tools off - survives an edit.
          field("Arguments", joinArgs(connection.args), (value) => {
            connection.args = splitArgs(value);
            void save();
          }),
          el("div", {
            class: "agent-note",
            text: "A bare command is looked for on the PATH, where these tools install themselves, and inside editor extensions. The prompt goes in on stdin.",
          }),
        ),
      ];
    }
    const fields: HTMLElement[] = [];
    if (connection.engine_model) {
      fields.push(
        el("div", {
          class: "agent-note",
          text: "Geis runs this model itself. The address is worked out each time, so there is no endpoint to set.",
        }),
      );
    } else {
      fields.push(
        field("Endpoint", connection.endpoint, (value) => {
          connection.endpoint = value;
          void save();
        }),
      );
    }
    fields.push(
      field("Model", connection.model, (value) => {
        connection.model = value;
        void save();
      }),
    );
    if (connection.secret_ref) {
      fields.push(
        field(
          "API key",
          "",
          (value) => {
            if (!value) return;
            void api
              .setSecret(connection.secret_ref!, value)
              .then(() => save())
              .then((kept) => {
                if (kept) note(`${connection.name}: key saved.`);
              })
              // A keychain that refuses the key is said, or the field just empties itself.
              .catch((error) => note(`${connection.name}: the key could not be saved: ${String(error)}`, true));
          },
          "password",
        ),
      );
    } else if (connection.kind === "local" && !connection.engine_model) {
      fields.push(el("div", { class: "agent-note", text: "A model on this machine needs no key." }));
    }
    return fields;
  }

  /** A progress bar that is only there when something is happening, and says how far. */
  function bar(id: string, have: number, total: number, show: boolean): HTMLElement {
    const percent = total > 0 ? Math.min(100, (have / total) * 100) : 0;
    return el(
      "div",
      { class: "agent-progress", dataset: { bar: id }, hidden: show ? undefined : "" },
      el("div", { class: "agent-progress-track" }, el("div", { class: "agent-progress-fill", style: `width:${percent.toFixed(1)}%` })),
      el("div", { class: "agent-progress-text", text: `${bytes(have)} of ${bytes(total)} · ${percent.toFixed(0)}%` }),
    );
  }

  function engineRow(): HTMLElement {
    const live = progress.get("engine");
    const downloading = !!live && live.state !== "done" && live.state !== "failed";
    const actions: (HTMLElement | null)[] = [];
    let state: string;
    if (!engine?.supported) {
      state = "No engine build for this kind of machine. Point the endpoint at Ollama instead.";
    } else if (downloading) {
      state = live.state === "unpacking" ? "unpacking…" : "downloading…";
    } else if (!engine.installed) {
      state = `not installed · ${bytes(engine.size)}`;
      actions.push(el("button", { class: "agent-btn go", text: "Install", onclick: () => void installEngine() }));
    } else if (engine.running) {
      const model = models.find((candidate) => candidate.id === engine?.model);
      state = `running · ${model?.name ?? engine.model}`;
      actions.push(
        el("button", {
          class: "agent-btn",
          text: "Stop",
          onclick: () => {
            void api
              .engineStop()
              .then((next) => {
                engine = next;
                draw();
                note("The local engine has stopped. It starts again on the next question.");
              })
              .catch((error) => note(`The engine could not be stopped: ${String(error)}`, true));
          },
        }),
      );
    } else {
      state = `installed · build ${engine.build}`;
    }
    return el(
      "div",
      { class: `agent-item${engine?.running ? " chosen" : ""}` },
      el(
        "div",
        { class: "agent-item-head" },
        el("span", { class: "agent-item-name", text: "Engine" }),
        el("span", { class: "spacer" }),
        el("span", { class: "agent-item-meta", text: "llama.cpp" }),
      ),
      el("div", { class: "agent-item-summary", text: "Runs the weights on this machine. Nothing else to install." }),
      bar("engine", live?.received ?? 0, live?.total ?? engine?.size ?? 0, downloading),
      el("div", { class: "agent-item-actions" }, el("span", { class: "agent-item-state", text: state }), ...actions),
    );
  }

  function modelRow(model: LocalModelState): HTMLElement {
    const live = progress.get(model.id);
    const downloading = !!live && live.state === "downloading";
    const chosen = active()?.engine_model === model.id;
    const actions: (HTMLElement | null)[] = [];
    let state: string;
    if (downloading) {
      state = "downloading…";
      actions.push(el("button", { class: "agent-btn", text: "Stop", onclick: () => void api.localModelCancel(model.id) }));
    } else if (model.installed) {
      const stray = model.id.startsWith("file:");
      state = stray ? "left over" : chosen && engine?.running ? "answering now" : "downloaded";
      if (!stray) actions.push(el("button", { class: "agent-btn go", text: "Use", onclick: () => void use(model) }));
      actions.push(el("button", { class: "agent-btn", text: "Remove", onclick: () => void remove(model) }));
    } else {
      state = model.have > 0 ? `${bytes(model.have)} of ${bytes(model.size)} downloaded` : "";
      actions.push(
        el("button", {
          class: "agent-btn go",
          text: model.have > 0 ? "Resume" : "Download",
          onclick: () => void download(model),
        }),
      );
    }
    return el(
      "div",
      { class: `agent-item${model.installed ? " installed" : ""}${chosen ? " chosen" : ""}`, dataset: { model: model.id } },
      el(
        "div",
        { class: "agent-item-head" },
        el("span", { class: "agent-item-name", text: model.name }),
        model.recommended ? el("span", { class: "agent-item-tag", text: "recommended" }) : null,
        model.thinks ? el("span", { class: "agent-item-tag think", text: "reasons" }) : null,
        el("span", { class: "spacer" }),
        el("span", {
          class: "agent-item-meta",
          text: model.licence ? `${bytes(model.size)} · ${model.licence}` : bytes(model.size),
        }),
      ),
      el("div", { class: "agent-item-summary", text: model.summary }),
      model.licence
        ? el("div", {
            class: "agent-item-meta",
            text: `${TOOL_USE_LABELS[model.tool_use] ?? model.tool_use} · about ${model.ram_gb} GB of memory while it answers`,
          })
        : null,
      bar(model.id, live?.received ?? model.have, live?.total ?? model.size, downloading),
      el("div", { class: "agent-item-actions" }, el("span", { class: "agent-item-state", text: state }), ...actions),
    );
  }

  // --- doing ------------------------------------------------------------------------

  /**
   * Waits for the core to be in a state, rather than for a command's promise.
   *
   * A window beside the main one loses a command's reply when anything else talks to the
   * core while it is running: the engine had started, the settings were written, and the
   * promise in this window never settled - so a button that had worked looked dead. The
   * work is asked for and then the world is watched until it shows the result.
   */
  async function until<T>(
    look: () => Promise<T>,
    done: (value: T) => boolean,
    patience = 200,
    say?: (seconds: number) => void,
    // Whether there is still anything to wait for. A download that failed or was stopped
    // will never arrive, and a watch that sits out its full patience regardless is still
    // running - and still tidying up after itself - when the next attempt has begun.
    abandoned?: () => boolean,
  ): Promise<T | null> {
    for (let tick = 0; tick < patience; tick += 1) {
      await new Promise((resolve) => setTimeout(resolve, 900));
      if (abandoned?.()) return null;
      say?.(Math.round(tick * 0.9) + 1);
      const value = await look().catch(() => null);
      if (value !== null && done(value)) return value;
    }
    return null;
  }

  /** Which watch is the current one for each download, so an old one cannot tidy up a new one. */
  const watching = new Map<string, object>();
  const gaveUp = (id: string) => {
    const state = progress.get(id)?.state;
    return state === "failed" || state === "cancelled";
  };

  async function installEngine(): Promise<void> {
    const watch = {};
    watching.set("engine", watch);
    progress.set("engine", { id: "engine", received: 0, total: engine?.size ?? 0, state: "downloading", error: null });
    draw();
    void api.engineInstall().catch((error) => note(`Engine: ${String(error)}`, true));
    const ready = await until(
      () => api.engineState(),
      (state) => state.installed,
      // As long as the download itself is allowed: two minutes, the first figure, reported
      // a failure on any connection too slow to fetch the engine in that time while the
      // download carried on behind the message.
      2200,
      undefined,
      () => watching.get("engine") !== watch || gaveUp("engine"),
    );
    if (watching.get("engine") !== watch) return;
    watching.delete("engine");
    progress.delete("engine");
    engine = ready ?? (await api.engineState().catch(() => engine));
    note(ready ? "The engine is ready. Press Use on a model." : "The engine did not install. See the status above.", !ready);
    draw();
  }

  async function download(model: LocalModelState): Promise<void> {
    const watch = {};
    watching.set(model.id, watch);
    progress.set(model.id, { id: model.id, received: model.have, total: model.size, state: "downloading", error: null });
    draw();
    note(`${model.name}: downloading ${bytes(model.size)}. You can leave this window open.`);
    void api.localModelDownload(model.id).catch((error) => note(`${model.name}: ${String(error)}`, true));
    // Watched rather than awaited, for the same reason as the engine: a download of five
    // gigabytes is exactly when something else will talk to the core and lose the reply.
    const done = await until(
      () => api.localModels(),
      (list) => list.some((entry) => entry.id === model.id && entry.installed),
      3600,
      undefined,
      () => watching.get(model.id) !== watch || gaveUp(model.id),
    );
    // A later attempt has taken over; its progress is not this one's to clear.
    if (watching.get(model.id) !== watch) return;
    watching.delete(model.id);
    progress.delete(model.id);
    models = done ?? (await api.localModels().catch(() => models));
    if (done) note(`${model.name} is ready.`);
    draw();
  }

  async function remove(model: LocalModelState): Promise<void> {
    const sure = await askConfirm({
      title: `Remove ${model.name}?`,
      detail: "Its file is deleted from this machine. Using it again means downloading it again.",
      accept: "Remove",
    });
    if (!sure) return;
    const gone = await api
      .localModelDelete(model.id)
      .then(() => true)
      .catch((error) => {
        note(`${model.name} could not be removed: ${String(error)}`, true);
        return false;
      });
    models = await api.localModels().catch(() => models);
    engine = await api.engineState().catch(() => engine);
    if (gone) note(`${model.name} removed.`);
    draw();
  }

  async function use(model: LocalModelState): Promise<void> {
    if (!engine?.installed) {
      note("Install the engine first - it is what runs the weights.", true);
      return;
    }
    note(`Starting ${model.name}… loading the weights takes a moment.`);
    void api.engineStart(model.id).catch((error) => note(`${model.name}: ${String(error)}`, true));
    const running = await until(
      () => api.engineState(),
      (state) => state.running && state.model === model.id,
      200,
      (seconds) => note(`Starting ${model.name}… ${seconds}s`),
    );
    if (running) {
      engine = running;
      await refresh();
      note(`${model.name} is answering locally.`);
    } else {
      engine = await api.engineState().catch(() => engine);
      note(`${model.name} did not start. See engine.log in the app's data directory.`, true);
      draw();
    }
  }

  /** Progress moves the bar in place; a change of state redraws, since the buttons change. */
  function onProgress(update: LocalModelProgress): void {
    const known = progress.get(update.id);
    progress.set(update.id, update);
    const element_ = element.querySelector<HTMLElement>(`[data-bar="${update.id}"]`);
    if (element_ && update.state === "downloading" && known?.state === "downloading") {
      const percent = update.total > 0 ? Math.min(100, (update.received / update.total) * 100) : 0;
      const fill = element_.querySelector<HTMLElement>(".agent-progress-fill");
      const text = element_.querySelector<HTMLElement>(".agent-progress-text");
      if (fill) fill.style.width = `${percent.toFixed(1)}%`;
      if (text) text.textContent = `${bytes(update.received)} of ${bytes(update.total)} · ${percent.toFixed(0)}%`;
      return;
    }
    draw();
  }

  void events.onAgentsChanged(() => void refresh());
  void events.onLocalModelProgress(onProgress);
  void events.onEngineProgress(onProgress);
  void refresh();
  return { element, refresh };
}
