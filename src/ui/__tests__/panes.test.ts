import { describe, expect, it } from "vitest";
import {
  activateTab,
  addPane,
  activeTab,
  addTab,
  bringToFront,
  closeTab,
  cycleFit,
  defaultDesk,
  workDesk,
  freeSpaceAround,
  detachTab,
  GRIP,
  magnetise,
  ALL_SIDES,
  movingSides,
  MIN_SIZE,
  moveTab,
  movePane,
  occupiedSnaps,
  paneShowing,
  parseScreen,
  removePane,
  resistEdges,
  resizePane,
  restorePane,
  setTabEditor,
  snapAt,
  snapPane,
  tabShowing,
  toggleCollapse,
  toggleMaximise,
} from "../panes";

const editors = (desk: ReturnType<typeof workDesk>) =>
  desk.panes.map((pane) => pane.tabs.map((tab) => tab.editor).join("+"));

describe("the desk of panes", () => {
  it("opens with the viewport, the BioMol panel and the graph, edge to edge", () => {
    const desk = workDesk();
    expect(editors(desk)).toEqual(["viewport", "biomol", "graph"]);
    expect(desk.panes[0]).toMatchObject({ x: 0, y: 0, w: 1, h: 0.5 });
    expect(desk.panes[1]).toMatchObject({ x: 0, y: 0.5, w: 1, h: 0.22 });
    expect(desk.panes[2]).toMatchObject({ x: 0, y: 0.72, w: 1, h: 0.28 });
  });

  it("moves a pane by its title and keeps a grip of it on the desk", () => {
    const desk = workDesk();
    const graph = paneShowing(desk, "graph")!;
    const moved = movePane(desk, graph.id, 0.4, 0.3);
    // A full-width pane can hang off the right: what is asked for is what happens.
    expect(paneShowing(moved, "graph")).toMatchObject({ x: 0.4, y: 0.3, w: 1, h: 0.28 });
    // Dragged off the edge, a grip's worth stays: a window you cannot reach is lost, but a
    // window that cannot leave the screen is a grid cell.
    const off = movePane(desk, graph.id, 2, 2);
    expect(paneShowing(off, "graph")).toMatchObject({ x: 1 - GRIP, y: 1 - GRIP });
    const left = movePane(desk, graph.id, -2, -2);
    expect(paneShowing(left, "graph")).toMatchObject({ x: GRIP - 1, y: 0 });
  });

  it("resizes from an edge with the opposite edge fixed, down to a minimum", () => {
    const desk = workDesk();
    const viewport = paneShowing(desk, "viewport")!;
    const fromEast = resizePane(desk, viewport.id, "e", -0.3, 0);
    expect(paneShowing(fromEast, "viewport")).toMatchObject({ x: 0, w: 0.7 });
    const fromNorth = resizePane(desk, viewport.id, "n", 0, 0.2);
    expect(paneShowing(fromNorth, "viewport")!.y).toBeCloseTo(0.2);
    expect(paneShowing(fromNorth, "viewport")!.h).toBeCloseTo(0.3);
    // The west edge moves x as it shrinks the width, so the east edge does not move.
    const fromWest = resizePane(desk, viewport.id, "w", 0.25, 0);
    expect(paneShowing(fromWest, "viewport")).toMatchObject({ x: 0.25, w: 0.75 });
    const tiny = resizePane(desk, viewport.id, "se", -5, -5);
    expect(paneShowing(tiny, "viewport")).toMatchObject({ w: MIN_SIZE, h: MIN_SIZE });
  });

  it("stacks the pane you touched last on top", () => {
    const desk = workDesk();
    const viewport = paneShowing(desk, "viewport")!;
    expect(desk.order.at(-1)).not.toBe(viewport.id);
    expect(bringToFront(desk, viewport.id).order.at(-1)).toBe(viewport.id);
  });

  it("adds panes on top, where asked, and removes down to an empty desk", () => {
    let desk = addPane(workDesk(), "image");
    expect(desk.panes).toHaveLength(4);
    expect(desk.order.at(-1)).toBe(desk.panes[3].id);
    desk = addPane(desk, "text", { x: 0.2, y: 0.3 });
    expect(desk.panes[4]).toMatchObject({ x: 0.2, y: 0.3 });
    expect(editors(desk)[4]).toBe("text");
    while (desk.panes.length) desk = removePane(desk, desk.panes[0].id);
    expect(desk.panes).toHaveLength(0);
    expect(desk.order).toHaveLength(0);
  });

  it("moves a singleton rather than showing it twice, leaving nothing behind", () => {
    let desk = addPane(workDesk(), "image");
    const image = desk.panes[3];
    // The graph pane held only the graph, so asking for it here takes that pane with it.
    desk = setTabEditor(desk, image.id, image.tabs[0].id, "graph");
    expect(editors(desk)).toEqual(["viewport", "biomol", "graph"]);
    // Adding a singleton pane moves it into the new pane; there is still only one.
    desk = addPane(desk, "viewport");
    expect(desk.panes.flatMap((pane) => pane.tabs).filter((tab) => tab.editor === "viewport")).toHaveLength(1);
    expect(tabShowing(desk, "viewport")!.pane.id).toBe(desk.panes.at(-1)!.id);
  });

  it("shows a singleton that is already open rather than making a second and an Image", () => {
    // The bug this is here for: asking for a panel that was already open turned the tab
    // that had it into an Image and added another - so pressing BioMol, or the Agents
    // panel that used to be here, looked like pressing Image.
    let desk = workDesk();
    const graphPane = desk.panes[2].id;
    desk = addTab(desk, graphPane, "biomol");
    expect(editors(desk)).toEqual(["viewport", "graph+biomol"]);
    // Asking again in the same pane just shows it.
    const before = desk.panes[1].tabs.length;
    desk = addTab(desk, graphPane, "biomol");
    expect(desk.panes[1].tabs).toHaveLength(before);
    expect(activeTab(desk.panes[1]).editor).toBe("biomol");
    expect(desk.panes.flatMap((pane) => pane.tabs).some((tab) => tab.editor === "image")).toBe(false);
    // Asking for it in another pane moves it there, and still leaves no Image.
    desk = addTab(desk, desk.panes[0].id, "biomol");
    expect(editors(desk)).toEqual(["viewport+biomol", "graph"]);
    expect(desk.panes.flatMap((pane) => pane.tabs).filter((tab) => tab.editor === "biomol")).toHaveLength(1);
    // And a new pane for it moves it again rather than copying it.
    desk = addPane(desk, "biomol", { x: 0.2, y: 0.2 });
    expect(editors(desk)).toEqual(["viewport", "graph", "biomol"]);
  });
});

