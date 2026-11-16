import { describe, expect, it } from "vitest";
import { buildTools, LOCAL_TOOLS, systemPrompt, type AgentHost } from "../tools";

// Every method answers nothing: only the specs are looked at here.
const host = new Proxy({}, { get: () => async () => null }) as unknown as AgentHost;
const size = (names?: ReadonlySet<string>) => {
  const tools = buildTools(host).filter((tool) => !names || names.has(tool.spec.name));
  return systemPrompt([], "auto", names).length + JSON.stringify(tools.map((tool) => tool.spec)).length;
};

describe("what a local model is sent", () => {
  it("names only tools that exist", () => {
    const all = new Set(buildTools(host).map((tool) => tool.spec.name));
    expect([...LOCAL_TOOLS].filter((name) => !all.has(name))).toEqual([]);
  });

  it("is a good deal less than what the hosted models are sent", () => {
    const local = size(LOCAL_TOOLS);
    const full = size();
    // The full set measured 9,026 tokens in the engine's own count at about 16,800
    // characters, so a little under two characters a token: this is about 5,300 tokens.
    expect(local).toBeLessThan(10_500);
    expect(local).toBeLessThan(full * 0.65);
  });

  it("does not point a model at tools it was not given", () => {
    const prompt = systemPrompt([], "auto", LOCAL_TOOLS);
    for (const absent of ["read_doc", "show_image", "render", "set_world", "measure_distance"]) {
      expect(prompt).not.toContain(`-> ${absent}`);
    }
    expect(prompt).toContain("-> colour_chain");
  });
});
