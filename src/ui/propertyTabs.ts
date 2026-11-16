// Which tabs the properties panel shows, and which one is open.
//
// Blender's properties editor has a vertical strip of icon tabs on its left edge, and
// which of them exist depends on what is active: an object gives you Object, Modifiers and
// Data, a mesh with materials adds Material, and Render/Output/World are always there
// because a scene always has them. `space_properties.py` draws the strip in one call -
// `layout.prop_tabs_enum(view, "context", icon_only=True)` - and the filtering happens in
// the C side's context, which is what this module stands in for.
//
// Pure and separate from the editor so the rule can be tested: the interesting part is not
// drawing the strip but deciding what belongs in it and which tab survives a change of
// selection.

/** The tabs Geis has. Blender's names where the meaning matches. */
export type TabKey = "render" | "output" | "world" | "structure" | "chain" | "style" | "light";

export interface TabSpec {
  key: TabKey;
  /** A glyph, standing in for Blender's icon set. */
  icon: string;
  label: string;
}

/**
 * What the graph and the selection make available.
 *
 * Deliberately not the graph itself: the rule is about what kind of thing is selected, and
 * a test should be able to state that in one line.
 */
export interface TabContext {
  /** The selected node's kind, or null when nothing is selected. */
  selection: string | null;
  /** A molecule is in the graph, or the selection belongs to one. */
  hasMolecule: boolean;
  /** The selection is a chain, or a chain is selected in the outliner. */
  hasChain: boolean;
  hasLight: boolean;
  hasRender: boolean;
  hasImage: boolean;
  hasWorld: boolean;
}

/** Every tab, in Blender's order: scene-wide first, then the active thing's own. */
const ORDER: TabSpec[] = [
  { key: "render", icon: "◑", label: "Render" },
  { key: "output", icon: "▤", label: "Output" },
  { key: "world", icon: "○", label: "World" },
  { key: "structure", icon: "◆", label: "Structure" },
  { key: "chain", icon: "●", label: "Chain" },
  { key: "style", icon: "◕", label: "Style" },
  { key: "light", icon: "☀", label: "Light" },
];

/**
 * The tabs available right now.
 *
 * Scene-wide tabs are there whenever the node they edit is in the graph, and the rest
 * follow the selection - the same way Blender hides the Data tab when nothing is active
 * rather than showing an empty one.
 */
export function propertyTabs(context: TabContext): TabSpec[] {
  const available = new Set<TabKey>();
  if (context.hasRender) available.add("render");
  if (context.hasImage) available.add("output");
  if (context.hasWorld) available.add("world");
  if (context.hasMolecule) {
    available.add("structure");
    // Styles live on the molecule, so its slots are editable whenever it is active - a
    // chain need not be selected to name a style, just as a Blender object's material
    // slots are there without entering edit mode.
    available.add("style");
  }
  if (context.hasChain) available.add("chain");
  if (context.hasLight) available.add("light");
  return ORDER.filter((tab) => available.has(tab.key));
}

/** The tab a kind of selection is about, so clicking a thing lands on its own tab. */
export function preferredTab(selection: string | null): TabKey | null {
  switch (selection) {
    case "molecule":
      return "structure";
    case "chain":
      return "chain";
    case "style":
    case "colouring":
    case "solid-colour":
    case "material":
      return "style";
    case "light":
      return "light";
    case "world":
      return "world";
    case "render":
    case "viewport":
      return "render";
    case "image":
      return "output";
    default:
      return null;
  }
}

/**
 * Tabs that are about the same thing, so staying put makes sense.
 *
 * Structure, Chain and Style are three views of one molecule: clicking another chain while
 * looking at its style should show that chain's style, not throw you onto the Chain tab.
 * Render and Output are two halves of one output. A light and a world are their own.
 */
const FAMILIES: TabKey[][] = [
  ["structure", "chain", "style"],
  ["render", "output"],
];

function familyOf(tab: TabKey): TabKey[] {
  return FAMILIES.find((family) => family.includes(tab)) ?? [tab];
}

/**
 * Which tab to show after the selection changes.
 *
 * Blender keeps the tab you are on, and mostly that is right - but only within one
 * subject. Clicking a chain while the Render tab is open used to leave the render's
 * settings on screen, which reads as the click having been ignored. So the tab is kept
 * when it belongs to the same family as what was selected, and otherwise the selection's
 * own tab wins: pick a chain, get the chain.
 */
export function resolveTab(current: TabKey | null, tabs: TabSpec[], preferred: TabKey | null): TabKey | null {
  const keys = new Set(tabs.map((tab) => tab.key));
  const related = current !== null && preferred !== null && familyOf(current).includes(preferred);
  if (current && keys.has(current) && (related || preferred === null)) return current;
  if (preferred && keys.has(preferred)) return preferred;
  if (current && keys.has(current)) return current;
  return tabs[0]?.key ?? null;
}
