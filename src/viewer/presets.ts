// A structure's preset analysis, as the core packs it: see `PresetResult::encode`.

/** Contacts between residues, held as flat arrays: a capsid has half a million of them. */
export interface ContactSet {
  count: number;
  /** Two residue indices per contact. */
  pairs: Uint32Array;
  /** The distance between the atoms that make each contact. */
  distance: Float32Array;
  /** x, y, z of one end and of the other, per contact. */
  ends: Float32Array;
}

export type PresetFound = { residues: Uint32Array } | { contacts: ContactSet };

export const EMPTY_CONTACTS: ContactSet = {
  count: 0,
  pairs: new Uint32Array(0),
  distance: new Float32Array(0),
  ends: new Float32Array(0),
};

/** Bytes per contact: two residue indices, the distance, and the two ends' coordinates. */
const RECORD = 36;

/** Reads what `structure_preset` sent. Throws on anything that is not the layout it writes. */
export function decodePreset(buffer: ArrayBuffer): PresetFound {
  if (buffer.byteLength < 8) throw new Error("the analysis came back truncated");
  const view = new DataView(buffer);
  const kind = view.getUint32(0, true);
  const count = view.getUint32(4, true);
  if (kind === 0) {
    if (buffer.byteLength !== 8 + count * 4) throw new Error("the analysis came back the wrong length");
    return { residues: new Uint32Array(buffer.slice(8)) };
  }
  if (kind !== 1) throw new Error(`the analysis came back as an unknown kind (${kind})`);
  if (buffer.byteLength !== 8 + count * RECORD) throw new Error("the analysis came back the wrong length");
  // Records are mixed integers and floats, so they are read out into arrays of each.
  const pairs = new Uint32Array(count * 2);
  const distance = new Float32Array(count);
  const ends = new Float32Array(count * 6);
  for (let at = 0; at < count; at += 1) {
    const base = 8 + at * RECORD;
    pairs[at * 2] = view.getUint32(base, true);
    pairs[at * 2 + 1] = view.getUint32(base + 4, true);
    distance[at] = view.getFloat32(base + 8, true);
    for (let axis = 0; axis < 6; axis += 1) ends[at * 6 + axis] = view.getFloat32(base + 12 + axis * 4, true);
  }
  return { contacts: { count, pairs, distance, ends } };
}
