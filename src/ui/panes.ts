// The left of the screen as a desk of floating panes, each a strip of tabs.
//
// Chrome's windows, not Blender's areas: each pane is a window with a title bar you drag to
// move it and edges you drag to resize it, and panes overlap, the one you touched last on
// top. And, as in Chrome, a window holds tabs: a pane can carry several editors and show
// one at a time. The outliner and properties are not in here - they are the fixed column on
// the right. This module is the desk and the operations on it, kept pure so moving,
// resizing, stacking and tabbing are unit-testable and the arrangement can be saved as
// JSON and read back.

/** What a tab can show. Singletons exist once; the others as often as wanted. */
export type EditorKind = "viewport" | "biomol" | "graph" | "image" | "text";

export const EDITOR_LABELS: Record<EditorKind, string> = {
  viewport: "3D viewport",
  biomol: "Sequence",
  graph: "Node graph",
  image: "File viewer",
  text: "Text",
};

/**
 * Editors of which there is exactly one - they hold a WebGL context, the graph state, or
 * the one BioMol panel whose selection the viewport follows.
 */
export const SINGLETONS: ReadonlySet<EditorKind> = new Set(["viewport", "biomol", "graph"]);

/** One tab in a pane. Its own id, so two file viewers show two files. */
export interface Tab {
  id: string;
  editor: EditorKind;
}

/** A pane's place on the desk, in fractions of the desk, so a resized window keeps it. */
export interface Pane {
  id: string;
  tabs: Tab[];
  /** The tab shown. Always one of `tabs`. */
  active: string;
  x: number;
  y: number;
  w: number;
  h: number;
  /** Where the pane was before it was snapped, so it can be put back. */
  before?: { x: number; y: number; w: number; h: number };
  /** Folded down to its title bar. */
  collapsed?: boolean;
}

export interface Desk {
  panes: Pane[];
  /** Stacking order, back to front. */
  order: string[];
}

/** A pane can be dragged this small and no smaller: a title bar with a sliver under it. */
export const MIN_SIZE = 0.08;

/**
 * How much of a pane must stay on the desk when it is dragged off the edge - enough of a
 * title bar to grab it back by, as a window half off a screen can still be reached.
 */
export const GRIP = 0.08;

let counter = 0;
export function freshId(prefix = "pane"): string {
  counter += 1;
  return `${prefix}-${counter}`;
}

/**
 * Moves the counter past an id that came from somewhere else - a saved desk. The counter
 * starts again from zero every launch while the saved ids do not, so without this the
 * next new pane is handed an id a restored one already has, and the two then share one
 * element on screen.
 */
function reserveId(id: string): void {
  const suffix = /-(\d+)$/.exec(id);
  if (suffix) counter = Math.max(counter, Number(suffix[1]));
}

function newPane(editor: EditorKind, rect: { x: number; y: number; w: number; h: number }): Pane {
  const tab: Tab = { id: freshId("tab"), editor };
  return { id: freshId(), tabs: [tab], active: tab.id, ...rect };
}

/**
 * The desk a new world opens with: the viewport, and nothing else. The node graph and the
 * sequence are a key or a tab away, and a world with nothing in it yet has nothing to show
 * in either.
 */
export function defaultDesk(): Desk {
  const viewport = newPane("viewport", { x: 0, y: 0, w: 1, h: 1 });
  return { panes: [viewport], order: [viewport.id] };
}

/** The viewport over the sequence over the node graph: the arrangement for working in all three. */
export function workDesk(): Desk {
  const viewport = newPane("viewport", { x: 0, y: 0, w: 1, h: 0.5 });
  const biomol = newPane("biomol", { x: 0, y: 0.5, w: 1, h: 0.22 });
  const graph = newPane("graph", { x: 0, y: 0.72, w: 1, h: 0.28 });
  return { panes: [viewport, biomol, graph], order: [viewport.id, biomol.id, graph.id] };
}

/** The pane and tab showing an editor, if any pane does. */
export function tabShowing(desk: Desk, editor: EditorKind): { pane: Pane; tab: Tab } | null {
  for (const pane of desk.panes) {
    const tab = pane.tabs.find((candidate) => candidate.editor === editor);
    if (tab) return { pane, tab };
  }
  return null;
}

