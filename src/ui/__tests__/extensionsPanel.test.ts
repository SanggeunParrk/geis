import { describe, expect, it } from "vitest";
import { matchesExtension } from "../extensionsPanel";

const card = { name: "TM-score (US-align)", task: "custom" as never, description: "Structural alignment of two models." };

describe("searching the extensions", () => {
  it("matches every word, anywhere in the name, task or description", () => {
    expect(matchesExtension(card, "align tm")).toBe(true);
    expect(matchesExtension(card, "STRUCTURAL")).toBe(true);
    expect(matchesExtension(card, "align docking")).toBe(false);
  });

  it("matches everything when nothing is typed", () => {
    expect(matchesExtension(card, "   ")).toBe(true);
  });
});
