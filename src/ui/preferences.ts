// Settings -> Preferences.
//
// Defaults, in four groups: how a molecule looks when it is first opened, what a new
// world is like, how selecting starts out, and which method each analysis uses. Every
// control saves as it is changed; nothing here restyles a world that already exists.

import { COLOR_SCHEME_LABELS, REPRESENTATION_LABELS, type ColorScheme, type Representation } from "../ipc";
import { PALETTES, loadPreferences, preferences, savePreferences, type Preferences } from "../prefs";
import { el, replace } from "./dom";
import { COMMANDS, DEFAULT_MENU } from "../commands";

const BACKGROUNDS: [string, string][] = [
  ["#d5d8dd", "Haze"],
  ["#eceef1", "Mist"],
  ["#ffffff", "White"],
  ["#f4f1ea", "Paper"],
  ["#1b1f27", "Slate"],
  ["#000000", "Black"],
];

const SHOW_AS: [string, Representation[]][] = [
  ["Cartoon", ["cartoon"]],
  ["Cartoon + sticks", ["cartoon", "sticks"]],
  ["Sticks", ["sticks"]],
  ["Ball and stick", ["ball-and-stick"]],
  ["Spacefill", ["spacefill"]],
  ["Backbone", ["backbone"]],
];

const ELEMENT_TABLES: { id: string; name: string; about: string; colors: string[] }[] = [
  { id: "jmol", name: "Geis", about: "Grey carbon, the usual red, blue and yellow.", colors: ["#495057", "#3050f8", "#ff0d0d", "#ffd43b", "#ff8000", "#868e96"] },
  { id: "cpk", name: "CPK", about: "As the plastic models were: black carbon.", colors: ["#333333", "#2233ff", "#ff2200", "#ffcc22", "#ffa500", "#ffffff"] },
  { id: "pymol", name: "PyMOL", about: "Green carbon.", colors: ["#33ff33", "#3333ff", "#ff4d4d", "#e6c640", "#ff8000", "#e6e6e6"] },
];

const SECONDARY_SETS: { id: string; name: string; about: string; colors: string[] }[] = [
  { id: "soft", name: "Soft", about: "Muted, easy to look at for long.", colors: ["#c98686", "#d8bd7c", "#86bfae"] },
  { id: "classic", name: "Classic", about: "The textbook three, at full strength.", colors: ["#e03131", "#f5c211", "#2f9e44"] },
  { id: "cool", name: "Cool", about: "Blues and a violet, when red is spoken for.", colors: ["#4c7fd0", "#41b8c4", "#8f7fd6"] },
];

const RAMPS: { id: string; name: string; about: string; gradient: string }[] = [
  { id: "rainbow", name: "Rainbow", about: "Blue to red through the spectrum.", gradient: "linear-gradient(90deg,#3b4cc0,#2fb5c4,#57c46a,#e8d53a,#e8783a,#c0303b)" },
  { id: "viridis", name: "Viridis", about: "Even in brightness, colour-vision safe.", gradient: "linear-gradient(90deg,#440154,#3b528b,#21918c,#5ec962,#fde725)" },
  { id: "blue-white-red", name: "Blue-white-red", about: "Diverging about the middle.", gradient: "linear-gradient(90deg,#3b4cc0,#f7f7f7,#b40426)" },
];