/** The pane whose *active* tab shows an editor. */
export function paneShowing(desk: Desk, editor: EditorKind): Pane | null {
  return desk.panes.find((pane) => pane.tabs.find((tab) => tab.id === pane.active)?.editor === editor) ?? null;
}

export function activeTab(pane: Pane): Tab {
  return pane.tabs.find((tab) => tab.id === pane.active) ?? pane.tabs[0];
}

/**
 * Keeps a pane reachable: never smaller than `MIN_SIZE`, and never so far off the desk that
 * less than `GRIP` of it is left. Panes may hang off the sides and the bottom - that is
 * what makes a desk feel like a desk rather than a grid - but not off the top, since the
 * title bar is what you drag them back by.
 */
function clampPane(pane: Pane): Pane {
  const w = Math.max(MIN_SIZE, pane.w);
  const h = Math.max(MIN_SIZE, pane.h);
  return {
    ...pane,
    w,
    h,
    x: Math.min(1 - GRIP, Math.max(GRIP - w, pane.x)),
    y: Math.min(1 - GRIP, Math.max(0, pane.y)),
  };
}

function update(desk: Desk, id: string, change: (pane: Pane) => Pane): Desk {
  return { ...desk, panes: desk.panes.map((pane) => (pane.id === id ? change(pane) : pane)) };
}

/**
 * Takes a singleton out of wherever it is shown, since it is about to be shown elsewhere.
 *
 * The tab that held it is *removed*, and its pane with it if it held nothing else - which
 * is what moving something means. An earlier version turned it into an Image tab instead,
 * so asking for the Agents panel while it was already open left an Image tab sitting where
 * it had been, which read as having pressed the wrong thing.
 */
function dropSingleton(desk: Desk, editor: EditorKind, exceptTab?: string): Desk {
  if (!SINGLETONS.has(editor)) return desk;
  let next = desk;
  for (const pane of desk.panes) {
    for (const tab of pane.tabs) {
      if (tab.editor !== editor || tab.id === exceptTab) continue;
      next = closeTab(next, pane.id, tab.id);
    }
  }
  return next;
}

/**
 * Adds a pane, on top, a little in from the corner - and each new one a little further, so
 * three added in a row do not land on each other exactly.
 */
export function addPane(desk: Desk, editor: EditorKind = "image", at?: { x: number; y: number }): Desk {
  const step = (desk.panes.length % 5) * 0.04;
  const rect = { x: at ? at.x : 0.3 + step, y: at ? at.y : 0.1 + step, w: 0.45, h: 0.5 };
  // A singleton that is already open moves into the new pane rather than being copied.
  const existing = SINGLETONS.has(editor) ? tabShowing(desk, editor) : null;
  if (existing) return detachTab(desk, existing.pane.id, existing.tab.id, rect);
  const pane = clampPane(newPane(editor, rect));
  return { panes: [...desk.panes, pane], order: [...desk.order, pane.id] };
}

/** Any pane can go, the last one too: a right-click on the empty desk makes a new one. */
export function removePane(desk: Desk, id: string): Desk {
  return {
    panes: desk.panes.filter((pane) => pane.id !== id),
    order: desk.order.filter((other) => other !== id),
  };
}

export function bringToFront(desk: Desk, id: string): Desk {
  if (!desk.order.includes(id)) return desk;
  return { ...desk, order: [...desk.order.filter((other) => other !== id), id] };
}

/** Moves a pane. A moved pane is no longer "snapped", so its remembered place is dropped. */
export function movePane(desk: Desk, id: string, x: number, y: number): Desk {
  return update(desk, id, (pane) => {
    const { before: _before, ...rest } = pane;
    return clampPane({ ...rest, x, y });
  });
}

/** Which edges a resize handle drags. */
export type Edge = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";

/**
 * Resizes by dragging an edge or a corner: the opposite edge stays put, which is what
 * every window does. `dx`/`dy` are in desk fractions.
 */