describe("tabs", () => {
  it("adds a tab and shows it, and pulls a singleton in from elsewhere", () => {
    let desk = workDesk();
    const viewport = desk.panes[0];
    desk = addTab(desk, viewport.id, "image");
    expect(editors(desk)[0]).toBe("viewport+image");
    expect(desk.panes[0].active).toBe(desk.panes[0].tabs[1].id);
    // The graph comes here from its own pane, which held nothing else and so goes with it.
    desk = addTab(desk, viewport.id, "graph");
    expect(editors(desk)).toEqual(["viewport+image+graph", "biomol"]);
  });

  it("activates, closes with the left neighbour taking over, and closes the pane with its last tab", () => {
    let desk = workDesk();
    const id = desk.panes[0].id;
    desk = addTab(desk, id, "image");
    desk = addTab(desk, id, "text");
    const [a, b, c] = desk.panes[0].tabs;
    desk = activateTab(desk, id, a.id);
    expect(desk.panes[0].active).toBe(a.id);
    desk = activateTab(desk, id, c.id);
    desk = closeTab(desk, id, c.id);
    expect(desk.panes[0].active).toBe(b.id);
    desk = closeTab(desk, id, a.id);
    expect(desk.panes[0].active).toBe(b.id);
    desk = closeTab(desk, id, b.id);
    expect(desk.panes.some((pane) => pane.id === id)).toBe(false);
  });

  it("detaches a tab into a pane of its own and moves a tab between panes", () => {
    let desk = workDesk();
    const id = desk.panes[0].id;
    desk = addTab(desk, id, "image");
    const image = desk.panes[0].tabs[1];
    desk = detachTab(desk, id, image.id, { x: 0.2, y: 0.2 });
    expect(editors(desk)).toEqual(["viewport", "biomol", "graph", "image"]);
    expect(desk.panes[3]).toMatchObject({ x: 0.2, y: 0.2, w: 1, h: 0.5 });
    expect(desk.order.at(-1)).toBe(desk.panes[3].id);
    desk = moveTab(desk, desk.panes[3].id, image.id, desk.panes[2].id);
    expect(editors(desk)).toEqual(["viewport", "biomol", "graph+image"]);
    expect(desk.panes[2].active).toBe(image.id);
  });

  it("reorders along a strip and drops into another strip at a place", () => {
    let desk = workDesk();
    const id = desk.panes[0].id;
    desk = addTab(desk, id, "image");
    desk = addTab(desk, id, "text");
    const [viewport, image, text] = desk.panes[0].tabs;
    desk = moveTab(desk, id, text.id, id, 0);
    expect(desk.panes[0].tabs.map((tab) => tab.id)).toEqual([text.id, viewport.id, image.id]);
    desk = moveTab(desk, id, text.id, id, 99);
    expect(desk.panes[0].tabs.map((tab) => tab.id)).toEqual([viewport.id, image.id, text.id]);
    // Into the graph pane, ahead of the graph tab.
    const graphPane = desk.panes[2].id;
    desk = moveTab(desk, id, image.id, graphPane, 0);
    expect(editors(desk)[2]).toBe("image+graph");
    expect(desk.panes[2].active).toBe(image.id);
    // The last tab of a pane moving out takes the pane with it.
    const seq = desk.panes[1];
    desk = moveTab(desk, seq.id, seq.tabs[0].id, graphPane);
    expect(editors(desk)).toEqual(["viewport+text", "image+graph+biomol"]);
  });
});