export function preferencesPane(): { element: HTMLElement; refresh: () => Promise<void> } {
  const body = el("div", { class: "prefs-body" });
  const status = el("div", { class: "agent-settings-status" });
  const element = el("div", { class: "prefs" }, body, status);

  function change(patch: (next: Preferences) => void): void {
    const next = structuredClone(preferences());
    patch(next);
    void savePreferences(next)
      .then(() => {
        status.textContent = "Saved. Applies to what is opened or made from now on.";
        draw();
      })
      .catch((error) => {
        status.textContent = String(error);
      });
  }

  const row = (label: string, help: string, control: HTMLElement) =>
    el(
      "div",
      { class: "prefs-row" },
      el("div", { class: "prefs-label" }, el("span", { text: label }), el("small", { text: help })),
      el("div", { class: "prefs-control" }, control),
    );

  function choice<T extends string>(options: [T, string][], value: T, onPick: (value: T) => void): HTMLElement {
    return el(
      "select",
      { onchange: (event: Event) => onPick((event.target as HTMLSelectElement).value as T) },
      ...options.map(([id, label]) => el("option", { value: id, text: label, selected: id === value ? "" : undefined })),
    );
  }

  const toggle = (value: boolean, onPick: (value: boolean) => void) => {
    const box = el("input", {
      type: "checkbox",
      onchange: (event: Event) => onPick((event.target as HTMLInputElement).checked),
    });
    // The property, not the attribute: an attribute of "" was read as not there at all.
    box.checked = value;
    return box;
  };

  /** A row of cards to choose one of: a strip of colours or a gradient, a name, a line. */
  function cards(
    options: { id: string; name: string; about: string; colors?: string[]; labels?: string[]; gradient?: string }[],
    chosen: string,
    onPick: (id: string) => void,
  ): HTMLElement {
    return el(
      "div",
      { class: "palette-list", role: "radiogroup" },
      ...options.map((option) =>
        el(
          "button",
          { class: `palette${option.id === chosen ? " chosen" : ""}`, role: "radio", "aria-checked": String(option.id === chosen), onclick: () => onPick(option.id) },
          option.gradient
            ? el("div", { class: "palette-chips", style: `background:${option.gradient}` })
            : el(
                "div",
                { class: "palette-chips labelled" },
                ...(option.colors ?? []).map((color, index) => el("span", { style: `background:${color}`, text: option.labels?.[index] ?? "" })),
              ),
          el("div", { class: "palette-name", text: option.name }),
          el("div", { class: "palette-about", text: option.about }),
        ),
      ),
    );
  }

  function draw(): void {
    const now = preferences();
    const shown = SHOW_AS.find(([, list]) => list.join("+") === now.representations.join("+"))?.[0] ?? SHOW_AS[0][0];
    replace(
      body,
      el(
        "section",
        { class: "prefs-group" },
        el("h2", { text: "Colour" }),
        el("div", { class: "prefs-hint", text: "The colours each way of colouring draws from. Elements and secondary structure change every world as soon as it is redrawn; the others are taken by what is coloured from now on." }),
        el("h3", { class: "prefs-sub", text: "Chains, and other categories" }),
        el(
          "div",
          { class: "palette-list", role: "radiogroup" },
          ...PALETTES.map((palette) =>
            el(
              "button",
              {
                class: `palette${palette.id === now.palette ? " chosen" : ""}`,
                role: "radio",
                "aria-checked": String(palette.id === now.palette),
                onclick: () => change((next) => (next.palette = palette.id)),
              },
              el("div", { class: "palette-chips" }, ...palette.colors.map((color) => el("span", { style: `background:${color}` }))),
              el("div", { class: "palette-name", text: palette.name }),
              el("div", { class: "palette-about", text: palette.about }),
            ),
          ),
        ),
        el("h3", { class: "prefs-sub", text: "Atoms, by element" }),
        cards(
          ELEMENT_TABLES.map((table) => ({ id: table.id, name: table.name, about: table.about, colors: table.colors, labels: ["C", "N", "O", "S", "P", "H"] })),
          now.elements,
          (id) => change((next) => (next.elements = id as Preferences["elements"])),
        ),
        el("h3", { class: "prefs-sub", text: "Secondary structure" }),
        cards(
          SECONDARY_SETS.map((set) => ({ id: set.id, name: set.name, about: set.about, colors: set.colors, labels: ["helix", "strand", "turn"] })),
          now.secondary,
          (id) => change((next) => (next.secondary = id as Preferences["secondary"])),
        ),
        el("h3", { class: "prefs-sub", text: "Gradients (B-factor, rainbow, chainbow)" }),
        cards(
          RAMPS.map((ramp) => ({ id: ramp.id, name: ramp.name, about: ramp.about, gradient: ramp.gradient })),
          now.ramp,
          (id) => change((next) => (next.ramp = id as Preferences["ramp"])),
        ),
        row(
          "Background",
          "A new world's background colour.",
          el(
            "div",
            { class: "prefs-swatches" },
            ...BACKGROUNDS.map(([color, name]) =>
              el("button", {
                class: `swatch${color === now.background.toLowerCase() ? " chosen" : ""}`,
                style: `background:${color}`,
                title: name,
                "aria-label": name,
                onclick: () => change((next) => (next.background = color)),
              }),
            ),
            el("input", {
              type: "color",
              value: now.background,
              title: "Any colour",
              onchange: (event: Event) => change((next) => (next.background = (event.target as HTMLInputElement).value)),
            }),
          ),
        ),
        row(
          "Atoms by element",
          "Oxygen red, nitrogen blue, sulfur yellow - whatever the scheme; only carbon follows it.",
          toggle(now.heteroatomsByElement, (value) => change((next) => (next.heteroatomsByElement = value))),
        ),
      ),
      el(
        "section",
        { class: "prefs-group" },
        el("h2", { text: "A new molecule" }),
        row(
          "Colour by",
          "The scheme a molecule is coloured by when it is first opened.",
          choice(Object.entries(COLOR_SCHEME_LABELS) as [ColorScheme, string][], now.scheme, (value) => change((next) => (next.scheme = value))),
        ),
        row(
          "Show as",
          "How a molecule is drawn when it is first opened.",
          choice(
            SHOW_AS.map(([label]) => [label, label] as [string, string]),
            shown,
            (value) => change((next) => (next.representations = [...(SHOW_AS.find(([label]) => label === value)?.[1] ?? ["cartoon"])])),
          ),
        ),
        row("Solvent", "Draw waters.", toggle(now.showWaters, (value) => change((next) => (next.showWaters = value)))),
        row("Ligands", "Draw ligands, ions and cofactors.", toggle(now.showLigands, (value) => change((next) => (next.showLigands = value)))),
        row("Hydrogens", "Draw hydrogens.", toggle(now.showHydrogens, (value) => change((next) => (next.showHydrogens = value)))),
      ),
      el(
        "section",
        { class: "prefs-group" },
        el("h2", { text: "A new world" }),
        row(
          "Layout",
          "Which panes a new world opens with.",
          choice(
            [
              ["viewport", "Viewport only"],
              ["work", "Viewport, sequence and node graph"],
            ],
            now.newWorldLayout,
            (value) => change((next) => (next.newWorldLayout = value)),
          ),
        ),
        row(
          "Select",
          "What a click selects when a world is opened.",
          choice(
            [
              ["chain", "Chain"],
              ["residue", "Residue"],
              ["atom", "Atom"],
            ],
            now.selectLevel,
            (value) => change((next) => (next.selectLevel = value)),
          ),
        ),
      ),
      el(
        "section",
        { class: "prefs-group" },
        el("h2", { text: "Right-click menu" }),
        el("div", {
          class: "prefs-hint",
          text: "The viewport's right-click menu is your shortcuts: tick what should be on it. They appear in the order they were ticked, and act on whatever is selected.",
        }),
        el(
          "div",
          { class: "command-list" },
          ...["Show", "Colour", "Find", "View", "File", "Select"].flatMap((group) => [
            el("div", { class: "command-group", text: group }),
            ...COMMANDS.filter((command) => command.group === group).map((command) => {
              const at = now.contextMenu.indexOf(command.id);
              const box = el("input", {
                type: "checkbox",
                onchange: (event: Event) =>
                  change((next) => {
                    const on = (event.target as HTMLInputElement).checked;
                    next.contextMenu = on ? [...next.contextMenu.filter((id) => id !== command.id), command.id] : next.contextMenu.filter((id) => id !== command.id);
                  }),
              });
              box.checked = at >= 0;
              return el("label", { class: "command-pick" }, box, el("span", { text: command.label }), at >= 0 ? el("span", { class: "command-order", text: `${at + 1}` }) : null);
            }),
          ]),
        ),
        el("button", {
          class: "btn",
          style: "justify-self:start;margin-top:8px",
          text: "Back to the default four",
          onclick: () => change((next) => (next.contextMenu = [...DEFAULT_MENU])),
        }),
      ),
      el(
        "section",
        { class: "prefs-group" },
        el("h2", { text: "Methods" }),
        el("div", { class: "prefs-hint", text: "Which method does each job that has more than one. Alignment is not in the app yet; these are what it will use." }),
        row(
          "Align with",
          "Kabsch superposes matched atoms by least squares. TM-align finds the match itself and needs the TM-score extension.",
          choice(
            [
              ["kabsch", "Kabsch (least squares)"],
              ["tm-align", "TM-align (US-align extension)"],
            ],
            now.methods.align,
            (value) => change((next) => (next.methods.align = value)),
          ),
        ),
        row(
          "Align over",
          "Which atoms a superposition and its RMSD are computed over.",
          choice(
            [
              ["ca", "Alpha carbons"],
              ["backbone", "Backbone"],
              ["all", "All heavy atoms"],
            ],
            now.methods.alignAtoms,
            (value) => change((next) => (next.methods.alignAtoms = value)),
          ),
        ),
        row(
          "Secondary structure",
          "Where helices and strands come from. DSSP is not in the app yet.",
          choice(
            [
              ["file", "As the file states it"],
              ["dssp", "DSSP"],
            ],
            now.methods.secondary,
            (value) => change((next) => (next.methods.secondary = value)),
          ),
        ),
      ),
    );
  }

  async function refresh(): Promise<void> {
    await loadPreferences();
    draw();
  }

  draw();
  return { element, refresh };
}

/** For the label of what is chosen, wherever a representation list has to be named. */
export function representationWords(list: Representation[]): string {
  return list.map((entry) => REPRESENTATION_LABELS[entry] ?? entry).join(" + ");
}