export function resizePane(desk: Desk, id: string, edge: Edge, dx: number, dy: number): Desk {
  return update(desk, id, (pane) => {
    const { before: _before, ...rest } = pane;
    let { x, y, w, h } = rest;
    if (edge.includes("e")) w = Math.max(MIN_SIZE, w + dx);
    if (edge.includes("s")) h = Math.max(MIN_SIZE, h + dy);
    if (edge.includes("w")) {
      const next = Math.max(MIN_SIZE, w - dx);
      x += w - next;
      w = next;
    }
    if (edge.includes("n")) {
      const next = Math.max(MIN_SIZE, h - dy);
      y += h - next;
      h = next;
    }
    return clampPane({ ...rest, x, y, w, h });
  });
}

// --- magnetism --------------------------------------------------------------------------

/** Which sides of a rectangle a gesture is moving: all four for a move, one or two for a resize. */
export interface Moving {
  left: boolean;
  right: boolean;
  top: boolean;
  bottom: boolean;
}

export const ALL_SIDES: Moving = { left: true, right: true, top: true, bottom: true };

export function movingSides(edge: Edge): Moving {
  return { left: edge.includes("w"), right: edge.includes("e"), top: edge.includes("n"), bottom: edge.includes("s") };
}

/**
 * Pulls a rectangle's moving sides onto nearby edges - the desk's, and the other panes'
 * - so that dragging a pane up to its neighbour leaves no sliver between them and no
 * overlap, the way window managers do. `tx`/`ty` are how close counts, in desk fractions;
 * the caller derives them from a few pixels. Of the candidates within reach the closest
 * wins, per axis. For a move the whole rectangle shifts; for a resize only the dragged
 * side moves and the opposite one stays put.
 */
export function magnetise(desk: Desk, id: string, rect: Rect, moving: Moving, tx: number, ty: number): Rect {
  const xs: number[] = [0, 1];
  const ys: number[] = [0, 1];
  for (const pane of desk.panes) {
    if (pane.id === id) continue;
    // KWin's rule: another window's edge attracts only when the two windows share some of
    // the other axis - a pane far below has no business pulling this one sideways.
    const sharesY = pane.y < rect.y + rect.h + ty && rect.y < pane.y + pane.h + ty;
    const sharesX = pane.x < rect.x + rect.w + tx && rect.x < pane.x + pane.w + tx;
    if (sharesY) xs.push(pane.x, pane.x + pane.w);
    if (sharesX) ys.push(pane.y, pane.y + pane.h);
  }
  const nearest = (value: number, candidates: number[], reach: number): number | null => {
    let best: number | null = null;
    for (const candidate of candidates) {
      const distance = Math.abs(candidate - value);
      if (distance <= reach && (best === null || distance < Math.abs(best - value))) best = candidate;
    }
    return best;
  };

  let { x, y, w, h } = rect;
  const leftHit = moving.left ? nearest(x, xs, tx) : null;
  const rightHit = moving.right ? nearest(x + w, xs, tx) : null;
  if (moving.left && moving.right) {
    // A move: shift by whichever side is closer to something.
    const dl = leftHit === null ? Infinity : Math.abs(leftHit - x);
    const dr = rightHit === null ? Infinity : Math.abs(rightHit - (x + w));
    if (dl <= dr && leftHit !== null) x = leftHit;
    else if (rightHit !== null) x = rightHit - w;
  } else if (moving.left && leftHit !== null) {
    w = Math.max(MIN_SIZE, x + w - leftHit);
    x = x + rect.w - w;
  } else if (moving.right && rightHit !== null) {
    w = Math.max(MIN_SIZE, rightHit - x);
  }

  const topHit = moving.top ? nearest(y, ys, ty) : null;
  const bottomHit = moving.bottom ? nearest(y + h, ys, ty) : null;
  if (moving.top && moving.bottom) {
    const dt = topHit === null ? Infinity : Math.abs(topHit - y);
    const db = bottomHit === null ? Infinity : Math.abs(bottomHit - (y + h));
    if (dt <= db && topHit !== null) y = topHit;
    else if (bottomHit !== null) y = bottomHit - h;
  } else if (moving.top && topHit !== null) {
    h = Math.max(MIN_SIZE, y + h - topHit);
    y = y + rect.h - h;
  } else if (moving.bottom && bottomHit !== null) {
    h = Math.max(MIN_SIZE, bottomHit - y);
  }
  return { x, y, w, h };
}

