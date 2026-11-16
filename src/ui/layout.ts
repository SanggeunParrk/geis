// Automatic layout for the node plane.
//
// Kept apart from the editor because it is pure graph arithmetic: ids, links and heights in,
// positions out. That makes the awkward cases - a cycle, an island, a node feeding two
// consumers - unit-testable without a DOM.

export interface LayoutNode {
  id: string;
  /** Where the node is now. Used only to seed the row order, so a tidy graph stays tidy. */
  y: number;
  height: number;
  /**
   * How wide it is, when that is not `options.width`.
   *
   * A group is as wide as the layout inside it, and a column sized for one ordinary node
   * puts whatever comes next on top of the group's frame - which is what "the chains and
   * the viewport are on the same line" was.
   */
  width?: number;
}

export interface LayoutLink {
  from: string;
  to: string;
}

/** A node and the nodes drawn inside its frame. */
export interface LayoutGroup {
  id: string;
  members: string[];
}

export interface LayoutOptions {
  width: number;
  gapX: number;
  gapY: number;
  /** Barycentre sweeps. Two or three settle a hand-built graph; more rarely changes it. */
  passes?: number;
  /**
   * Groups, laid out as units.
   *
   * Without this the members scatter into whichever column their links put them in, the
   * frame drawn around them stretches across the plane and swallows unrelated nodes, and
   * the arrangement is worse than the one it replaced.
   */
  groups?: LayoutGroup[];
  /** Padding between a group's frame and the members inside it. */
  inset?: number;
  /** Height of a group's header, which its members sit below. */
  headerHeight?: number;
  /**
   * How many nodes a column holds before it wraps into a second one beside it.
   *
   * Ten chain nodes of one rank stacked in a single column is a tower taller than the
   * screen; three columns of four is a block. Default 4. Wrapping keeps the rank - the
   * sub-columns all sit at the same depth, side by side - so the left-to-right reading of
   * "producer before consumer" still holds.
   */
  maxPerColumn?: number;
}

/**
 * Lays nodes out in columns, left to right, and returns a position for each.
 *
 * A node's column comes from the longest path from it to a sink, counted backwards, so
 * every producer sits immediately left of what consumes it. Ranking forwards from the
 * sources instead would push a light that feeds only the viewport all the way to column
 * zero, next to the structure sources, where it has no business being.
 *
 * Within a column, nodes are ordered by the mean row of what feeds them - one barycentre
 * sweep per pass, the standard cheap way to keep links from crossing.
 */
export function layoutGraph(
  nodes: LayoutNode[],
  links: LayoutLink[],
  options: LayoutOptions,
): Map<string, [number, number]> {
  if (nodes.length === 0) return new Map();
  const groups = (options.groups ?? []).filter((group) =>
    nodes.some((node) => node.id === group.id),
  );
  if (groups.length > 0) return layoutWithGroups(nodes, links, options, groups);
  return layoutFlat(nodes, links, options);
}

/**
 * Lays out groups as single units, then their members inside them.
 *
 * Each group is condensed to one node as tall as its header plus its members stacked, the
 * condensed graph is laid out by the same algorithm, and the members are then placed
 * *inside* the group - below its header, in its own column. Inside rather than beside,
 * because the group's frame is drawn around them: members in the next column along end up
 * outside the frame that is supposed to contain them.
 */
