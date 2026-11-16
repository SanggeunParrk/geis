// The shell: main menu, world select, and the settings screens behind it.

import { cellStage } from "./ui/cellStage";
import "./styles.css";
import { api, events } from "./ipc";
import { el, clear } from "./ui/dom";
import { followFullscreen, makeDraggable, screenBar } from "./ui/window";
import { capabilities } from "./viewer/gl";
import { jobsPane } from "./ui/jobs";
import { modelsPane } from "./ui/models";
import { providersPane } from "./ui/providers";
import { viewerScreen, type ViewerScreenHandle } from "./ui/viewerScreen";
import { agentSettingsPanel } from "./ui/agentSettings";
import { preferencesPane } from "./ui/preferences";
import { loadPreferences } from "./prefs";
import { worldsScreen, type WorldsHandle } from "./ui/worlds";
import { errorLine, toast } from "./ui/toast";

type Screen = "menu" | "worlds" | "settings" | "viewer";
type SettingsTab = "preferences" | "agents" | "compute" | "models" | "jobs";

const root = document.querySelector<HTMLElement>("#app");
// The cell behind the opening screens: one canvas for the life of the window.
if (root && !document.body.classList.contains("panel-window")) document.body.prepend(cellStage.element);
if (!root) throw new Error("#app is missing from index.html");

const providers = providersPane();
const models = modelsPane(async (jobId) => {
  // Submitting a job moves you to where you can watch it, which is the only reason
  // anyone submits one.
  await showSettings("jobs");
  await jobs.show(jobId);
});
const jobs = jobsPane((path) => void openInViewer(path));

let settingsTab: SettingsTab = "preferences";
const prefsPane = preferencesPane();
// Read once at the start, so the first world opened already has them.
void loadPreferences();

/**
 * The viewer is built once and reused. It owns a WebGL context, and browsers cap how many
 * of those can exist - recreating it on every visit would eventually lose the oldest.
 */
let viewer: ViewerScreenHandle | null = null;

function ensureViewer(): ViewerScreenHandle {
  if (!viewer) viewer = viewerScreen(() => show("worlds"));
  return viewer;
}

/**
 * The shelf of worlds, built once.
 *
 * It owns a WebGL context for its preview, and browsers hand out few of those; rebuilding
 * it on every visit would eventually lose the viewer's.
 */
let shelf: WorldsHandle | null = null;
function worlds(): WorldsHandle {
  if (!shelf) {
    shelf = worldsScreen({
      onBack: () => show("menu"),
      onEnter: (id) => void enterWorld(id),
    });
  }
  return shelf;
}

/** Opens a world and shows it: this is what used to be "Viewer" on the menu. */
async function enterWorld(id: string): Promise<void> {
  const handle = ensureViewer();
  // Dark until the world is on screen, then put together over it - not assembled round an
  // empty viewport that the molecule then pops into. A world that takes long to read is
  // not waited for past a second and a half.
  handle.hold();
  show("viewer");
  const opened = handle.openWorld(id);
  await Promise.race([opened.catch(() => {}), new Promise((resolve) => setTimeout(resolve, 1500))]);
  handle.assemble();
  await opened;
}

async function openInViewer(path: string): Promise<void> {
  const handle = ensureViewer();
  show("viewer");
  await handle.openPath(path);
}

function mainMenu(): HTMLElement {
  return el(
    "div",
    { class: "menu" },
    // The window has no native title bar - the frame is an overlay, so the traffic lights
    // float over the content. Without a drag region the window cannot be moved at all,
    // and the main menu had none.
    makeDraggable(el("div", { class: "window-drag" })),
    el(
      "div",
      { class: "menu-title" },
      el("h1", { text: "Geis" }),
      el("p", { text: "structure prediction workbench" }),
    ),
    el(
      "div",
      { class: "menu-items" },
      el(
        "button",
        { class: "menu-item", onclick: () => show("worlds") },
        el("span", { text: "Worlds" }),
        el("span", { class: "hint", id: "menu-worlds-hint", text: "" }),
      ),

      el(
        "button",
        { class: "menu-item", onclick: () => void showSettings(settingsTab) },
        el("span", { text: "Settings" }),
        el("span", { class: "hint", id: "menu-compute-hint", text: "" }),
      ),
    ),
    el("div", { class: "menu-version", id: "menu-version" }),
  );
}