/**
 * The desk's edges resist, as xfwm4's do: a pane dragged into the left, right or bottom
 * edge stops there, and only a further push - past `rx`/`ry`, a few dozen pixels - lets it
 * hang off the desk. Distinct from the magnet, which only pulls from inside: this catches
 * a pane however fast it arrives, since the whole band beyond the edge counts, and it is
 * what makes a boundary feel like one. The top edge is a hard stop, since the title bar
 * must stay reachable. Stateless: the drag hands over where the pane would be from the
 * pointer's travel alone, so easing off the pointer eases the pane back too.
 */
export function resistEdges(rect: Rect, rx: number, ry: number): Rect {
  let { x, y } = rect;
  const right = x + rect.w;
  const bottom = y + rect.h;
  if (x < 0 && x > -rx) x = 0;
  else if (right > 1 && right < 1 + rx) x = 1 - rect.w;
  if (bottom > 1 && bottom < 1 + ry) y = 1 - rect.h;
  if (y < 0) y = 0;
  return { ...rect, x, y };
}

/** Puts a pane exactly where told. Used after magnetising a dragged rectangle. */
export function placePane(desk: Desk, id: string, rect: Rect): Desk {
  return update(desk, id, (pane) => {
    const { before: _before, ...rest } = pane;
    return clampPane({ ...rest, ...rect });
  });
}

// --- tabs -------------------------------------------------------------------------------

/**
 * Adds a tab to a pane and shows it. A singleton can be in one place only, so adding one
 * that is shown elsewhere *moves* it here - "put the graph here" is what was meant, not
 * "tell me where it is".
 */
export function addTab(desk: Desk, paneId: string, editor: EditorKind): Desk {
  const existing = SINGLETONS.has(editor) ? tabShowing(desk, editor) : null;
  if (existing) {
    // Already open: show it, here or where it is. Asking for the one graph twice does not
    // make two of it, and must not leave a stray tab behind either.
    if (existing.pane.id === paneId) return activateTab(desk, paneId, existing.tab.id);
    return activateTab(moveTab(desk, existing.pane.id, existing.tab.id, paneId), paneId, existing.tab.id);
  }
  const tab: Tab = { id: freshId("tab"), editor };
  return update(desk, paneId, (pane) => ({ ...pane, tabs: [...pane.tabs, tab], active: tab.id }));
}

/** Changes what a tab shows, with the same singleton rule. */
export function setTabEditor(desk: Desk, paneId: string, tabId: string, editor: EditorKind): Desk {
  const cleared = dropSingleton(desk, editor, tabId);
  // Dropping the singleton's old tab can take its pane with it - and that pane may be this
  // one, in which case there is nothing left to change.
  if (!cleared.panes.some((pane) => pane.id === paneId && pane.tabs.some((tab) => tab.id === tabId))) return cleared;
  return update(cleared, paneId, (pane) => ({
    ...pane,
    tabs: pane.tabs.map((tab) => (tab.id === tabId ? { ...tab, editor } : tab)),
  }));
}

export function activateTab(desk: Desk, paneId: string, tabId: string): Desk {
  return update(desk, paneId, (pane) => (pane.tabs.some((tab) => tab.id === tabId) ? { ...pane, active: tabId } : pane));
}

/** Closes a tab. Closing the last tab closes the pane, as it does in a browser. */
export function closeTab(desk: Desk, paneId: string, tabId: string): Desk {
  const pane = desk.panes.find((candidate) => candidate.id === paneId);
  if (!pane) return desk;
  const remaining = pane.tabs.filter((tab) => tab.id !== tabId);
  if (remaining.length === 0) return removePane(desk, paneId);
  const index = pane.tabs.findIndex((tab) => tab.id === tabId);
  // The neighbour to the left takes over, or the first tab when the first was closed.
  const active = pane.active === tabId ? remaining[Math.max(0, index - 1)].id : pane.active;
  return update(desk, paneId, (current) => ({ ...current, tabs: remaining, active }));
}

