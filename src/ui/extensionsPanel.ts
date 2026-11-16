// The Extensions side panel.
//
// VS Code's arrangement: a narrow bar of icons at the far left chooses what the side
// panel holds, and one of them is what can be added to the app. This is the short form -
// what there is and whether it is installed, found by typing - and a row opens the full
// page in the settings, where it is installed, removed and run.

import { api, type ExtensionState, type ModelCard } from "../ipc";
import { el, replace } from "./dom";
import { errorLine } from "./toast";

export interface ExtensionsPanel {
  element: HTMLElement;
  /** Reads the list again; called whenever the panel is brought up. */
  refresh: () => Promise<void>;
}

/** Whether a query matches a card: every word of it somewhere in the name, task or description. */
export function matchesExtension(card: Pick<ModelCard, "name" | "task" | "description">, query: string): boolean {
  const hay = `${card.name} ${card.task} ${card.description ?? ""}`.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => hay.includes(word));
}

export function extensionsPanel(open: () => void): ExtensionsPanel {
  let cards: ModelCard[] = [];
  /** Where each extension is installed, by its id. */
  let states = new Map<string, ExtensionState[]>();
  let query = "";
  let problem: string | null = null;

  const search = el("input", {
    class: "ext-search",
    type: "search",
    placeholder: "Search extensions",
    spellcheck: "false",
    "aria-label": "Search extensions",
    oninput: (event: Event) => {
      query = (event.target as HTMLInputElement).value;
      draw();
    },
  });
  const list = el("div", { class: "ext-list" });
  const element = el(
    "div",
    { class: "ext-panel" },
    el("div", { class: "ext-head" }, el("span", { text: "Extensions" })),
    search,
    list,
  );

  function status(card: ModelCard): { text: string; kind: "on" | "busy" | "off" } {
    if (card.kind !== "extension") return { text: card.eligible_providers.length > 0 ? "ready to run" : "needs a machine", kind: card.eligible_providers.length > 0 ? "on" : "off" };
    const mine = states.get(card.id) ?? [];
    if (mine.some((entry) => entry.state === "installing" || entry.state === "removing")) return { text: "working...", kind: "busy" };
    const installed = mine.filter((entry) => entry.state === "installed").length;
    if (installed > 0) return { text: installed === 1 ? "installed" : `installed on ${installed}`, kind: "on" };
    return { text: mine.some((entry) => entry.state === "failed") ? "install failed" : "not installed", kind: "off" };
  }

  function row(card: ModelCard): HTMLElement {
    const state = status(card);
    return el(
      "button",
      { class: "ext-row", title: `${card.name} - open its page`, onclick: () => open() },
      el("span", { class: "ext-name", text: card.name }),
      el("span", { class: `ext-state ${state.kind}`, text: state.text }),
      el("span", { class: "ext-about", text: card.description ?? card.task }),
    );
  }

  function section(title: string, members: ModelCard[]): HTMLElement[] {
    if (members.length === 0) return [];
    return [el("div", { class: "ext-section", text: `${title} (${members.length})` }), ...members.map(row)];
  }

  function draw(): void {
    if (problem) {
      replace(list, el("div", { class: "properties-empty", text: problem }));
      return;
    }
    const shown = cards.filter((card) => matchesExtension(card, query));
    const isInstalled = (card: ModelCard) => card.kind === "extension" && status(card).kind !== "off";
    const installed = shown.filter(isInstalled);
    const tools = shown.filter((card) => card.kind === "extension" && !isInstalled(card));
    const models = shown.filter((card) => card.kind !== "extension");
    const rows = [...section("Installed", installed), ...section("Tools", tools), ...section("Models", models)];
    replace(
      list,
      ...(rows.length > 0 ? rows : [el("div", { class: "properties-empty", text: query ? "Nothing by that name." : "No extensions are known." })]),
    );
  }

  async function refresh(): Promise<void> {
    try {
      cards = await api.listModels();
      const found = await Promise.all(
        cards.filter((card) => card.kind === "extension").map(async (card) => [card.id, await api.extensionStates(card.id).catch(() => [])] as const),
      );
      states = new Map(found);
      problem = null;
    } catch (error) {
      problem = errorLine(error);
    }
    draw();
  }

  draw();
  return { element, refresh };
}
