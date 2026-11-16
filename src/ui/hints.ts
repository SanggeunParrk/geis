// What each control is, in one sentence.
//
// Blender puts a tooltip on every field, and the reason is not politeness: a panel of
// twenty numbers with no explanation is a panel people change at random until the picture
// looks right. The text belongs here rather than on the node - the instruction was no
// descriptions *in* the node body, and a tooltip is where an explanation goes instead.
//
// Keyed by the label the control already shows, so a field gets its tooltip by existing
// rather than by being registered twice. Labels are unique enough in practice: "Roughness"
// means the same thing on a style and on a material.

/** One line per control, keyed by its visible label. */
const FIELDS: Record<string, string> = {
  // --- styles ---------------------------------------------------------------
  Style: "Which registered style this is. Pick another to replace it.",
  Cartoon: "The fold as a ribbon: a helix as a coil, a strand as an arrow.",
  Sticks: "Every bond as a rod, so side chains and ligands are readable.",
  Spacefill: "One sphere per atom at its van der Waals radius - the shape a molecule fills.",
  "Ball and stick": "Small spheres for atoms and rods for bonds, the chemist's drawing.",
  Backbone: "A tube along the trace, without the ribbon's twist or width.",
  "Colour by": "The rule that decides each atom's colour.",
  Palette: "Which set of distinct colours the categorical schemes hand out.",
  "Continuous scale": "Read the confidence column as a smooth ramp instead of AlphaFold's bands.",
  Ramp: "The colours a continuous scheme runs through, blue to red or otherwise.",
  "Flat colour": "One colour for everything, instead of a rule. PyMOL's `color orange`.",
  Colour: "The flat colour itself. Click the swatch to pick, or sample it from the viewport.",
  "Heteroatoms by element":
    "Keep oxygen red, nitrogen blue and sulfur yellow whatever the scheme says; only carbon takes the scheme's colour.",
  Roughness: "How diffuse the surface is. 0 is a mirror, 1 is chalk.",
  Specular: "How bright the highlight is where the light reflects.",
  Metallic: "Metals tint their reflections and have no diffuse colour of their own.",
  Slot: "Which of the molecule's styles this chain is drawn with.",

  // --- molecules ------------------------------------------------------------
  Hydrogens: "Draw hydrogens. Most crystal structures have none to draw.",
  Waters: "Draw the solvent. Usually noise, occasionally the point.",
  Ligands: "Draw anything that is not protein or nucleic acid: cofactors, ions, drugs.",
  "PDB id": "The entry this molecule was fetched from.",
  File: "The file this molecule was read from.",
  Chains: "The chains the structure turned out to have.",
  Id: "The chain identifier the structure gives, as it appears in the file.",

  // --- lighting and world ---------------------------------------------------
  Type: "A sun has no falloff, so the same energy means the same brightness at any size.",
  Energy: "Brightness. Irradiance for a sun, watts for a point or area light.",
  Softness: "The light's apparent size: larger means softer, more gradual shadows.",
  "Follow the view":
    "Keep the light over the viewer's shoulder as the structure is turned, as PyMOL does. Off pins it to the scene instead.",
  Background: "The colour behind everything, and the colour the ambient light comes from.",
  Ambient: "Light from every direction at once, standing in for a sky. Fills what no lamp reaches.",
  Occlusion: "Darken creases and pockets, which is most of what makes a surface read as a shape.",
  Outline: "Draw a dark edge around the silhouette, as a diagram would.",

  // --- render ---------------------------------------------------------------
  Engine: "Cycles traces paths and is slow and correct; EEVEE rasterises and is fast.",
  Samples: "Paths per pixel. More is less noise and more time.",
  Width: "Output width in pixels.",
  Height: "Output height in pixels. Match the viewport's shape to get its exact framing.",
  Transparent: "Leave the background empty, for a figure that goes on a coloured page.",

  // --- compute --------------------------------------------------------------
  Sequence: "One or more sequences to predict from. Plain letters or FASTA.",
  Model: "Which structure prediction model runs.",
  "Run on": "Where it runs: this machine, or a cluster.",
};

