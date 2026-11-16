// The structure as a graph of its chemical components.
//
// One sphere per component - a residue, a nucleotide, a ligand, an ion, a water - standing
// where the core says that component is, and one stick per bond between two of them: the
// backbone that threads a chain, and the links that are not backbone. This file only lays
// that out as the arrays the viewer's sphere and cylinder passes take; it draws nothing.

import type { StructureMap } from "../ipc";
import type { GraphEdge, GraphView } from "./viewer";
import type { Scene } from "./scene";
import { srgbToLinear } from "./shading";
import { packLabel } from "./spheres";

/** Which components to leave out, the way the molecule's own options leave out their atoms. */
export interface GraphShow {
  waters: boolean;
  ligands: boolean;
}

/** Sphere radius by kind of component, in angstroms. A water is the least of them. */
const RADIUS: Record<string, number> = { a: 0.95, n: 1.4, l: 1.8, i: 1.4, w: 0.55 };
const BACKBONE_RADIUS = 0.28;
const LINK_RADIUS = 0.42;

/** Chain colours: distinct, and soft enough to sit beside the app's default scheme. */
const CHAIN_COLORS = [
  "#5b8fd9", "#e0884a", "#59b38c", "#c96aa8", "#8f7fd6", "#c9b04a", "#4fb3c4", "#d96a6a",
  "#7da453", "#b0855e", "#6a7fb8", "#d98fb0",
];
const KIND_COLORS: Record<string, string> = { l: "#f0a030", i: "#a66bd4", w: "#7fbfe8" };
const LINK_COLORS: Record<string, string> = { disulfide: "#e6c829", metal: "#a66bd4", covalent: "#f0a030" };
const BACKBONE_BLEND = 0.55;
const LINK_WORDS: Record<string, string> = {
  disulfide: "disulfide bridge",
  metal: "metal coordination",
  covalent: "covalent link",
};

type Rgb = [number, number, number];

/** A contact found between two components, to be drawn as an edge between them. */
export interface ContactEdge {
  /** Which structure on screen, and the two residues in it. */
  structure: number;
  residues: [number, number];
  /** `polar`, `salt-bridge`, `pi-stacking` or `hydrophobic`. */
  kind: string;
  /** In words, one of them: `hydrogen bond`. */
  what: string;
  /** Between the atoms that make it, in angstroms. */
  distance: number;
  color: Rgb;
}

export interface GraphExtras {
  /** The colour each component is drawn in on screen, by its scene residue id. */
  colours?: Map<number, Rgb>;
  contacts?: ContactEdge[];
}

const CONTACT_RADIUS = 0.2;

/**
 * The colour each residue is drawn in, read off the scene itself.
 *
 * The graph is the same structure seen another way, so a component is the colour its
 * residue has in the structure view - whatever scheme, palette or one-off colour put it
 * there - rather than a colouring of the graph's own that agrees with it only by luck.
 * A ribbon's colour is taken where there is a ribbon; otherwise the colour most of the
 * residue's atoms share, which under "heteroatoms by element" is its carbons'.
 */
