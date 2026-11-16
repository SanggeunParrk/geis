import { describe, expect, it } from "vitest";
import { parseInline, parseMarkdown } from "../markdown";

describe("markdown, as far as an answer uses it", () => {
  it("reads bold, italic and code within a line", () => {
    expect(parseInline("Chain **A_1** is now *red* (`#ff0000`).")).toEqual([
      { kind: "text", text: "Chain " },
      { kind: "bold", text: "A_1" },
      { kind: "text", text: " is now " },
      { kind: "italic", text: "red" },
      { kind: "text", text: " (" },
      { kind: "code", text: "#ff0000" },
      { kind: "text", text: ")." },
    ]);
  });

  it("leaves markup inside code alone, and a lone asterisk as it is", () => {
    expect(parseInline("`a * b ** c`")).toEqual([{ kind: "code", text: "a * b ** c" }]);
    expect(parseInline("2 * 3 = 6")).toEqual([{ kind: "text", text: "2 * 3 = 6" }]);
    expect(parseInline("단백질 **사슬 B**")).toEqual([
      { kind: "text", text: "단백질 " },
      { kind: "bold", text: "사슬 B" },
    ]);
  });

  it("reads paragraphs, headings, lists and code blocks", () => {
    const blocks = parseMarkdown(
      ["## What changed", "", "Two things", "happened.", "", "- chain A is red", "- chain B is blue", "", "1. first", "2. second", "", "```json", '{"a": 1}', "```"].join("\n"),
    );
    expect(blocks.map((block) => block.kind)).toEqual(["heading", "paragraph", "list", "list", "code"]);
    expect(blocks[1]).toEqual({ kind: "paragraph", inline: [{ kind: "text", text: "Two things happened." }] });
    expect(blocks[2]).toMatchObject({ kind: "list", ordered: false });
    expect((blocks[2] as { items: unknown[] }).items).toHaveLength(2);
    expect(blocks[3]).toMatchObject({ kind: "list", ordered: true });
    expect(blocks[4]).toEqual({ kind: "code", text: '{"a": 1}' });
  });

  it("reads a reply that is still being written", () => {
    // A fence that has opened and not yet closed is still a code block.
    expect(parseMarkdown("Here:\n```\nx = 1")).toEqual([
      { kind: "paragraph", inline: [{ kind: "text", text: "Here:" }] },
      { kind: "code", text: "x = 1" },
    ]);
    expect(parseMarkdown("")).toEqual([]);
  });
});

describe("markdown from a model, which is not trusted", () => {
  const texts = (source: string) =>
    parseMarkdown(source).flatMap((block) =>
      block.kind === "code" ? [block.text] : block.kind === "list" ? block.items.flat().map((run) => run.text) : block.inline.map((run) => run.text),
    );

  it("keeps raw HTML, links and handlers as the characters they are", () => {
    const hostile = [
      '<img src=x onerror="alert(1)">',
      "[click](javascript:alert(1))",
      "<script>alert(1)</script>",
      '**<a href="javascript:alert(1)">bold</a>**',
      "- <iframe src=//evil></iframe>",
    ].join("\n\n");
    const blocks = parseMarkdown(hostile);
    // Only the four inline kinds and the four block kinds exist; nothing is a link or markup.
    for (const block of blocks) expect(["paragraph", "heading", "list", "code"]).toContain(block.kind);
    const all = texts(hostile).join("");
    expect(all).toContain('<img src=x onerror="alert(1)">');
    expect(all).toContain("[click](javascript:alert(1))");
    expect(all).toContain("<script>alert(1)</script>");
  });

  it("loses no text to unbalanced markers", () => {
    for (const line of ["**open bold", "`open code", "a * b", "__x", "**", "``", "2 * 3 ** 4"]) {
      expect(texts(line).join("")).toBe(line);
    }
  });
});
