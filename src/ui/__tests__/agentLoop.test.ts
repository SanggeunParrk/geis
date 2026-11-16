import { describe, expect, it, vi } from "vitest";
import type { AgentMessage, AgentToolCall, ChatReply } from "../../ipc";
import { converse } from "../../agent/loop";

const reply = (text: string, tool_calls: AgentToolCall[] = [], stop = tool_calls.length ? "tool" : "end"): ChatReply => ({
  text,
  thinking: "",
  tool_calls,
  stop_reason: stop,
  usage: { input: 10, output: 5 },
  model: "test",
});

const user = (content: string): AgentMessage => ({ role: "user", content, tool_calls: [], tool_call_id: null });

describe("the conversation loop", () => {
  it("stops as soon as the model answers without asking for a tool", async () => {
    const complete = vi.fn().mockResolvedValue(reply("chain B is red now"));
    const result = await converse([user("colour B red")], { complete, runTool: vi.fn() });
    expect(result.stopped).toBe("end");
    expect(result.added).toHaveLength(1);
    expect(result.added[0]).toMatchObject({ role: "assistant", content: "chain B is red now" });
    expect(complete).toHaveBeenCalledTimes(1);
    expect(result.usage).toEqual({ input: 10, output: 5 });
  });

  it("runs the tools it is asked for and sends each result back against its own call", async () => {
    const calls: AgentToolCall[] = [
      { id: "a", name: "read_graph", arguments: {} },
      { id: "b", name: "list_molecules", arguments: {} },
    ];
    const complete = vi
      .fn()
      .mockResolvedValueOnce(reply("looking", calls))
      .mockResolvedValueOnce(reply("there are two chains"));
    const runTool = vi.fn(async (call: AgentToolCall) => `result of ${call.name}`);
    const seen: AgentMessage[] = [];
    const result = await converse([user("what is open?")], { complete, runTool, onMessage: (message: AgentMessage) => seen.push(message) });

    expect(result.stopped).toBe("end");
    expect(result.added.map((message: AgentMessage) => message.role)).toEqual(["assistant", "tool", "tool", "assistant"]);
    expect(result.added[1]).toMatchObject({ tool_call_id: "a", content: "result of read_graph" });
    expect(result.added[2]).toMatchObject({ tool_call_id: "b", content: "result of list_molecules" });
    // The second request carries the whole conversation, results included.
    expect(complete.mock.calls[1][0]).toHaveLength(4);
    expect(seen).toHaveLength(4);
    // Two requests, so twice the tokens.
    expect(result.usage).toEqual({ input: 20, output: 10 });
  });

  it("turns a failing or refused tool into a result rather than an exception", async () => {
    const call: AgentToolCall = { id: "a", name: "open_pdb", arguments: { id: "nope" } };
    const complete = vi
      .fn()
      .mockResolvedValueOnce(reply("opening", [call]))
      .mockResolvedValueOnce(reply("that entry does not exist"));
    const runTool = vi.fn().mockRejectedValue(new Error("no such entry"));
    const result = await converse([user("open nope")], { complete, runTool });
    expect(result.stopped).toBe("end");
    expect(result.added[1].content).toContain("no such entry");
    // And the model got to answer, which is the point of not throwing.
    expect(result.added.at(-1)).toMatchObject({ role: "assistant", content: "that entry does not exist" });
  });

  it("keeps a reasoning model's thought for the panel and out of what is sent back", async () => {
    const complete = vi.fn().mockResolvedValue({ ...reply("done"), thinking: "read the graph first" });
    const result = await converse([user("go")], { complete, runTool: vi.fn() });
    expect(result.added[0].thinking).toBe("read the graph first");
    // It is on the message the panel draws, but the providers never see it again.
    expect(result.added[0].content).toBe("done");
  });

  it("does not send a message twice when the caller keeps the history it passed", async () => {
    // The panel draws each message as it arrives and keeps the turn afterwards. When it
    // kept them as they arrived - into the very array it had passed in - every request
    // carried each tool exchange twice, which wastes the context and, on Anthropic, repeats
    // a tool_use id the model has already answered.
    const call: AgentToolCall = { id: "a", name: "read_graph", arguments: {} };
    const complete = vi
      .fn()
      .mockResolvedValueOnce(reply("looking", [call]))
      .mockResolvedValueOnce(reply("two chains"));
    const history: AgentMessage[] = [user("what is open?")];
    await converse(history, {
      complete,
      runTool: async () => "{}",
      onMessage: (message: AgentMessage) => history.push(message),
    });
    const second = complete.mock.calls[1][0] as AgentMessage[];
    expect(second.map((message: AgentMessage) => message.role)).toEqual(["user", "assistant", "tool"]);
  });

  it("gives up after the step limit rather than looping forever", async () => {
    const call: AgentToolCall = { id: "a", name: "read_graph", arguments: {} };
    const complete = vi.fn().mockResolvedValue(reply("again", [call]));
    const runTool = vi.fn().mockResolvedValue("{}");
    const result = await converse([user("go")], { complete, runTool }, 3);
    expect(result.stopped).toBe("steps");
    expect(complete).toHaveBeenCalledTimes(3);
    expect(runTool).toHaveBeenCalledTimes(3);
  });

  it("reports a provider failure without losing what was already said", async () => {
    const call: AgentToolCall = { id: "a", name: "read_graph", arguments: {} };
    const complete = vi
      .fn()
      .mockResolvedValueOnce(reply("looking", [call]))
      .mockRejectedValueOnce(new Error("401: no key"));
    const result = await converse([user("go")], { complete, runTool: async () => "{}" });
    expect(result.stopped).toBe("error");
    expect(result.error).toContain("401");
    expect(result.added.map((message: AgentMessage) => message.role)).toEqual(["assistant", "tool"]);
  });
});
