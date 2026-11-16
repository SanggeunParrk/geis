import { describe, expect, it } from "vitest";
import { COMMANDS, DEFAULT_MENU } from "../commands";
import { DEFAULT_PREFERENCES, mergePreferences } from "../prefs";

describe("preferences", () => {
  it("follow a default that changed, where the old one was never chosen", () => {
    const old = mergePreferences({ background: "#eceef1", palette: "okabe-ito", scheme: "chain" });
    expect(old.background).toBe(DEFAULT_PREFERENCES.background);
    expect(old.palette).toBe(DEFAULT_PREFERENCES.palette);
    expect(old.scheme).toBe("chain");
    // Chosen again since, and saved with the version: it is kept.
    expect(mergePreferences({ palette: "okabe-ito", version: 2 }).palette).toBe("okabe-ito");
  });

  it("are the defaults when nothing was saved", () => {
    expect(mergePreferences(null)).toEqual(DEFAULT_PREFERENCES);
    expect(mergePreferences({})).toEqual(DEFAULT_PREFERENCES);
  });

  it("take what was saved and keep the default for what was not", () => {
    const merged = mergePreferences({ palette: "tableau10", methods: { align: "tm-align" } });
    expect(merged.palette).toBe("tableau10");
    expect(merged.methods).toEqual({ align: "tm-align", alignAtoms: "ca", secondary: "file" });
    expect(merged.background).toBe(DEFAULT_PREFERENCES.background);
  });

  it("ignore a value they do not understand rather than keep it", () => {
    const merged = mergePreferences({ palette: "neon", background: "blue", selectLevel: "link", showWaters: "yes" });
    expect(merged).toEqual(DEFAULT_PREFERENCES);
  });
});

describe("preferences from a hand-edited or older store", () => {
  it("leave the object they were given as it was", () => {
    const saved = { background: "#eceef1", palette: "okabe-ito" };
    mergePreferences(saved);
    expect(saved).toEqual({ background: "#eceef1", palette: "okabe-ito" });
  });

  it("drop a menu command that no longer exists and keep the rest", () => {
    const merged = mergePreferences({ contextMenu: ["zoom.all", "show.ribbon", 7, "fetch", "zoom.all"], version: 2 });
    expect(merged.contextMenu).toEqual(["zoom.all", "fetch"]);
  });

  it("keep a menu the user emptied, but not one emptied only by forgotten commands", () => {
    expect(mergePreferences({ contextMenu: [] }).contextMenu).toEqual([]);
    expect(mergePreferences({ contextMenu: ["gone.one"] }).contextMenu).toEqual(DEFAULT_PREFERENCES.contextMenu);
  });

  it("refuse a representation or scheme the core cannot draw", () => {
    expect(mergePreferences({ representations: ["ribbon"] }).representations).toEqual(["cartoon"]);
    expect(mergePreferences({ representations: ["cartoon", "sticks", "sticks"] }).representations).toEqual(["cartoon", "sticks"]);
    expect(mergePreferences({ scheme: "hydrophobicity" }).scheme).toBe(DEFAULT_PREFERENCES.scheme);
    expect(mergePreferences({ scheme: "toString" }).scheme).toBe(DEFAULT_PREFERENCES.scheme);
    expect(mergePreferences({ scheme: "b-factor" }).scheme).toBe("b-factor");
  });

  it("survive wrong types everywhere", () => {
    for (const saved of [[], "text", 3, { methods: "kabsch", representations: "cartoon", contextMenu: "fetch" }, { methods: null }]) {
      expect(mergePreferences(saved)).toEqual(DEFAULT_PREFERENCES);
    }
  });

  it("never hand back the defaults themselves, which a caller might then change", () => {
    const merged = mergePreferences(null);
    merged.contextMenu.push("hide");
    merged.methods.align = "tm-align";
    expect(mergePreferences(null)).toEqual(DEFAULT_PREFERENCES);
    expect(DEFAULT_PREFERENCES.contextMenu).not.toContain("hide");
  });
});

describe("commands", () => {
  it("name every command on the default menu, and each id once", () => {
    const ids = COMMANDS.map((command) => command.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of DEFAULT_MENU) expect(ids).toContain(id);
  });
});
