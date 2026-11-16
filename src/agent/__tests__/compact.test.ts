import { describe, expect, it } from "vitest";
import type { AgentMessage } from "../../ipc";
import { compact, DROPPED, ELIDED, splitForSummary, summaryMessage, SUMMARY_MARK, transcript } from "../compact";

const user = (content: string): AgentMessage => ({ role: "user", content, tool_calls: [], tool_call_id: null });
const assistant = (content: string, call?: string): AgentMessage => ({
  role: "assistant",
  content,
  tool_calls: call ? [{ id: call, name: "list_molecules", arguments: {} }] : [],
  tool_call_id: null,
});
const tool = (id: string, content: string): AgentMessage => ({ role: "tool", content, tool_calls: [], tool_call_id: id });

const long = "x".repeat(5000);
const talk = [
  user("first"),
  assistant("", "a"),
  tool("a", long),
  assistant("done one"),
  user("second"),
  assistant("", "b"),
  tool("b", long),
  assistant("done two"),
  user("third"),
  assistant("", "c"),
  tool("c", long),
];

describe("compacting a conversation", () => {
  it("leaves one that fits alone", () => {
    expect(compact(talk, 100_000)).toBe(talk);
  });

  it("empties old tool results before dropping anything, and never the turn in progress", () => {
    const sent = compact(talk, 7000);
    expect(sent).toHaveLength(talk.length);
    expect(sent[2].content).toBe(ELIDED);
    expect(sent[6].content).toBe(ELIDED);
    expect(sent[10].content).toBe(long);
    // What was kept by the caller is not what was shortened.
    expect(talk[2].content).toBe(long);
  });

  it("then drops whole exchanges from the start, keeping calls with their results", () => {
    const sent = compact(talk, 5200);
    expect(sent[0].role).toBe("user");
    expect(sent[0].content.startsWith(DROPPED)).toBe(true);
    expect(sent[0].content.endsWith("third")).toBe(true);
    const answered = new Set(sent.filter((m) => m.role === "tool").map((m) => m.tool_call_id));
    const asked = sent.flatMap((m) => m.tool_calls.map((call) => call.id));
    expect([...answered].every((id) => asked.includes(id as string))).toBe(true);
  });

  it("sends the last exchange whole even when it alone is over", () => {
    const sent = compact(talk, 100);
    expect(sent.at(-1)?.content).toBe(long);
    expect(sent.some((m) => m.role === "user" && m.content.endsWith("third"))).toBe(true);
  });
});

describe("summarising a conversation", () => {
  it("is not asked for while there is room", () => {
    expect(splitForSummary(talk, 100_000)).toBeNull();
  });

  it("cuts before a user message, keeping the recent exchanges whole", () => {
    const split = splitForSummary(talk, 14_000);
    expect(split).not.toBeNull();
    expect(split!.recent[0].role).toBe("user");
    expect(split!.recent[0].content).toBe("third");
    expect(split!.early.length + split!.recent.length).toBe(talk.length);
    expect(split!.early.at(-1)?.content).toBe("done two");
  });

  it("has nothing to summarise when the conversation is one exchange", () => {
    expect(splitForSummary(talk.slice(8), 100)).toBeNull();
  });

  it("writes the early part down with long results cut short", () => {
    const text = transcript(talk.slice(0, 4), 10_000);
    expect(text).toContain("USER: first");
    expect(text).toContain("CALL list_molecules");
    expect(text).toContain("… [cut]");
    expect(text.length).toBeLessThan(900);
    expect(transcript(talk, 300).startsWith("[the beginning is left out]")).toBe(true);
  });

  it("keeps the summary as a message that says what it is", () => {
    const message = summaryMessage("  the user opened 1UBQ  ");
    expect(message.role).toBe("user");
    expect(message.content).toBe(`${SUMMARY_MARK}\nthe user opened 1UBQ`);
  });
});

describe("compacting, at the edges", () => {
  const sizeOf = (list: AgentMessage[]) => list.reduce((sum, message) => sum + message.content.length, 0);

  it("leaves an empty conversation, and one with no user message, alone", () => {
    expect(compact([], 10)).toEqual([]);
    expect(splitForSummary([], 10)).toBeNull();
    const orphan = [assistant("", "a"), tool("a", long)];
    expect(compact(orphan, 10)).toBe(orphan);
    expect(transcript([], 100)).toBe("");
  });

  it("sends one huge user message as it is: there is nothing older to give up", () => {
    const one = [user(long)];
    expect(compact(one, 100)).toBe(one);
    expect(splitForSummary(one, 100)).toBeNull();
  });

  it("always ends with the newest user message and everything after it, word for word", () => {
    for (const budget of [50, 500, 5000, 9000, 12000]) {
      const sent = compact(talk, budget);
      // Word for word, apart from the note that earlier parts were left out, which goes on
      // the first message sent - the newest user message when it is all that is left.
      const [first, ...after] = sent.slice(-3);
      expect(["third", `${DROPPED}\n\nthird`]).toContain(first.content);
      expect(after).toEqual(talk.slice(-2));
      // Every tool result that is sent still has the call it answers.
      const called = new Set(sent.flatMap((message) => message.tool_calls.map((call) => call.id)));
      for (const message of sent) if (message.role === "tool") expect(called.has(message.tool_call_id!)).toBe(true);
      // And the first message sent is from the user, as providers require.
      expect(sent[0].role).toBe("user");
    }
  });

  it("splits for a summary without ever putting the newest user turn in the early part", () => {
    for (const budget of [100, 1000, 8000, 14000]) {
      const split = splitForSummary(talk, budget);
      if (!split) continue;
      expect(split.recent[0].role).toBe("user");
      expect(split.recent).toContainEqual(user("third"));
      expect([...split.early, ...split.recent]).toEqual(talk);
    }
  });

  it("cuts a transcript to its limit from the front, keeping the latest", () => {
    const text = transcript(talk, 200);
    expect(text.startsWith("[the beginning is left out]")).toBe(true);
    expect(text.endsWith("… [cut]")).toBe(true);
    expect(sizeOf([user(text)])).toBeLessThan(260);
  });
});