describe("magnetism", () => {
  it("pulls a moved pane onto a neighbour's edge or the desk's, the closer one", () => {
    const desk = workDesk();
    const [viewport, sequence] = desk.panes;
    // Just below the BioMol pane's bottom edge (0.72): lands on it.
    const near = magnetise(desk, viewport.id, { x: 0.3, y: 0.727, w: 0.4, h: 0.2 }, ALL_SIDES, 0.01, 0.01);
    expect(near.y).toBeCloseTo(0.72);
    expect(near.x).toBe(0.3);
    // The right side within reach of the desk's edge: shifts so it touches it.
    const right = magnetise(desk, viewport.id, { x: 0.595, y: 0.3, w: 0.4, h: 0.2 }, ALL_SIDES, 0.01, 0.01);
    expect(right.x).toBeCloseTo(0.6);
    // Too far from anything: untouched.
    const free = magnetise(desk, sequence.id, { x: 0.3, y: 0.3, w: 0.3, h: 0.2 }, ALL_SIDES, 0.01, 0.01);
    expect(free).toEqual({ x: 0.3, y: 0.3, w: 0.3, h: 0.2 });
  });

  it("ignores edges of panes that share none of the other axis", () => {
    // A narrow pane at the far right; another dragged along the bottom-left, whose top is
    // near the narrow pane's bottom but which shares no x range with it: no pull.
    let desk = workDesk();
    while (desk.panes.length) desk = removePane(desk, desk.panes[0].id);
    desk = addPane(desk, "image", { x: 0.8, y: 0 });
    desk = { ...desk, panes: [{ ...desk.panes[0], w: 0.2, h: 0.5 }] };
    desk = addPane(desk, "text", { x: 0, y: 0.7 });
    const mover = desk.panes[1];
    const away = magnetise(desk, mover.id, { x: 0.1, y: 0.505, w: 0.3, h: 0.2 }, ALL_SIDES, 0.01, 0.01);
    expect(away.y).toBe(0.505);
    // Sharing x range, the same top is pulled onto the bottom.
    const under = magnetise(desk, mover.id, { x: 0.7, y: 0.505, w: 0.3, h: 0.2 }, ALL_SIDES, 0.01, 0.01);
    expect(under.y).toBeCloseTo(0.5);
  });

  it("resists at the desk's edges and lets go past the resistance", () => {
    // Pushed 2% past the right edge with 5% of resistance: held at the edge.
    expect(resistEdges({ x: 0.72, y: 0.2, w: 0.3, h: 0.2 }, 0.05, 0.05).x).toBeCloseTo(0.7);
    // Pushed 8% past: released, hangs off.
    expect(resistEdges({ x: 0.78, y: 0.2, w: 0.3, h: 0.2 }, 0.05, 0.05).x).toBeCloseTo(0.78);
    // The left edge and the bottom edge the same way.
    expect(resistEdges({ x: -0.03, y: 0.2, w: 0.3, h: 0.2 }, 0.05, 0.05).x).toBe(0);
    expect(resistEdges({ x: -0.2, y: 0.2, w: 0.3, h: 0.2 }, 0.05, 0.05).x).toBe(-0.2);
    expect(resistEdges({ x: 0.2, y: 0.83, w: 0.3, h: 0.2 }, 0.05, 0.05).y).toBeCloseTo(0.8);
    // The top is a hard stop.
    expect(resistEdges({ x: 0.2, y: -0.4, w: 0.3, h: 0.2 }, 0.05, 0.05).y).toBe(0);
    // Inside the desk, untouched.
    expect(resistEdges({ x: 0.2, y: 0.2, w: 0.3, h: 0.2 }, 0.05, 0.05)).toEqual({ x: 0.2, y: 0.2, w: 0.3, h: 0.2 });
  });

  it("moves only the dragged side of a resize", () => {
    const desk = workDesk();
    const graph = desk.panes[2];
    // Dragging the top edge of the graph up towards the viewport's bottom (0.5): the
    // bottom stays where it was, the top lands on 0.5.
    const rect = { x: 0, y: 0.507, w: 1, h: 0.493 };
    const pulled = magnetise(desk, graph.id, rect, movingSides("n"), 0.01, 0.01);
    expect(pulled.y).toBeCloseTo(0.5);
    expect(pulled.y + pulled.h).toBeCloseTo(1);
    // The east edge near the desk's right: width grows to meet it, x fixed.
    const east = magnetise(desk, graph.id, { x: 0.2, y: 0.7, w: 0.795, h: 0.2 }, movingSides("e"), 0.01, 0.01);
    expect(east.x).toBe(0.2);
    expect(east.w).toBeCloseTo(0.8);
  });
});

