// Just enough Markdown for an answer.
//
// Models write Markdown whether or not they are asked to - a bold word, a list of three
// things, a line of code - and shown as plain text that is a scatter of asterisks and
// backticks. This reads the handful of forms an answer in this app actually uses and
// leaves the rest alone: paragraphs, headings, bullet and numbered lists, fenced code, and
// bold, italic and code within a line. No HTML is ever produced from the text; the blocks
// are data, and the renderer builds elements from them with `textContent`.

export type Inline =
  | { kind: "text"; text: string }
  | { kind: "bold"; text: string }
  | { kind: "italic"; text: string }
  | { kind: "code"; text: string };

export type Block =
  | { kind: "paragraph"; inline: Inline[] }
  | { kind: "heading"; inline: Inline[] }
  | { kind: "list"; ordered: boolean; items: Inline[][] }
  | { kind: "code"; text: string };

/** Splits one line into runs of plain, bold, italic and code text. */
export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  const push = (kind: Inline["kind"], value: string) => {
    if (!value) return;
    const last = out[out.length - 1];
    if (kind === "text" && last?.kind === "text") last.text += value;
    else out.push({ kind, text: value });
  };
  // Code first, since nothing inside backticks is markup; then bold before italic, since
  // `**` would otherwise be read as two empty italics.
  const pattern = /`([^`\n]+)`|\*\*([^*\n]+?)\*\*|__([^_\n]+?)__|\*([^*\s][^*\n]*?)\*/g;
  let at = 0;
  for (let match = pattern.exec(text); match; match = pattern.exec(text)) {
    push("text", text.slice(at, match.index));
    if (match[1] !== undefined) push("code", match[1]);
    else if (match[2] !== undefined) push("bold", match[2]);
    else if (match[3] !== undefined) push("bold", match[3]);
    else push("italic", match[4]);
    at = match.index + match[0].length;
  }
  push("text", text.slice(at));
  return out;
}

const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*\d+[.)]\s+(.*)$/;
const HEADING = /^\s*#{1,6}\s+(.*)$/;
const FENCE = /^\s*```/;

/** Reads text into blocks. Unfinished input - a fence not yet closed - is still read. */
export function parseMarkdown(source: string): Block[] {
  const blocks: Block[] = [];
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  let paragraph: string[] = [];
  const flush = () => {
    if (paragraph.length === 0) return;
    blocks.push({ kind: "paragraph", inline: parseInline(paragraph.join(" ")) });
    paragraph = [];
  };

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (FENCE.test(line)) {
      flush();
      const body: string[] = [];
      index += 1;
      // To the closing fence, or to the end: a reply still being written has not closed it.
      while (index < lines.length && !FENCE.test(lines[index])) {
        body.push(lines[index]);
        index += 1;
      }
      blocks.push({ kind: "code", text: body.join("\n") });
      continue;
    }
    if (line.trim() === "") {
      flush();
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      flush();
      blocks.push({ kind: "heading", inline: parseInline(heading[1]) });
      continue;
    }
    const bullet = BULLET.exec(line);
    const numbered = bullet ? null : NUMBERED.exec(line);
    if (bullet || numbered) {
      flush();
      const ordered = numbered !== null;
      const last = blocks[blocks.length - 1];
      const item = parseInline((bullet ?? numbered)![1]);
      if (last?.kind === "list" && last.ordered === ordered) last.items.push(item);
      else blocks.push({ kind: "list", ordered, items: [item] });
      continue;
    }
    paragraph.push(line.trim());
  }
  flush();
  return blocks;
}

function inlineNodes(inline: Inline[]): Node[] {
  return inline.map((run) => {
    if (run.kind === "text") return document.createTextNode(run.text);
    const node = document.createElement(run.kind === "bold" ? "strong" : run.kind === "italic" ? "em" : "code");
    node.textContent = run.text;
    return node;
  });
}

/** Replaces `target`'s contents with the text, rendered. */
export function renderMarkdown(target: HTMLElement, source: string): void {
  const nodes = parseMarkdown(source).map((block) => {
    if (block.kind === "code") {
      const pre = document.createElement("pre");
      pre.textContent = block.text;
      return pre;
    }
    if (block.kind === "list") {
      const list = document.createElement(block.ordered ? "ol" : "ul");
      for (const item of block.items) {
        const entry = document.createElement("li");
        entry.append(...inlineNodes(item));
        list.append(entry);
      }
      return list;
    }
    const node = document.createElement(block.kind === "heading" ? "h4" : "p");
    node.append(...inlineNodes(block.inline));
    return node;
  });
  target.replaceChildren(...nodes);
}
