import { describe, expect, it } from "vitest";
import { preferredTab, propertyTabs, resolveTab, type TabContext } from "../propertyTabs";

/** A graph with the default pipeline in it and nothing selected. */
function base(): TabContext {
  return {
    selection: null,
    hasMolecule: false,
    hasChain: false,
    hasLight: false,
    hasRender: true,
    hasImage: false,
    hasWorld: true,
  };
}

describe("propertyTabs", () => {
  it("offers only the tabs the scene has something for", () => {
    const tabs = propertyTabs(base()).map((tab) => tab.key);
    // No molecule yet, so no Structure and no Style: an empty tab is worse than none.
    expect(tabs).toEqual(["render", "world"]);
  });

  it("puts a molecule's styles on the Style tab without a chain being selected", () => {
    // Blender's material slots belong to the object, so they are reachable whenever the
    // object is active - you do not have to enter edit mode to name a material.
    const tabs = propertyTabs({ ...base(), hasMolecule: true }).map((tab) => tab.key);
    expect(tabs).toEqual(["render", "world", "structure", "style"]);
  });

  it("keeps the tabs in Blender's order, scene-wide before the active thing", () => {
    const tabs = propertyTabs({
      ...base(),
      hasMolecule: true,
      hasChain: true,
      hasLight: true,
      hasImage: true,
    }).map((tab) => tab.key);
    expect(tabs).toEqual(["render", "output", "world", "structure", "chain", "style", "light"]);
  });
});

describe("resolveTab", () => {
  const tabs = propertyTabs({ ...base(), hasMolecule: true, hasChain: true });

  it("stays on the tab you are looking at within one subject", () => {
    // Selecting another chain while the Style tab is open shows that chain's style; it
    // does not throw you back to the Chain tab. Structure, Chain and Style are three
    // views of one molecule.
    expect(resolveTab("style", tabs, preferredTab("chain"))).toBe("style");
    expect(resolveTab("structure", tabs, preferredTab("chain"))).toBe("structure");
  });

  it("follows the selection when the tab is about something else", () => {
    // Clicking a chain while the Render tab is open used to leave the render's settings on
    // screen, which reads as the click having done nothing.
    const withRender = propertyTabs({ ...base(), hasMolecule: true, hasChain: true });
    expect(resolveTab("render", withRender, preferredTab("chain"))).toBe("chain");
    expect(resolveTab("world", withRender, preferredTab("molecule"))).toBe("structure");
    // And the output pair stays together.
    const withImage = propertyTabs({ ...base(), hasImage: true });
    expect(resolveTab("render", withImage, preferredTab("image"))).toBe("render");
  });

  it("keeps the tab when the selection has no tab of its own", () => {
    // A Sequence node has nowhere to send you, so nothing moves.
    expect(resolveTab("style", tabs, preferredTab("sequence-input"))).toBe("style");
  });

  it("falls back to the selection's own tab when the current one is gone", () => {
    const withoutLight = propertyTabs({ ...base(), hasMolecule: true, hasChain: true });
    expect(resolveTab("light", withoutLight, preferredTab("chain"))).toBe("chain");
  });

  it("falls back to the first tab when neither the current nor the preferred exists", () => {
    expect(resolveTab("light", tabs, preferredTab("sequence-input"))).toBe("render");
  });

  it("has nothing to show when the graph is empty", () => {
    expect(resolveTab(null, [], null)).toBeNull();
  });
});

describe("preferredTab", () => {
  it("sends a style's own nodes to the Style tab", () => {
    // A Colouring or a Material node is part of a style network, so selecting one should
    // land where that style is edited rather than nowhere.
    for (const kind of ["style", "colouring", "solid-colour", "material"]) {
      expect(preferredTab(kind)).toBe("style");
    }
  });

  it("treats the viewport as the render's own tab", () => {
    // A render follows the viewport's View, so they are two settings of one output.
    expect(preferredTab("viewport")).toBe("render");
    expect(preferredTab("render")).toBe("render");
  });
});
