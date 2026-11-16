// The geometry payload decoder.
//
// This is a wire format between two languages. The failure it guards against is silent:
// a block offset off by one array reads positions as colours, which looks like a
// rendering bug and takes hours to trace back to a layout change.

import { describe, expect, it } from "vitest";
import { decodeScene } from "../scene";
import { decodePick } from "../shading";

interface Block {
  offset: number;
  length: number;
}

/** Builds a payload the same way the Rust `Scene::encode` does. */
function encode(options: {
  version?: number;
  magic?: number;
  spheres?: number[];
  sphereColors?: number[];
  sphereAtoms?: number[];
  meshPositions?: number[];
  meshIndices?: number[];
  meshVertexResidue?: number[];
  triangleCountOverride?: number;
}): ArrayBuffer {
  const spheres = options.spheres ?? [];
  const sphereColors = options.sphereColors ?? [];
  const sphereAtoms = options.sphereAtoms ?? [];
  const meshPositions = options.meshPositions ?? [];
  const meshIndices = options.meshIndices ?? [];

  const parts: { floats?: number[]; uints?: number[] }[] = [
    { floats: spheres },
    { floats: sphereColors },
    { uints: sphereAtoms },
    { floats: [] }, // cylinder instances
    { floats: [] }, // cylinder colours
    { uints: [] }, // cylinder atoms
    { floats: meshPositions },
    // Four bytes a vertex each, packed: one word per vertex.
    { uints: meshPositions.filter((_, index) => index % 3 === 0).map(() => 0) }, // normals
    { uints: meshPositions.filter((_, index) => index % 3 === 0).map(() => 0xffffffff) }, // colours
    { uints: meshIndices },
    { uints: options.meshVertexResidue ?? [] },
  ];

  const body: number[] = [];
  const blocks: Block[] = [];
  const bytes = new Uint8Array(parts.reduce((sum, part) => sum + ((part.floats ?? part.uints)!.length * 4), 0));
  const bodyView = new DataView(bytes.buffer);
  let cursor = 0;
  for (const part of parts) {
    const start = cursor;
    for (const value of part.floats ?? []) {
      bodyView.setFloat32(cursor, value, true);
      cursor += 4;
    }
    for (const value of part.uints ?? []) {
      bodyView.setUint32(cursor, value, true);
      cursor += 4;
    }
    blocks.push({ offset: start, length: cursor - start });
  }
  void body;

  const header = {
    version: options.version ?? 2,
    representation: "cartoon",
    sphere_count: spheres.length / 4,
    cylinder_count: 0,
    vertex_count: meshPositions.length / 3,
    triangle_count: options.triangleCountOverride ?? meshIndices.length / 3,
    drawn_atoms: spheres.length / 4,
    // The id space the masks are keyed by, and where each merged part starts in it.
    atom_count: sphereAtoms.length,
    residue_count: (options.meshVertexResidue ?? []).length,
    parts: [
      {
        atom_offset: 0,
        residue_offset: 0,
        atom_count: sphereAtoms.length,
        residue_count: (options.meshVertexResidue ?? []).length,
      },
    ],
    bounds_min: [0, 0, 0],
    bounds_max: [1, 1, 1],
    blocks: {
      sphere_instances: blocks[0],
      sphere_colors: blocks[1],
      sphere_atoms: blocks[2],
      cylinder_instances: blocks[3],
      cylinder_colors: blocks[4],
      cylinder_atoms: blocks[5],
      mesh_positions: blocks[6],
      mesh_normals: blocks[7],
      mesh_colors: blocks[8],
      mesh_indices: blocks[9],
      mesh_vertex_residue: blocks[10],
    },
  };

  const headerBytes = new TextEncoder().encode(JSON.stringify(header));
  let headerEnd = 8 + headerBytes.length;
  const padding = (4 - (headerEnd % 4)) % 4;
  headerEnd += padding;

  const payload = new Uint8Array(headerEnd + bytes.length);
  const view = new DataView(payload.buffer);
  view.setUint32(0, options.magic ?? 0x4f45474d, true);
  view.setUint32(4, headerBytes.length, true);
  payload.set(headerBytes, 8);
  payload.set(bytes, headerEnd);
  return payload.buffer;
}