/** Moves a tab out of its pane into a new pane of its own - dragging a tab off the strip. */
export function detachTab(desk: Desk, paneId: string, tabId: string, at: { x: number; y: number }): Desk {
  const pane = desk.panes.find((candidate) => candidate.id === paneId);
  const tab = pane?.tabs.find((candidate) => candidate.id === tabId);
  if (!pane || !tab) return desk;
  const fresh: Pane = clampPane({ id: freshId(), tabs: [tab], active: tab.id, x: at.x, y: at.y, w: pane.w, h: pane.h });
  const without = closeTab(desk, paneId, tabId);
  return { panes: [...without.panes, fresh], order: [...without.order, fresh.id] };
}

/**
 * Moves a tab into another pane's strip - at `index`, or the end - and shows it there.
 * Within one pane this is a reorder, the drag along the strip a browser has.
 */
export function moveTab(desk: Desk, fromPane: string, tabId: string, toPane: string, index?: number): Desk {
  const pane = desk.panes.find((candidate) => candidate.id === fromPane);
  const tab = pane?.tabs.find((candidate) => candidate.id === tabId);
  if (!pane || !tab || !desk.panes.some((candidate) => candidate.id === toPane)) return desk;
  if (fromPane === toPane) {
    const rest = pane.tabs.filter((candidate) => candidate.id !== tabId);
    const at = Math.min(rest.length, Math.max(0, index ?? rest.length));
    return update(desk, fromPane, (current) => ({
      ...current,
      tabs: [...rest.slice(0, at), tab, ...rest.slice(at)],
      active: tab.id,
    }));
  }
  const without = pane.tabs.length === 1 ? removePane(desk, fromPane) : closeTab(desk, fromPane, tabId);
  return update(without, toPane, (target) => {
    const at = Math.min(target.tabs.length, Math.max(0, index ?? target.tabs.length));
    return { ...target, tabs: [...target.tabs.slice(0, at), tab, ...target.tabs.slice(at)], active: tab.id };
  });
}

// --- snapping ---------------------------------------------------------------------------

/** The places a dragged pane can snap to, as a desktop's window manager offers them. */
export type Snap =
  | "full"
  | "left"
  | "right"
  | "top"
  | "bottom"
  | "top-left"
  | "top-right"
  | "bottom-left"
  | "bottom-right";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

const SNAP_RECTS: Record<Snap, Rect> = {
  full: { x: 0, y: 0, w: 1, h: 1 },
  left: { x: 0, y: 0, w: 0.5, h: 1 },
  right: { x: 0.5, y: 0, w: 0.5, h: 1 },
  top: { x: 0, y: 0, w: 1, h: 0.5 },
  bottom: { x: 0, y: 0.5, w: 1, h: 0.5 },
  "top-left": { x: 0, y: 0, w: 0.5, h: 0.5 },
  "top-right": { x: 0.5, y: 0, w: 0.5, h: 0.5 },
  "bottom-left": { x: 0, y: 0.5, w: 0.5, h: 0.5 },
  "bottom-right": { x: 0.5, y: 0.5, w: 0.5, h: 0.5 },
};

export const SNAPS: Snap[] = [
  "top-left",
  "top-right",
  "bottom-left",
  "bottom-right",
  "left",
  "right",
  "top",
  "bottom",
  "full",
];

export const SNAP_LABELS: Record<Snap, string> = {
  "top-left": "Top left quarter",
  "top-right": "Top right quarter",
  "bottom-left": "Bottom left quarter",
  "bottom-right": "Bottom right quarter",
  left: "Left half",
  right: "Right half",
  top: "Top half",
  bottom: "Bottom half",
  full: "Fill the desk",
};

export function snapRect(snap: Snap): Rect {
  return SNAP_RECTS[snap];
}

const EPSILON = 1e-6;

function sameRect(a: Rect, b: Rect): boolean {
  return (
    Math.abs(a.x - b.x) < EPSILON &&
    Math.abs(a.y - b.y) < EPSILON &&
    Math.abs(a.w - b.w) < EPSILON &&
    Math.abs(a.h - b.h) < EPSILON
  );
}

function overlaps(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w - EPSILON && b.x < a.x + a.w - EPSILON && a.y < b.y + b.h - EPSILON && b.y < a.y + a.h - EPSILON;
}

function contains(rect: Rect, point: { x: number; y: number }): boolean {
  return point.x >= rect.x && point.x <= rect.x + rect.w && point.y >= rect.y && point.y <= rect.y + rect.h;
}

