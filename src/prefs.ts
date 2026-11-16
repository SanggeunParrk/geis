// The user's preferences: defaults for what is made next, not state of what exists.
//
// A new molecule's style, a new world's background and layout, which palette "colour by
// chain" hands out, and which method each kind of analysis uses when there is a choice.
// Changing one never restyles a world that is already open - those are the world's own.
// Kept in the core's settings as one object; this module owns its shape and its defaults,
// so a preference added later reads as its default from every older save.

import { COMMANDS, DEFAULT_MENU, EARLIER_MENUS } from "./commands";
import { api, COLOR_SCHEME_LABELS, REPRESENTATION_LABELS, type Categorical, type ColorScheme, type Representation } from "./ipc";

export interface Preferences {
  /** The palette colour-by-chain and other categorical schemes draw from. */
  palette: Categorical;
  /** The gradient that B-factor, rainbow and chainbow run along. */
  ramp: "rainbow" | "viridis" | "blue-white-red";
  /** Which table atoms take their element colours from. */
  elements: "jmol" | "cpk" | "pymol";
  /** The colours helices, strands and turns are drawn in. */
  secondary: "soft" | "classic" | "cool";
  /** A new world's background, `#rrggbb`. */
  background: string;
  /** How a molecule is drawn when it is first opened. */
  representations: Representation[];
  scheme: ColorScheme;
  /** Atoms other than carbon keep their element's colour under any scheme. */
  heteroatomsByElement: boolean;
  showWaters: boolean;
  showHydrogens: boolean;
  showLigands: boolean;
  /** Whether an entry fetched from the PDB is shown as its biological assembly. */
  fetchAssembly: boolean;
  /** What a click selects when a world is opened. */
  selectLevel: "atom" | "residue" | "chain";
  /** `viewport` is the viewport alone; `work` adds the sequence and the node graph. */
  newWorldLayout: "viewport" | "work";
  /** The commands on the viewport's right-click menu, in order - see `COMMANDS`. */
  contextMenu: string[];
  /** Which method does each job that has more than one. */
  methods: {
    /** Superposing two structures. */
    align: "kabsch" | "tm-align";
    /** Which atoms a superposition and its RMSD are computed over. */
    alignAtoms: "ca" | "backbone" | "all";
    /** Where helices and strands come from. */
    secondary: "file" | "dssp";
  };
}

/** Raised when a default changes in a way older saves should follow. */
export const PREFERENCES_VERSION = 2;

export const DEFAULT_PREFERENCES: Preferences = {
  palette: "soft",
  ramp: "rainbow",
  elements: "jmol",
  secondary: "soft",
  background: "#d5d8dd",
  representations: ["cartoon"],
  scheme: "secondary",
  heteroatomsByElement: true,
  showWaters: false,
  showHydrogens: false,
  showLigands: true,
  fetchAssembly: true,
  selectLevel: "chain",
  newWorldLayout: "viewport",
  contextMenu: [...DEFAULT_MENU],
  methods: { align: "kabsch", alignAtoms: "ca", secondary: "file" },
};

/**
 * The palettes, as they are drawn in the settings. The colours themselves are the core's
 * (`geometry/palette.rs`); these are the same values, for the swatches.
 */
export const PALETTES: { id: Categorical; name: string; about: string; colors: string[] }[] = [
  {
    id: "soft",
    name: "Soft",
    about: "Quiet, half-strength hues. Easy on the eyes for long sessions.",
    colors: ["#86a8cf", "#dba77f", "#8cc0a4", "#c79bbd", "#b3a8d8", "#d3c486", "#8fc3cb", "#d49a9a"],
  },
  {
    id: "okabe-ito",
    name: "Okabe-Ito",
    about: "Colour-vision safe. Okabe & Ito, Color Universal Design.",
    colors: ["#0072b2", "#e69f00", "#009e73", "#cc79a7", "#56b4e9", "#d55e00", "#f0e442", "#3b4252"],
  },
  {
    id: "tol-bright",
    name: "Tol Bright",
    about: "Colour-vision safe, livelier. Paul Tol, SRON.",
    colors: ["#4477aa", "#ee6677", "#228833", "#ccbb44", "#66ccee", "#aa3377", "#bbbbbb"],
  },
  {
    id: "tol-muted",
    name: "Tol Muted",
    about: "Colour-vision safe, quiet. Paul Tol, SRON.",
    colors: ["#cc6677", "#332288", "#ddcc77", "#117733", "#88ccee", "#882255", "#44aa99", "#999933", "#aa4499"],
  },
  {
    id: "tableau10",
    name: "Tableau 10",
    about: "The ten most charts are drawn in.",
    colors: ["#4e79a7", "#f28e2b", "#e15759", "#76b7b2", "#59a14f", "#edc948", "#b07aa1", "#ff9da7", "#9c755f", "#bab0ac"],
  },
  {
    id: "set2",
    name: "ColorBrewer Set2",
    about: "Soft pastels, for print. Cynthia Brewer, Apache 2.0.",
    colors: ["#66c2a5", "#fc8d62", "#8da0cb", "#e78ac3", "#a6d854", "#ffd92f", "#e5c494", "#b3b3b3"],
  },
  {
    id: "vivid",
    name: "Vivid",
    about: "Twelve strong hues, for many chains. Not colour-vision safe.",
    colors: ["#e03131", "#f08c00", "#2f9e44", "#1971c2", "#9c36b5", "#0c8599", "#e8590c", "#5f3dc4", "#74b816", "#c2255c", "#087f5b", "#495057"],
  },
];

