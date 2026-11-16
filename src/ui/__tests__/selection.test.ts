import { describe, expect, it } from "vitest";
import { combine, layMask, rangeBetween, sameView, viewKey, type View } from "../selection";

const view = (indices: number[], level: View["level"] = "residue", structureId = "s"): View => ({
  structureId,
  level,
  indices,
});

describe("views", () => {
  it("compares as sets and keys the same set the same way", () => {
    expect(sameView(view([1, 2]), view([2, 1]))).toBe(true);
    expect(sameView(view([1]), view([1], "atom"))).toBe(false);
    expect(sameView(null, null)).toBe(true);
    expect(viewKey(view([3, 1]))).toBe(viewKey(view([1, 3])));
    expect(viewKey(null)).toBe("");
  });

  it("replaces, adds and toggles, and starts over across levels", () => {
    const current = view([1, 2, 3]);
    expect(combine(current, view([5]), "replace").indices).toEqual([5]);
    expect(combine(current, view([3, 5]), "add").indices).toEqual([1, 2, 3, 5]);
    expect(combine(current, view([3, 5]), "toggle").indices).toEqual([1, 2, 5]);
    expect(combine(current, view([7], "atom"), "add").indices).toEqual([7]);
    expect(combine(null, view([7]), "toggle").indices).toEqual([7]);
  });

  it("takes the inclusive range between an anchor and a target, either way round", () => {
    const rows = ["a", "b", "c", "d", "e"];
    expect(rangeBetween(rows, "b", "d")).toEqual(["b", "c", "d"]);
    expect(rangeBetween(rows, "d", "b")).toEqual(["b", "c", "d"]);
    expect(rangeBetween(rows, "zz", "d")).toEqual(["d"]);
  });

  it("lays a structure's mask at its part's offset in the scene", () => {
    const laid = layMask(6, 4, { atom_offset: 3, atom_count: 3, residue_offset: 2, residue_count: 2 }, {
      atoms: [1, 0, 1],
      residues: [0, 1],
    });
    expect([...laid.atoms]).toEqual([0, 0, 0, 1, 0, 1]);
    expect([...laid.residues]).toEqual([0, 0, 0, 1]);
  });
});

describe("views, at the edges", () => {
  it("does not take a list with a repeat for a different selection", () => {
    expect(sameView(view([1, 1]), view([1, 2]))).toBe(false);
    expect(sameView(view([1, 2]), view([1, 1]))).toBe(false);
    expect(sameView(view([1, 1, 2]), view([2, 1]))).toBe(true);
    expect(sameView(view([]), view([]))).toBe(true);
    expect(sameView(null, null)).toBe(true);
    expect(sameView(view([1]), null)).toBe(false);
  });

  it("lays a mask that would run past the scene up to its end, not into an exception", () => {
    const part = { atom_offset: 8, atom_count: 5, residue_offset: 3, residue_count: 2 };
    const laid = layMask(10, 4, part, { atoms: [1, 1, 1, 1, 1], residues: [1, 1] });
    expect(Array.from(laid.atoms)).toEqual([0, 0, 0, 0, 0, 0, 0, 0, 1, 1]);
    expect(Array.from(laid.residues)).toEqual([0, 0, 0, 1]);
    const gone = layMask(4, 4, { atom_offset: 9, atom_count: 1, residue_offset: 9, residue_count: 1 }, { atoms: [1], residues: [1] });
    expect(Array.from(gone.atoms)).toEqual([0, 0, 0, 0]);
  });

  it("toggles the same pick back off", () => {
    const once = combine(view([1, 2]), view([3]), "toggle");
    expect(sameView(combine(once, view([3]), "toggle"), view([1, 2]))).toBe(true);
  });
});
