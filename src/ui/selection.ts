// The selection, as one kind of thing.
//
// BioMol's `View`: a structure, a level, and the indices of the elements in it. Everything
// that shows a selection - the outline in the viewport, the card, the BioMol panel's tree,
// the sequence strip - derives from a `View`, and everything that makes one - a click in
// the viewport, a row in the tree, a drag across the strip, a chain in the outliner -
// produces one. Before this each of those had its own path to the outline and they
// disagreed in small ways. Pure, so the merging rules (click, shift-click, cmd-click) and
// the laying of a structure's mask into the scene's id space are unit-tested.

export type Level = "chain" | "residue" | "atom";

export interface View {
  structureId: string;
  level: Level;
  indices: number[];
}

/** How a new pick combines with the current selection: the desktop's three gestures. */
export type Combine = "replace" | "add" | "toggle" | "range";

export function sameView(a: View | null, b: View | null): boolean {
  if (a === null || b === null) return a === b;
  if (a.structureId !== b.structureId || a.level !== b.level) return false;
  // As sets: equal lengths and one inside the other is not enough once an index can be
  // listed twice - [1, 1] would pass for [1, 2].
  const left = new Set(a.indices);
  const right = new Set(b.indices);
  return left.size === right.size && [...right].every((index) => left.has(index));
}

/** A key for caches: the same selection always maps to the same string. */
export function viewKey(view: View | null): string {
  if (!view) return "";
  return `${view.structureId}:${view.level}:${[...view.indices].sort((a, b) => a - b).join(",")}`;
}

/**
 * Combines a pick with the current selection.
 *
 * `replace` is a click. `add` (shift) unions. `toggle` (cmd/ctrl) flips the picked
 * elements. Any of them across a different structure or level starts over, since a view
 * is at one level of one structure - BioMol's rule, and the only one that keeps "what is
 * selected" a single answer. `range` is handled by the caller, which knows the row order.
 */
export function combine(current: View | null, pick: View, mode: Combine): View {
  if (mode === "replace" || mode === "range" || !current) return pick;
  if (current.structureId !== pick.structureId || current.level !== pick.level) return pick;
  if (mode === "add") {
    const seen = new Set(current.indices);
    return { ...pick, indices: [...current.indices, ...pick.indices.filter((index) => !seen.has(index))] };
  }
  // toggle
  const picked = new Set(pick.indices);
  const kept = current.indices.filter((index) => !picked.has(index));
  const had = new Set(current.indices);
  const added = pick.indices.filter((index) => !had.has(index));
  return { ...pick, indices: [...kept, ...added] };
}

/**
 * The elements between two positions in an ordered list of siblings, inclusive - a
 * shift-click's range in a tree or a strip. `anchor` is where the last plain click landed.
 */
export function rangeBetween<T>(ordered: T[], anchor: T, target: T): T[] {
  const a = ordered.indexOf(anchor);
  const b = ordered.indexOf(target);
  if (a < 0 || b < 0) return [target];
  const [low, high] = a <= b ? [a, b] : [b, a];
  return ordered.slice(low, high + 1);
}

/** The scene's id space for one structure: where its atoms and residues start. */
export interface ScenePart {
  atom_offset: number;
  atom_count: number;
  residue_offset: number;
  residue_count: number;
}

/**
 * Lays one structure's masks into the scene's. The scene numbers all molecules' atoms in
 * one sequence, so a structure's mask - which starts at zero - has to move to where that
 * structure's part begins; two molecules both starting at zero would light parts of the
 * wrong one.
 */
export function layMask(
  sceneAtoms: number,
  sceneResidues: number,
  part: ScenePart,
  mask: { atoms: ArrayLike<number>; residues: ArrayLike<number> },
): { atoms: Uint8Array; residues: Uint8Array } {
  const atoms = new Uint8Array(sceneAtoms);
  const residues = new Uint8Array(sceneResidues);
  // Clipped to the scene as well as to the part: a mask can arrive for a part the scene
  // no longer has room for - a structure rebuilt smaller while the request was out - and
  // a typed array's `set` past its end throws instead of drawing what does fit.
  const lay = (into: Uint8Array, from: ArrayLike<number>, offset: number, count: number) => {
    if (!(offset >= 0) || offset >= into.length) return;
    into.set(Uint8Array.from(from).subarray(0, Math.min(count, into.length - offset)), offset);
  };
  lay(atoms, mask.atoms, part.atom_offset, part.atom_count);
  lay(residues, mask.residues, part.residue_offset, part.residue_count);
  return { atoms, residues };
}
