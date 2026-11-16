// The inside of a cell, behind the opening screens.
//
// The pictures are rendered by Blender from `scripts/blender/cell_scene.py`: the crowded
// cytoplasm far off, a few molecules close by and out of focus, and the vesicle each world
// lives in. Two layers rather than one picture, so the near one can move against the far
// one - with the pointer, and on its own, slowly - and the space reads as a space.

import cellFar from "../assets/cell/cell-far.webp";
import cellNear from "../assets/cell/cell-near.webp";
import vesicleBack from "../assets/cell/vesicle-back.webp";
import vesicleFront from "../assets/cell/vesicle-front.webp";
import { el } from "./dom";

/** The two halves of a vesicle, for a world to be drawn between. */
export const VESICLE = { back: vesicleBack, front: vesicleFront };

/**
 * How much of a vesicle picture's width the membrane spans: the sphere is rendered at a
 * radius of 4 seen from 15.5 through a 50 mm lens, which is this share of the frame.
 */
export const VESICLE_SPAN = 4 / (15.5 * (18 / 50));

/**
 * The backdrop: the far layer and the near one over it. `calm` is for a screen with work
 * on it - the shelf - where the cell sits back, darker and softer, behind what is chosen.
 */
export function cellBackdrop(calm = false): HTMLElement {
  const far = el("div", { class: "cell-layer cell-far" });
  far.style.backgroundImage = `url(${cellFar})`;
  const near = el("div", { class: "cell-layer cell-near" });
  near.style.backgroundImage = `url(${cellNear})`;
  const backdrop = el("div", { class: `cell-backdrop${calm ? " calm" : ""}`, "aria-hidden": "true" }, far, near, el("div", { class: "cell-shade" }));

  // The pointer leans the space a little: the near layer moves more than the far one,
  // which is all parallax is. Eased towards where the pointer is, so it drifts rather than
  // twitches.
  let wantX = 0;
  let wantY = 0;
  let x = 0;
  let y = 0;
  let frame = 0;
  const still = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  const step = () => {
    frame = 0;
    if (!backdrop.isConnected) return;
    x += (wantX - x) * 0.06;
    y += (wantY - y) * 0.06;
    backdrop.style.setProperty("--lean-x", x.toFixed(4));
    backdrop.style.setProperty("--lean-y", y.toFixed(4));
    if (Math.abs(wantX - x) > 0.001 || Math.abs(wantY - y) > 0.001) frame = requestAnimationFrame(step);
  };
  const lean = (event: PointerEvent) => {
    if (still || !backdrop.isConnected) return;
    wantX = event.clientX / Math.max(window.innerWidth, 1) - 0.5;
    wantY = event.clientY / Math.max(window.innerHeight, 1) - 0.5;
    if (!frame) frame = requestAnimationFrame(step);
  };
  window.addEventListener("pointermove", lean, { passive: true });
  // Taken off when the screen goes, so a backdrop left behind does not keep listening.
  const watch = new MutationObserver(() => {
    if (!backdrop.isConnected && !document.body.contains(backdrop)) {
      window.removeEventListener("pointermove", lean);
      watch.disconnect();
    }
  });
  requestAnimationFrame(() => {
    if (backdrop.parentElement) watch.observe(document.body, { childList: true, subtree: true });
  });
  return backdrop;
}
