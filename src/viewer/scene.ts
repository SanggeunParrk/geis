// Decoding the geometry payload.
//
// The Rust core builds all geometry and packs it into one ArrayBuffer; this creates typed
// array views over it rather than copying, so a 400,000-triangle cartoon costs one
// allocation - the buffer that already arrived.

export interface Block {
  offset: number;
  length: number;
}

export interface SceneHeader {
  version: number;
  representation: string;
  sphere_count: number;
  cylinder_count: number;
  vertex_count: number;
  triangle_count: number;
  drawn_atoms: number;
  /** The size of the scene's id space, for a mask keyed by atom or residue. */
  atom_count: number;
  residue_count: number;
  /** Where each merged structure's ids start, in the plan's order. */
  parts: {
    atom_offset: number;
    residue_offset: number;
    atom_count: number;
    residue_count: number;
  }[];
  bounds_min: [number, number, number];
  bounds_max: [number, number, number];
  blocks: {
    sphere_instances: Block;
    sphere_colors: Block;
    sphere_atoms: Block;
    cylinder_instances: Block;
    cylinder_colors: Block;
    cylinder_atoms: Block;
    mesh_positions: Block;
    mesh_normals: Block;
    mesh_colors: Block;
    mesh_indices: Block;
    mesh_vertex_residue: Block;
  };
}

export interface Scene {
  header: SceneHeader;
  /** x, y, z, radius per sphere. */
  sphereInstances: Float32Array;
  sphereColors: Float32Array;
  sphereAtoms: Uint32Array;
  /** x1, y1, z1, x2, y2, z2, radius per cylinder. */
  cylinderInstances: Float32Array;
  /** r1, g1, b1, r2, g2, b2 per cylinder. */
  cylinderColors: Float32Array;
  /** The two atoms at each cylinder's ends, for a mask keyed by atom. */
  cylinderAtoms: Uint32Array;
  meshPositions: Float32Array;
  /** x, y, z per vertex as signed bytes - -127 to 127 for -1 to 1 - and a byte of padding. */
  meshNormals: Int8Array;
  /** r, g, b per vertex as bytes, 255 for 1, and a byte of padding. */
  meshColors: Uint8Array;
  meshIndices: Uint32Array;
  /** The residue each vertex belongs to, for a mask keyed by residue. */
  meshVertexResidue: Uint32Array;
}

/** Must match `GEOMETRY_WIRE_VERSION` in the Rust `geometry::mesh`. */
const SUPPORTED_VERSION = 2;
const MAGIC = 0x4f45474d; // "MGEO" little-endian

export function decodeScene(buffer: ArrayBuffer): Scene {
  if (buffer.byteLength < 8) throw new Error("geometry payload is truncated");
  const view = new DataView(buffer);
  if (view.getUint32(0, true) !== MAGIC) {
    throw new Error("geometry payload has the wrong magic - the backend sent something else");
  }

  const headerLength = view.getUint32(4, true);
  if (8 + headerLength > buffer.byteLength) throw new Error("geometry payload is truncated");
  const header = JSON.parse(
    new TextDecoder().decode(new Uint8Array(buffer, 8, headerLength)),
  ) as SceneHeader;

  if (header.version !== SUPPORTED_VERSION) {
    // A silent mismatch reads one array as another, which looks like a rendering bug and
    // is nearly impossible to trace back to a stale build.
    throw new Error(
      `geometry payload is version ${header.version}, this build reads version ${SUPPORTED_VERSION}`,
    );
  }

  // The Rust side pads the header so the body starts 4-byte aligned; typed-array views
  // would throw otherwise.
  let bodyStart = 8 + headerLength;
  bodyStart += (4 - (bodyStart % 4)) % 4;

  // Said in words when a block points outside the payload: the typed array would throw
  // "invalid typed array length", which names nothing a reader could go and look at.
  const within = (block: Block) => {
    const start = bodyStart + block.offset;
    if (block.offset < 0 || block.length < 0 || block.length % 4 !== 0 || start + block.length > buffer.byteLength) {
      throw new Error(`geometry payload is truncated: a block of ${block.length} bytes at ${block.offset} runs past its end`);
    }
    return start;
  };
  const floats = (block: Block) => new Float32Array(buffer, within(block), block.length / 4);
  const uints = (block: Block) => new Uint32Array(buffer, within(block), block.length / 4);
  const signedBytes = (block: Block) => new Int8Array(buffer, within(block), block.length);
  const bytes = (block: Block) => new Uint8Array(buffer, within(block), block.length);

  const blocks = header.blocks;
  const scene: Scene = {
    header,
    sphereInstances: floats(blocks.sphere_instances),
    sphereColors: floats(blocks.sphere_colors),
    sphereAtoms: uints(blocks.sphere_atoms),
    cylinderInstances: floats(blocks.cylinder_instances),
    cylinderColors: floats(blocks.cylinder_colors),
    cylinderAtoms: uints(blocks.cylinder_atoms),
    meshPositions: floats(blocks.mesh_positions),
    meshNormals: signedBytes(blocks.mesh_normals),
    meshColors: bytes(blocks.mesh_colors),
    meshIndices: uints(blocks.mesh_indices),
    meshVertexResidue: uints(blocks.mesh_vertex_residue),
  };

  if (scene.sphereInstances.length !== header.sphere_count * 4) {
    throw new Error(
      `geometry payload is inconsistent: ${header.sphere_count} spheres but ` +
        `${scene.sphereInstances.length} floats`,
    );
  }
  if (scene.cylinderInstances.length !== header.cylinder_count * 7) {
    throw new Error(
      `geometry payload is inconsistent: ${header.cylinder_count} cylinders but ` +
        `${scene.cylinderInstances.length} floats`,
    );
  }
  if (scene.meshIndices.length !== header.triangle_count * 3) {
    throw new Error(
      `geometry payload is inconsistent: ${header.triangle_count} triangles but ` +
        `${scene.meshIndices.length} indices`,
    );
  }
  return scene;
}
