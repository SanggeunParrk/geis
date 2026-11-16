import { describe, expect, it } from "vitest";
import type { StructureMap } from "../../ipc";
import { buildGraphView, residueColours } from "../graphView";
import type { Scene } from "../scene";

const map: StructureMap = {
  id: "s",
  entry_id: "1ABC",
  title: "",
  atom_count: 0,
  chains: [
    { id: "A_1", auth_id: "A", entity_type: "polymer", residue_indices: [0, 1], codes: "AG", secondary: "--", numbers: [1, 2], b_factors: [0, 0], names: ["ALA", "GLY"], centers: [0, 0, 0, 3.8, 0, 0], kinds: "aa" },
    { id: "B_1", auth_id: "A", entity_type: "non-polymer", residue_indices: [2, 3], codes: "XX", secondary: "--", numbers: [101, 201], b_factors: [0, 0], names: ["ZN", "HOH"], centers: [1, 2, 0, 9, 9, 9], kinds: "iw" },
  ],
  links: [{ residues: [0, 2], kind: "metal" }],
  backbone: [[0, 1]],
};

describe("the graph view", () => {
  it("has a sphere per component, keyed by the scene's residue ids", () => {
    const view = buildGraphView([map, map], [0, 100]);
    expect(view.nodes.length / 4).toBe(8);
    expect([...view.nodeIds]).toEqual([0, 1, 2, 3, 100, 101, 102, 103]);
    // A water is smaller than a residue, and a residue's two spheres share its chain's colour.
    expect(view.nodes[15]).toBeLessThan(view.nodes[3]);
    expect([...view.nodeColors.slice(0, 3)]).toEqual([...view.nodeColors.slice(3, 6)]);
  });

  it("joins them by the backbone and by the links", () => {
    const view = buildGraphView([map], [0]);
    expect(view.edges.length / 7).toBe(2);
    expect(view.edgeColors.length / 6).toBe(2);
    // And each says what it is, in the order they are drawn.
    expect(view.edgeInfo.map((edge) => edge.what)).toEqual(["peptide bond", "metal coordination"]);
    expect(view.edgeInfo[1]).toMatchObject({ names: ["ALA 1", "ZN 101"], chains: ["A_1", "B_1"], residues: [0, 2] });
    expect(view.edgeInfo[0].length).toBeCloseTo(3.8);
    // The backbone bond runs between the two residues' centres.
    expect([...view.edges.slice(0, 6)]).toEqual([0, 0, 0, expect.closeTo(3.8), 0, 0]);
  });

  it("leaves out a bond to a component it was not given", () => {
    const view = buildGraphView([{ ...map, backbone: [[0, 7]] }], [0]);
    expect(view.edges.length / 7).toBe(1);
  });

  it("takes each component's colour from what is drawn, where it is drawn", () => {
    const view = buildGraphView([map], [0], [], { colours: new Map([[1, [0.9, 0.1, 0.2]]]) });
    expect([...view.nodeColors.slice(3, 6)].map((value) => Number(value.toFixed(2)))).toEqual([0.9, 0.1, 0.2]);
    // The residue beside it is not drawn, and keeps the graph's own colour.
    expect([...view.nodeColors.slice(0, 3)]).not.toEqual([...view.nodeColors.slice(3, 6)]);
  });

  it("adds what Find found as edges, one per pair of components", () => {
    const contact = { structure: 0, kind: "polar", what: "hydrogen bond", color: [1, 0.8, 0.1] as [number, number, number] };
    const view = buildGraphView([map], [0], [], {
      contacts: [
        { ...contact, residues: [1, 0], distance: 3.1 },
        { ...contact, residues: [0, 1], distance: 2.8 },
        // One in another structure, and one within a single residue: neither is an edge here.
        { ...contact, structure: 1, residues: [0, 1], distance: 3 },
        { ...contact, residues: [1, 1], distance: 3 },
      ],
    });
    expect(view.edges.length / 7).toBe(3);
    const found = view.edgeInfo[2];
    expect(found.kind).toBe("polar");
    expect(found.what).toBe("2 hydrogen bonds");
    expect(found.measured).toBe(2.8);
    expect([...view.edgeColors.slice(12, 15)].map((value) => Number(value.toFixed(1)))).toEqual([1, 0.8, 0.1]);
  });

  it("reads a residue's colour off the ribbon, or off most of its atoms", () => {
    const withAtoms: StructureMap = {
      ...map,
      chains: map.chains.map((chain, index) => ({ ...chain, first_atoms: index === 0 ? [0, 5] : [9, 10], atom_counts: index === 0 ? [5, 4] : [1, 1] })),
    };
    const scene = {
      meshVertexResidue: new Uint32Array([0, 0]),
      meshColors: new Float32Array([0.1, 0.2, 0.3, 0.1, 0.2, 0.3]),
      // Residue 1's atoms: three carbons in the scheme's colour and one oxygen.
      sphereAtoms: new Uint32Array([5, 6, 7, 8]),
      sphereColors: new Float32Array([0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 1, 0, 0, 0.5, 0.5, 0.5]),
      // The ion is drawn only as the end of a stick.
      cylinderAtoms: new Uint32Array([8, 9]),
      cylinderColors: new Float32Array([0.5, 0.5, 0.5, 0.6, 0.3, 0.8]),
    } as unknown as Scene;
    const colours = residueColours(scene, [withAtoms], [{ atom_offset: 0, residue_offset: 0 }]);
    const rounded = (id: number) => colours.get(id)?.map((value) => Number(value.toFixed(1)));
    expect(rounded(0)).toEqual([0.1, 0.2, 0.3]);
    expect(rounded(1)).toEqual([0.5, 0.5, 0.5]);
    expect(rounded(2)).toEqual([0.6, 0.3, 0.8]);
    // The water is not drawn at all.
    expect(colours.has(3)).toBe(false);
  });
});