describe("snapping", () => {
  it("snaps to the edge the pointer reaches, corners being regions", () => {
    expect(snapAt({ x: 0.5, y: 0.01 })).toBe("full");
    expect(snapAt({ x: 0.0, y: 0.5 })).toBe("left");
    expect(snapAt({ x: 1.0, y: 0.5 })).toBe("right");
    expect(snapAt({ x: 0.5, y: 1.0 })).toBe("bottom");
    expect(snapAt({ x: 0.0, y: 0.0 })).toBe("top-left");
    expect(snapAt({ x: 1.0, y: 1.0 })).toBe("bottom-right");
    expect(snapAt({ x: 0.5, y: 0.5 })).toBeNull();
    // Corners are regions: the left edge in its top fifth is the top-left quadrant, and the
    // top edge near its right end is the top-right one - Chrome's rule.
    expect(snapAt({ x: 0.0, y: 0.15 })).toBe("top-left");
    expect(snapAt({ x: 0.9, y: 0.0 })).toBe("top-right");
    expect(snapAt({ x: 1.0, y: 0.9 })).toBe("bottom-right");
  });

  it("takes the free part of a tiled desk without reaching an edge", () => {
    // Two quarters across the top; a pane dropped anywhere below gets the bottom half.
    expect(snapAt({ x: 0.5, y: 0.7 }, ["top-left", "top-right"])).toBe("bottom");
    // Top-left quarter and right half taken: the bottom-left quarter is what is left.
    expect(snapAt({ x: 0.2, y: 0.7 }, ["top-left", "right"])).toBe("bottom-left");
    // Over a taken tile, nothing.
    expect(snapAt({ x: 0.2, y: 0.2 }, ["top-left", "right"])).toBeNull();
    // An untiled desk does not snap away from the edges.
    expect(snapAt({ x: 0.5, y: 0.7 }, [])).toBeNull();
  });

  it("reports the tiles the other panes occupy", () => {
    let desk = workDesk();
    const [viewport, sequence, graph] = desk.panes;
    desk = snapPane(desk, viewport.id, "top-left");
    desk = snapPane(desk, sequence.id, "top-right");
    expect(occupiedSnaps(desk, graph.id)).toEqual(["top-left", "top-right"]);
    expect(occupiedSnaps(desk, viewport.id)).toEqual(["top-right"]);
  });

  it("snaps a pane and comes back", () => {
    let desk = workDesk();
    const graph = paneShowing(desk, "graph")!;
    desk = snapPane(desk, graph.id, "right");
    expect(paneShowing(desk, "graph")).toMatchObject({ x: 0.5, y: 0, w: 0.5, h: 1 });
    // A second snap keeps the first remembered place.
    desk = snapPane(desk, graph.id, "left");
    desk = restorePane(desk, graph.id);
    expect(paneShowing(desk, "graph")).toMatchObject({ x: 0, y: 0.72, w: 1, h: 0.28 });
    expect(paneShowing(desk, "graph")!.before).toBeUndefined();
  });

  it("maximises and restores from the title bar, and forgets on a move", () => {
    let desk = workDesk();
    const graph = paneShowing(desk, "graph")!;
    desk = toggleMaximise(desk, graph.id);
    expect(paneShowing(desk, "graph")).toMatchObject({ x: 0, y: 0, w: 1, h: 1 });
    desk = toggleMaximise(desk, graph.id);
    expect(paneShowing(desk, "graph")).toMatchObject({ x: 0, y: 0.72, w: 1, h: 0.28 });
    desk = snapPane(desk, graph.id, "left");
    desk = movePane(desk, graph.id, 0.1, 0.1);
    expect(paneShowing(desk, "graph")!.before).toBeUndefined();
  });

  it("finds the free space around a pane, bounded by neighbours that share an axis", () => {
    let desk = workDesk();
    while (desk.panes.length) desk = removePane(desk, desk.panes[0].id);
    desk = addPane(desk, "image", { x: 0.6, y: 0 });
    desk = { ...desk, panes: [{ ...desk.panes[0], w: 0.4, h: 0.5 }] };
    desk = addPane(desk, "text", { x: 0.1, y: 0.1 });
    desk = { ...desk, panes: [desk.panes[0], { ...desk.panes[1], w: 0.3, h: 0.3 }] };
    const small = desk.panes[1].id;
    // Grows left, up and down to the desk; right stops at the image pane's left edge.
    expect(freeSpaceAround(desk, small)).toEqual({ x: 0, y: 0, w: 0.6, h: 1 });
  });

  it("double-click grows into the free space, then fills, then comes back", () => {
    let desk = workDesk();
    while (desk.panes.length) desk = removePane(desk, desk.panes[0].id);
    desk = addPane(desk, "image", { x: 0.6, y: 0 });
    desk = { ...desk, panes: [{ ...desk.panes[0], w: 0.4, h: 0.5 }] };
    desk = addPane(desk, "text", { x: 0.1, y: 0.1 });
    desk = { ...desk, panes: [desk.panes[0], { ...desk.panes[1], w: 0.3, h: 0.3 }] };
    const small = desk.panes[1].id;
    desk = cycleFit(desk, small);
    expect(desk.panes[1]).toMatchObject({ x: 0, y: 0, w: 0.6, h: 1 });
    // The image pane was not touched.
    expect(desk.panes[0]).toMatchObject({ x: 0.6, y: 0, w: 0.4, h: 0.5 });
    desk = cycleFit(desk, small);
    expect(desk.panes[1]).toMatchObject({ x: 0, y: 0, w: 1, h: 1 });
    desk = cycleFit(desk, small);
    expect(desk.panes[1]).toMatchObject({ x: 0.1, y: 0.1, w: 0.3, h: 0.3 });
    expect(desk.panes[1].before).toBeUndefined();
  });

  it("folds a pane to its title bar and back", () => {
    let desk = workDesk();
    const graph = paneShowing(desk, "graph")!;
    desk = toggleCollapse(desk, graph.id);
    expect(paneShowing(desk, "graph")!.collapsed).toBe(true);
    // The rectangle is untouched: unfolding puts the content back where it was.
    expect(paneShowing(desk, "graph")).toMatchObject({ x: 0, y: 0.72, w: 1, h: 0.28 });
    expect(paneShowing(toggleCollapse(desk, graph.id), "graph")!.collapsed).toBe(false);
  });
});