/** The snap positions the other panes already sit in - the tiles of the desk in use. */
export function occupiedSnaps(desk: Desk, except: string): Snap[] {
  return SNAPS.filter((snap) =>
    desk.panes.some((pane) => pane.id !== except && !pane.collapsed && sameRect(pane, SNAP_RECTS[snap])),
  );
}

/**
 * Which snap the pointer is asking for while a pane is dragged, or null for none.
 *
 * Two rules. At the desk's edges, the one Windows and most Linux desktops share: the top
 * edge means fill the desk, a side edge means that half, a corner means that quarter -
 * and a corner is a region along both edges rather than a point, as Chrome's window does
 * when dragged to a corner. Away from the edges, Chrome's other rule: when the desk is
 * already tiled - some other pane sits in a half or a quarter - dropping over the part
 * still free takes that part, the largest free tile under the pointer. Two quarters
 * across the top and a third pane dragged below them gets the bottom half without
 * having to find the bottom edge.
 */
export function snapAt(
  pointer: { x: number; y: number },
  occupied: Snap[] = [],
  margin = 0.03,
  corner = 0.18,
): Snap | null {
  const nearLeft = pointer.x <= margin;
  const nearRight = pointer.x >= 1 - margin;
  const nearTop = pointer.y <= margin;
  const nearBottom = pointer.y >= 1 - margin;
  const highUp = pointer.y <= corner;
  const lowDown = pointer.y >= 1 - corner;
  const farLeft = pointer.x <= corner;
  const farRight = pointer.x >= 1 - corner;
  if ((nearTop && farLeft) || (nearLeft && highUp)) return "top-left";
  if ((nearTop && farRight) || (nearRight && highUp)) return "top-right";
  if ((nearBottom && farLeft) || (nearLeft && lowDown)) return "bottom-left";
  if ((nearBottom && farRight) || (nearRight && lowDown)) return "bottom-right";
  if (nearTop) return "full";
  if (nearLeft) return "left";
  if (nearRight) return "right";
  if (nearBottom) return "bottom";

  if (occupied.length === 0) return null;
  const taken = occupied.map((snap) => SNAP_RECTS[snap]);
  const free = SNAPS.filter((snap) => snap !== "full")
    .map((snap) => ({ snap, rect: SNAP_RECTS[snap] }))
    .filter(({ rect }) => contains(rect, pointer) && !taken.some((other) => overlaps(rect, other)));
  if (free.length === 0) return null;
  free.sort((a, b) => b.rect.w * b.rect.h - a.rect.w * a.rect.h);
  return free[0].snap;
}

/**
 * Puts a pane into a snap position, remembering where it was so `restorePane` can bring
 * it back - the round trip a maximise button makes. Snapping a pane that is already
 * snapped keeps the first remembered place, so left-then-right-then-restore returns to
 * where it was before either.
 */
export function snapPane(desk: Desk, id: string, snap: Snap): Desk {
  return update(desk, id, (pane) => {
    const before = pane.before ?? { x: pane.x, y: pane.y, w: pane.w, h: pane.h };
    return { ...pane, ...SNAP_RECTS[snap], before, collapsed: false };
  });
}

/** Back to where a pane was before it was snapped. A pane that was never snapped stays. */
export function restorePane(desk: Desk, id: string): Desk {
  return update(desk, id, (pane) => {
    if (!pane.before) return pane;
    const { before, ...rest } = pane;
    return clampPane({ ...rest, ...before });
  });
}

/** Fills the desk, or comes back from having filled it - what a title bar's button does. */
export function toggleMaximise(desk: Desk, id: string): Desk {
  const pane = desk.panes.find((candidate) => candidate.id === id);
  if (!pane) return desk;
  const filling = sameRect(pane, SNAP_RECTS.full);
  return filling && pane.before ? restorePane(desk, id) : snapPane(desk, id, "full");
}

/**
 * The largest rectangle a pane can grow to from where it is without covering any other
 * pane: each side moves out until it meets the desk's edge or the nearest edge of a pane
 * that shares some of the other axis. Panes already overlapping this one are not in the
 * way - their edges lie inside it, not beyond it.
 */
