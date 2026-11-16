// The layered map's geometry, apart from its drawing.
//
// A molecule as layers, top to bottom: the molecule as one bar, its chains as cells across
// the bar's width, their residues as cells across each chain's width, and - for a residue
// that has been opened - its atoms as cells across the residue's. A parent's cell literally
// spans its children, so belonging is read at a glance (an icicle partition); order runs
// left to right, so the residue layer is the sequence. Links that are not the backbone are
// arcs beneath the layers between the cells they join (an arc diagram): a link between
// neighbours is a low arc, one across the molecule a high one.
//
// Pure: given the structure's map, which residues are open and how far (an animation
// hands in fractions), it returns cells and arcs in world units, and it hit-tests them. The
// renderer only draws.

import type { AtomRow, StructureMap } from "../../ipc";

/** World units. One residue is one unit wide when closed; the camera scales the rest. */
export const UNIT = 1;
/** A ligand of one residue still gets a cell wide enough to see and click. */
export const MIN_CHAIN_UNITS = 3;
export const CHAIN_GAP = 1.2;
export const ATOM_UNITS = 0.75;

export const ROWS = {
  molecule: { y: 0, h: 5 },
  chain: { y: 6.2, h: 5.5 },
  residue: { y: 12.9, h: 6.5 },
  atom: { y: 20.6, h: 5 },
  /** Arcs hang below the deepest layer. */
  arcTop: 27,
  arcMaxHeight: 14,
} as const;

export type CellLevel = "molecule" | "chain" | "residue" | "atom";

export interface Cell {
  level: CellLevel;
  /** Structure index at the level: chain index, residue index or atom index; -1 for the molecule. */
  index: number;
  x: number;
  w: number;
  y: number;
  h: number;
  colour: string;
  label: string;
  /** For a residue: its chain; for an atom: its residue. */
  parent: number;
  /** Extra text: the residue's number, the atom's element, the chain's auth id. */
  detail: string;
  entityType?: string;
  /** For a residue cell: how far open it is, 0 closed to 1 fully open. */
  open?: number;
}

export interface Arc {
  x1: number;
  x2: number;
  /** In world units above the arc's baseline. */
  height: number;
  kind: string;
  residues: [number, number];
  /** Where the arc starts and ends vertically: the residue layer's bottom, or an atom's. */
  y: number;
}

export interface MapLayout {
  structureId: string;
  entryId: string;
  atomCount: number;
  /** Total width in world units. */
  width: number;
  /** Total height, arcs included. */
  height: number;
  cells: Cell[];
  chainCells: Cell[];
  residueCells: Cell[];
  atomCells: Cell[];
  cellByResidue: Map<number, Cell>;
  cellByAtom: Map<number, Cell>;
  arcs: Arc[];
  /** Bonds among the atoms of open residues, as arcs of their own. */
  bonds: Arc[];
}

export const SECONDARY_COLOURS: Record<string, string> = {
  H: "#fa5252",
  E: "#fab005",
  "-": "#6f7a92",
};

export const ELEMENT_COLOURS: Record<string, string> = {
  C: "#9aa3b5",
  N: "#4c8dff",
  O: "#ff5a5a",
  S: "#f2c230",
  P: "#ff9a3c",
  H: "#d8dde8",
  FE: "#e06c3a",
  ZN: "#8fa4c0",
  MG: "#5fd18a",
};

export function elementColour(symbol: string): string {
  return ELEMENT_COLOURS[symbol.toUpperCase()] ?? "#c48cff";
}

/** A hue sweep over the polymer chains - the chainbow the viewport can draw. */
export function chainColour(polymerIndex: number, polymerCount: number, entityType: string): string {
  if (entityType !== "polymer") return "#d9a34a";
  const hue = 210 - (polymerCount <= 1 ? 0 : (polymerIndex / (polymerCount - 1)) * 210);
  return `hsl(${hue.toFixed(0)} 70% 58%)`;
}

/** What is open: residue index → 0..1, and the atoms fetched for it. */
export interface Openings {
  open: Map<number, number>;
  atoms: Map<number, { atoms: AtomRow[]; bonds: [number, number][] }>;
}

