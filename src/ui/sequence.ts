// The molecule as sequence.
//
// What the BioMol panel is now. It was a map: every chain a bar on one endless horizontal
// strip, every residue a sliver a pixel or two wide with no letter on it, links drawn as
// arcs that left the pane. One chain of four fitted on screen, nothing on it could be
// read, and most of the pane was empty - a picture of the hierarchy rather than a way to
// use it.
//
// This is the form every structure tool settles on, because it is the one that answers the
// questions people bring to it: which residue is that, where is the helix, what is bound.
// Each chain is its sequence in one-letter codes, wrapped to the width of the pane so all
// of it is on screen, tinted by secondary structure and numbered every ten. What is not
// polymer - ligands, ions, waters - is a row of chips. The bonds between residues that are
// not the backbone are a list that says what they join. Everything is a selection: click
// a residue, drag across a run, shift-click to extend, cmd-click to add; click a chain's
// name for the chain; and whatever is selected or hovered in the viewport lights up here
// and is scrolled into view.
//
// Plain elements, not a canvas: text that can be read at any zoom, laid out by the
// browser, with nothing to animate and no frame loop to keep alive.

import { api, type MapChain, type StructureInfo, type StructureMap } from "../ipc";
import { count, el } from "./dom";
import type { BioMolHandle, BioMolOptions } from "./biomol";
import type { Combine, View } from "./selection";

/** Past this many residues in one structure, chains start folded: a ribosome is not read end to end. */
const UNFOLDED_LIMIT = 6000;
/** Residues to a group, with a gap and a number after each. */
const GROUP = 10;

const LINK_NAMES: Record<string, string> = {
  disulfide: "disulfide",
  covalent: "covalent link",
  metal: "metal coordination",
};

interface Shown {
  data: StructureMap;
  /** Each residue's element, by its index in the structure. */
  cells: Map<number, HTMLElement>;
  /** Each chain's heading, by chain index in the structure. */
  heads: Map<number, HTMLElement>;
  /** Which chain a residue is in, and where in it - for ranges. */
  place: Map<number, { chain: number; at: number }>;
}