function layoutWithGroups(
  nodes: LayoutNode[],
  links: LayoutLink[],
  options: LayoutOptions,
  given: LayoutGroup[],
): Map<string, [number, number]> {
  // A group listed as a member - of itself, or of another group - is left where it is.
  // Folded away as a member it would vanish from the condensed graph, its anchor with it,
  // and it and everything inside it would come back with no position at all.
  const groupIds = new Set(given.map((group) => group.id));
  const groups = given.map((group) => ({ id: group.id, members: group.members.filter((member) => !groupIds.has(member)) }));
  const memberOf = new Map<string, string>();
  for (const group of groups) {
    for (const member of group.members) {
      if (nodes.some((node) => node.id === member)) memberOf.set(member, group.id);
    }
  }

  const heightOf = new Map(nodes.map((node) => [node.id, node.height]));
  const inset = options.inset ?? 0;
  const header = options.headerHeight ?? 0;

  const condensed: LayoutNode[] = nodes
    .filter((node) => !memberOf.has(node.id))
    .map((node) => {
      const group = groups.find((candidate) => candidate.id === node.id);
      if (!group) return node;
      const members = group.members.filter((member) => heightOf.has(member));
      if (members.length === 0) return node;
      // Measured from the inner layout, not from the members stacked: once they are
      // ranked among themselves the group is shorter and wider than a single column, and
      // anything laid out after it has to clear the frame it actually occupies.
      const inner = members.map((member) => ({
        id: member,
        y: 0,
        height: heightOf.get(member) ?? 0,
      }));
      const innerLinks = links.filter(
        (link) => members.includes(link.from) && members.includes(link.to),
      );
      const local = layoutFlat(inner, innerLinks, options);
      const bottom = Math.max(
        ...members.map((member) => local.get(member)![1] + (heightOf.get(member) ?? 0)),
      );
      const top = Math.min(...members.map((member) => local.get(member)![1]));
      const right = Math.max(
        ...members.map((member) => local.get(member)![0] + options.width),
      );
      const left = Math.min(...members.map((member) => local.get(member)![0]));
      return {
        ...node,
        height: Math.max(node.height, header + (bottom - top) + inset * 2),
        width: Math.max(node.width ?? options.width, (right - left) + inset * 2),
      };
    });

  // A member's links are re-pointed at its group, so the condensed graph has the same
  // shape - a style feeding a chain of the group is the group's own input.
  const condensedLinks: LayoutLink[] = [];
  for (const link of links) {
    const from = memberOf.get(link.from) ?? link.from;
    const to = memberOf.get(link.to) ?? link.to;
    if (from === to) continue;
    if (condensedLinks.some((seen) => seen.from === from && seen.to === to)) continue;
    condensedLinks.push({ from, to });
  }

  const placed = layoutFlat(condensed, condensedLinks, options);

  const positions = new Map(placed);
  for (const group of groups) {
    const anchor = placed.get(group.id);
    if (!anchor) continue;
    const members = group.members.filter((member) => heightOf.has(member));
    if (members.length === 0) continue;

    // Inside the group, the members are laid out by the same rule as everything else: a
    // style that feeds three chains belongs in front of them, not stacked alongside as if
    // the four were peers. Ranked among themselves, on their own links.
    const inner = members.map((member) => ({
      id: member,
      y: 0,
      height: heightOf.get(member) ?? 0,
    }));
    const innerLinks = links.filter(
      (link) => members.includes(link.from) && members.includes(link.to),
    );
    const local = layoutFlat(inner, innerLinks, options);

    // Translated into the group, below its header, with the top-left member at the inset.
    const left = Math.min(...members.map((member) => local.get(member)![0]));
    const top = Math.min(...members.map((member) => local.get(member)![1]));
    for (const member of members) {
      const at = local.get(member)!;
      positions.set(member, [
        anchor[0] + inset + (at[0] - left),
        anchor[1] + header + inset + (at[1] - top),
      ]);
    }
  }
  return positions;
}

