import { describe, expect, it } from "vitest";

import { layoutGraph, type LayoutLink, type LayoutNode } from "../layout";

const OPTIONS = { width: 200, gapX: 100, gapY: 20, maxPerColumn: 99 };
/** One column step, so the tests talk in columns rather than pixels. */
const STEP = OPTIONS.width + OPTIONS.gapX;

function node(id: string, y = 0, height = 100): LayoutNode {
  return { id, y, height };
}

function column(positions: Map<string, [number, number]>, id: string): number {
  const placed = positions.get(id);
  if (!placed) throw new Error(`${id} was not placed`);
  return placed[0] / STEP;
}

/** The pipeline the app opens with, which is the layout that has to come out right. */
function defaultGraph(): { nodes: LayoutNode[]; links: LayoutLink[] } {
  return {
    nodes: [
      node("source"),
      node("colour"),
      node("draw"),
      node("world"),
      node("material"),
      node("light-key"),
      node("light-fill"),
      node("light-rim"),
      node("viewport"),
    ],
    links: [
      { from: "source", to: "draw" },
      { from: "colour", to: "draw" },
      { from: "draw", to: "viewport" },
      { from: "world", to: "viewport" },
      { from: "material", to: "viewport" },
      { from: "light-key", to: "viewport" },
      { from: "light-fill", to: "viewport" },
      { from: "light-rim", to: "viewport" },
    ],
  };
}

describe("layoutGraph", () => {
  it("puts the output in the last column and its producers just left of it", () => {
    const { nodes, links } = defaultGraph();
    const positions = layoutGraph(nodes, links, OPTIONS);

    expect(column(positions, "viewport")).toBe(2);
    expect(column(positions, "draw")).toBe(1);
    // The whole point of ranking backwards from the sink: a light that feeds only the
    // viewport belongs beside it, not exiled to column zero with the structure sources.
    expect(column(positions, "light-key")).toBe(1);
    expect(column(positions, "world")).toBe(1);
    expect(column(positions, "material")).toBe(1);
    expect(column(positions, "source")).toBe(0);
    expect(column(positions, "colour")).toBe(0);
  });

  it("never places a consumer left of a producer", () => {
    const { nodes, links } = defaultGraph();
    const positions = layoutGraph(nodes, links, OPTIONS);
    for (const link of links) {
      expect(column(positions, link.from)).toBeLessThan(column(positions, link.to));
    }
  });

  it("stacks a column without overlaps and centres it on zero", () => {
    const nodes = [node("a", 0, 60), node("b", 0, 140), node("c", 0, 80), node("out")];
    const links = [
      { from: "a", to: "out" },
      { from: "b", to: "out" },
      { from: "c", to: "out" },
    ];
    const positions = layoutGraph(nodes, links, OPTIONS);

    const rows = ["a", "b", "c"]
      .map((id) => ({ id, y: positions.get(id)![1], height: nodes.find((n) => n.id === id)!.height }))
      .sort((left, right) => left.y - right.y);
    for (let index = 1; index < rows.length; index += 1) {
      const previous = rows[index - 1];
      expect(rows[index].y).toBeGreaterThanOrEqual(previous.y + previous.height);
    }

    const top = rows[0].y;
    const bottom = rows[rows.length - 1].y + rows[rows.length - 1].height;
    expect(top + bottom).toBeCloseTo(0, 5);
  });

  it("orders a column by what feeds it, so links do not cross", () => {
    // Two producers, deliberately seeded upside down relative to their consumers.
    const nodes = [
      node("top-in", 500),
      node("bottom-in", 0),
      node("top-out", 0),
      node("bottom-out", 500),
      node("sink"),
    ];
    const links = [
      { from: "top-in", to: "top-out" },
      { from: "bottom-in", to: "bottom-out" },
      { from: "top-out", to: "sink" },
      { from: "bottom-out", to: "sink" },
    ];
    const positions = layoutGraph(nodes, links, OPTIONS);

    // Whatever order the first column settles into, the second must follow it.
    const inputsFirst = positions.get("top-in")![1] < positions.get("bottom-in")![1];
    const outputsFirst = positions.get("top-out")![1] < positions.get("bottom-out")![1];
    expect(outputsFirst).toBe(inputsFirst);
  });

  it("terminates on a cycle instead of ranking forever", () => {
    const nodes = [node("a"), node("b"), node("c"), node("out")];
    const links = [
      { from: "a", to: "b" },
      { from: "b", to: "c" },
      { from: "c", to: "a" },
      { from: "c", to: "out" },
    ];
    const positions = layoutGraph(nodes, links, OPTIONS);

    expect(positions.size).toBe(4);
    for (const [, placed] of positions) {
      expect(Number.isFinite(placed[0])).toBe(true);
      expect(Number.isFinite(placed[1])).toBe(true);
    }
  });

  it("places an island that connects to nothing", () => {
    const nodes = [node("a"), node("out"), node("lonely")];
    const positions = layoutGraph(nodes, [{ from: "a", to: "out" }], OPTIONS);

    // No outgoing links means rank zero, which is the rightmost column - the same place
    // the output sits, since neither feeds anything.
    expect(column(positions, "lonely")).toBe(column(positions, "out"));
    expect(positions.get("lonely")![1]).not.toBe(positions.get("out")![1]);
  });

  it("ignores links that name a node that is not there", () => {
    const positions = layoutGraph(
      [node("a"), node("out")],
      [
        { from: "a", to: "out" },
        { from: "ghost", to: "out" },
        { from: "a", to: "ghost" },
      ],
      OPTIONS,
    );
    expect(positions.size).toBe(2);
    expect(column(positions, "a")).toBe(0);
    expect(column(positions, "out")).toBe(1);
  });

  it("returns nothing for an empty graph", () => {
    expect(layoutGraph([], [], OPTIONS).size).toBe(0);
  });
});

