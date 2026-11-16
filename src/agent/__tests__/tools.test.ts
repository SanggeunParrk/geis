import { describe, expect, it } from "vitest";
import { buildTools, toolText, type AgentHost } from "../tools";

describe("the tools an agent is given", () => {
  it("have unique names and object schemas a provider accepts", () => {
    const host = new Proxy({}, { get: () => async () => null }) as unknown as AgentHost;
    const tools = buildTools(host);
    const names = tools.map((tool) => tool.spec.name);
    expect(new Set(names).size).toBe(names.length);
    for (const tool of tools) {
      expect(tool.spec.name).toMatch(/^[a-z_]+$/);
      expect(tool.spec.parameters).toMatchObject({ type: "object" });
      const required = (tool.spec.parameters as { required?: string[]; properties: Record<string, unknown> }).required ?? [];
      for (const key of required) expect(Object.keys((tool.spec.parameters as { properties: object }).properties)).toContain(key);
    }
  });

  it("each reach the host when a model leaves every argument out", async () => {
    const reached: string[] = [];
    const host = new Proxy({}, { get: (_, name) => async () => reached.push(String(name)) }) as unknown as AgentHost;
    for (const tool of buildTools(host)) await tool.run({});
    expect(reached).toHaveLength(buildTools(host).length);
  });
});

describe("a tool's result as text", () => {
  it("is always a string, whatever the tool handed back", () => {
    expect(toolText("chain A is red")).toBe("chain A is red");
    expect(toolText({ framed: true })).toBe('{"framed":true}');
    expect(toolText(null)).toBe("null");
    // A tool that returned nothing still answers its call.
    expect(toolText(undefined)).toBe("done");
    expect(toolText(() => 1)).toBe("done");
  });
});