/** How wide an open residue is: room for its atoms, scaled by how far open it is. */
export function residueWidth(atomCount: number, open: number): number {
  const opened = Math.max(UNIT, atomCount * ATOM_UNITS);
  return UNIT + (opened - UNIT) * open;
}

/**
 * Lays a structure out. Waters are left out, as the viewport leaves them out by default.
 * Chains take their residues' widths plus a gap; a residue takes one unit, or room for its
 * atoms when open; everything above spans what is below it.
 */
export function layoutMap(map: StructureMap, openings: Openings, residueAtomCounts: Map<number, number>): MapLayout {
  const shown = map.chains
    .map((chain, chainIndex) => ({ chain, chainIndex }))
    .filter(({ chain }) => chain.entity_type !== "water" && chain.residue_indices.length > 0);
  const polymers = shown.filter(({ chain }) => chain.entity_type === "polymer");

  const cells: Cell[] = [];
  const chainCells: Cell[] = [];
  const residueCells: Cell[] = [];
  const atomCells: Cell[] = [];
  const cellByResidue = new Map<number, Cell>();
  const cellByAtom = new Map<number, Cell>();
  const bonds: Arc[] = [];
  let cursor = 0;

  shown.forEach(({ chain, chainIndex }) => {
    const start = cursor;
    const polymerIndex = polymers.findIndex((entry) => entry.chainIndex === chainIndex);
    const colour = chainColour(Math.max(polymerIndex, 0), polymers.length, chain.entity_type);
    const first: Cell[] = [];
    chain.residue_indices.forEach((residueIndex, position) => {
      const open = openings.open.get(residueIndex) ?? 0;
      const atomCount = residueAtomCounts.get(residueIndex) ?? 0;
      const w = residueWidth(atomCount, open);
      const cell: Cell = {
        level: "residue",
        index: residueIndex,
        x: cursor,
        w,
        y: ROWS.residue.y,
        h: ROWS.residue.h,
        colour: SECONDARY_COLOURS[chain.secondary[position] ?? "-"] ?? SECONDARY_COLOURS["-"],
        label: chain.codes[position] ?? "X",
        parent: chainIndex,
        detail: `${chain.names[position] ?? ""} ${chain.numbers[position] ?? ""}`.trim(),
        open,
      };
      first.push(cell);
      cellByResidue.set(residueIndex, cell);
      // Atoms of an open residue, laid across it, fading in with the opening.
      const opened = openings.atoms.get(residueIndex);
      if (open > 0 && opened) {
        const each = w / Math.max(opened.atoms.length, 1);
        opened.atoms.forEach((atom, at) => {
          const atomCell: Cell = {
            level: "atom",
            index: atom.index,
            x: cursor + each * at,
            w: each,
            y: ROWS.atom.y,
            h: ROWS.atom.h * open,
            colour: elementColour(atom.element),
            label: atom.name,
            parent: residueIndex,
            detail: atom.element,
            open,
          };
          atomCells.push(atomCell);
          cellByAtom.set(atom.index, atomCell);
        });
        for (const [a, b] of opened.bonds) {
          const ca = cellByAtom.get(a);
          const cb = cellByAtom.get(b);
          if (!ca || !cb) continue;
          const x1 = ca.x + ca.w / 2;
          const x2 = cb.x + cb.w / 2;
          bonds.push({
            x1: Math.min(x1, x2),
            x2: Math.max(x1, x2),
            height: Math.min(3, Math.abs(x2 - x1) * 0.6) * open,
            kind: "bond",
            residues: [residueIndex, residueIndex],
            y: ROWS.atom.y + ROWS.atom.h * open,
          });
        }
      }
      cursor += w;
    });
    // A ligand of one residue is widened to the floor, its residue with it.
    const natural = cursor - start;
    if (natural < MIN_CHAIN_UNITS) {
      const scale = MIN_CHAIN_UNITS / natural;
      for (const cell of first) {
        cell.x = start + (cell.x - start) * scale;
        cell.w *= scale;
      }
      cursor = start + MIN_CHAIN_UNITS;
    }
    residueCells.push(...first);
    const chainCell: Cell = {
      level: "chain",
      index: chainIndex,
      x: start,
      w: cursor - start,
      y: ROWS.chain.y,
      h: ROWS.chain.h,
      colour,
      label: chain.id,
      parent: -1,
      detail: `auth ${chain.auth_id} · ${chain.entity_type} · ${chain.residue_indices.length} res`,
      entityType: chain.entity_type,
    };
    chainCells.push(chainCell);
    cursor += CHAIN_GAP;
  });
  const width = Math.max(cursor - CHAIN_GAP, MIN_CHAIN_UNITS);

  const molecule: Cell = {
    level: "molecule",
    index: -1,
    x: 0,
    w: width,
    y: ROWS.molecule.y,
    h: ROWS.molecule.h,
    colour: "#2a3550",
    label: map.entry_id.toUpperCase(),
    parent: -1,
    detail: `${shown.length} chains · ${map.atom_count.toLocaleString()} atoms`,
  };
  cells.push(molecule, ...chainCells, ...residueCells, ...atomCells);

  const arcs: Arc[] = [];
  for (const link of map.links) {
    const a = cellByResidue.get(link.residues[0]);
    const b = cellByResidue.get(link.residues[1]);
    if (!a || !b) continue;
    const x1 = a.x + a.w / 2;
    const x2 = b.x + b.w / 2;
    const span = Math.abs(x2 - x1);
    arcs.push({
      x1: Math.min(x1, x2),
      x2: Math.max(x1, x2),
      height: Math.min(ROWS.arcMaxHeight, 2 + Math.sqrt(span) * 1.1),
      kind: link.kind,
      residues: link.residues,
      y: ROWS.arcTop,
    });
  }
  const deepest = atomCells.length > 0 ? ROWS.arcTop : ROWS.residue.y + ROWS.residue.h + 1.5;
  for (const arc of arcs) arc.y = deepest;
  const height = deepest + (arcs.length > 0 ? ROWS.arcMaxHeight + 1 : 0);

  return {
    structureId: map.id,
    entryId: map.entry_id,
    atomCount: map.atom_count,
    width,
    height,
    cells,
    chainCells,
    residueCells,
    atomCells,
    cellByResidue,
    cellByAtom,
    arcs,
    bonds,
  };
}

