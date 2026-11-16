import { describe, expect, it } from "vitest";
import { hexToRgb, hexToSrgb, hexToUnit, normalizeHex, rgbToHex, srgbToHex, unitToHex } from "../colorPicker";

/**
 * The core's own conversion, from `geometry::color::to_linear`.
 *
 * Duplicated here on purpose: the point of these tests is that the two implementations
 * agree, so a colour picked from a swatch and the same colour chosen from the named list
 * are the same number in the graph.
 */
function coreToLinear(srgb: [number, number, number]): [number, number, number] {
  const channel = (value: number) => {
    const unit = value / 255;
    return unit <= 0.04045 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4;
  };
  return [channel(srgb[0]), channel(srgb[1]), channel(srgb[2])];
}

describe("hexToUnit", () => {
  it("converts to linear, not to a plain fraction", () => {
    // The bug this pins: mid grey is 0.216 in linear light, not 0.5. Stored as 0.5 it
    // renders pale, because the renderer adds light in linear space.
    const [r] = hexToUnit("#808080");
    expect(r).toBeCloseTo(0.2158, 3);
    expect(r).not.toBeCloseTo(0.5019, 3);
  });

  it("agrees with the core's conversion of the same swatch", () => {
    for (const hex of ["#e78284", "#8caaee", "#a6d189", "#000000", "#ffffff", "#1b1e28"]) {
      const mine = hexToUnit(hex);
      const theirs = coreToLinear(hexToRgb(hex));
      for (let channel = 0; channel < 3; channel += 1) {
        expect(mine[channel]).toBeCloseTo(theirs[channel], 6);
      }
    }
  });

  it("keeps black and white exact", () => {
    expect(hexToUnit("#000000")).toEqual([0, 0, 0]);
    expect(hexToUnit("#ffffff")).toEqual([1, 1, 1]);
  });
});

describe("unitToHex", () => {
  it("round trips every swatch a picker can show", () => {
    for (const hex of ["#e78284", "#8caaee", "#a6d189", "#ef9f76", "#303446", "#ffffff"]) {
      expect(unitToHex(hexToUnit(hex))).toBe(hex);
    }
  });

  it("clamps rather than producing a broken hex string", () => {
    // A colour arriving out of range - a light's tint scaled up, say - must still name a
    // colour a swatch can be painted with.
    expect(unitToHex([2, -1, 0.5])).toBe("#ff00bc");
  });

  it("is the inverse of the core's linear values", () => {
    expect(unitToHex(coreToLinear(hexToRgb("#e78284")))).toBe("#e78284");
  });
});

describe("rgbToHex", () => {
  it("rounds and clamps each channel", () => {
    expect(rgbToHex(255.4, -3, 127.5)).toBe("#ff0080");
  });
});

describe("colours that are not quite colours", () => {
  it("always come out as a valid #rrggbb", () => {
    expect(rgbToHex(Number.NaN, 300, -4)).toBe("#00ff00");
    expect(rgbToHex(Infinity, -Infinity, 127.5)).toBe("#ff0080");
    expect(unitToHex([Number.NaN, 2, -1])).toBe("#00ff00");
    expect(srgbToHex([Number.NaN, 0.5, 1])).toBe("#0080ff");
  });

  it("read short, upper-case and padded hex, and refuse the rest", () => {
    expect(normalizeHex("  #ABC ")).toBe("#aabbcc");
    expect(normalizeHex("12ab3F")).toBe("#12ab3f");
    for (const bad of ["", "#", "#12345", "#1234567", "##123456", "#ggg000", "rgb(1,2,3)"]) expect(normalizeHex(bad)).toBeNull();
    expect(hexToRgb("nonsense")).toEqual([0, 0, 0]);
  });

  it("round-trip every sRGB byte through the linear floats the graph stores", () => {
    for (let value = 0; value < 256; value += 1) {
      const hex = rgbToHex(value, 255 - value, (value * 7) % 256);
      expect(unitToHex(hexToUnit(hex))).toBe(hex);
      expect(srgbToHex(hexToSrgb(hex))).toBe(hex);
    }
  });
});
