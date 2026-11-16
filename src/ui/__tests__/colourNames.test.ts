import { describe, expect, it } from "vitest";
import { colourName, parseColour } from "../colourNames";

describe("colour names", () => {
  it("names a colour by the nearest name there is", () => {
    expect(colourName("#ff7f50")).toBe("coral");
    expect(colourName("#ff8055")).toBe("coral");
    expect(colourName("#44aa99")).toBe("teal");
    expect(colourName("#fefefe")).toBe("white");
  });

  it("reads a name, a hex code, or a short one", () => {
    expect(parseColour("Coral")).toBe("#ff7f50");
    expect(parseColour("sky blue")).toBe("#56b4e9");
    expect(parseColour("skyblue")).toBe("#56b4e9");
    expect(parseColour("ff7f50")).toBe("#ff7f50");
    expect(parseColour("#F80")).toBe("#ff8800");
    expect(parseColour("")).toBeNull();
    expect(parseColour("#12")).toBeNull();
  });

  it("gives the colours of a palette different names", () => {
    const soft = ["#86a8cf", "#dba77f", "#8cc0a4", "#c79bbd", "#b3a8d8", "#d3c486", "#8fc3cb", "#d49a9a"];
    expect(new Set(soft.map(colourName)).size).toBe(soft.length);
  });
});

describe("colour names, at the edges", () => {
  it("names a short code the same as its long form", () => {
    expect(colourName("#f80")).toBe(colourName("#ff8800"));
    expect(colourName("#FFF")).toBe("white");
    expect(colourName("FF7F50")).toBe("coral");
  });

  it("names whatever parseColour reads back as itself", () => {
    for (const name of ["coral", "teal", "navy", "hot pink", "sky blue", "maroon"]) {
      expect(colourName(parseColour(name)!)).toBe(name);
    }
  });

  it("refuses text that only looks like a colour", () => {
    expect(parseColour("#ff7f5")).toBeNull();
    expect(parseColour("#ff7f500")).toBeNull();
    expect(parseColour("javascript:alert(1)")).toBeNull();
    // No document here, so a CSS-only name cannot be checked and is not guessed at.
    expect(parseColour("tomato")).toBeNull();
  });
});