/**
 * What the settings are about, in the order someone setting the app up meets them: who
 * answers in the agent panel, where work runs, what can be run, and what has been.
 */
const SETTINGS_SECTIONS: { id: SettingsTab; label: string; about: string }[] = [
  { id: "preferences", label: "Preferences", about: "Colours, defaults for what is new, and which method does what." },
  { id: "agents", label: "Agents", about: "Who answers in the agent panel, and on which account." },
  { id: "compute", label: "Compute", about: "The machines predictions run on." },
  { id: "models", label: "Extensions", about: "Models and tools: what can be run, and what is installed where." },
  { id: "jobs", label: "Jobs", about: "Everything submitted, from every world." },
];

/** Built once: it holds downloads in flight and listens for their progress. */
let agentSettings: ReturnType<typeof agentSettingsPanel> | null = null;
function agentsSection(): ReturnType<typeof agentSettingsPanel> {
  if (!agentSettings) agentSettings = agentSettingsPanel();
  return agentSettings;
}

function settingsScreen(): HTMLElement {
  const bar = screenBar(
    el("button", { class: "btn", text: "< Back", onclick: () => show("menu") }),
    el("h2", { text: "Settings" }),
    el("span", { class: "spacer" }),
  );
  return el("div", { class: "screen" }, bar, settingsBody());
}

/** The settings themselves - the rail and the section it has open - for either place they are shown. */
function settingsBody(): HTMLElement {
  const section = SETTINGS_SECTIONS.find((entry) => entry.id === settingsTab) ?? SETTINGS_SECTIONS[0];

  // A rail down the side rather than tabs in the bar: the bar is the window's drag handle
  // and shares its width with the traffic lights, and four sections with a line each about
  // what they hold do not fit in it.
  const rail = el(
    "nav",
    { class: "settings-rail", role: "tablist", "aria-orientation": "vertical" },
    ...SETTINGS_SECTIONS.map((entry) =>
      el(
        "button",
        {
          class: "settings-link",
          role: "tab",
          "aria-selected": String(entry.id === section.id),
          onclick: () => void showSettings(entry.id),
        },
        el("span", { class: "settings-link-name", text: entry.label }),
        el("span", { class: "settings-link-about", text: entry.about }),
      ),
    ),
  );

  const pane =
    section.id === "preferences"
      ? prefsPane.element
      : section.id === "agents"
      ? agentsSection().element
      : section.id === "compute"
        ? providers.element
        : section.id === "models"
          ? models.element
          : jobs.element;

  const content = el(
    "div",
    { class: "settings-content", role: "tabpanel" },
    el(
      "header",
      { class: "settings-head" },
      el("h1", { text: section.label }),
      el("p", { text: section.about }),
    ),
    el("div", { class: `settings-pane settings-pane-${section.id}` }, pane),
  );

  return el("div", { class: "settings" }, rail, content);
}

/**
 * The settings over whatever is on screen, the way a game's pause menu is: a window in
 * the middle that does not move, and the world behind it dimmed and out of reach until
 * it is closed. Opened from inside a world, so changing a preference does not mean
 * leaving it.
 */
let overlay: HTMLElement | null = null;

function drawOverlay(): void {
  if (!overlay) return;
  const close = el("button", { class: "settings-close", text: "×", title: "Close (Esc)", onclick: () => closeOverlay() });
  overlay.replaceChildren(
    el(
      "div",
      { class: "settings-window", role: "dialog", "aria-modal": "true", "aria-label": "Settings" },
      el("div", { class: "settings-window-bar" }, el("h2", { text: "Settings" }), el("span", { class: "spacer" }), close),
      settingsBody(),
    ),
  );
}

function openOverlay(tab: SettingsTab): void {
  if (!overlay) {
    overlay = el("div", { class: "settings-overlay" });
    // A press on the dimmed part closes it; one inside the window is the window's.
    overlay.addEventListener("pointerdown", (event) => {
      if (event.target === overlay) closeOverlay();
    });
    document.body.append(overlay);
  }
  void showSettings(tab);
}

function closeOverlay(): void {
  overlay?.remove();
  overlay = null;
}

