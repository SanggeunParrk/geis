import { describe, expect, it } from "vitest";
import type { StructureMap } from "../../ipc";
import {
  ATOM_UNITS,
  CHAIN_GAP,
  fitCamera,
  hitTest,
  layoutMap,
  mapOrigins,
  MIN_CHAIN_UNITS,
  residueWidth,
  ROWS,
  type Openings,
} from "../molmap/layout";
import { easeOutCubic, Pulse, Tween } from "../molmap/tween";

function map(): StructureMap {
  const chain = (id: string, entity_type: string, n: number, first: number) => ({
    id,
    auth_id: id[0],
    entity_type,
    residue_indices: Array.from({ length: n }, (_, i) => first + i),
    codes: "A".repeat(n),
    secondary: "H".repeat(Math.floor(n / 2)) + "-".repeat(n - Math.floor(n / 2)),
    numbers: Array.from({ length: n }, (_, i) => i + 1),
    b_factors: Array.from({ length: n }, () => 10),
    names: Array.from({ length: n }, () => "ALA"),
    centers: Array.from({ length: n * 3 }, () => 0),
    kinds: "a".repeat(n),
  });
  return {
    id: "s",
    entry_id: "1ABC",
    title: "test",
    atom_count: 100,
    chains: [chain("A_1", "polymer", 100, 0), chain("B_1", "polymer", 50, 100), chain("H_1", "non-polymer", 1, 150), chain("W_1", "water", 30, 151)],
    links: [{ residues: [10, 120], kind: "disulfide" }, { residues: [10, 151], kind: "metal" }],
    backbone: [],
  };
}

const closed: Openings = { open: new Map(), atoms: new Map() };
const counts = new Map<number, number>([[5, 5]]);

describe("the layered map's layout", () => {
  it("spans a parent over its children and leaves waters out", () => {
    const layout = layoutMap(map(), closed, counts);
    expect(layout.chainCells.map((cell) => cell.label)).toEqual(["A_1", "B_1", "H_1"]);
    const [a, b, h] = layout.chainCells;
    expect(a.w).toBe(100);
    expect(b.x).toBeCloseTo(100 + CHAIN_GAP);
    // The one-residue ligand is widened to the floor, and so is its residue.
    expect(h.w).toBe(MIN_CHAIN_UNITS);
    expect(layout.cellByResidue.get(150)!.w).toBe(MIN_CHAIN_UNITS);
    // The molecule bar spans everything.
    expect(layout.cells[0].w).toBeCloseTo(layout.width);
    expect(layout.width).toBeCloseTo(100 + CHAIN_GAP + 50 + CHAIN_GAP + MIN_CHAIN_UNITS);
    // Layers are stacked in order.
    expect(layout.cells[0].y).toBeLessThan(a.y);
    expect(a.y).toBeLessThan(layout.residueCells[0].y);
    expect(layout.cellByResidue.get(0)!.colour).toBe("#fa5252");
  });

  it("opens a residue into its atoms and pushes its neighbours along", () => {
    const openings: Openings = {
      open: new Map([[5, 1]]),
      atoms: new Map([
        [
          5,
          {
            atoms: [
              { index: 20, name: "N", element: "N", b_factor: 1 },
              { index: 21, name: "CA", element: "C", b_factor: 1 },
              { index: 22, name: "C", element: "C", b_factor: 1 },
              { index: 23, name: "O", element: "O", b_factor: 1 },
              { index: 24, name: "CB", element: "C", b_factor: 1 },
            ],
            bonds: [[20, 21], [21, 22], [22, 23], [21, 24]],
          },
        ],
      ]),
    };
    const layout = layoutMap(map(), openings, counts);
    const open = layout.cellByResidue.get(5)!;
    expect(open.w).toBeCloseTo(5 * ATOM_UNITS);
    expect(residueWidth(5, 0.5)).toBeCloseTo(1 + (5 * ATOM_UNITS - 1) * 0.5);
    // The next residue starts where the open one ends; the chain grew by the difference.
    expect(layout.cellByResidue.get(6)!.x).toBeCloseTo(5 + 5 * ATOM_UNITS);
    expect(layout.chainCells[0].w).toBeCloseTo(100 - 1 + 5 * ATOM_UNITS);
    expect(layout.atomCells).toHaveLength(5);
    expect(layout.atomCells[0]).toMatchObject({ label: "N", parent: 5, y: ROWS.atom.y });
    expect(layout.bonds).toHaveLength(4);
    // Arcs hang below the atom layer once one is open.
    expect(layout.arcs[0].y).toBe(ROWS.arcTop);
  });

  it("draws arcs only between residues that are on the map, higher the further apart", () => {
    const layout = layoutMap(map(), closed, counts);
    // The metal link to a water is dropped with the water.
    expect(layout.arcs).toHaveLength(1);
    expect(layout.arcs[0].kind).toBe("disulfide");
    expect(layout.arcs[0].x1).toBeCloseTo(10.5);
    expect(layout.arcs[0].x2).toBeCloseTo(100 + CHAIN_GAP + 20.5);
    expect(layout.arcs[0].height).toBeGreaterThan(2);
    expect(layout.arcs[0].height).toBeLessThanOrEqual(ROWS.arcMaxHeight);
  });

  it("hit-tests deepest layer first", () => {
    const layout = layoutMap(map(), closed, counts);
    expect(hitTest(layout, 3.5, ROWS.residue.y + 1)?.index).toBe(3);
    expect(hitTest(layout, 3.5, ROWS.chain.y + 1)).toMatchObject({ level: "chain", index: 0 });
    expect(hitTest(layout, 3.5, ROWS.molecule.y + 1)?.level).toBe("molecule");
    expect(hitTest(layout, 100.5, ROWS.chain.y + 1)).toBeNull();
    expect(hitTest(layout, 3.5, ROWS.arcTop + 5)).toBeNull();
  });

  it("fits a camera to a box and stacks maps", () => {
    const camera = fitCamera({ x: 0, y: 0, w: 200, h: 40 }, 800, 400, 0);
    expect(camera.k).toBeCloseTo(4);
    expect(camera.x).toBeCloseTo(0);
    expect(camera.y).toBeCloseTo(200 - 80);
    expect(mapOrigins([30, 20], 10)).toEqual([{ x: 0, y: 0 }, { x: 0, y: 40 }]);
  });
});