export function residueColours(scene: Scene, maps: StructureMap[], parts: { atom_offset: number; residue_offset: number }[]): Map<number, Rgb> {
  const colours = new Map<number, Rgb>();
  const vertices = scene.meshVertexResidue.length;
  const stride = vertices > 0 ? Math.round(scene.meshColors.length / vertices) : 4;
  // Packed as bytes on the wire; a test or an older caller may hand floats.
  const scale = scene.meshColors instanceof Uint8Array ? 1 / 255 : 1;
  for (let vertex = 0; vertex < vertices; vertex += 1) {
    const residue = scene.meshVertexResidue[vertex];
    if (colours.has(residue)) continue;
    const at = vertex * stride;
    colours.set(residue, [scene.meshColors[at] * scale, scene.meshColors[at + 1] * scale, scene.meshColors[at + 2] * scale]);
  }

  // Per atom: a sphere's colour, or failing that the end of a stick that is the atom's.
  const atoms = new Map<number, Rgb>();
  for (let sphere = 0; sphere < scene.sphereAtoms.length; sphere += 1) {
    const at = sphere * 3;
    atoms.set(scene.sphereAtoms[sphere], [scene.sphereColors[at], scene.sphereColors[at + 1], scene.sphereColors[at + 2]]);
  }
  for (let end = 0; end < scene.cylinderAtoms.length; end += 1) {
    const atom = scene.cylinderAtoms[end];
    if (atoms.has(atom)) continue;
    const at = end * 3;
    atoms.set(atom, [scene.cylinderColors[at], scene.cylinderColors[at + 1], scene.cylinderColors[at + 2]]);
  }
  if (atoms.size === 0) return colours;

  maps.forEach((map, index) => {
    const part = parts[index];
    if (!part) return;
    for (const chain of map.chains) {
      chain.residue_indices.forEach((residue, row) => {
        const id = part.residue_offset + residue;
        if (colours.has(id)) return;
        const first = chain.first_atoms?.[row];
        const count = chain.atom_counts?.[row] ?? 0;
        if (first === undefined) return;
        const tally = new Map<string, { colour: Rgb; seen: number }>();
        let best: { colour: Rgb; seen: number } | null = null;
        for (let atom = first; atom < first + count; atom += 1) {
          const colour = atoms.get(part.atom_offset + atom);
          if (!colour) continue;
          const key = colour.map((value) => Math.round(value * 255)).join(",");
          const entry = tally.get(key) ?? { colour, seen: 0 };
          entry.seen += 1;
          tally.set(key, entry);
          if (!best || entry.seen > best.seen) best = entry;
        }
        if (best) colours.set(id, best.colour);
      });
    }
  });
  return colours;
}

function linear(hex: string): [number, number, number] {
  const value = Number.parseInt(hex.slice(1), 16);
  return [srgbToLinear(((value >> 16) & 255) / 255), srgbToLinear(((value >> 8) & 255) / 255), srgbToLinear((value & 255) / 255)];
}

/**
 * Lays the graph out for the structures on screen.
 *
 * `residueOffsets` is where each structure's residue ids start in the scene - the same
 * offsets its masks are keyed by - so a sphere's id is the id of the residue it stands for.
 */
