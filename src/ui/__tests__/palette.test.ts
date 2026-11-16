import { describe, expect, it } from "vitest";
import { COMMANDS } from "../../commands";
import { matchScore, rankItems } from "../palette";

describe("the command palette's search", () => {
  it("finds a command from the first letters of its words", () => {
    const found = rankItems("hs", COMMANDS);
    expect(found[0].id).toBe("hide.sticks");
  });

  it("puts an exact word ahead of the same letters scattered", () => {
    const found = rankItems("zoom", COMMANDS).map((command) => command.id);
    expect(found.slice(0, 2).sort()).toEqual(["zoom.all", "zoom.selection"]);
  });

  it("matches on the group as well, behind matches on the name", () => {
    const found = rankItems("file", COMMANDS).map((command) => command.id);
    expect(found).toContain("fetch");
    // "Open a file..." has the word itself and comes first.
    expect(found[0]).toBe("open");
  });

  it("ignores case and spaces in what is typed", () => {
    expect(matchScore("Hydrogen Bonds", "Find hydrogen bonds")).not.toBeNull();
  });

  it("does not match letters out of order", () => {
    expect(matchScore("skcits", "Show sticks")).toBeNull();
  });

  it("lists everything, in its own order, when nothing is typed", () => {
    expect(rankItems("  ", COMMANDS)).toEqual(COMMANDS);
  });

  it("offers nothing for a query that matches nothing", () => {
    expect(rankItems("qqqq", COMMANDS)).toEqual([]);
  });
});