describe("tweens", () => {
  it("moves from the current value to the target over the duration, eased", () => {
    const tween = new Tween(0);
    tween.go(10, 1000, 100);
    expect(tween.at(1000)).toBe(0);
    expect(tween.at(1050)).toBeCloseTo(10 * easeOutCubic(0.5));
    expect(tween.at(1100)).toBe(10);
    expect(tween.done(1099)).toBe(false);
    expect(tween.done(1100)).toBe(true);
    tween.go(0, 1050, 100);
    expect(tween.at(1050)).toBeCloseTo(10 * easeOutCubic(0.5));
  });

  it("pulses once", () => {
    const pulse = new Pulse();
    expect(pulse.active(0)).toBe(false);
    pulse.fire(100, 200);
    expect(pulse.phase(200)).toBeCloseTo(0.5);
    expect(pulse.active(299)).toBe(true);
    expect(pulse.active(300)).toBe(false);
  });
});

describe("the map at the edges", () => {
  it("never mirrors the map in a pane smaller than its padding", () => {
    const camera = fitCamera({ x: 0, y: 0, w: 100, h: 20 }, 0, 0);
    expect(camera.k).toBeGreaterThan(0);
    expect(Number.isFinite(camera.x) && Number.isFinite(camera.y)).toBe(true);
    // An empty box is not a division by zero.
    expect(Number.isFinite(fitCamera({ x: 0, y: 0, w: 0, h: 0 }, 300, 200).k)).toBe(true);
  });

  it("ends a pulse of no length at once, rather than at NaN", () => {
    const pulse = new Pulse();
    pulse.fire(100, 0);
    expect(pulse.phase(100)).toBe(1);
    expect(pulse.active(100)).toBe(false);
  });

  it("holds a tween of no duration at its target", () => {
    const tween = new Tween(0);
    tween.go(10, 50, 0);
    expect(tween.at(50)).toBe(10);
    expect(tween.done(50)).toBe(true);
  });
});