describe("persistence", () => {
  it("round-trips through JSON with the side width, clamped", () => {
    const desk = workDesk();
    expect(parseScreen(JSON.stringify({ desk, sideWidth: 300 }))).toEqual({
      desk,
      sideWidth: 300,
      chatWidth: 320,
      chatOpen: true,
    });
    // The version that kept the conversation as a section called this `agentsOpen`.
    expect(parseScreen(JSON.stringify({ desk, agentsOpen: false }))!.chatOpen).toBe(false);
    expect(parseScreen(JSON.stringify({ desk, sideWidth: 20 }))!.sideWidth).toBe(180);
    expect(parseScreen(JSON.stringify(desk))).toBeNull();
    expect(parseScreen("nope")).toBeNull();
  });

  it("never hands a new pane an id a restored one has", () => {
    // Saved by a session that had counted this far; the counter starts again from zero.
    const saved = {
      desk: {
        panes: [
          { id: "pane-900", tabs: [{ id: "tab-899", editor: "viewport" }], active: "tab-899", x: 0, y: 0, w: 1, h: 1 },
        ],
        order: ["pane-900"],
      },
    };
    const read = parseScreen(JSON.stringify(saved))!;
    const added = addPane(read.desk, "image");
    const ids = added.panes.flatMap((pane) => [pane.id, ...pane.tabs.map((tab) => tab.id)]);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids.slice(2)) expect(Number(id.split("-")[1])).toBeGreaterThan(900);
  });

  it("repairs a saved desk rather than trusting it", () => {
    const saved = {
      desk: {
        panes: [
          { id: "a", tabs: [{ id: "t1", editor: "viewport" }], active: "t1", x: 0, y: 0, w: 0.5, h: 1 },
          // The same pane twice, a pane of no size far off the desk, and one the stacking
          // order forgot.
          { id: "a", tabs: [{ id: "t9", editor: "graph" }], active: "t9", x: 0, y: 0, w: 0.5, h: 1 },
          { id: "b", tabs: [{ id: "t2", editor: "graph" }], active: "t2", x: 40, y: -3, w: 0, h: -1 },
          { id: "c", tabs: [{ id: "t3", editor: "image" }], active: "t3", x: 0.5, y: 0, w: 0.5, h: 1 },
        ],
        order: ["b", "ghost", "a", "a"],
      },
    };
    const { desk } = parseScreen(JSON.stringify(saved))!;
    expect(desk.panes.map((pane) => pane.id)).toEqual(["a", "b", "c"]);
    // Every pane is in the order exactly once, the forgotten one underneath.
    expect(desk.order).toEqual(["c", "b", "a"]);
    const lost = desk.panes[1];
    expect(lost.w).toBeGreaterThanOrEqual(MIN_SIZE);
    expect(lost.h).toBeGreaterThanOrEqual(MIN_SIZE);
    expect(lost.x).toBeLessThanOrEqual(1 - GRIP);
    expect(lost.y).toBeGreaterThanOrEqual(0);
  });

  it("reads the shape saved before tabs existed", () => {
    const old = {
      desk: {
        panes: [{ id: "p1", editor: "graph", x: 0, y: 0, w: 1, h: 1, collapsed: true }],
        order: ["p1"],
      },
      sideWidth: 272,
    };
    const read = parseScreen(JSON.stringify(old))!;
    expect(read.desk.panes[0]).toMatchObject({ id: "p1", collapsed: true, x: 0, w: 1 });
    expect(read.desk.panes[0].tabs.map((tab) => tab.editor)).toEqual(["graph"]);
    expect(read.desk.panes[0].active).toBe(read.desk.panes[0].tabs[0].id);
    // A tab this version has no editor for is dropped, and the rest of the layout survives.
    const gone = {
      desk: {
        panes: [
          { id: "p1", tabs: [{ id: "t1", editor: "agents" }], active: "t1", x: 0, y: 0, w: 0.5, h: 1 },
          { id: "p2", tabs: [{ id: "t2", editor: "viewport" }, { id: "t3", editor: "agents" }], active: "t3", x: 0.5, y: 0, w: 0.5, h: 1 },
        ],
        order: ["p1", "p2"],
      },
    };
    const read2 = parseScreen(JSON.stringify(gone))!;
    expect(read2.desk.panes).toHaveLength(1);
    expect(read2.desk.panes[0].tabs.map((tab) => tab.editor)).toEqual(["viewport"]);

    // And the strip's old name, as the panel it became.
    const strip = { desk: { panes: [{ id: "p", editor: "sequence", x: 0, y: 0, w: 1, h: 1 }], order: ["p"] } };
    expect(parseScreen(JSON.stringify(strip))!.desk.panes[0].tabs[0].editor).toBe("biomol");
  });
});