export function buildGraphView(
  maps: StructureMap[],
  residueOffsets: number[],
  show: GraphShow[] = [],
  extras: GraphExtras = {},
): GraphView {
  const nodeLabels: number[] = [];
  const edgeInfo: GraphEdge[] = [];
  const nodes: number[] = [];
  const nodeColors: number[] = [];
  const nodeIds: number[] = [];
  const edges: number[] = [];
  const edgeColors: number[] = [];
  let chainSeen = 0;

  maps.forEach((map, at) => {
    const offset = residueOffsets[at] ?? 0;
    /** Each residue's centre and colour, by its index in the structure, for the bonds. */
    const where = new Map<
      number,
      { p: [number, number, number]; c: [number, number, number]; name: string; chain: string; kind: string }
    >();

    for (const chain of map.chains) {
      const polymer = chain.kinds.includes("a") || chain.kinds.includes("n");
      const chainColor = linear(CHAIN_COLORS[chainSeen % CHAIN_COLORS.length]);
      if (polymer) chainSeen += 1;
      chain.residue_indices.forEach((residue, row) => {
        const kind = chain.kinds[row] ?? "l";
        const p: [number, number, number] = [chain.centers[row * 3], chain.centers[row * 3 + 1], chain.centers[row * 3 + 2]];
        if (!p.every(Number.isFinite)) return;
        const shown = show[at];
        if (shown && ((kind === "w" && !shown.waters) || ((kind === "l" || kind === "i") && !shown.ligands))) return;
        // As it is drawn in the structure view, where it is drawn there at all; a component
        // that is hidden there still has a sphere here, in the graph's own colours.
        const c =
          extras.colours?.get(offset + residue) ??
          (kind === "a" || kind === "n" ? chainColor : linear(KIND_COLORS[kind] ?? KIND_COLORS.l));
        where.set(residue, { p, c, name: `${chain.names[row] ?? "?"} ${chain.numbers[row] ?? ""}`.trim(), chain: chain.id, kind });
        nodes.push(p[0], p[1], p[2], RADIUS[kind] ?? RADIUS.l);
        nodeColors.push(c[0], c[1], c[2]);
        nodeIds.push(offset + residue);
        // A residue is its one-letter code, which is what a sequence is read in; anything
        // else is its component id, which is what it is called. A water says nothing: it
        // is the smallest sphere there is and there are hundreds of them.
        const code = chain.codes[row] ?? "";
        nodeLabels.push(kind === "w" ? 0 : packLabel((kind === "a" || kind === "n") && code && code !== "X" ? code : (chain.names[row] ?? "")));
      });
    }

    const bond = (a: number, b: number, radius: number, color: [number, number, number] | null, kind = "backbone") => {
      const from = where.get(a);
      const to = where.get(b);
      if (!from || !to) return;
      edgeInfo.push({
        kind,
        structure: at,
        residues: [a, b],
        names: [from.name, to.name],
        chains: [from.chain, to.chain],
        length: Math.hypot(from.p[0] - to.p[0], from.p[1] - to.p[1], from.p[2] - to.p[2]),
        what:
          kind === "backbone"
            ? from.kind === "n" && to.kind === "n"
              ? "phosphodiester bond"
              : from.kind === "a" && to.kind === "a"
                ? "peptide bond"
                : "backbone bond"
            : (LINK_WORDS[kind] ?? "link"),
      });
      edges.push(from.p[0], from.p[1], from.p[2], to.p[0], to.p[1], to.p[2], radius);
      // A backbone bond is its chain's colour, quietened; a link is the colour of its kind.
      const ends = color ? [color, color] : [from.c, to.c].map((c) => c.map((v) => v * BACKBONE_BLEND + 0.5 * (1 - BACKBONE_BLEND)));
      edgeColors.push(...(ends[0]), ...(ends[1]));
    };
    for (const [a, b] of map.backbone ?? []) bond(a, b, BACKBONE_RADIUS, null);
    for (const link of map.links) bond(link.residues[0], link.residues[1], LINK_RADIUS, linear(LINK_COLORS[link.kind] ?? LINK_COLORS.covalent), link.kind);

    // What Find found, as edges of the same graph: one per pair of components, however
    // many contacts the pair makes - the count and the closest of them go on its card.
    const pairs = new Map<string, { contact: ContactEdge; count: number }>();
    for (const contact of extras.contacts ?? []) {
      if (contact.structure !== at || contact.residues[0] === contact.residues[1]) continue;
      const [a, b] = contact.residues[0] < contact.residues[1] ? contact.residues : [contact.residues[1], contact.residues[0]];
      const key = `${contact.kind}:${a}:${b}`;
      const seen = pairs.get(key);
      if (!seen) pairs.set(key, { contact: { ...contact, residues: [a, b] }, count: 1 });
      else {
        seen.count += 1;
        if (contact.distance < seen.contact.distance) seen.contact = { ...contact, residues: [a, b] };
      }
    }
    for (const { contact, count } of pairs.values()) {
      const before = edgeInfo.length;
      bond(contact.residues[0], contact.residues[1], CONTACT_RADIUS, contact.color, contact.kind);
      const made = edgeInfo[before];
      if (!made) continue;
      made.what = count > 1 ? `${count} ${contact.what}s` : contact.what;
      made.measured = contact.distance;
    }
  });

  return {
    nodes: new Float32Array(nodes),
    nodeColors: new Float32Array(nodeColors),
    nodeIds: new Uint32Array(nodeIds),
    nodeLabels: new Uint32Array(nodeLabels),
    edges: new Float32Array(edges),
    edgeColors: new Float32Array(edgeColors),
    edgeInfo,
  };
}
