import { describe, expect, it } from "vitest";
import { decodePreset } from "../presets";

function packed(words: (number | { f: number })[]): ArrayBuffer {
  const view = new DataView(new ArrayBuffer(words.length * 4));
  words.forEach((word, at) => (typeof word === "number" ? view.setUint32(at * 4, word, true) : view.setFloat32(at * 4, word.f, true)));
  return view.buffer;
}

describe("reading a preset analysis", () => {
  it("reads a set of residues", () => {
    const found = decodePreset(packed([0, 2, 3, 70000]));
    expect("residues" in found && [...found.residues]).toEqual([3, 70000]);
  });

  it("reads contacts into flat arrays", () => {
    const found = decodePreset(packed([1, 1, 4, 9, { f: 2.5 }, { f: 1 }, { f: 2 }, { f: 3 }, { f: 4 }, { f: 5 }, { f: 6 }]));
    if (!("contacts" in found)) throw new Error("expected contacts");
    expect(found.contacts.count).toBe(1);
    expect([...found.contacts.pairs]).toEqual([4, 9]);
    expect(found.contacts.distance[0]).toBe(2.5);
    expect([...found.contacts.ends]).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("refuses a buffer that is not what was written", () => {
    expect(() => decodePreset(new ArrayBuffer(4))).toThrow(/truncated/);
    expect(() => decodePreset(packed([1, 2, 4, 9]))).toThrow(/wrong length/);
    expect(() => decodePreset(packed([7, 0]))).toThrow(/unknown kind/);
  });
});