let current: Preferences = structuredClone(DEFAULT_PREFERENCES);
const listeners = new Set<(preferences: Preferences) => void>();

/** Lays what was saved over the defaults, field by field, ignoring what is not understood. */
export function mergePreferences(saved: unknown): Preferences {
  const merged = structuredClone(DEFAULT_PREFERENCES);
  if (typeof saved !== "object" || saved === null) return merged;
  // A copy, so that forgetting an old default below does not reach into the caller's object.
  const source = { ...(saved as Record<string, unknown>) };
  const take = <K extends keyof Preferences>(key: K, ok: (value: unknown) => boolean) => {
    if (key in source && ok(source[key])) merged[key] = source[key] as Preferences[K];
  };
  // A save from before the defaults were quietened holds the old ones in full - every
  // field is written whenever any is changed - so a value that is exactly an old default
  // is read as "the default", and follows it.
  const before = typeof source.version !== "number" || source.version < PREFERENCES_VERSION;
  if (before) {
    if (source.background === "#eceef1") delete source.background;
    if (source.palette === "okabe-ito") delete source.palette;
  }
  take("palette", (value) => PALETTES.some((palette) => palette.id === value));
  take("ramp", (value) => value === "rainbow" || value === "viridis" || value === "blue-white-red");
  take("elements", (value) => value === "jmol" || value === "cpk" || value === "pymol");
  take("secondary", (value) => value === "soft" || value === "classic" || value === "cool");
  take("background", (value) => typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value));
  // Only names the core knows: a hand-edited "ribbon" would otherwise reach it with every
  // molecule opened, and the core refuses a style it cannot read.
  const known = (table: Record<string, string>, value: unknown) => typeof value === "string" && Object.hasOwn(table, value);
  take("representations", (value) => Array.isArray(value) && value.length > 0 && value.every((entry) => known(REPRESENTATION_LABELS, entry)));
  merged.representations = [...new Set(merged.representations)];
  take("scheme", (value) => known(COLOR_SCHEME_LABELS, value));
  for (const flag of ["heteroatomsByElement", "showWaters", "showHydrogens", "showLigands", "fetchAssembly"] as const) {
    take(flag, (value) => typeof value === "boolean");
  }
  take("selectLevel", (value) => value === "atom" || value === "residue" || value === "chain");
  // A command that is no longer there (renamed, or removed in a later version) is dropped
  // from the menu on its own; the rest of the user's choice stands.
  if (Array.isArray(source.contextMenu)) {
    const ids = source.contextMenu.filter((entry): entry is string => typeof entry === "string" && COMMANDS.some((command) => command.id === entry));
    // Emptied by the user is kept empty; emptied only by forgetting is the default menu.
    // And a menu that is exactly an earlier version's default was not chosen by anyone:
    // it follows the default, so a command added to it since reaches people who have
    // only ever changed something else in the preferences.
    const untouched = EARLIER_MENUS.some((menu) => menu.length === ids.length && menu.every((id, index) => id === ids[index]));
    if (!untouched && (ids.length > 0 || source.contextMenu.length === 0)) merged.contextMenu = [...new Set(ids)];
  }
  take("newWorldLayout", (value) => value === "viewport" || value === "work");
  if (typeof source.methods === "object" && source.methods !== null) {
    const methods = source.methods as Record<string, unknown>;
    if (methods.align === "kabsch" || methods.align === "tm-align") merged.methods.align = methods.align;
    if (methods.alignAtoms === "ca" || methods.alignAtoms === "backbone" || methods.alignAtoms === "all") {
      merged.methods.alignAtoms = methods.alignAtoms;
    }
    if (methods.secondary === "file" || methods.secondary === "dssp") merged.methods.secondary = methods.secondary;
  }
  return merged;
}

/** The preferences as they stand. Defaults until `loadPreferences` has answered. */
export function preferences(): Preferences {
  return current;
}

export async function loadPreferences(): Promise<Preferences> {
  current = mergePreferences(await api.preferences().catch(() => null));
  return current;
}

export async function savePreferences(next: Preferences): Promise<void> {
  current = next;
  await api.preferencesSave({ ...(next as unknown as Record<string, unknown>), version: PREFERENCES_VERSION });
  for (const listener of listeners) listener(current);
}

export function onPreferences(listener: (preferences: Preferences) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