describe("the desk a new world opens with", () => {
  it("is the viewport and nothing else, filling the desk", () => {
    const desk = defaultDesk();
    expect(desk.panes.map((pane) => pane.tabs.map((tab) => tab.editor))).toEqual([["viewport"]]);
    expect(desk.panes[0]).toMatchObject({ x: 0, y: 0, w: 1, h: 1 });
  });
});

describe("persistence, from a saved layout that cannot be trusted", () => {
  const pane = (id: string, tabs: { id: string; editor: string }[], extra: Record<string, unknown> = {}) => ({
    id,
    tabs,
    active: tabs[0]?.id,
    x: 0,
    y: 0,
    w: 0.5,
    h: 0.5,
    ...extra,
  });

  it("drops a tab whose editor is only an inherited property name", () => {
    const saved = {
      desk: {
        panes: [pane("pane-1", [{ id: "tab-1", editor: "viewport" }, { id: "tab-2", editor: "toString" }, { id: "tab-3", editor: "constructor" }])],
        order: ["pane-1"],
      },
    };
    expect(parseScreen(JSON.stringify(saved))!.desk.panes[0].tabs.map((tab) => tab.editor)).toEqual(["viewport"]);
    // The pre-tabs shape goes through the same door.
    const old = { desk: { panes: [{ id: "pane-1", editor: "__proto__", x: 0, y: 0, w: 1, h: 1 }], order: ["pane-1"] } };
    expect(parseScreen(JSON.stringify(old))!.desk.panes).toEqual([]);
  });

  it("keeps one of each singleton, where it first appears, and every Image", () => {
    const saved = {
      desk: {
        panes: [
          pane("pane-1", [{ id: "tab-1", editor: "viewport" }, { id: "tab-2", editor: "image" }]),
          pane("pane-2", [{ id: "tab-3", editor: "viewport" }]),
          pane("pane-3", [{ id: "tab-4", editor: "graph" }, { id: "tab-5", editor: "graph" }, { id: "tab-6", editor: "image" }]),
        ],
        order: ["pane-1", "pane-2", "pane-3"],
      },
    };
    const { desk } = parseScreen(JSON.stringify(saved))!;
    expect(editors(desk)).toEqual(["viewport+image", "graph+image"]);
    expect(desk.order).toEqual(["pane-1", "pane-3"]);
    expect(tabShowing(desk, "viewport")!.tab.id).toBe("tab-1");
  });

  it("refuses a rectangle that is not a number, and reads nothing from junk", () => {
    const saved = { desk: { panes: [pane("pane-1", [{ id: "tab-1", editor: "viewport" }], { w: "wide" })], order: ["pane-1"] } };
    expect(parseScreen(JSON.stringify(saved))).toBeNull();
    for (const junk of ["", "null", "[]", "42", '{"desk":null}', '{"desk":{"panes":{},"order":[]}}', "{"]) {
      expect(parseScreen(junk)).toBeNull();
    }
  });

  it("brings back a pane saved off the desk or of no size where it can be grabbed", () => {
    const saved = {
      desk: { panes: [pane("pane-1", [{ id: "tab-1", editor: "viewport" }], { x: 5, y: -3, w: 0, h: -1 })], order: [] },
    };
    const [restored] = parseScreen(JSON.stringify(saved))!.desk.panes;
    expect(restored.w).toBeGreaterThanOrEqual(MIN_SIZE);
    expect(restored.h).toBeGreaterThanOrEqual(MIN_SIZE);
    expect(restored.x).toBeLessThanOrEqual(1 - GRIP);
    expect(restored.y).toBe(0);
  });

  it("leaves a desk with no panes once the last is closed, and a new one can still be added", () => {
    const desk = defaultDesk();
    const empty = closeTab(desk, desk.panes[0].id, desk.panes[0].tabs[0].id);
    expect(empty).toEqual({ panes: [], order: [] });
    const again = addPane(empty, "viewport");
    expect(editors(again)).toEqual(["viewport"]);
    expect(again.order).toEqual([again.panes[0].id]);
  });
});
