// Window chrome.
//
// The title bar is an overlay, so the window has no frame to grab: dragging it is the
// app's job. The first attempt used `-webkit-app-region: drag`, which is a Chromium
// property - WKWebView, which is what Tauri uses on macOS, ignores it silently, so the
// window could not be moved at all. The real mechanism is Tauri's own command, and it
// needs `core:window:allow-start-dragging` in the capability file; without that the call
// is denied, which is why this reports failures instead of swallowing them.

import { getCurrentWindow } from "@tauri-apps/api/window";

import { api } from "../ipc";
import { el, type Child } from "./dom";

/** Anything that takes a click of its own must not start a window drag. */
const INTERACTIVE = "button, input, select, textarea, a, canvas, [contenteditable], .no-drag";

let warned = false;

/**
 * Makes the empty parts of a bar drag the window, and a double-click on it zoom.
 *
 * The whole bar is the handle, minus its controls - which is how every titlebar behaves,
 * and means there is no invisible strip to discover.
 */
export function makeDraggable(element: HTMLElement): HTMLElement {
  // Not `data-tauri-drag-region`. Tauri's own handler for that attribute also zooms the
  // window on a double-click - on macOS, on mouseup - and the handler below does the same
  // on `dblclick`. Zoom is a toggle, so the two together zoomed the window and at once put
  // it back: a double-click on the bar looked like a window that refused to grow. One
  // handler, here, which unlike the attribute also covers the bar's labels and gaps and
  // not only the strip of bar between them.
  element.classList.add("draggable");

  element.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest(INTERACTIVE)) return;
    void getCurrentWindow()
      .startDragging()
      .catch((error) => {
        if (warned) return;
        warned = true;
        void api.uiLog(
          "error",
          `the window cannot be dragged: ${String(error)} - check that ` +
            "core:window:allow-start-dragging is in src-tauri/capabilities",
        );
      });
  });

  element.addEventListener("dblclick", (event) => {
    const target = event.target as HTMLElement | null;
    if (target?.closest(INTERACTIVE)) return;
    void getCurrentWindow().toggleMaximize().catch(() => {
      // Zooming is a convenience; a denied permission here is not worth a second warning.
    });
  });

  return element;
}

/**
 * Keeps `body.fullscreen` in step with the window.
 *
 * The bars leave eighty-eight pixels on the left for the traffic lights. Full screen has
 * no traffic lights, and the gap they leave is just a gap.
 */
export function followFullscreen(): void {
  const current = getCurrentWindow();
  const check = () =>
    void current
      .isFullscreen()
      .then((full) => document.body.classList.toggle("fullscreen", full))
      .catch(() => {});
  window.addEventListener("resize", check);
  check();
}

/**
 * The bar across the top of a screen, which is also what the window is dragged by.
 *
 * A helper rather than a convention, so a screen added later cannot forget: forgetting is
 * exactly how the main menu ended up with no way to move the window.
 */
export function screenBar(...children: Child[]): HTMLElement {
  return makeDraggable(el("div", { class: "screen-bar" }, ...children));
}
