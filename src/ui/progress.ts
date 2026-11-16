// Work in progress, shown on the status bar at the foot of the window.
//
// Blender's arrangement: a render or a save that takes a while puts a small bar on the
// status bar along the bottom, with what it is doing, how far it has got and a cross to
// stop it - and the window stays usable. A dialog in the middle of the screen for the
// same thing stopped the work being looked at while it ran.

import { el } from "./dom";

export interface ProgressHandle {
  /** How far along, 0 to 1, or null when there is no telling; and what is being done. */
  set(fraction: number | null, stage: string): void;
  /** Done: says so for a moment, then goes. */
  finish(message: string): void;
  /** Failed: says why, and stays until it is dismissed. */
  fail(message: string): void;
}

/** Turns a fraction into a percentage for the bar, clamped and rounded. */
export function percent(fraction: number | null): number | null {
  if (fraction === null || !Number.isFinite(fraction)) return null;
  return Math.round(Math.min(Math.max(fraction, 0), 1) * 100);
}

/** The status bar the tasks go on; one per window. */
let host: HTMLElement | null = null;
/** Its left end: what the window has to say - a file saved, a structure being analysed. */
let notes: HTMLElement | null = null;
/** Its right end, where the tasks with a gauge go. */
let tasks: HTMLElement | null = null;

/**
 * Makes the status bar, for the screen to put at its foot. Blender's is the model: what
 * happened is said at the left, work under way runs at the right, and nothing is put over
 * the picture being worked on.
 */
export function statusBar(): HTMLElement {
  notes = el("div", { class: "status-notes" });
  tasks = el("div", { class: "status-tasks" });
  host = el("div", { class: "status-bar", role: "status", "aria-live": "polite" }, notes, tasks);
  return host;
}

/** Replaces what the left of the status bar says. */
export function setStatusNotes(...lines: (HTMLElement | null)[]): void {
  if (!notes) return;
  const shown = lines.filter((line): line is HTMLElement => line !== null);
  // Left alone when nothing has changed, so a line's spinner does not restart on every redraw.
  const key = shown.map((line) => `${line.className}|${line.textContent}`).join("\n");
  if (notes.dataset.key === key) return;
  notes.dataset.key = key;
  notes.replaceChildren(...shown);
}

/** A line for the left of the status bar: a message, with a spinner while something runs. */
export function statusNote(text: string, tone: "plain" | "busy" | "bad" = "plain"): HTMLElement {
  return el(
    "span",
    { class: `status-note ${tone}`, title: text },
    tone === "busy" ? el("span", { class: "spinner" }) : null,
    el("span", { class: "status-note-text", text }),
  );
}

/**
 * Puts a task on the status bar. `cancel`, when given, is what the cross does.
 * Without a status bar on screen - a screen that has none - nothing is shown.
 */
export function showProgress(title: string, cancel?: () => void): ProgressHandle {
  const fill = el("div", { class: "status-fill" });
  const bar = el("div", { class: "status-gauge", role: "progressbar", "aria-valuemin": "0", "aria-valuemax": "100" }, fill);
  const stage = el("span", { class: "status-stage", text: "Starting..." });
  const figure = el("span", { class: "status-figure" });
  const stop = el("button", { class: "status-cancel", type: "button", text: "×", title: cancel ? "Cancel" : "Dismiss" });
  const task = el("div", { class: "status-task" }, el("span", { class: "status-title", text: title }), bar, figure, stage, stop);
  const remove = () => task.remove();
  let over = false;
  stop.addEventListener("click", () => {
    if (!over && cancel) {
      cancel();
      stage.textContent = "Cancelling...";
      stop.disabled = true;
    } else remove();
  });
  if (!cancel) stop.hidden = true;
  (tasks ?? host)?.append(task);

  const set = (fraction: number | null, words: string) => {
    const value = percent(fraction);
    bar.classList.toggle("unknown", value === null);
    fill.style.width = value === null ? "" : `${value}%`;
    if (value === null) bar.removeAttribute("aria-valuenow");
    else bar.setAttribute("aria-valuenow", String(value));
    figure.textContent = value === null ? "" : `${value}%`;
    stage.textContent = words;
  };
  return {
    set,
    finish(message) {
      over = true;
      set(1, message);
      task.classList.add("done");
      stop.hidden = true;
      setTimeout(remove, 2500);
    },
    fail(message) {
      over = true;
      task.classList.add("failed");
      bar.classList.remove("unknown");
      figure.textContent = "";
      stage.textContent = message;
      stop.hidden = false;
      stop.disabled = false;
      stop.title = "Dismiss";
    },
  };
}