describe("layoutGraph with groups", () => {
  /** A molecule group with two styles wired into it, plus the output. */
  function grouped() {
    return {
      nodes: [
        node("mol", 0, 200),
        node("style-all", 0, 120),
        node("style-a", 0, 120),
        node("viewport", 0, 100),
        node("light", 0, 140),
      ],
      links: [
        { from: "style-all", to: "mol" },
        { from: "style-a", to: "mol" },
        { from: "mol", to: "viewport" },
        { from: "light", to: "viewport" },
      ],
      groups: [{ id: "mol", members: ["style-all", "style-a"] }],
    };
  }

  it("ranks a group's members among themselves", () => {
    // A style feeding the chains belongs in front of them, not stacked alongside as if
    // the four were peers.
    const nodes = [
      node("mol", 0, 40),
      node("shared-style", 0, 120),
      node("chain-a", 0, 80),
      node("chain-b", 0, 80),
      node("viewport", 0, 100),
    ];
    const links = [
      { from: "shared-style", to: "chain-a" },
      { from: "shared-style", to: "chain-b" },
      { from: "chain-a", to: "viewport" },
      { from: "chain-b", to: "viewport" },
    ];
    const groups = [
      { id: "mol", members: ["shared-style", "chain-a", "chain-b"] },
    ];
    const positions = layoutGraph(nodes, links, {
      ...OPTIONS,
      groups,
      inset: 14,
      headerHeight: 34,
    });

    const style = positions.get("shared-style")!;
    const a = positions.get("chain-a")!;
    const b = positions.get("chain-b")!;
    expect(style[0]).toBeLessThan(a[0]);
    expect(style[0]).toBeLessThan(b[0]);
    // The two chains are peers, so they share a column and stack.
    expect(a[0]).toBe(b[0]);
    expect(Math.abs(a[1] - b[1])).toBeGreaterThanOrEqual(80);
  });

  it("puts a group's members inside it, below the header", () => {
    const { nodes, links, groups } = grouped();
    const inset = 14;
    const headerHeight = 34;
    const positions = layoutGraph(nodes, links, {
      ...OPTIONS,
      groups,
      inset,
      headerHeight,
    });

    const mol = positions.get("mol")!;
    for (const member of ["style-all", "style-a"]) {
      const at = positions.get(member)!;
      // Inside, not beside: the group's frame is drawn around them, and a member in the
      // next column along ends up outside the frame that is meant to contain it.
      expect(at[0]).toBe(mol[0] + inset);
      expect(at[1]).toBeGreaterThanOrEqual(mol[1] + headerHeight);
    }
    // Stacked, not on top of each other.
    const first = positions.get("style-all")!;
    const second = positions.get("style-a")!;
    expect(Math.abs(first[1] - second[1])).toBeGreaterThanOrEqual(120);
  });

  it("makes the group tall enough for what is inside it", () => {
    // Otherwise whatever is laid out after it is placed on top of its frame.
    const { nodes, links, groups } = grouped();
    const positions = layoutGraph(nodes, links, {
      ...OPTIONS,
      groups,
      inset: 14,
      headerHeight: 34,
    });
    const mol = positions.get("mol")![1];
    const lowest = Math.max(
      ...["style-all", "style-a"].map((id) => positions.get(id)![1] + 120),
    );
    // The light shares the molecule's column, so it must clear the whole frame.
    const light = positions.get("light")![1];
    expect(light >= lowest || light + 140 <= mol).toBe(true);
  });

  it("does not let a member's own links pull it into another column", () => {
    // Without condensing, `style-all -> mol -> viewport` puts the styles one column left
    // of the molecule *and* the molecule one left of the viewport, which is right by
    // luck; add a light and the barycentre pass moves them apart. This checks the
    // condensed graph instead: the group occupies one column, its members another.
    const { nodes, links, groups } = grouped();
    const positions = layoutGraph(nodes, links, { ...OPTIONS, groups });
    // The members share the group's column now, so there are two columns of nodes plus
    // the viewport's.
    const columns = new Set(
      ["mol", "viewport", "light"].map((id) => positions.get(id)![0] / STEP),
    );
    expect(columns.size).toBe(2);
    expect(positions.get("viewport")![0]).toBeGreaterThan(positions.get("mol")![0]);
    expect(positions.get("light")![0]).toBe(positions.get("mol")![0]);
  });

  it("ignores a group whose node is not in the graph", () => {
    const positions = layoutGraph(
      [node("a"), node("out")],
      [{ from: "a", to: "out" }],
      { ...OPTIONS, groups: [{ id: "ghost", members: ["a"] }] },
    );
    expect(positions.size).toBe(2);
    expect(positions.get("a")![0]).toBeLessThan(positions.get("out")![0]);
  });
});