describe("decodePick", () => {
  it("tells an atom id from a residue id", () => {
    // The viewport can only report what its geometry knows - a sphere knows its atom, a
    // ribbon vertex knows its residue - so the tag is what stops the two being confused.
    const encode = (kind: number, index: number) => {
      const tagged = ((kind << 30) | (index & 0x3fffffff)) >>> 0;
      return new Uint8Array([
        tagged & 0xff,
        (tagged >> 8) & 0xff,
        (tagged >> 16) & 0xff,
        (tagged >> 24) & 0xff,
      ]);
    };
    expect(decodePick(encode(1, 4460))).toEqual({ kind: "atom", index: 4460 });
    expect(decodePick(encode(2, 237))).toEqual({ kind: "residue", index: 237 });
    // Index zero is a real atom, not "nothing": the tag is what says nothing was hit.
    expect(decodePick(encode(1, 0))).toEqual({ kind: "atom", index: 0 });
    // The cleared background decodes to nothing, which is how a click on empty space
    // clears the selection.
    expect(decodePick(new Uint8Array([0, 0, 0, 0]))).toBeNull();
  });

  it("survives the largest id the encoding can hold", () => {
    // Thirty bits, so a billion atoms - far past anything the tessellator will draw, but
    // an id that silently wraps would highlight the wrong thing.
    const index = 0x3fffffff;
    const tagged = ((1 << 30) | index) >>> 0;
    const pixel = new Uint8Array([
      tagged & 0xff,
      (tagged >> 8) & 0xff,
      (tagged >> 16) & 0xff,
      (tagged >> 24) & 0xff,
    ]);
    expect(decodePick(pixel)).toEqual({ kind: "atom", index });
  });
});

describe("decodeScene", () => {
  it("reads the ids the selection mask is keyed by", () => {
    // The masks are indexed by these: an off-by-one block here highlights the wrong atoms
    // rather than failing, which is exactly the sort of silent wire bug this file exists
    // for.
    const scene = decodeScene(
      encode({
        spheres: [1, 2, 3, 1.7, 4, 5, 6, 1.5],
        sphereColors: [1, 0, 0, 0, 1, 0],
        sphereAtoms: [10, 11],
        meshPositions: [7, 8, 9],
        meshIndices: [0, 0, 0],
        meshVertexResidue: [4],
      }),
    );
    expect(Array.from(scene.sphereAtoms)).toEqual([10, 11]);
    expect(Array.from(scene.meshVertexResidue)).toEqual([4]);
    expect(scene.header.atom_count).toBe(2);
    expect(scene.header.residue_count).toBe(1);
    expect(scene.header.parts[0].atom_offset).toBe(0);
  });

  it("reads spheres and mesh back out", () => {
    const scene = decodeScene(
      encode({
        spheres: [1, 2, 3, 1.7, 4, 5, 6, 1.5],
        sphereColors: [1, 0, 0, 0, 1, 0],
        sphereAtoms: [10, 11],
        meshPositions: [7, 8, 9],
        meshIndices: [0, 0, 0],
      }),
    );

    expect(scene.header.sphere_count).toBe(2);
    // Radii round-trip through f32, so 1.7 comes back as the nearest float.
    expect([...scene.sphereInstances.slice(0, 3)]).toEqual([1, 2, 3]);
    expect(scene.sphereInstances[3]).toBeCloseTo(1.7, 5);
    expect([...scene.sphereAtoms]).toEqual([10, 11]);
    expect([...scene.meshPositions]).toEqual([7, 8, 9]);
    expect(scene.header.triangle_count).toBe(1);
  });

  it("survives an empty scene", () => {
    // A cartoon of a structure with nothing to draw is legitimate, not an error.
    const scene = decodeScene(encode({}));
    expect(scene.header.sphere_count).toBe(0);
    expect(scene.meshIndices.length).toBe(0);
  });

  it("rejects a payload from a different build", () => {
    // Reading version 1 blocks - floats - as version 2 bytes would render garbage silently.
    expect(() => decodeScene(encode({ version: 1 }))).toThrow(/version 1/);
  });

  it("rejects something that is not a geometry payload", () => {
    expect(() => decodeScene(encode({ magic: 0x12345678 }))).toThrow(/magic/);
    expect(() => decodeScene(new ArrayBuffer(4))).toThrow(/truncated/);
  });

  it("catches a header that disagrees with its blocks", () => {
    // The off-by-one-array failure this whole test file exists for.
    expect(() =>
      decodeScene(encode({ meshIndices: [0, 0, 0], triangleCountOverride: 7 })),
    ).toThrow(/inconsistent/);
  });
});

describe("decodeScene, given a payload cut short", () => {
  it("says the payload is truncated rather than throwing a bare range error", () => {
    const whole = encode({ spheres: [0, 0, 0, 1, 1, 1, 1, 1], sphereColors: new Array(6).fill(1), sphereAtoms: [0, 1] });
    expect(() => decodeScene(whole.slice(0, whole.byteLength - 8))).toThrow(/truncated/);
    // A header length that runs past the end.
    const header = new Uint8Array(whole.slice(0, 16));
    expect(() => decodeScene(header.buffer)).toThrow(/truncated/);
  });

  it("catches a cylinder count that disagrees with its block", () => {
    const payload = encode({});
    const view = new DataView(payload);
    const length = view.getUint32(4, true);
    const text = new TextDecoder().decode(new Uint8Array(payload, 8, length)).replace('"cylinder_count":0', '"cylinder_count":3');
    const bytes = new TextEncoder().encode(text);
    // Same length (one digit for one), so the body offsets still line up.
    expect(bytes.length).toBe(length);
    new Uint8Array(payload, 8, length).set(bytes);
    expect(() => decodeScene(payload)).toThrow(/3 cylinders/);
  });
});