export function freeSpaceAround(desk: Desk, id: string): Rect | null {
  const pane = desk.panes.find((candidate) => candidate.id === id);
  if (!pane) return null;
  const right = pane.x + pane.w;
  const bottom = pane.y + pane.h;
  let left = 0;
  let rightBound = 1;
  let top = 0;
  let bottomBound = 1;
  for (const other of desk.panes) {
    if (other.id === id) continue;
    const otherRight = other.x + other.w;
    const otherBottom = other.y + other.h;
    const sharesY = other.y < bottom - EPSILON && pane.y < otherBottom - EPSILON;
    const sharesX = other.x < right - EPSILON && pane.x < otherRight - EPSILON;
    if (sharesY) {
      if (otherRight <= pane.x + EPSILON) left = Math.max(left, otherRight);
      if (other.x >= right - EPSILON) rightBound = Math.min(rightBound, other.x);
    }
    if (sharesX) {
      if (otherBottom <= pane.y + EPSILON) top = Math.max(top, otherBottom);
      if (other.y >= bottom - EPSILON) bottomBound = Math.min(bottomBound, other.y);
    }
  }
  // A pane hanging off the desk keeps what hangs off; only the free side grows.
  left = Math.min(left, pane.x);
  top = Math.min(top, pane.y);
  rightBound = Math.max(rightBound, right);
  bottomBound = Math.max(bottomBound, bottom);
  return { x: left, y: top, w: rightBound - left, h: bottomBound - top };
}

/**
 * What a double-click on the title does, in two steps: first grow into the free space
 * around the pane, touching nothing else; from there, fill the desk; and from filled, come
 * back to where it started. One gesture, three states, each remembered through `before`.
 */
export function cycleFit(desk: Desk, id: string): Desk {
  const pane = desk.panes.find((candidate) => candidate.id === id);
  if (!pane) return desk;
  if (sameRect(pane, SNAP_RECTS.full)) return pane.before ? restorePane(desk, id) : desk;
  const fit = freeSpaceAround(desk, id);
  if (fit && !sameRect(fit, pane)) {
    return update(desk, id, (current) => {
      const before = current.before ?? { x: current.x, y: current.y, w: current.w, h: current.h };
      return { ...current, ...fit, before, collapsed: false };
    });
  }
  return snapPane(desk, id, "full");
}

/** A pane folded down to its title bar, as a minimised window that stays where it is. */
export function toggleCollapse(desk: Desk, id: string): Desk {
  return update(desk, id, (pane) => ({ ...pane, collapsed: !pane.collapsed }));
}

// --- persistence ------------------------------------------------------------------------

/** What the whole screen remembers: the desk in the middle, and a column each side. */
export interface ScreenLayout {
  desk: Desk;
  /** The left column - the outliner and the properties. */
  sideWidth: number;
  /** The right column - the conversation. */
  chatWidth: number;
  /** Whether the conversation is open, or folded to a strip. */
  chatOpen: boolean;
}

export function parseScreen(text: string | null): ScreenLayout | null {
  if (!text) return null;
  try {
    const parsed: unknown = JSON.parse(text);
    if (typeof parsed !== "object" || parsed === null) return null;
    const candidate = parsed as {
      desk?: unknown;
      sideWidth?: unknown;
      chatWidth?: unknown;
      chatOpen?: unknown;
      agentsOpen?: unknown;
    };
    const desk = readDesk(candidate.desk);
    if (!desk) return null;
    const width = typeof candidate.sideWidth === "number" ? candidate.sideWidth : 272;
    const chat = typeof candidate.chatWidth === "number" ? candidate.chatWidth : 320;
    // `agentsOpen` is what the version that kept the conversation in the right column
    // called this, when it was a section rather than a column of its own.
    const open = candidate.chatOpen ?? candidate.agentsOpen;
    return {
      desk,
      sideWidth: Math.min(600, Math.max(180, width)),
      chatWidth: Math.min(700, Math.max(220, chat)),
      chatOpen: open !== false,
    };
  } catch {
    return null;
  }
}

/**
 * An editor name from a saved layout.
 *
 * `sequence` was the strip before it became the BioMol panel. A name this version has no
 * editor for - `agents`, which moved to the right column - reads as nothing, and the tab
 * is dropped rather than taking the whole saved layout down with it.
 */
