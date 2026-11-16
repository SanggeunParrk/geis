// A line at the bottom of the window that says something went wrong, and goes away.
//
// For the failures nothing else on screen would show: a promise nobody was waiting on, an
// error thrown from an event handler. Without it they are only in the log, and the user
// is left with a button that appeared to do nothing.

import { el } from "./dom";

let host: HTMLElement | null = null;
let last = { text: "", at: 0 };

/** Shortens an error to the part worth reading: its first line, without the stack. */
export function errorLine(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  const line = text.split("\n")[0].trim();
  return line.length > 220 ? `${line.slice(0, 217)}...` : line || "Something went wrong.";
}

export function toast(text: string, kind: "bad" | "info" = "bad"): void {
  // The same failure repeating - a drag that errors on every move - is said once.
  const now = Date.now();
  if (text === last.text && now - last.at < 4000) return;
  last = { text, at: now };

  if (!host) {
    host = el("div", { class: "toasts", role: "status", "aria-live": "polite" });
    document.body.append(host);
  }
  const close = el("button", { class: "toast-close", text: "×", title: "Dismiss" });
  const item = el("div", { class: `toast ${kind}` }, el("span", { text }), close);
  const remove = () => item.remove();
  close.addEventListener("click", remove);
  host.append(item);
  // Never more than three: a burst of failures should not cover the window.
  while (host.children.length > 3) host.firstElementChild?.remove();
  setTimeout(remove, kind === "bad" ? 8000 : 4000);
}