/** Buttons and other controls whose label is not a field name. */
const CONTROLS: Record<string, string> = {
  "+ Add node": "Add a node to the graph (shift+A on the plane).",
  Arrange: "Lay the graph out left to right, dependencies first (L).",
  "+ Pane": "Open another pane, for an image or a text. Drag its title to move it, its edges to resize it; or right-click the empty desk.",
  "Reset layout": "Back to the viewport over the graph, edge to edge.",
  Assign: "Draw the active chain with the slot selected above.",
  Render: "Render this node with Blender.",
  Copy: "Copy the image to the clipboard.",
  "Save as...": "Write the image somewhere.",
  Select: "Select this node in the graph editor.",
  "Add a Style node": "Give this chain a style node of its own, which overrides its slot.",
};

/** The tab strip, keyed by tab label. */
const TABS: Record<string, string> = {
  Render: "Output settings: engine, samples, size.",
  Output: "The rendered image, and what to do with it.",
  World: "Background and ambient light.",
  Structure: "The molecule itself: where it came from, what it contains, what it is drawn with.",
  Chain: "One chain of the molecule.",
  Style: "The style library. Write a style here, then assign it.",
  Light: "The selected light.",
};

/** What each kind of node does, for the node's header and the outliner. */
const NODES: Record<string, string> = {
  molecule: "A structure, fetched from the PDB or read from a file. Its chains are nodes inside it.",
  chain: "One chain, drawn with a style. Unwire it to leave it out of the picture.",
  "sequence-input": "Sequences to predict from.",
  predict: "Runs a structure prediction model, on this machine or on a cluster.",
  style: "How something is drawn: representation, colour, surface. A material, for molecules.",
  colouring: "A colouring rule, feeding a style's Colour socket.",
  "solid-colour": "One flat colour, feeding a style's Colour socket.",
  material: "A surface - roughness, specular, metallic - feeding a style's Surface socket.",
  light: "One lamp. Several can feed one viewport.",
  world: "The background and the ambient term.",
  viewport: "What the window shows. The graph's output.",
  render: "A Blender render of the view it is given.",
  image: "A rendered image on disk.",
};

/** What flows along a socket, keyed by the socket's kind label. */
const SOCKETS: Record<string, string> = {
  Sequence: "One or more sequences.",
  Structure: "A loaded structure.",
  Geometry: "A structure and how to draw it.",
  Colour: "A colouring rule.",
  Light: "One or more lamps.",
  World: "Background and ambient light.",
  Material: "A surface.",
  Style: "How something is drawn. Overrides the slot it is wired to.",
  View: "Everything a viewport shows: geometry, lighting, world, camera.",
  Image: "A rendered image on disk.",
};

/**
 * A table's own entry. Indexing a plain object directly also finds what every object
 * inherits - a field labelled "constructor" would get a function as its tooltip.
 */
function own(table: Record<string, string>, key: string): string | undefined {
  return Object.hasOwn(table, key) ? table[key] : undefined;
}

/** The tooltip for a labelled field, or undefined when there is nothing to say. */
export function fieldHint(label: string): string | undefined {
  return own(FIELDS, label) ?? own(CONTROLS, label);
}

export function tabHint(label: string): string | undefined {
  return own(TABS, label);
}

export function nodeHint(type: string): string | undefined {
  return own(NODES, type);
}

/**
 * The tooltip for a socket row: what it carries, and which way it goes.
 *
 * Built rather than looked up whole, because the name and the kind are both worth saying -
 * a socket called `All` that carries a Style is two facts.
 */
export function socketHint(name: string, kindLabel: string, direction: "in" | "out"): string {
  const what = own(SOCKETS, kindLabel) ?? kindLabel;
  const arrow = direction === "in" ? "Input" : "Output";
  return name === kindLabel ? `${arrow}: ${what}` : `${arrow} ${name} - ${what}`;
}