function readEditor(name: string): EditorKind | null {
  if (name === "sequence") return "biomol";
  // Own keys only: `in` would also say yes to "toString" and "constructor".
  return Object.hasOwn(EDITOR_LABELS, name) ? (name as EditorKind) : null;
}

function isRect(value: Record<string, unknown>): boolean {
  return ["x", "y", "w", "h"].every((key) => typeof value[key] === "number" && Number.isFinite(value[key]));
}

/**
 * Reads a saved desk, accepting the shape before tabs existed - a pane with one `editor`
 * - as a pane with one tab, so an arrangement saved then still comes back.
 */
function readDesk(value: unknown): Desk | null {
  if (typeof value !== "object" || value === null) return null;
  const desk = value as { panes?: unknown; order?: unknown };
  if (!Array.isArray(desk.panes) || !Array.isArray(desk.order)) return null;
  if (!desk.order.every((id: unknown) => typeof id === "string")) return null;
  const panes: Pane[] = [];
  const seen = new Set<string>();
  // A singleton named twice - a hand-edited file, or one written by a bug - is kept where
  // it first appears. Two viewport tabs would be two WebGL contexts fighting over one scene.
  const singletons = new Set<EditorKind>();
  const admit = (editor: EditorKind | null): editor is EditorKind => {
    if (!editor) return false;
    if (!SINGLETONS.has(editor)) return true;
    if (singletons.has(editor)) return false;
    singletons.add(editor);
    return true;
  };
  for (const raw of desk.panes) {
    if (typeof raw !== "object" || raw === null) return null;
    const p = raw as Record<string, unknown>;
    if (typeof p.id !== "string" || !isRect(p)) return null;
    // Two panes with one id are one pane as far as the screen can tell.
    if (seen.has(p.id)) continue;
    seen.add(p.id);
    reserveId(p.id);
    const rect = { x: p.x as number, y: p.y as number, w: p.w as number, h: p.h as number };
    const extra: Partial<Pane> = {};
    if (typeof p.collapsed === "boolean") extra.collapsed = p.collapsed;
    if (typeof p.before === "object" && p.before !== null && isRect(p.before as Record<string, unknown>)) {
      const b = p.before as Record<string, number>;
      extra.before = { x: b.x, y: b.y, w: b.w, h: b.h };
    }
    if (typeof p.editor === "string") {
      const editor = readEditor(p.editor);
      if (!admit(editor)) continue;
      const tab: Tab = { id: freshId("tab"), editor };
      panes.push({ id: p.id, tabs: [tab], active: tab.id, ...rect, ...extra });
      continue;
    }
    if (!Array.isArray(p.tabs) || p.tabs.length === 0) return null;
    const tabs: Tab[] = [];
    for (const t of p.tabs) {
      if (typeof t !== "object" || t === null) return null;
      const tab = t as Record<string, unknown>;
      if (typeof tab.id !== "string" || typeof tab.editor !== "string") return null;
      const editor = readEditor(tab.editor);
      if (seen.has(tab.id)) continue;
      seen.add(tab.id);
      reserveId(tab.id);
      // A tab this version cannot show is left out; a pane left with none goes with it.
      if (admit(editor)) tabs.push({ id: tab.id, editor });
    }
    if (tabs.length === 0) continue;
    const active = typeof p.active === "string" && tabs.some((tab) => tab.id === p.active) ? p.active : tabs[0].id;
    panes.push({ id: p.id, tabs, active, ...rect, ...extra });
  }
  // The stacking order is rebuilt around the panes that survived: a pane it leaves out
  // could never be raised, and an id it names that is not a pane is just noise. Each pane
  // is also put back within reach - a saved rectangle of no size, or one far off the
  // desk, is a pane that exists and cannot be found.
  const ids = new Set(panes.map((pane) => pane.id));
  const stacked = (desk.order).filter(
    (id, at, all) => ids.has(id) && all.indexOf(id) === at,
  );
  const order = [...panes.map((pane) => pane.id).filter((id) => !stacked.includes(id)), ...stacked];
  return { panes: panes.map(clampPane), order };
}