describe("column widths", () => {
  it("gives a wide node room instead of overlapping it", () => {
    // The bug this exists for: a group holding its own two-column layout is wider than an
    // ordinary node, and a fixed column step put the next column on top of its frame.
    const nodes = [
      { id: "wide", y: 0, height: 100, width: 500 },
      node("next", 0, 100),
    ];
    const positions = layoutGraph(nodes, [{ from: "wide", to: "next" }], OPTIONS);
    const wide = positions.get("wide")!;
    const next = positions.get("next")!;
    expect(next[0]).toBeGreaterThanOrEqual(wide[0] + 500 + OPTIONS.gapX);
  });

  it("keeps a group clear of the column after it", () => {
    const nodes = [
      node("mol", 0, 40),
      node("style", 0, 120),
      node("chain-a", 0, 80),
      node("chain-b", 0, 80),
      node("viewport", 0, 100),
    ];
    const links = [
      { from: "style", to: "chain-a" },
      { from: "style", to: "chain-b" },
      { from: "chain-a", to: "viewport" },
      { from: "chain-b", to: "viewport" },
    ];
    const groups = [{ id: "mol", members: ["style", "chain-a", "chain-b"] }];
    const positions = layoutGraph(nodes, links, {
      ...OPTIONS,
      groups,
      inset: 14,
      headerHeight: 34,
    });

    // Everything inside the group, and the viewport clear of all of it.
    const inside = ["style", "chain-a", "chain-b"].map((id) => positions.get(id)!);
    const frameRight = Math.max(...inside.map((at) => at[0] + OPTIONS.width)) + 14;
    const viewport = positions.get("viewport")!;
    expect(viewport[0]).toBeGreaterThanOrEqual(frameRight);
    // And still to the right of the chains that feed it.
    for (const chain of ["chain-a", "chain-b"]) {
      expect(viewport[0]).toBeGreaterThan(positions.get(chain)![0]);
    }
  });
});