/** The cell under a point in the map's own coordinates, deepest level first. */
export function hitTest(layout: MapLayout, x: number, y: number): Cell | null {
  const inside = (cell: Cell) => x >= cell.x && x < cell.x + cell.w && y >= cell.y && y < cell.y + cell.h;
  for (const list of [layout.atomCells, layout.residueCells, layout.chainCells]) {
    // Cells in a row are ordered by x, so a binary search would do; rows are short enough
    // that a scan is fine until a ribosome says otherwise.
    const hit = list.find(inside);
    if (hit) return hit;
  }
  const molecule = layout.cells[0];
  return molecule && inside(molecule) ? molecule : null;
}

/** A camera: where world (0,0) lands on the canvas, and the scale. */
export interface Camera {
  x: number;
  y: number;
  k: number;
}

/** The camera that shows a world-space box. */
export function fitCamera(box: { x: number; y: number; w: number; h: number }, width: number, height: number, padding = 18): Camera {
  // At least a pixel of room: a pane folded smaller than its padding would otherwise give a
  // negative scale, and the map would be drawn mirrored and hit-tested back to front.
  const room = (size: number) => Math.max(size - padding * 2, 1);
  const k = Math.min(room(width) / Math.max(box.w, 1e-6), room(height) / Math.max(box.h, 1e-6));
  return {
    k,
    x: width / 2 - (box.x + box.w / 2) * k,
    y: height / 2 - (box.y + box.h / 2) * k,
  };
}

/** Where a set of maps sit: stacked, each below the last with a gap. */
export function mapOrigins(heights: number[], gap = 10): { x: number; y: number }[] {
  let y = 0;
  return heights.map((height) => {
    const origin = { x: 0, y };
    y += height + gap;
    return origin;
  });
}
