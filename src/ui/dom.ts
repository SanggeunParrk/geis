// Just enough of a view layer. No framework: the viewport this app is heading towards is
// a WebGPU canvas with its own render loop, and a virtual DOM would be in the way.

export type Child = Node | string | null | undefined | false;

type Attrs = Record<string, unknown>;

/**
 * The text of a value that should have been a string or a number.
 *
 * Attributes here and arguments from a model's tool call are `unknown`; `String()` on one
 * that turns out to be an object writes "[object Object]" into a field or a file path.
 * Anything that is not plain text or a number is no text at all.
 */
export function plain(value: unknown, otherwise = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean" || typeof value === "bigint") return String(value);
  return otherwise;
}

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === "class") node.className = plain(value);
    else if (key === "text") node.textContent = plain(value);
    else if (key === "dataset") Object.assign(node.dataset, value);
    else if (key.startsWith("on") && typeof value === "function") {
      node.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
    } else if (
      key === "value" &&
      (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement)
    ) {
      // The property, not the attribute. A textarea has no `value` attribute at all - its
      // content is its text - so setting one leaves the box empty while the state behind
      // it still holds the sequence, which is then submitted unseen.
      node.value = plain(value);
    } else if (key === "checked" && node instanceof HTMLInputElement) {
      node.checked = Boolean(value);
    } else {
      node.setAttribute(key, plain(value));
    }
  }
  append(node, children);
  return node;
}

export function append(parent: Node, children: Child[]): void {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    parent.appendChild(typeof child === "string" ? document.createTextNode(child) : child);
  }
}

export function clear(node: Node): void {
  while (node.firstChild) node.removeChild(node.firstChild);
}

export function replace(node: Node, ...children: Child[]): void {
  clear(node);
  append(node, children);
}

/**
 * A link that is safe to put in an `href`: http or https only, else null.
 *
 * Links come from places this app does not write - a model manifest dropped into the
 * models folder - and a `javascript:` or `file:` address there would run or open inside
 * the app's own window when clicked.
 */
export function webLink(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const parsed = new URL(url.trim());
    return parsed.protocol === "https:" || parsed.protocol === "http:" ? parsed.href : null;
  } catch {
    return null;
  }
}

/** Byte counts appear all over the job UI; one formatting rule for all of them. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes)) return "-";
  if (bytes < 1024) return `${Math.max(0, Math.round(bytes))} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  // Promoted on the value as it will be printed: 1023.7 KB rounds to "1024 KB" otherwise.
  const shown = (amount: number) => (amount < 10 ? Number(amount.toFixed(1)) : Math.round(amount));
  while (shown(value) >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export function formatWhen(iso: string | null): string {
  if (!iso) return "-";
  const then = new Date(iso).getTime();
  // A timestamp that does not parse says so rather than printing "Invalid Date".
  if (Number.isNaN(then)) return "-";
  const seconds = Math.round((Date.now() - then) / 1000);
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return new Date(iso).toLocaleDateString();
}

export function formatDuration(from: string | null, to: string | null): string {
  if (!from) return "-";
  const start = new Date(from).getTime();
  const end = to ? new Date(to).getTime() : Date.now();
  if (Number.isNaN(start) || Number.isNaN(end)) return "-";
  const seconds = Math.max(0, Math.round((end - start) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** `1 chain`, `2 chains`: a count with its noun, which a bare template gets wrong at one. */
export function count(value: number, noun: string, plural = `${noun}s`): string {
  return `${value.toLocaleString()} ${value === 1 ? noun : plural}`;
}

/**
 * Whether a key press belongs to an input method rather than to the page.
 *
 * `isComposing` alone is not enough in WebKit: the Enter that commits a Hangul or Kana
 * candidate arrives after composition has ended, with `isComposing` false, and is marked
 * only by the legacy key code 229. Treating it as Enter sends a message half typed.
 */
export function composing(event: KeyboardEvent): boolean {
  return event.isComposing || event.keyCode === 229;
}