describe("wrapping a tall rank", () => {
  const node = (id: string) => ({ id, y: 0, height: 100 });
  const options = { width: 200, gapX: 50, gapY: 20 };

  it("deals a rank with too many nodes across several columns at the same depth", () => {
    // Ten chains feeding one viewport: a single column of ten is a tower taller than the
    // screen. With four to a column they become 4 + 3 + 3, side by side.
    const chains = Array.from({ length: 10 }, (_, index) => `chain-${index}`);
    const positions = layoutGraph(
      [...chains.map(node), node("viewport")],
      chains.map((id) => ({ from: id, to: "viewport" })),
      options,
    );
    const xs = new Set(chains.map((id) => positions.get(id)![0]));
    expect(xs.size).toBe(3);
    const tallest = Math.max(
      ...[...xs].map(
        (x) => chains.filter((id) => positions.get(id)![0] === x).length,
      ),
    );
    expect(tallest).toBe(4);
    // Every chain still sits left of the viewport it feeds.
    const viewportX = positions.get("viewport")![0];
    for (const id of chains) expect(positions.get(id)![0]).toBeLessThan(viewportX);
    // And the sub-columns do not overlap: consecutive ones are a node width plus the gap
    // apart.
    const sorted = [...xs].sort((a, b) => a - b);
    expect(sorted[1] - sorted[0]).toBe(250);
    expect(sorted[2] - sorted[1]).toBe(250);
  });

  it("leaves a short rank in one column", () => {
    const positions = layoutGraph(
      [node("a"), node("b"), node("c"), node("sink")],
      ["a", "b", "c"].map((id) => ({ from: id, to: "sink" })),
      options,
    );
    const xs = new Set(["a", "b", "c"].map((id) => positions.get(id)![0]));
    expect(xs.size).toBe(1);
  });

  it("respects the per-column limit an editor asks for", () => {
    const chains = Array.from({ length: 6 }, (_, index) => `chain-${index}`);
    const positions = layoutGraph(
      [...chains.map(node), node("sink")],
      chains.map((id) => ({ from: id, to: "sink" })),
      { ...options, maxPerColumn: 2 },
    );
    expect(new Set(chains.map((id) => positions.get(id)![0])).size).toBe(3);
  });
});

describe("wrapping inside a group", () => {
  it("deals a group's link-less members across sub-columns too", () => {
    // Ten chains inside one molecule frame, feeding a viewport outside it: inside the
    // group they have no links to rank by, so they are one rank - and one rank of ten is
    // the tower this exists to prevent.
    const chains = Array.from({ length: 10 }, (_, index) => `chain-${index}`);
    const nodes = [
      { id: "molecule", y: 0, height: 60 },
      ...chains.map((id) => ({ id, y: 0, height: 78 })),
      { id: "viewport", y: 0, height: 120 },
    ];
    const links = chains.map((id) => ({ from: id, to: "viewport" }));
    const positions = layoutGraph(nodes, links, {
      width: 208,
      gapX: 60,
      gapY: 16,
      groups: [{ id: "molecule", members: chains }],
      inset: 12,
      headerHeight: 28,
    });
    const xs = new Set(chains.map((id) => Math.round(positions.get(id)![0])));
    expect(xs.size).toBe(3);
    // And the group's frame still sits left of the viewport its members feed.
    expect(positions.get("molecule")![0]).toBeLessThan(positions.get("viewport")![0]);
  });
});

describe("layoutGraph with groups that are not well formed", () => {
  it("places every node when a group lists itself or another group as a member", () => {
    const nodes = [node("mol"), node("chain-a"), node("other"), node("chain-b"), node("viewport")];
    const links = [
      { from: "mol", to: "viewport" },
      { from: "other", to: "viewport" },
    ];
    const groups = [
      { id: "mol", members: ["mol", "chain-a", "other"] },
      { id: "other", members: ["chain-b"] },
    ];
    const positions = layoutGraph(nodes, links, { ...OPTIONS, groups });
    for (const each of nodes) expect(positions.get(each.id), each.id).toBeDefined();
    // And what was handed in is not rewritten.
    expect(groups[0].members).toEqual(["mol", "chain-a", "other"]);
  });

  it("places a group whose members are all missing as an ordinary node", () => {
    const positions = layoutGraph([node("mol"), node("viewport")], [{ from: "mol", to: "viewport" }], {
      ...OPTIONS,
      groups: [{ id: "mol", members: ["gone"] }],
    });
    expect(column(positions, "mol")).toBe(0);
    expect(column(positions, "viewport")).toBe(1);
  });
});