window.addEventListener("keydown", (event) => {
  // A question or the command palette raised over the settings takes Escape first: it
  // closes that, and the settings stay.
  if (event.key === "Escape" && overlay && !document.querySelector(".ask-backdrop, .cmdk-backdrop")) {
    event.stopPropagation();
    closeOverlay();
  }
}, true);
// Cmd+, is where every Mac app keeps its settings. Over a world they open as the overlay,
// so the world is not left; anywhere else they are a screen of their own.
function openSettings(): void {
  if (overlay) return;
  if (viewer?.element.isConnected) openOverlay(settingsTab);
  else void showSettings(settingsTab);
}
window.addEventListener("keydown", (event) => {
  if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey || event.key !== ",") return;
  event.preventDefault();
  openSettings();
});

/**
 * An item chosen from the menu bar.
 *
 * Most are the open world's commands and are handed to it. Undo and Redo are the text
 * field's when one has the keyboard - the menu took Cmd+Z from it, so it is given back.
 */
function onMenu(id: string): void {
  if (id === "settings") {
    openSettings();
    return;
  }
  const focused = document.activeElement;
  const typing = focused instanceof Element && focused.matches("input, textarea, [contenteditable]");
  if (typing && (id === "undo" || id === "redo")) {
    document.execCommand(id);
    return;
  }
  if (viewer?.element.isConnected) {
    window.dispatchEvent(new CustomEvent("geis:command", { detail: id }));
  } else if (id !== "undo" && id !== "redo") {
    toast("Enter a world first - that is where files are opened and commands run.", "info");
  }
}
window.addEventListener("geis:settings", (event) => {
  const wanted = (event as CustomEvent<string>).detail;
  openOverlay(SETTINGS_SECTIONS.some((entry) => entry.id === wanted) ? (wanted as SettingsTab) : settingsTab);
});

async function showSettings(tab: SettingsTab): Promise<void> {
  settingsTab = tab;
  // Wherever they are open: over a world, or as the screen of their own.
  if (overlay) drawOverlay();
  else show("settings");
  if (tab === "preferences") await prefsPane.refresh();
  if (tab === "agents") await agentsSection().refresh();
  if (tab === "compute") await providers.refresh();
  if (tab === "models") await models.refresh();
  if (tab === "jobs") await jobs.refresh();
}

/**
 * A second window renders one panel and nothing else.
 *
 * The same bundle serves both windows; which one this is comes from the query string,
 * because a window is opened with a URL and that is the one thing it is given.
 */
function panelWindow(): boolean {
  const panel = new URLSearchParams(location.search).get("panel");
  if (panel !== "agent-settings") return false;
  document.body.classList.add("panel-window");
  clear(root!);
  root!.append(agentSettingsPanel().element);
  return true;
}

function show(screen: Screen): void {
  closeOverlay();
  clear(root!);
  // The cell is behind the home screen, the worlds' place behind the shelf (the shelf
  // asks for it); inside a world, or in the settings, nothing is drawn there.
  if (screen === "menu") cellStage.home();
  else if (screen !== "worlds") cellStage.hide();
  switch (screen) {
    case "menu":
      root!.append(mainMenu());
      void describeCompute();
      break;
    case "worlds": {
      const shelf = worlds();
      root!.append(shelf.element);
      // Read again on the way in: a world just left may have gained a molecule, and the
      // row's own frame loop stops when the screen is taken off the page.
      void shelf.refresh();
      break;
    }
    case "settings":
      root!.append(settingsScreen());
      break;
    case "viewer":
      root!.append(ensureViewer().element);
      break;
  }
}

/** Whether this launch has already looked at this machine; once is enough. */
let checkedLocal = false;

