import { describe, expect, it } from "vitest";
import { fieldHint, nodeHint, socketHint, tabHint } from "../hints";

describe("tooltips", () => {
  it("say nothing for a label that is only an inherited property name", () => {
    for (const name of ["constructor", "toString", "__proto__", "hasOwnProperty"]) {
      expect(fieldHint(name)).toBeUndefined();
      expect(tabHint(name)).toBeUndefined();
      expect(nodeHint(name)).toBeUndefined();
      expect(socketHint("x", name, "in")).toBe(`Input x - ${name}`);
    }
  });

  it("name a socket and what it carries, once when they are the same", () => {
    expect(socketHint("Style", "Style", "out")).toMatch(/^Output: /);
    expect(socketHint("All", "Style", "in")).toMatch(/^Input All - /);
  });
});