function layoutFlat(
  nodes: LayoutNode[],
  links: LayoutLink[],
  options: LayoutOptions,
): Map<string, [number, number]> {
  const positions = new Map<string, [number, number]>();
  if (nodes.length === 0) return positions;

  const ids = nodes.map((node) => node.id);
  const known = new Set(ids);
  const incoming = new Map<string, string[]>(ids.map((id) => [id, []]));
  const outgoing = new Map<string, string[]>(ids.map((id) => [id, []]));
  for (const link of links) {
    if (!known.has(link.from) || !known.has(link.to)) continue;
    incoming.get(link.to)?.push(link.from);
    outgoing.get(link.from)?.push(link.to);
  }

  // Relaxation rather than a topological sort, because the graph may contain a cycle:
  // evaluation reports those, it does not refuse to draw them. n passes terminate, and a
  // cycle simply ends up spread across as many columns as it has members.
  const rank = new Map<string, number>(ids.map((id) => [id, 0]));
  for (let pass = 0; pass < ids.length; pass += 1) {
    let changed = false;
    for (const id of ids) {
      const want = (outgoing.get(id) ?? []).reduce(
        (best, child) => Math.max(best, (rank.get(child) ?? 0) + 1),
        0,
      );
      if (want > (rank.get(id) ?? 0)) {
        rank.set(id, want);
        changed = true;
      }
    }
    if (!changed) break;
  }

  const deepest = Math.max(...[...rank.values()]);
  const columnOf = (id: string): number => deepest - (rank.get(id) ?? 0);

  const columns = new Map<number, string[]>();
  for (const id of ids) {
    const column = columnOf(id);
    const entries = columns.get(column) ?? [];
    entries.push(id);
    columns.set(column, entries);
  }

  const height = new Map(nodes.map((node) => [node.id, node.height]));
  const widthOf = new Map(nodes.map((node) => [node.id, node.width ?? options.width]));
  const y = new Map(nodes.map((node) => [node.id, node.y]));
  const order = [...columns.keys()].sort((a, b) => a - b);
  const maxPerColumn = Math.max(1, options.maxPerColumn ?? 4);

  // Which sub-column of its rank a node ends up in, and the x offset of each sub-column.
  const subColumnOf = new Map<string, number>();
  const subColumnCount = new Map<number, number>();

  for (let pass = 0; pass < (options.passes ?? 3); pass += 1) {
    for (const column of order) {
      const entries = columns.get(column) ?? [];
      // Producers live in earlier columns, so by the time this column is reached their
      // rows are already final for this pass.
      const barycentre = (id: string): number => {
        const parents = incoming.get(id) ?? [];
        if (parents.length === 0) return y.get(id) ?? 0;
        return parents.reduce((sum, parent) => sum + (y.get(parent) ?? 0), 0) / parents.length;
      };
      entries.sort((a, b) => barycentre(a) - barycentre(b));

      // A rank with more nodes than fit one column is dealt out across several, in the
      // barycentre order: the first few in the leftmost sub-column, the next few beside
      // them. Each sub-column stacks and centres on its own.
      const count = Math.ceil(entries.length / maxPerColumn);
      const perColumn = Math.ceil(entries.length / count);
      subColumnCount.set(column, count);
      for (let sub = 0; sub < count; sub += 1) {
        const slice = entries.slice(sub * perColumn, (sub + 1) * perColumn);
        let cursor = 0;
        for (const id of slice) {
          subColumnOf.set(id, sub);
          y.set(id, cursor);
          cursor += (height.get(id) ?? 0) + options.gapY;
        }
        // Columns are centred on a common line, so the graph reads as one band rather
        // than a staircase hanging off the top edge.
        const total = Math.max(0, cursor - options.gapY);
        for (const id of slice) y.set(id, (y.get(id) ?? 0) - total / 2);
      }
    }
  }

  // Column positions accumulate by the widest node in each sub-column, rather than
  // assuming every column is one node wide. A group holding its own two-column layout is
  // wider than that, and a fixed step would overlap it with the next column along.
  const subWidth = new Map<string, number>(); // `${column}:${sub}` -> width
  for (const id of ids) {
    const key = `${columnOf(id)}:${subColumnOf.get(id) ?? 0}`;
    subWidth.set(key, Math.max(subWidth.get(key) ?? options.width, widthOf.get(id) ?? options.width));
  }
  const subLeft = new Map<string, number>();
  let cursor = 0;
  for (const column of order) {
    const count = subColumnCount.get(column) ?? 1;
    for (let sub = 0; sub < count; sub += 1) {
      const key = `${column}:${sub}`;
      subLeft.set(key, cursor);
      cursor += (subWidth.get(key) ?? options.width) + options.gapX;
    }
  }

  for (const id of ids) {
    const key = `${columnOf(id)}:${subColumnOf.get(id) ?? 0}`;
    positions.set(id, [subLeft.get(key) ?? 0, y.get(id) ?? 0]);
  }
  return positions;
}
