// The command palette: every command the app has, found by typing a few letters of it.
//
// Pies and right-click menus are fast once learnt, and invisible until then. This is the
// way in for someone who knows what they want and not where it lives - Blender's F3,
// VS Code's command palette. It lists the same commands those menus run, so nothing
// here exists only here.

import { composing, el } from "./dom";

export interface PaletteItem {
  label: string;
  /** Where it belongs - "Show", "Find", "File" - shown beside the label. */
  group: string;
  /** The keys that do the same thing, if there are any. */
  keys?: string;
  run: () => void;
}

/**
 * How well `query` matches `text`, or null if it does not.
 *
 * Every letter of the query has to appear in order. A run of letters that sits together
 * and starts a word scores higher than the same letters scattered, so "hs" finds
 * "Hide sticks" before "Show cartoon as sticks"; lower is better.
 */
export function matchScore(query: string, text: string): number | null {
  const wanted = query.toLowerCase().replace(/\s+/g, "");
  if (wanted === "") return 0;
  const hay = text.toLowerCase();
  let score = 0;
  let from = 0;
  let last = -2;
  for (const letter of wanted) {
    const at = hay.indexOf(letter, from);
    if (at < 0) return null;
    const startsWord = at === 0 || /[\s.\-/]/.test(hay[at - 1]);
    if (at === last + 1) score += 0;
    else if (startsWord) score += 1;
    else score += 3 + Math.min(at - from, 5);
    last = at;
    from = at + 1;
  }
  // Letters found one here and one there are a coincidence, not a match: "col" is not
  // asking for "Show as sticks only".
  if (score > wanted.length * 2.5) return null;
  // Among equal matches the shorter label is the closer one.
  return score + hay.length / 1000;
}

/** The items that match, best first; with nothing typed, all of them in their own order. */
export function rankItems<T extends { label: string; group: string }>(query: string, items: T[]): T[] {
  if (query.trim() === "") return items;
  return items
    .map((item, index) => {
      const byLabel = matchScore(query, item.label);
      const byGroup = matchScore(query, `${item.group} ${item.label}`);
      const score = byLabel ?? (byGroup === null ? null : byGroup + 2);
      return { item, index, score };
    })
    .filter((entry): entry is { item: T; index: number; score: number } => entry.score !== null)
    .sort((a, b) => a.score - b.score || a.index - b.index)
    .map((entry) => entry.item);
}

let open: (() => void) | null = null;

/** Whether the palette is on screen - the screen's own shortcuts stand down while it is. */
export function paletteOpen(): boolean {
  return open !== null;
}

/** Shows the palette over the window. Picking an item closes it and runs the item. */
export function openPalette(items: PaletteItem[], placeholder = "Type a command"): void {
  if (open) {
    open();
    return;
  }
  const input = el("input", {
    class: "cmdk-input",
    type: "text",
    placeholder,
    spellcheck: "false",
    autocomplete: "off",
    "aria-label": "Command",
  });
  const list = el("div", { class: "cmdk-list", role: "listbox" });
  const box = el("div", { class: "cmdk", role: "dialog", "aria-modal": "true", "aria-label": "Commands" }, input, list);
  const backdrop = el("div", { class: "cmdk-backdrop" }, box);

  const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  let shown: PaletteItem[] = [];
  let cursor = 0;

  const close = () => {
    window.removeEventListener("keydown", onKey, true);
    backdrop.remove();
    open = null;
    before?.focus({ preventScroll: true });
  };
  open = close;

  const pick = (item: PaletteItem | undefined) => {
    if (!item) return;
    close();
    item.run();
  };

  const draw = () => {
    shown = rankItems(input.value, items);
    cursor = Math.min(cursor, Math.max(shown.length - 1, 0));
    list.replaceChildren(
      ...(shown.length === 0
        ? [el("div", { class: "cmdk-empty", text: "Nothing by that name." })]
        : shown.map((item, index) => {
            const row = el(
              "div",
              { class: "cmdk-row", role: "option", "aria-selected": String(index === cursor) },
              el("span", { class: "cmdk-label", text: item.label }),
              el("span", { class: "cmdk-group", text: item.group }),
              item.keys ? el("kbd", { class: "cmdk-keys", text: item.keys }) : null,
            );
            row.addEventListener("pointermove", () => {
              if (cursor === index) return;
              cursor = index;
              mark();
            });
            row.addEventListener("click", () => pick(item));
            return row;
          })),
    );
    mark();
  };

  const mark = () => {
    [...list.children].forEach((row, index) => {
      row.setAttribute("aria-selected", String(index === cursor));
      if (index === cursor && row instanceof HTMLElement) row.scrollIntoView({ block: "nearest" });
    });
  };

  // Captured at the window, so the screen underneath does not also act on the keys.
  const onKey = (event: KeyboardEvent) => {
    event.stopPropagation();
    if (composing(event)) return;
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (shown.length === 0) return;
      const step = event.key === "ArrowDown" ? 1 : -1;
      cursor = (cursor + step + shown.length) % shown.length;
      mark();
    } else if (event.key === "Enter") {
      event.preventDefault();
      pick(shown[cursor]);
    }
  };
  window.addEventListener("keydown", onKey, true);

  input.addEventListener("input", () => {
    cursor = 0;
    draw();
  });
  backdrop.addEventListener("pointerdown", (event) => {
    if (event.target === backdrop) close();
  });

  document.body.append(backdrop);
  draw();
  input.focus();
}

export interface ShortcutGroup {
  title: string;
  rows: [keys: string, what: string][];
}

/** Shows what the keys do: a sheet over the window, closed by Escape or a click outside. */
export function openShortcuts(groups: ShortcutGroup[]): void {
  open?.();
  const sheet = el(
    "div",
    { class: "shortcuts", role: "dialog", "aria-modal": "true", "aria-label": "Keyboard shortcuts" },
    el("div", { class: "shortcuts-title", text: "Keyboard and mouse" }),
    el(
      "div",
      { class: "shortcuts-columns" },
      ...groups.map((group) =>
        el(
          "section",
          { class: "shortcuts-group" },
          el("h3", { text: group.title }),
          ...group.rows.map(([keys, what]) =>
            el("div", { class: "shortcuts-row" }, el("kbd", { text: keys }), el("span", { text: what })),
          ),
        ),
      ),
    ),
  );
  const backdrop = el("div", { class: "cmdk-backdrop" }, sheet);
  const close = () => {
    window.removeEventListener("keydown", onKey, true);
    backdrop.remove();
    open = null;
  };
  const onKey = (event: KeyboardEvent) => {
    event.stopPropagation();
    if (event.key === "Escape" || event.key === "?" || event.key === "Enter") {
      event.preventDefault();
      close();
    }
  };
  open = close;
  window.addEventListener("keydown", onKey, true);
  backdrop.addEventListener("pointerdown", (event) => {
    if (event.target === backdrop) close();
  });
  document.body.append(backdrop);
}