/** A one-line answer to "is anything set up?" on the menu itself. */
async function describeCompute(): Promise<void> {
  const hint = document.querySelector<HTMLElement>("#menu-compute-hint");
  if (!hint) return;
  try {
    const [records, info] = await Promise.all([
      api.listProviders(),
      api.appInfo(),
    ]);
    // This machine is looked at once without being asked: what it has - its GPU, its
    // memory, which tools are installed - costs a second to find out, and "not checked
    // yet" is not something a first launch should have to be told to fix.
    const unchecked = records.find((record) => record.id === "local" && record.enabled && record.status.state === "unknown");
    if (unchecked && !checkedLocal) {
      checkedLocal = true;
      void api
        .probeProvider(unchecked.id)
        .then(() => describeCompute())
        .catch(() => {});
    }
    const ready = records.filter(
      (record) => record.enabled && record.status.state === "ready",
    );
    const version = document.querySelector<HTMLElement>("#menu-version");
    if (version) version.textContent = `version ${info.version}`;
    // What a newcomer needs from this line is what there is to run and where, not a ratio.
    // A machine that has not been checked yet is not a machine that is missing, so only
    // the ones known to be unusable are left out of the count.
    const usable = records.filter(
      (record) => record.enabled && record.status.state !== "unreachable",
    );
    const machines = usable.length === 1 ? "1 machine" : `${usable.length} machines`;
    hint.textContent =
      usable.length === 0
        ? "no machine available for predictions"
        : `${info.model_count} models - ${machines}${ready.length < usable.length ? "" : " ready"}`;

    // The Worlds line says what there is to come back to, and what will draw it.
    const worldsHint = document.querySelector<HTMLElement>("#menu-worlds-hint");
    if (worldsHint) {
      const gpu = capabilities();
      const shelf = await api.worldsList().catch(() => []);
      const renderer = gpu.webgl2
        ? gpu.software
          ? "software renderer"
          : gpu.renderer.slice(0, 32)
        : "no WebGL2 on this system";
      // The renderer is worth a word only when it is the reason things will be slow.
      const count = shelf.length === 1 ? "1 world" : `${shelf.length} worlds`;
      worldsHint.textContent = gpu.webgl2 && !gpu.software ? count : `${count} - ${renderer}`;
    }
  } catch (error) {
    hint.textContent = errorLine(error);
  }
}

followFullscreen();

window.addEventListener("beforeunload", () => {
  jobs.dispose();
  viewer?.dispose();
});

/**
 * Anything that escapes to here would otherwise leave a blank window, which is the least
 * debuggable failure this app can have: the webview's console is not visible from the
 * terminal that launched it. So a boot failure gets painted on screen *and* sent to
 * stderr, where `tauri dev` shows it.
 */
function reportFatal(what: string, error: unknown, always = false): void {
  const detail =
    error instanceof Error
      ? `${error.message}\n\n${error.stack ?? ""}`
      : String(error);
  void api.uiLog(
    "error",
    `${what}: ${detail.replace(/\s+/g, " ").slice(0, 800)}`,
  );
  // Once the interface is up, a stray error is reported and survived. Painting over the
  // window is for a boot that left nothing to show; done for every rejected promise
  // afterwards it throws away a working session - the graph on screen, an agent's
  // conversation, a running render's progress - over a rename that failed.
  if (booted && !always) {
    toast(errorLine(error));
    return;
  }
  const target = document.querySelector<HTMLElement>("#app") ?? document.body;
  target.textContent = "";
  const panel = document.createElement("pre");
  panel.style.cssText =
    "margin:0;padding:32px 36px;font:12.5px/1.6 ui-monospace,Menlo,monospace;" +
    "color:#ffb4b4;background:#0b0d12;white-space:pre-wrap;height:100%;overflow:auto";
  panel.textContent = `${what}\n\n${detail}`;
  target.appendChild(panel);
}

/** Set once something is on screen; see `reportFatal`. */
let booted = false;

window.addEventListener("error", (event) =>
  reportFatal("Uncaught error", event.error ?? event.message),
);
window.addEventListener("unhandledrejection", (event) =>
  reportFatal("Unhandled rejection", event.reason),
);

try {
  // A window opened for one panel shows that panel and stops; only the main window has a
  // menu, a viewer and files to open.
  if (panelWindow()) {
    booted = true;
    void api.uiLog("ready", "agent settings window");
  } else {
    show("menu");
    booted = true;
    void events.onMenu(onMenu);
    const takeOpened = () =>
      void api
        .pendingOpen()
        .then((path) => (path ? openInViewer(path) : undefined))
        .catch((error) => toast(errorLine(error)));
    // Asked once more as soon as the listener is in place: a file the app was launched
    // with can be announced in the moment between the first look below and this
    // listener existing, and would otherwise wait unopened until the next one.
    void events.onOpenFile(takeOpened).then(takeOpened);
    void api.uiLog("ready", "main menu rendered");
    // `geis model.cif` opens straight into the viewer.
    void api
      .pendingOpen()
      .then((path) => {
        if (path) return openInViewer(path);
      })
      .catch((error) =>
        // Asked for by name on the command line, so its failure is the whole result.
        reportFatal("Could not open the file given on the command line", error, true),
      );
  }
} catch (error) {
  reportFatal("The interface failed to start", error);
}