export function sequencePanel(options: BioMolOptions): BioMolHandle {
  const body = el("div", { class: "seq-body" });
  const element = el("div", { class: "seq", tabindex: "0" }, body);

  let shown: Shown[] = [];
  let drawnKey = "";
  let asked = 0;
  /** Where the last plain click landed, for shift-click and for a drag. */
  let anchor: { structure: string; residue: number } | null = null;
  let dragging = false;
  let lit: HTMLElement[] = [];
  let hovered: HTMLElement[] = [];

  const polymer = (chain: MapChain) => chain.entity_type === "polymer" || chain.entity_type === "branched";

  function empty(title: string, detail: string): void {
    body.replaceChildren(
      el("div", { class: "seq-empty" }, el("div", { class: "seq-empty-title", text: title }), el("div", { text: detail })),
    );
  }

  async function setStructures(structures: StructureInfo[]): Promise<void> {
    const key = structures.map((structure) => structure.id).join(",");
    if (key === drawnKey) return;
    // The latest call wins: a large structure's map can still be loading when a smaller
    // one is asked for, and finishing second it would put the old molecule back.
    const mine = ++asked;
    const loaded: StructureMap[] = [];
    let problem: string | null = null;
    for (const structure of structures) {
      try {
        loaded.push(await api.structureMap(structure.id));
      } catch (error) {
        problem = String(error);
        void api.uiLog("warn", `sequence: could not read ${structure.source}: ${problem}`);
      }
    }
    if (mine !== asked) return;
    // Anything that failed is not remembered as drawn, so the next evaluation tries again.
    drawnKey = problem === null ? key : "";
    draw(loaded, problem);
  }

  function draw(maps: StructureMap[], problem: string | null): void {
    shown = [];
    lit = [];
    hovered = [];
    anchor = null;
    if (maps.length === 0) {
      if (problem) empty("The molecule could not be read.", problem);
      else empty("No molecule.", "Add one in the node graph, or open a file.");
      return;
    }
    body.replaceChildren(...maps.map((data, index) => structureBlock(data, index)));
  }

  function structureBlock(data: StructureMap, structureIndex: number): HTMLElement {
    const entry: Shown = { data, cells: new Map(), heads: new Map(), place: new Map() };
    shown.push(entry);
    const residues = data.chains.reduce((sum, chain) => sum + chain.residue_indices.length, 0);
    const folded = residues > UNFOLDED_LIMIT;

    const polymers: HTMLElement[] = [];
    const others: HTMLElement[] = [];
    const waters: { index: number; chain: MapChain }[] = [];
    data.chains.forEach((chain, chainIndex) => {
      chain.residue_indices.forEach((residue, at) => entry.place.set(residue, { chain: chainIndex, at }));
      if (chain.entity_type === "water") waters.push({ index: chainIndex, chain });
      else if (polymer(chain)) polymers.push(chainBlock(entry, structureIndex, chain, chainIndex, folded));
      else others.push(ligandChip(entry, structureIndex, chain, chainIndex));
    });
    if (waters.length > 0) {
      const total = waters.reduce((sum, water) => sum + water.chain.residue_indices.length, 0);
      const chip = el("button", {
        class: "seq-chip water",
        title: `Select the waters: ${waters.map((water) => water.chain.id).join(", ")}`,
        text: `${count(total, "water")}`,
        dataset: { s: String(structureIndex), chains: waters.map((water) => water.index).join(",") },
      });
      for (const water of waters) entry.heads.set(water.index, chip);
      others.push(chip);
    }

    return el(
      "section",
      { class: "seq-structure" },
      el(
        "header",
        { class: "seq-title" },
        el("span", { class: "seq-entry", text: (data.entry_id || "structure").toUpperCase() }),
        el("span", { class: "seq-what", text: data.title, title: data.title }),
        el("span", {
          class: "seq-counts",
          text: `${count(polymers.length, "chain")} · ${count(residues, "residue")} · ${count(data.atom_count, "atom")}`,
        }),
      ),
      ...polymers,
      others.length > 0
        ? el("div", { class: "seq-row" }, el("div", { class: "seq-label", text: "Bound" }), el("div", { class: "seq-chips" }, ...others))
        : null,
      linksBlock(entry, structureIndex),
    );
  }

  function chainBlock(entry: Shown, structureIndex: number, chain: MapChain, chainIndex: number, folded: boolean): HTMLElement {
    const letters = el("div", { class: "seq-letters", hidden: folded ? "true" : undefined });
    const fill = () => {
      if (letters.childElementCount > 0) return;
      const groups: HTMLElement[] = [];
      for (let start = 0; start < chain.residue_indices.length; start += GROUP) {
        const group = el("span", { class: "seq-group" });
        // The number of the group's first residue, over it: the author's number, which
        // is the one a paper cites, and which need not start at one or run without gaps.
        group.append(el("span", { class: "seq-number", text: String(chain.numbers[start] ?? "") }));
        const run = el("span", { class: "seq-run" });
        for (let at = start; at < Math.min(start + GROUP, chain.residue_indices.length); at += 1) {
          const residue = chain.residue_indices[at];
          const secondary = chain.secondary[at];
          const cell = el("span", {
            class: `seq-cell${secondary === "H" ? " helix" : secondary === "E" ? " strand" : ""}`,
            text: chain.codes[at] ?? "X",
            title: `${chain.names[at] ?? ""} ${chain.numbers[at] ?? ""} · chain ${chain.id}${
              secondary === "H" ? " · helix" : secondary === "E" ? " · strand" : ""
            }`,
            dataset: { s: String(structureIndex), r: String(residue) },
          });
          entry.cells.set(residue, cell);
          run.append(cell);
        }
        group.append(run);
        groups.push(group);
      }
      letters.append(...groups);
    };
    if (!folded) fill();

    const head = el(
      "button",
      {
        class: "seq-chain",
        title: "Select this chain",
        dataset: { s: String(structureIndex), chains: String(chainIndex) },
      },
      el("span", { class: "seq-chain-id", text: chain.id }),
      el("span", {
        class: "seq-chain-meta",
        text: `${chain.auth_id && chain.auth_id !== chain.id.split("_")[0] ? `auth ${chain.auth_id} · ` : ""}${count(chain.residue_indices.length, "residue")}`,
      }),
    );
    entry.heads.set(chainIndex, head);
    const fold = el("button", {
      class: "seq-fold",
      title: "Show or hide this chain's sequence",
      text: folded ? "▸" : "▾",
      onclick: () => {
        fill();
        letters.hidden = !letters.hidden;
        fold.textContent = letters.hidden ? "▸" : "▾";
      },
    });
    return el("div", { class: "seq-row" }, el("div", { class: "seq-label" }, fold, head), letters);
  }

  function ligandChip(entry: Shown, structureIndex: number, chain: MapChain, chainIndex: number): HTMLElement {
    // Named for what it is - HEM, not E_1 - with the chain id as the detail: the residue
    // names of a non-polymer chain are the thing someone is looking for.
    const names = [...new Set(chain.names.filter(Boolean))];
    const label = names.length === 0 ? chain.id : names.length <= 2 ? names.join(" + ") : `${names[0]} +${names.length - 1}`;
    const chip = el(
      "button",
      {
        class: "seq-chip",
        title: `${names.join(", ") || "non-polymer"} · chain ${chain.id}${chain.auth_id ? ` (auth ${chain.auth_id})` : ""}`,
        dataset: { s: String(structureIndex), chains: String(chainIndex) },
      },
      el("span", { class: "seq-chip-name", text: label }),
      el("span", { class: "seq-chip-id", text: chain.id }),
    );
    entry.heads.set(chainIndex, chip);
    return chip;
  }

  /** The bonds between residues that are not the backbone, as a list that says what they join. */
  function linksBlock(entry: Shown, structureIndex: number): HTMLElement | null {
    const { data } = entry;
    if (data.links.length === 0) return null;
    const name = (residue: number): string => {
      const place = entry.place.get(residue);
      if (!place) return "?";
      const chain = data.chains[place.chain];
      return `${chain.names[place.at] ?? ""} ${chain.numbers[place.at] ?? ""} (${chain.id})`;
    };
    const SHOWN = 40;
    const items = data.links.slice(0, SHOWN).map((link) =>
      el(
        "button",
        {
          class: `seq-link ${link.kind}`,
          title: "Select both residues",
          dataset: { s: String(structureIndex), pair: link.residues.join(",") },
        },
        el("span", { class: "seq-link-kind", text: LINK_NAMES[link.kind] ?? link.kind }),
        el("span", { text: `${name(link.residues[0])} — ${name(link.residues[1])}` }),
      ),
    );
    return el(
      "div",
      { class: "seq-row" },
      el("div", { class: "seq-label", text: "Links" }),
      el(
        "div",
        { class: "seq-links" },
        ...items,
        data.links.length > SHOWN ? el("span", { class: "seq-more", text: `and ${data.links.length - SHOWN} more` }) : null,
      ),
    );
  }

  // --- selecting ------------------------------------------------------------------------

  const modeOf = (event: MouseEvent | PointerEvent): Combine => (event.metaKey || event.ctrlKey ? "toggle" : "replace");

  /** The residue under an event, if it is over one. */
  function residueAt(target: EventTarget | null): { entry: Shown; residue: number } | null {
    const cell = target instanceof Element ? target.closest<HTMLElement>(".seq-cell") : null;
    if (!cell) return null;
    const entry = shown[Number(cell.dataset.s)];
    return entry ? { entry, residue: Number(cell.dataset.r) } : null;
  }

  /** Every residue between two in the same chain, inclusive; the one alone if they are not. */
  function span(entry: Shown, from: number, to: number): number[] {
    const a = entry.place.get(from);
    const b = entry.place.get(to);
    if (!a || !b || a.chain !== b.chain) return [to];
    const [low, high] = a.at <= b.at ? [a.at, b.at] : [b.at, a.at];
    return entry.data.chains[a.chain].residue_indices.slice(low, high + 1);
  }

  body.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    const hit = residueAt(event.target);
    if (!hit) return;
    event.preventDefault();
    const { entry, residue } = hit;
    const extending = event.shiftKey && anchor?.structure === entry.data.id;
    const indices = extending ? span(entry, anchor!.residue, residue) : [residue];
    if (!extending) anchor = { structure: entry.data.id, residue };
    dragging = true;
    options.onSelect({ structureId: entry.data.id, level: "residue", indices }, extending ? "replace" : modeOf(event));
  });
  body.addEventListener("pointerover", (event) => {
    const hit = residueAt(event.target);
    if (dragging && hit && anchor?.structure === hit.entry.data.id) {
      // A drag is a range from where it started to where it is.
      options.onSelect(
        { structureId: hit.entry.data.id, level: "residue", indices: span(hit.entry, anchor.residue, hit.residue) },
        "replace",
      );
      return;
    }
    options.onHover?.(hit ? { structureId: hit.entry.data.id, level: "residue", indices: [hit.residue] } : null);
  });
  body.addEventListener("pointerleave", () => options.onHover?.(null));
  window.addEventListener("pointerup", () => {
    dragging = false;
  });

  body.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const chains = target?.closest<HTMLElement>("[data-chains]");
    if (chains) {
      const entry = shown[Number(chains.dataset.s)];
      if (!entry) return;
      const indices = (chains.dataset.chains ?? "").split(",").filter(Boolean).map(Number);
      options.onSelect({ structureId: entry.data.id, level: "chain", indices }, event.shiftKey ? "add" : modeOf(event));
      return;
    }
    const pair = target?.closest<HTMLElement>("[data-pair]");
    if (pair) {
      const entry = shown[Number(pair.dataset.s)];
      if (!entry) return;
      const indices = (pair.dataset.pair ?? "").split(",").map(Number);
      options.onSelect({ structureId: entry.data.id, level: "residue", indices }, "replace");
    }
  });

  // --- showing what is selected -----------------------------------------------------------

  function setSelection(view: View | null, residues: Set<number>, chainIds: string[]): void {
    for (const node of lit) node.classList.remove("selected");
    lit = [];
    if (!view) return;
    const entry = shown.find((candidate) => candidate.data.id === view.structureId);
    if (!entry) return;
    for (const residue of residues) {
      const cell = entry.cells.get(residue);
      if (cell) {
        cell.classList.add("selected");
        lit.push(cell);
      }
    }
    // A chain's name lights when the whole chain is what is selected, not when one
    // residue of it is - otherwise every selection lights a heading and it says nothing.
    if (view.level === "chain") {
      entry.data.chains.forEach((chain, index) => {
        const head = entry.heads.get(index);
        if (head && chainIds.includes(chain.id) && !lit.includes(head)) {
          head.classList.add("selected");
          lit.push(head);
        }
      });
    }
    // Brought into view when it was chosen somewhere else - a click in the viewport - and
    // left alone when it was chosen here, where it is already under the pointer.
    const first = lit.find((node) => node.classList.contains("seq-cell")) ?? lit[0];
    if (first && !dragging && !element.matches(":hover")) first.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }

  function setHover(view: View | null): void {
    for (const node of hovered) node.classList.remove("hovered");
    hovered = [];
    if (!view) return;
    const entry = shown.find((candidate) => candidate.data.id === view.structureId);
    if (!entry) return;
    const mark = (node: HTMLElement | undefined) => {
      if (!node) return;
      node.classList.add("hovered");
      hovered.push(node);
    };
    if (view.level === "chain") for (const index of view.indices) mark(entry.heads.get(index));
    else if (view.level === "residue") for (const index of view.indices) mark(entry.cells.get(index));
  }

  empty("No molecule.", "Add one in the node graph, or open a file.");
  return { element, setStructures, setSelection, setHover };
}
