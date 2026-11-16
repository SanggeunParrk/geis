// The node editor.
//
// A pannable, zoomable plane of nodes joined by typed links - the same idea as Blender's
// shader editor, and for the same reason: a pipeline is a graph, and a list can only show
// one order through it.
//
// Nodes are DOM elements rather than canvas drawings so their parameters can be real
// inputs, selects and colour pickers. Links are one SVG layer underneath, inside the same
// transform, so panning and zooming move both together.
//
// Socket names, types and colours all come from the core. The editor never decides what
// may connect to what; it asks, and refuses the drop when the answer is no.
//
// Drawing reconciles rather than rebuilds. That is not an optimisation: a parameter change
// re-evaluates the graph, and re-evaluating used to replace the whole node layer - which
// destroyed the very control the pointer was holding, so no drag could ever finish.

import {
  api,
  COLOR_SCHEME_LABELS,
  MOLECULE_WIDE_SCHEMES,
  RAMP_LABELS,
  RAMP_SCHEMES,
  RENDER_ENGINE_LABELS,
  REPRESENTATION_LABELS,
  type Graph,
  type GraphNode,
  type ModelCard,
  type NodeDescription,
  type NodeState,
  type NodeTemplate,
  type Representation,
  type ColorScheme,
  type ProviderRecord,
  type Ramp,
  type Style,
  type StyleLibrary,
  type StyleSlot,
} from "../ipc";
import { colorField, hexToSrgb, hexToUnit, srgbToHex, unitToHex } from "./colorPicker";
import { layoutGraph, type LayoutGroup } from "./layout";
import { menu, type MenuEntry } from "./menu";
import {
  preferredTab,
  propertyTabs,
  resolveTab,
  type TabContext,
  type TabKey,
} from "./propertyTabs";
import { askText } from "./ask";
import { icon, type StructureIcon } from "./icons";
import { count, el, replace } from "./dom";
import { fieldHint, nodeHint, socketHint, tabHint } from "./hints";

/** What a Render node is doing, drawn on the node itself. */
export interface RenderProgress {
  rendering: boolean;
  image?: string | null;
  seconds?: number;
  error?: string | null;
}

/** What a row of the scene list stands for, for its right-click menu. */
export type SceneRow =
  | { kind: "molecule"; moleculeId: string }
  /** What the molecule is read as: its assembly, model and alternate locations. */
  | { kind: "version"; moleculeId: string }
  | { kind: "chain"; moleculeId: string; chain: string; nodeId: string }
  | { kind: "object"; moleculeId: string; name: string }
  | { kind: "group"; moleculeId: string; id: string; edges: boolean };

/** One group of a molecule's preset analysis, as the scene list shows it. */
export interface PresetRow {
  id: string;
  label: string;
  /** Edges between residues - shown and hidden - rather than residues to select. */
  edges: boolean;
  state: "loading" | "ready" | "failed";
  count: number;
  shown: boolean;
  /** The colour its edges are drawn in. */
  swatch?: string;
  error?: string;
}

export interface NodeEditorHandle {
  element: HTMLElement;
  /** Replaces the graph and redraws. */
  setGraph: (graph: Graph) => void;
  /** Draws evaluation results onto the nodes. */
  setEvaluation: (nodes: NodeDescription[], states: Record<string, NodeState>) => void;
  /** Draws render state onto the Render nodes. */
  setRenderContext: (context: RenderContext) => void;
  /** Replaces the style library the panel manages. */
  setStyleLibrary: (library: StyleLibrary) => void;
  /**
   * Redraws. Needed after the pane is unhidden: link positions are measured from the
   * layout, and a hidden pane has none.
   */
  refresh: () => void;
  /**
   * The properties panel for whatever is selected.
   *
   * The editor owns it rather than the screen because it is the same controls the node
   * body shows, built by the same code - two implementations of "the settings of a Style"
   * would drift, and one of them would be the one you were not looking at.
   */
  properties: HTMLElement;
  /** A one-line description of what is selected, for the viewport's corner. */
  selection: HTMLElement;
  graph: () => Graph;
  /**
   * Selects a node from outside - a click in the viewport.
   *
   * The same call the outliner makes, so a chain picked in the viewport lands in the
   * properties panel exactly as one picked from the tree does.
   */
  select: (nodeId: string | null) => void;
  /** Frames every node in the pane - what a double-click on the plane does. */
  fit: () => void;
  /**
   * Extra lines for the viewport's corner card - the entry id, the author chain, what a
   * click landed on. The editor knows the graph; the screen knows the structures, and it
   * is the screen that fills these in.
   */
  setSelectionDetail: (lines: string[] | null) => void;
  /** What the pointer is over, shown under the selection - or alone when nothing is. */
  setHoverDetail: (line: string | null) => void;
}

/** Everything the Render nodes need that is not in the graph. */
export interface RenderContext {
  /** Why nothing can be rendered right now, or null. Usually a missing Blender. */
  unavailable: string | null;
  progress: Record<string, RenderProgress>;
}

export interface NodeEditorOptions {
  onChange: (graph: Graph) => void;
  /** Asked to render one Render node. The editor never renders anything itself. */
  onRender?: (nodeId: string) => void;
  /** Asked to throw the saved graph away and start from the default pipeline. */
  onReset?: () => void;
  /** Asked to fetch a PDB entry and build its group. */
  onFetch?: (id: string, at: { x: number; y: number }) => void;
  /** Asked to open a structure file and build its group. */
  onOpenFile?: (at: { x: number; y: number }) => void;
  /** Asked to put a style in the library under a name. */
  onStyleSave?: (name: string, style: unknown) => void;
  /** The world's cameras, as rows for the viewport's panel. They are the screen's to keep. */
  cameraRows?: () => HTMLElement[];
  /** A named object under a molecule was clicked: select what it names. */
  onObject?: (moleculeId: string, name: string) => void;
  onObjectRemove?: (moleculeId: string, name: string) => void;
  /**
   * What a molecule was analysed for when it was opened - its contacts, its core, its
   * binding site - as groups to list under it; null while there is nothing to list.
   */
  presetRows?: (moleculeId: string) => PresetRow[] | null;
  onPreset?: (moleculeId: string, id: string) => void;
  /** A row of the scene list was right-clicked: what it is, and where. */
  onSceneMenu?: (row: SceneRow, at: { x: number; y: number }) => void;
  /**
   * What a molecule is read as, in a few words - which assembly, which model - for the row
   * under it that changes it; null when the file offers no choice.
   */
  versionRow?: (moleculeId: string) => { label: string; value: string; title: string } | null;
  /** Asked to remove a style from the library. */
  onStyleDelete?: (name: string) => void;
  /** The selected node changed. The viewport uses it to know what it is manipulating. */
  onSelect?: (nodeId: string | null) => void;
  /** Opens the file a Render node produced. */
  onReveal?: (path: string) => void;
  /** Puts an image on the clipboard. */
  onImageCopy?: (path: string) => void;
  /** Saves an image somewhere the user chooses. */
  onImageSave?: (path: string) => void;
  /** Turns a path into something an <img> can load. */
  imageSource?: (path: string) => string;
  /** The viewport's width/height, for a Render node that wants to match it. */
  viewportAspect?: () => number | null;
  sampleViewport?: (client: { x: number; y: number }) => Promise<string | null>;
  setPickingCursor?: (picking: boolean) => void;
}

const NODE_WIDTH = 208;
/**
 * Where the socket rows go, in the node's own coordinates.
 *
 * The rows are laid out from these, and the *links* are drawn from the dots' measured
 * positions rather than from these numbers - which is the whole reason this comment
 * exists. The two used to be the same constants, and every link was drawn 32px above its
 * dot, because `.node-sockets` starts below the header and the link maths measured from
 * the node's top. Constants that have to agree with a stylesheet do not stay agreeing.
 */
const SOCKET_PITCH = 22;
const SOCKET_TOP = 34;
/** Fallback node height for layout, used only before a node has been measured. */
const NODE_HEIGHT = 130;

/**
 * The light types, by what they do rather than by their Blender names.
 *
 * A sun has no falloff, which is why it is the default: the same energy number means the
 * same brightness whether the subject is a 10 A ligand or a 300 A capsid.
 */
const LIGHT_KINDS: [string, string][] = [
  ["sun", "Sun"],
  ["point", "Point"],
  ["area", "Area"],
];

/** Socket names come from the core, but a selector still has to be safe to build. */
function cssEscape(value: string): string {
  const escape = (globalThis as { CSS?: { escape?: (value: string) => string } }).CSS?.escape;
  return escape ? escape(value) : value.replace(/["\\]/g, "\\$&");
}

/** The live DOM for one node, kept so redrawing can update it instead of replacing it. */
interface NodeEntry {
  root: HTMLElement;
  header: HTMLElement;
  title: HTMLElement;
  fold: HTMLElement;
  spinner: HTMLElement;
  /** Where a folded node's two bundle dots go, since its rows are hidden. */
  headerSockets: HTMLElement;
  sockets: HTMLElement;
  body: HTMLElement;
  error: HTMLElement;
  /** The node object the body's widgets close over; a new object needs new widgets. */
  node: GraphNode;
  bodyKey: string;
  socketKey: string;
}

export function nodeEditor(options: NodeEditorOptions): NodeEditorHandle {
  let graph: Graph = { nodes: [], links: [] };
  let descriptions = new Map<string, NodeDescription>();
  let states: Record<string, NodeState> = {};
  let catalog: NodeTemplate[] = [];
  let models: ModelCard[] = [];
  let providers: ProviderRecord[] = [];
  let renderContext: RenderContext = { unavailable: null, progress: {} };
  let library: StyleLibrary = { styles: [], colours: [] };

  let pan = { x: 24, y: 16 };
  let zoom = 1;
  let selected: string | null = null;
  /** The last selection handed out, so the same one is not reported twice. */
  let reported: string | null = null;
  /** A link being dragged out of an output socket. */
  let dragging: { from: string; x: number; y: number } | null = null;
  /**
   * How many widgets are mid-drag. While any is, node bodies are left alone: rebuilding
   * one would remove the element the pointer is captured on and strand the drag.
   */
  let interacting = 0;
  /** Where the pointer last was over the plane, so keyboard-opened menus have a place. */
  let pointerAt = { x: 0, y: 0 };
  /**
   * Which properties tab is open, and what the panel is looking at.
   *
   * Blender keeps the tab across a change of selection and keeps the *active object* even
   * when something else is selected, which is why the molecule and chain are remembered
   * here rather than derived from `selected` alone: clicking a light must not empty the
   * Style tab.
   */
  let tab: TabKey | null = null;
  let activeMolecule: string | null = null;
  let activeChain: string | null = null;
  /** A pinned node keeps the panel on it, as Blender's pin does. */
  let pinned: string | null = null;
  /** Which library style the Style tab is editing, by name. */
  let styleDraft: string | null = null;
  /**
   * Unsaved edits to library styles, by name.
   *
   * Kept here rather than written straight through, so a drag reads back the value it is
   * being dragged to while the library itself only changes when the widget lets go.
   */
  const styleEdits = new Map<string, Style>();
  /** Which panels are unfolded. Blender's `DEFAULT_CLOSED` is the ones missing here. */
  const openPanels = new Set<string>([
    // A chain's own style: how it is drawn and coloured are what the panel is opened for.
    "chain-style-id",
    "chain-draw",
    "chain-colour",
    // A tab whose only panel is folded looks like a tab with nothing in it, so the
    // single-panel tabs start open.
    "render-settings",
    "world-data",
    "image-data",
    "light-data",
    "styles",
    // Draw, Colour and Surface start folded, as Blender's optional panels do with
    // `DEFAULT_CLOSED`: the tab's job is to say which style you are on, and three open
    // panels of fields push the assign buttons off the bottom of a 272px column.
    "slots",
    "source",
    "contents",
    "chain-data",
    "chain-style",
    "molecule-chains",
    "group-colour",
    "settings",
  ]);

  const nodeElements = new Map<string, NodeEntry>();

  const links = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  links.classList.add("node-links");
  const nodeLayer = el("div", { class: "node-layer" });
  /** Group frames, behind the links and the nodes and inside the same transform. */
  const frameLayer = el("div", { class: "frame-layer" });
  const world = el("div", { class: "node-world" }, frameLayer, links, nodeLayer);
  const surface = el("div", { class: "node-surface" }, world);
  const status = el("div", { class: "node-status" });

  const element = el(
    "div",
    { class: "node-editor" },
    el(
      "div",
      { class: "node-toolbar" },
      el("button", {
        class: "btn",
        text: "+ Add node",
        onclick: (event: MouseEvent) => {
          event.stopPropagation();
          const button = event.currentTarget as HTMLElement;
          const rect = button.getBoundingClientRect();
          openSurfaceMenu({ x: rect.left, y: rect.bottom + 4 }, centerOfView());
        },
      }),
      el("button", { class: "btn", text: "Arrange", onclick: () => arrange() }),
      el("span", { class: "spacer" }),
      el("span", { class: "node-hint", text: "right-click for menu, double-click to fit, \u2318Z to undo" }),
      status,
    ),
    el("div", { class: "node-canvas" }, surface),
  );

  /**
   * The right-hand column, in two halves like Blender's: an outliner listing what the
   * scene is made of, and a properties half below it where the selected thing is edited -
   * with the style library managed there, the way materials are.
   */
  const outliner = el("div", { class: "outliner" });
  /**
   * The breadcrumb, and the properties body with its tab strip.
   *
   * Blender's properties editor is four fixed rows and then a stack of panels: a header, a
   * navigation bar of icon tabs down the *left edge* of the editor, the breadcrumb naming
   * what is being edited with a pin to freeze it, and the datablock block. This is that,
   * with Geis's tabs - see `propertyTabs`.
   */
  const propsCrumbs = el("div", { class: "props-crumbs" });
  const propsTabs = el("div", { class: "props-tabs" });
  const propsBody = el("div", { class: "props-body" });
  /**
   * The divider between the outliner and the properties.
   *
   * The same gesture as the one between the viewport and the graph: an editor boundary you
   * can drag is how every editor of this shape behaves, and a fixed 0.9fr / 1.4fr split
   * meant a long outliner and a long panel could not both be read.
   */
  const panelSplitter = el("div", { class: "splitter vertical", title: "Drag to resize" });
  const properties = el(
    "div",
    { class: "properties" },
    el("div", { class: "panel-head" }, el("span", { text: "Scene" })),
    outliner,
    panelSplitter,
    el("div", { class: "props-header" }, propsCrumbs),
    el("div", { class: "props-main" }, propsTabs, propsBody),
  );

  /**
   * Whether the divider has been dragged. Until it has, the outliner is as tall as what
   * is in it - up to under half the column - rather than a fixed 190px, which cut a scene
   * of one molecule and two lights off at its fourth row while the panel underneath stood
   * half empty. Once someone has set the height it is theirs.
   */
  let outlinerSized = false;
  const fitOutliner = () => {
    if (outlinerSized) return;
    const available = properties.getBoundingClientRect().height;
    // Not laid out yet: the stylesheet's default stands until it is.
    if (available === 0) return;
    // The height of what is in it, not `scrollHeight`: that is never less than the box
    // itself, so once the box was taller than its rows each fit read back its own height,
    // added the two pixels of border, and grew - the divider crept down a step on every
    // redraw, and zooming the graph is a redraw per wheel notch.
    const last = outliner.lastElementChild as HTMLElement | null;
    const content = last
      ? last.getBoundingClientRect().bottom - outliner.getBoundingClientRect().top + outliner.scrollTop
      : 0;
    // Plus the box's own bottom padding and border, which the rows do not reach.
    const wanted = content > 0 ? Math.ceil(content) + 8 : outliner.scrollHeight + 2;
    const height = Math.min(Math.max(wanted, 80), Math.max(available * 0.45, 120));
    properties.style.setProperty("--outliner-height", `${Math.round(height)}px`);
  };

  panelSplitter.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    outlinerSized = true;
    const startY = event.clientY;
    const startHeight = outliner.getBoundingClientRect().height;
    const available = properties.getBoundingClientRect().height;
    const move = (moveEvent: PointerEvent) => {
      // Both halves keep a floor: a pane dragged to nothing is a pane you cannot get back.
      const height = Math.min(
        Math.max(startHeight + (moveEvent.clientY - startY), 80),
        Math.max(available - 160, 120),
      );
      properties.style.setProperty("--outliner-height", `${height}px`);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });
  /**
   * What is selected, for the viewport's top-left corner.
   *
   * Empty until something is clicked. The corner used to hold a legend and a triangle
   * count nobody asked for; a viewport should show the molecule and get out of the way.
   */
  const selection = el("div", { class: "selection-card", hidden: "true" });
  /** Lines the screen adds under the card's title. */
  let selectionDetail: string[] | null = null;
  /** What the pointer is over right now, if anything. */
  let hoverDetail: string | null = null;
  /** Which outliner branches are open. Kept here so redrawing does not collapse them. */
  const openBranches = new Set<string>(["scene", "lighting", "output", "other"]);

  // Hosted on the editor rather than the plane, so panning cannot drag a menu along.
  const menus = menu(element);

  // The catalog and the compute records arrive after the first draw, and the nodes that
  // list models or providers cannot be filled in without them.
  void api.graphCatalog().then((entries) => {
    catalog = entries;
  });
  void api.listModels().then((entries) => {
    models = entries;
    draw();
  });
  void api.listProviders().then((entries) => {
    providers = entries;
    draw();
  });
  void api.styleLibrary().then((entries) => {
    library = entries;
    draw();
  });

  function commit(): void {
    options.onChange(structuredClone(graph));
    draw();
  }

  function nodeById(id: string): GraphNode | undefined {
    return graph.nodes.find((node) => node.id === id);
  }

  // --- geometry of sockets ---------------------------------------------------

  /**
   * The centre of a dot, in the node's coordinates, read from the layout.
   *
   * Summed up the `offsetParent` chain rather than from `getBoundingClientRect`, because
   * the plane is inside a CSS scale transform: `offsetTop` is in layout pixels, which is
   * the same space the links are drawn in, and needs no dividing by the zoom.
   *
   * Returns null when the node is not laid out - the graph pane folded away, say - and the
   * callers fall back to the nominal rows.
   */
  function dotOffset(dot: HTMLElement | null, root: HTMLElement): number | null {
    if (!dot || dot.offsetHeight === 0) return null;
    let y = 0;
    let walk: HTMLElement | null = dot;
    while (walk && walk !== root) {
      y += walk.offsetTop;
      walk = walk.offsetParent as HTMLElement | null;
    }
    return walk === root ? y + dot.offsetHeight / 2 : null;
  }

  /** The visible node a link endpoint resolves to: itself, or the group hiding it. */
  function visibleAnchor(id: string): string {
    const node = nodeById(id);
    if (!node) return id;
    for (const molecule of graph.nodes) {
      if (molecule.type !== "molecule" || molecule.expanded) continue;
      if (membersOf(molecule).some((member) => member.id === id)) return molecule.id;
    }
    return id;
  }

  function inputPosition(id: string, socket: string): { x: number; y: number } | null {
    const node = nodeById(id);
    const description = descriptions.get(id);
    if (!node || !description) return null;
    if (isGroupOwner(node)) return bundlePosition(id, "in");
    const index = description.inputs.findIndex((input) => input.name === socket);
    if (index < 0) return null;

    const entry = nodeElements.get(id);
    const measured = entry
      ? dotOffset(
          // Folded, there is one bundle dot and every link converges on it.
          entry.root.querySelector<HTMLElement>(
            node.collapsed
              ? ".socket.bundle.in"
              : `.socket.in[data-socket="${cssEscape(socket)}"]`,
          ),
          entry.root,
        )
      : null;
    return {
      x: node.position[0],
      y:
        node.position[1] +
        (measured ?? SOCKET_TOP + (node.collapsed ? 0 : index * SOCKET_PITCH)),
    };
  }

  function outputPosition(id: string): { x: number; y: number } | null {
    const node = nodeById(id);
    const description = descriptions.get(id);
    if (!node || !description?.output) return null;
    if (isGroupOwner(node)) return bundlePosition(id, "out");
    const entry = nodeElements.get(id);
    const measured = entry
      ? dotOffset(entry.root.querySelector<HTMLElement>(".socket.out"), entry.root)
      : null;
    return {
      x: node.position[0] + NODE_WIDTH,
      y: node.position[1] + (measured ?? SOCKET_TOP),
    };
  }

  /** Where a folded group's links converge: its own edge, level with its header. */
  function bundlePosition(id: string, side: "in" | "out"): { x: number; y: number } | null {
    const node = nodeById(id);
    if (!node) return null;
    // A molecule has no node box: its links meet its frame header.
    const anchor = isGroupOwner(node) ? 15 : SOCKET_TOP;
    return {
      x: node.position[0] + (side === "out" ? NODE_WIDTH : 0),
      y: node.position[1] + anchor,
    };
  }

  /** Measured height, so layout and framing use what is on screen, not a guess. */
  function nodeHeight(id: string): number {
    return nodeElements.get(id)?.root.offsetHeight || NODE_HEIGHT;
  }

  // --- drawing ---------------------------------------------------------------

  function draw(): void {
    world.style.transform = `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`;
    drawNodes();
    drawFrames();
    drawLinks();
    drawStatus();
    drawProperties();
  }

  function drawStatus(): void {
    const problems = Object.entries(states).filter(
      ([, state]) =>
        state.state === "failed" || state.state === "incomplete" || state.state === "warned",
    );
    // A graph with no molecule in it is not broken, it is new: what it needs is said
    // plainly rather than as the red complaint of whichever node is waiting for one.
    if (!graph.nodes.some((node) => node.type === "molecule")) {
      replace(status, el("span", { class: "dim", text: "no molecule yet - open a file or fetch one" }));
      return;
    }
    if (problems.length === 0) {
      replace(status, el("span", { class: "ok", text: "graph complete" }));
      return;
    }
    const [id, state] = problems[0];
    const title = descriptions.get(id)?.title ?? id;
    const message =
      state.state === "failed"
        ? state.message
        : state.state === "warned"
          ? state.message
          : state.state === "incomplete"
            ? state.reason
            : "";
    replace(
      status,
      el("span", {
        class: state.state === "warned" ? "warn" : "bad",
        text: `${title}: ${message}`,
      }),
    );
  }

  function drawLinks(): void {
    while (links.firstChild) links.removeChild(links.firstChild);

    const path = (from: { x: number; y: number }, to: { x: number; y: number }, color: string) => {
      const line = document.createElementNS("http://www.w3.org/2000/svg", "path");
      // A horizontal-tangent bezier, so a link leaves an output going right and enters an
      // input coming from the left however the nodes are arranged.
      const reach = Math.max(40, Math.abs(to.x - from.x) * 0.5);
      line.setAttribute(
        "d",
        `M ${from.x} ${from.y} C ${from.x + reach} ${from.y}, ${to.x - reach} ${to.y}, ${to.x} ${to.y}`,
      );
      line.setAttribute("stroke", color);
      line.setAttribute("fill", "none");
      line.setAttribute("stroke-width", "2");
      links.appendChild(line);
    };

    for (const link of graph.links) {
      // A link into or out of a folded group's member is drawn to the group itself, so
      // the graph still reads as connected rather than losing edges.
      const fromId = visibleAnchor(link.from);
      const toId = visibleAnchor(link.to);
      if (fromId === toId) continue;
      // A molecule feeding one of its own chains is internal to the frame: the frame
      // already says they belong together, and the line only adds clutter.
      if (link.socket === "Molecule" && nodeById(link.from)?.type === "molecule") continue;
      const from =
        fromId === link.from ? outputPosition(link.from) : bundlePosition(fromId, "out");
      const to =
        toId === link.to ? inputPosition(link.to, link.socket) : bundlePosition(toId, "in");
      if (!from || !to) continue;
      const kind = descriptions.get(link.from)?.output;
      const color = kind ? `rgb(${kind.color.join(",")})` : "#8b95ad";
      path(from, to, color);
    }

    if (dragging) {
      const from = outputPosition(dragging.from);
      if (from) path(from, { x: dragging.x, y: dragging.y }, "#6cc2ff");
    }
  }

  /**
   * The nodes a molecule's frame contains: its chain nodes and the styles feeding them.
   *
   * A chain belongs to its molecule - it is one of its parts - so the frame holds both,
   * and dragging the molecule carries them.
   */
  function membersOf(group: GraphNode): GraphNode[] {
    const chains = graph.links
      .filter((link) => link.from === group.id && link.socket === "Molecule")
      .map((link) => nodeById(link.to))
      .filter((node): node is GraphNode => node?.type === "chain");
    const styles = chains.flatMap((chain) =>
      graph.links
        .filter((link) => link.to === chain.id && link.socket === "Style")
        .map((link) => nodeById(link.from))
        .filter((node): node is GraphNode => node?.type === "style"),
    );
    // A style shared by every chain appears once.
    const seen = new Set<string>();
    return [...chains, ...styles].filter((node) => {
      if (seen.has(node.id)) return false;
      seen.add(node.id);
      return true;
    });
  }

  /**
   * Draws a frame around each Molecule group and the styles feeding it.
   *
   * A group whose members are scattered across the plane is a group in name only - the
   * point of calling it one is that you can see what is in it. The frame is measured from
   * the members rather than stored, so it follows them as they move, and dragging the
   * group's header carries them along.
   */
  function drawFrames(): void {
    const frames: (HTMLElement | null)[] = [];
    for (const group of graph.nodes) {
      if (group.type !== "molecule") continue;
      const all = membersOf(group);
      if (all.length === 0) continue;
      const members = group.expanded ? all : [];

      // Measured from the members, because the molecule has no box of its own. Folded,
      // the frame is just its own header.
      const pad = 16;
      const boxes = members.map((node) => ({
        left: node.position[0],
        top: node.position[1],
        right: node.position[0] + NODE_WIDTH,
        bottom: node.position[1] + nodeHeight(node.id),
      }));
      const left = group.position[0];
      const top = group.position[1];
      const right = Math.max(
        left + NODE_WIDTH,
        ...boxes.map((box) => box.right + pad),
      );
      const bottom = Math.max(
        top + 30,
        ...boxes.map((box) => box.bottom + pad),
      );

      const header = el(
        "div",
        { class: "frame-header" },
        el("button", {
          class: "frame-twisty",
          text: group.expanded ? "\u25be" : "\u25b8",
          title: group.expanded ? "Fold this group" : "Unfold this group",
          // Kept from the header, which starts a drag on it - and a drag redraws the
          // frames, replacing this button before its click can arrive.
          onpointerdown: (event: PointerEvent) => event.stopPropagation(),
          onclick: (event: MouseEvent) => {
            event.stopPropagation();
            const node = nodeById(group.id);
            if (node?.type !== "molecule") return;
            node.expanded = !node.expanded;
            commit();
          },
        }),
        el("span", {
          class: "frame-title",
          text: descriptions.get(group.id)?.title ?? "Molecule",
        }),
        group.expanded
          ? null
          : el("span", { class: "frame-count", text: `${all.length}` }),
        // The molecule's own output, so a chain outside the group can still read from it.
        el("span", {
          class: "socket out frame-socket",
          style: `background: rgb(${(descriptions.get(group.id)?.output?.color ?? [
            51, 154, 240,
          ]).join(",")})`,
          title: socketHint("Structure", "Structure", "out"),
          onpointerdown: (event: PointerEvent) => {
            event.stopPropagation();
            startLink(group.id, event);
          },
        }),
      );
      // Dragging the header moves the group, folded or not.
      header.addEventListener("pointerdown", (event) => startNodeDrag(group.id, event));

      const frame = el(
        "div",
        {
          class:
            `node-frame${selected === group.id ? " selected" : ""}` +
            `${group.expanded ? "" : " folded"}`,
          style:
            `left:${left}px; top:${top}px; ` +
            `width:${right - left}px; height:${bottom - top}px`,
          onpointerdown: () => {
            selected = group.id;
            applySelection();
          },
        },
        header,
      );
      frames.push(frame);
    }
    replace(frameLayer, ...frames);
  }

  /**
   * Everything that decides which widgets a body contains. Values are in here too, so a
   * body is rebuilt after an edit settles and reflects whatever the graph now says - but
   * never while a pointer or the keyboard is on it.
   */
  function bodyKey(node: GraphNode): string {
    const fields = { ...node } as Record<string, unknown>;
    delete fields.position;
    // Render progress and node state live outside the node but are drawn inside it.
    const extra = JSON.stringify([
      library.styles.length,
      renderContext.unavailable,
      renderContext.progress[node.id] ?? null,
      states[node.id] ?? null,
    ]);
    return `${models.length}:${providers.length}:${JSON.stringify(fields)}:${extra}`;
  }

  function socketKey(id: string): string {
    const description = descriptions.get(id);
    if (!description) return "";
    const folded = nodeById(id)?.collapsed ? "folded" : "open";
    const inputs = description.inputs
      .map((input) => {
        const connected = graph.links.some((link) => link.to === id && link.socket === input.name);
        return `${input.name}/${input.kind}/${input.color.join("-")}/${input.required ? 1 : 0}/${connected ? 1 : 0}`;
      })
      .join(",");
    return `${folded}|${inputs}|${description.output ? `${description.output.kind}/${description.output.color.join("-")}` : ""}`;
  }

  /**
   * Nodes inside a folded group.
   *
   * A group with twenty chains is twenty nodes on the plane whether or not you are working
   * on them, so folding the group has to take them with it - node folding alone only ever
   * shrinks one node at a time.
   */
  function hiddenNodes(): Set<string> {
    const hidden = new Set<string>();
    for (const node of graph.nodes) {
      if (node.type !== "molecule" || node.expanded) continue;
      for (const member of membersOf(node)) hidden.add(member.id);
    }
    return hidden;
  }

  /**
   * Nodes that are not drawn as boxes.
   *
   * A molecule is a group, not a node: its frame *is* it, carrying the title, the fold
   * and the drag. Drawing a box as well put the same thing on screen twice and left the
   * group with a member that was also its owner.
   */
  function isGroupOwner(node: GraphNode): boolean {
    return node.type === "molecule";
  }

  function drawNodes(): void {
    const hidden = hiddenNodes();
    for (const [id, entry] of [...nodeElements]) {
      if (!graph.nodes.some((node) => node.id === id) || hidden.has(id)) {
        entry.root.remove();
        nodeElements.delete(id);
      }
    }
    for (const [id, entry] of [...nodeElements]) {
      const node = nodeById(id);
      if (node && isGroupOwner(node)) {
        entry.root.remove();
        nodeElements.delete(id);
      }
    }
    for (const node of graph.nodes) {
      if (hidden.has(node.id) || isGroupOwner(node)) continue;
      let entry = nodeElements.get(node.id);
      if (!entry) {
        entry = createNode(node);
        nodeElements.set(node.id, entry);
        nodeLayer.appendChild(entry.root);
      }
      updateNode(node, entry);
    }
  }

  function createNode(node: GraphNode): NodeEntry {
    const title = el("span", { class: "node-title" });
    const spinner = el("span", { class: "node-spinner", text: "...", hidden: "true" });
    const fold = el("button", {
      class: "node-fold",
      title: "Collapse or expand (H)",
      onpointerdown: (event: PointerEvent) => event.stopPropagation(),
      onclick: (event: MouseEvent) => {
        event.stopPropagation();
        toggleCollapse(node.id);
      },
    });
    const headerSockets = el("div", { class: "header-sockets" });
    const header = el("div", { class: "node-header" }, headerSockets, fold, title, spinner);
    header.addEventListener("pointerdown", (event) => startNodeDrag(node.id, event));

    const sockets = el("div", { class: "node-sockets" });
    const body = el("div", { class: "node-body" });
    const error = el("div", { class: "node-error", hidden: "true" });

    const root = el(
      "div",
      { class: "node", style: `width:${NODE_WIDTH}px` },
      header,
      sockets,
      body,
      error,
    );
    root.addEventListener("pointerdown", () => {
      // Selecting only moves a class around. It used to redraw, which replaced the node
      // under the pointer - the reason sliders could not be dragged at all.
      selected = node.id;
      applySelection();
    });
    root.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      event.stopPropagation();
      openNodeMenu({ x: event.clientX, y: event.clientY }, node.id);
    });

    return {
      root,
      header,
      title,
      fold,
      spinner,
      headerSockets,
      sockets,
      body,
      error,
      node,
      bodyKey: "",
      socketKey: "",
    };
  }

  function updateNode(node: GraphNode, entry: NodeEntry): void {
    const description = descriptions.get(node.id);
    const state = states[node.id];

    entry.root.style.left = `${node.position[0]}px`;
    entry.root.style.top = `${node.position[1]}px`;
    // Both outputs get the tinted header: they are where a pipeline ends up.
    const isOutput = node.type === "viewport" || node.type === "render";
    entry.root.className = `node${isOutput ? " output" : ""}`;
    entry.root.classList.toggle("collapsed", node.collapsed === true);
    entry.root.classList.toggle("failed", state?.state === "failed");
    entry.root.classList.toggle("warned", state?.state === "warned");
    entry.root.classList.toggle("running", state?.state === "running");
    entry.root.classList.toggle("incomplete", state?.state === "incomplete");
    entry.root.classList.toggle("unused", state?.state === "unused");
    entry.root.classList.toggle("selected", selected === node.id);

    entry.title.textContent = description?.title ?? node.type;
    // The explanation lives in the tooltip rather than in the node's body: a node should
    // show its settings, and what the node *is* is what you hover it to find out.
    const hint = nodeHint(node.type);
    entry.header.title = `${description?.title ?? node.type}${
      description?.category ? ` (${description.category})` : ""
    }${hint ? `\n${hint}` : ""}`;
    entry.fold.textContent = node.collapsed ? ">" : "v";
    // Tinted by family, as Blender does: it is the fastest way to read a graph you did
    // not build. The colour comes from the core, so it matches the sockets that family
    // produces rather than being chosen twice.
    const accent = description?.accent;
    entry.header.style.background = accent
      ? `rgba(${accent.join(",")}, 0.16)`
      : "";
    entry.header.style.borderBottomColor = accent ? `rgba(${accent.join(",")}, 0.5)` : "";
    entry.root.style.setProperty(
      "--node-accent",
      accent ? `rgb(${accent.join(",")})` : "var(--line-bright)",
    );
    entry.spinner.hidden = state?.state !== "running";

    if (state?.state === "failed" || state?.state === "warned") {
      entry.error.textContent = state.message;
      entry.error.hidden = false;
    } else {
      entry.error.hidden = true;
    }

    const nextSocketKey = socketKey(node.id);
    if (nextSocketKey !== entry.socketKey) {
      entry.socketKey = nextSocketKey;
      renderSockets(node, entry);
    }

    const nextBodyKey = bodyKey(node);
    // A body is only replaced when nothing is holding it: no widget mid-drag, no field
    // focused. Otherwise the edit in progress would be interrupted by its own effect.
    const held = interacting > 0 || entry.body.contains(document.activeElement);
    if (entry.node !== node || (nextBodyKey !== entry.bodyKey && !held)) {
      entry.node = node;
      entry.bodyKey = nextBodyKey;
      replace(entry.body, ...parameters(node));
    }
  }

  /**
   * Fills the outliner.
   *
   * A tree with disclosure triangles, like Blender's: the scene, then each molecule with
   * its chains and their styles, then the lighting, then the outputs. Everything in the
   * graph appears exactly once and in one place, which is what the flat list of sections
   * was not - it showed some things twice and others not at all.
   */
  function drawOutliner(): void {
    const rows: Node[] = [];

    const molecules = graph.nodes.filter((node) => node.type === "molecule");
    if (molecules.length === 0) rows.push(el("div", { class: "properties-empty", text: "Nothing in the scene yet." }));
    {
      for (const molecule of molecules) {
        if (molecule.type !== "molecule") continue;
        const key = `mol:${molecule.id}`;
        const chains = graph.links
          .filter((link) => link.from === molecule.id && link.socket === "Molecule")
          .map((link) => nodeById(link.to))
          .filter((node): node is GraphNode => node?.type === "chain");
        rows.push(
          withMenu(
            outlinerRow({
              depth: 0,
              id: molecule.id,
              icon: "\u25c6",
              mark: "molecule",
              label: descriptions.get(molecule.id)?.title ?? "Molecule",
              value: chains.length ? count(chains.length, "chain") : "",
              twisty: openBranches.has(key) ? "open" : "closed",
              onTwisty: () => toggleBranch(key),
            }),
            { kind: "molecule", moleculeId: molecule.id },
          ),
        );
        // What it is read as - the biological assembly, the crystal's unit, a model of an
        // ensemble - one click from being changed.
        const version = options.versionRow?.(molecule.id);
        if (version) {
          const row: SceneRow = { kind: "version", moleculeId: molecule.id };
          rows.push(
            withMenu(
              el(
                "div",
                { class: "panel-row version-row", style: "padding-left:21px" },
                el("span", { class: "row-twisty" }),
                el(
                  "button",
                  {
                    class: "row-body",
                    title: version.title,
                    onclick: (event: MouseEvent) => {
                      const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
                      options.onSceneMenu?.(row, { x: box.left + 18, y: box.bottom + 2 });
                    },
                  },
                  icon("molecule", 13),
                  el("span", { class: "panel-row-label", text: version.label }),
                  el("span", { class: "panel-row-value", text: version.value }),
                  el("span", { class: "version-caret", text: "\u25be" }),
                ),
              ),
              row,
            ),
          );
        }
        // The molecule's named objects, with it whether or not its chains are unfolded:
        // they are what was picked out to be found again.
        for (const object of molecule.objects ?? []) {
          rows.push(
            withMenu(el(
              "div",
              { class: "panel-row object-row", style: "padding-left:21px" },
              el("span", { class: "row-twisty" }),
              el(
                "button",
                {
                  class: "row-body",
                  title: `${object.name} - select it`,
                  onclick: () => options.onObject?.(molecule.id, object.name),
                },
                icon("object", 13),
                el("span", { class: "panel-row-label", text: object.name }),
                el("span", { class: "panel-row-value", text: "object" }),
              ),
              el("button", {
                class: "id-action",
                text: "×",
                title: `Remove ${object.name} (the molecule is not changed)`,
                onclick: () => options.onObjectRemove?.(molecule.id, object.name),
              }),
            ), { kind: "object", moleculeId: molecule.id, name: object.name }),
          );
        }
        rows.push(...presetBranch(molecule.id));
        if (!openBranches.has(key)) continue;
        // A chain is a node, so it is a row of its own with its style beside it.
        for (const chain of chains) {
          // The slot the chain draws with, which is what the Style tab is editing - or
          // the name of the node that overrides it.
          rows.push(
            withMenu(
              outlinerRow({
                depth: 1,
                id: chain.id,
                icon: "\u25cf",
                mark: "chain",
                label: chain.type === "chain" ? chain.chain : "chain",
                value: slotName(molecule, chain),
              }),
              { kind: "chain", moleculeId: molecule.id, chain: chain.type === "chain" ? chain.chain : "", nodeId: chain.id },
            ),
          );
        }
      }
    }

    // The scene is its molecules, and that is all this list holds. The lights, the
    // outputs and the world are how it is looked at; they are in the panel below, which
    // shows them whenever nothing in the scene is selected.
    replace(outliner, ...rows);
    fitOutliner();
  }

  /**
   * The groups a molecule was analysed into, under one row that folds.
   *
   * Folded it is one line that says how many groups there are - or, while the analysis is
   * still running, how far along it is, which on a large structure is the only sign that
   * anything is happening. Unfolded, a set of residues is selected by a click and a set of
   * edges is shown or hidden by one.
   */
  function presetBranch(moleculeId: string): HTMLElement[] {
    const groups = options.presetRows?.(moleculeId);
    if (!groups) return [];
    const key = `groups:${moleculeId}`;
    const open = openBranches.has(key);
    const pending = groups.filter((group) => group.state === "loading").length;
    const out = [
      el(
        "div",
        { class: "panel-row preset-head", style: "padding-left:21px" },
        el("button", {
          class: "row-twisty",
          text: open ? "\u25be" : "\u25b8",
          title: open ? "Fold" : "Unfold",
          "aria-expanded": String(open),
          onclick: () => toggleBranch(key),
        }),
        el(
          "button",
          { class: "row-body", title: "What this molecule was analysed for when it was opened", onclick: () => toggleBranch(key) },
          pending > 0 ? el("span", { class: "spinner" }) : icon("bond", 13),
          el("span", { class: "panel-row-label", text: "Groups" }),
          el("span", {
            class: "panel-row-value",
            text: pending > 0 ? `analysing ${groups.length - pending}/${groups.length}` : String(groups.filter((group) => group.count > 0).length),
          }),
        ),
      ),
    ];
    if (!open) return out;
    for (const group of groups) {
      const empty = group.state === "ready" && group.count === 0;
      out.push(
        withMenu(el(
          "div",
          { class: `panel-row preset-row${group.shown ? " shown" : ""}${empty ? " none" : ""}`, style: "padding-left:34px" },
          el("span", { class: "row-twisty" }),
          el(
            "button",
            {
              class: "row-body",
              disabled: group.state !== "ready" || empty ? "" : undefined,
              title:
                group.state === "failed"
                  ? (group.error ?? "This could not be worked out.")
                  : group.edges
                    ? `${group.label} - ${group.shown ? "hide them" : "show them"}`
                    : `${group.label} - select these residues`,
              onclick: () => options.onPreset?.(moleculeId, group.id),
            },
            group.state === "loading"
              ? el("span", { class: "spinner" })
              : group.edges
                ? el("span", { class: "preset-swatch", style: `background:${group.shown ? group.swatch : "transparent"};border-color:${group.swatch ?? "currentColor"}` })
                : icon("residue", 13),
            el("span", { class: "panel-row-label", text: group.label }),
            el("span", {
              class: "panel-row-value",
              text: group.state === "loading" ? "..." : group.state === "failed" ? "failed" : group.count.toLocaleString(),
            }),
          ),
        ), { kind: "group", moleculeId, id: group.id, edges: group.edges }),
      );
    }
    return out;
  }

  /** Gives a scene row its right-click menu. */
  function withMenu<T extends HTMLElement>(element: T, row: SceneRow): T {
    element.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      event.stopPropagation();
      options.onSceneMenu?.(row, { x: event.clientX, y: event.clientY });
    });
    return element;
  }

  function toggleBranch(key: string): void {
    if (openBranches.has(key)) openBranches.delete(key);
    else openBranches.add(key);
    drawOutliner();
  }


  interface OutlinerRow {
    depth: number;
    /** The node this row is about, if there is one. */
    id: string | null;
    icon: string;
    /** One of the structure's own marks, in place of the glyph. */
    mark?: StructureIcon;
    label: string;
    value: string;
    heading?: boolean;
    twisty?: "open" | "closed";
    onTwisty?: () => void;
    /** Clicked when the row has no node yet - assigning a style to a chain. */
    onCreate?: () => void;
    /** Overrides the tooltip, which otherwise comes from the node the row is about. */
    hint?: string;
  }

  /** What an outliner row says on hover: the node's kind, or that it is a heading. */
  function rowHint(row: OutlinerRow): string {
    if (row.heading) return `${row.label} - click to fold this branch`;
    const node = row.id ? nodeById(row.id) : undefined;
    if (!node) return row.label;
    const hint = nodeHint(node.type);
    return `${row.label}${hint ? ` - ${hint}` : ""}`;
  }

  function outlinerRow(row: OutlinerRow): HTMLElement {
    const chosen = row.id !== null && row.id === selected;
    return el(
      "div",
      {
        class:
          `panel-row${chosen ? " selected" : ""}` +
          `${row.id === null && !row.heading ? " empty" : ""}${row.heading ? " heading" : ""}`,
        style: `padding-left:${8 + row.depth * 13}px`,
      },
      row.twisty
        ? el("button", {
            class: "row-twisty",
            text: row.twisty === "open" ? "\u25be" : "\u25b8",
            title: row.twisty === "open" ? "Fold" : "Unfold",
            "aria-expanded": String(row.twisty === "open"),
            onclick: (event: MouseEvent) => {
              event.stopPropagation();
              row.onTwisty?.();
            },
          })
        : el("span", { class: "row-twisty" }),
      el(
        "button",
        {
          class: "row-body",
          title: row.hint ?? rowHint(row),
          onclick: () => {
            if (row.id) {
              selected = row.id;
              applySelection();
            } else {
              row.onCreate?.();
            }
          },
        },
        row.mark ? icon(row.mark, 13) : row.icon ? el("span", { class: "row-icon", text: row.icon }) : null,
        el("span", { class: "panel-row-label", text: row.label }),
        el("span", { class: "panel-row-value", text: row.value }),
      ),
    );
  }

  /** The library name a style matches exactly, if any. */
  function nameOf(style: unknown): string | null {
    const encoded = JSON.stringify(style);
    return library.styles.find((entry) => JSON.stringify(entry.style) === encoded)?.name ?? null;
  }

  /** The style a new library entry starts from: whatever the app's own default is. */
  function defaultStyle(): unknown {
    // By name: "the first shipped entry" is only the cartoon by accident of ordering, and
    // the default a new style starts from is a decision, not an accident.
    const shipped =
      library.styles.find((entry) => entry.builtin && entry.name === "Cartoon") ??
      library.styles.find((entry) => entry.builtin);
    return shipped
      ? structuredClone(shipped.style)
      : {
          representations: ["cartoon"],
          scheme: "chain",
          categorical: "okabe-ito",
          continuous: false,
          solid: null,
          ramp: "rainbow",
          material: { roughness: 0.28, specular: 0.5, metallic: 0 },
          heteroatoms_by_element: true,
        };
  }

  /** A swatch for a style: its flat colour, or a hint that it is a rule. */
  function swatchOf(style: { solid: [number, number, number] | null } | undefined): string {
    if (!style) return "background: var(--line)";
    if (style.solid) {
      const srgb = style.solid.map((channel) =>
        Math.round(255 * (channel <= 0.0031308 ? channel * 12.92 : 1.055 * channel ** (1 / 2.4) - 0.055)),
      );
      return `background: rgb(${srgb.join(",")})`;
    }
    // A colouring rule has no one colour, so the swatch says "several".
    return "background: linear-gradient(135deg, #e78284 0%, #e5c890 50%, #8caaee 100%)";
  }

  /** Adds a catalogue node and wires it into a socket. */
  function addAndWire(typeName: string, into: string, socket: string): void {
    const template = catalog.find((entry) => entry.type_name === typeName);
    const target = nodeById(into);
    if (!template || !target) return;
    const id = `${typeName}-${Date.now().toString(36)}`;
    graph.nodes.push({
      ...structuredClone(template.kind),
      id,
      position: [target.position[0] - 296, target.position[1] + 160],
    } as GraphNode);
    graph.links = graph.links.filter((link) => !(link.to === into && link.socket === socket));
    graph.links.push({ from: id, to: into, socket });
    selected = id;
    commit();
  }

  /** Adds a catalogue node by type name, for the panel's own add buttons. */
  function addFromCatalog(typeName: string, at: { x: number; y: number }): void {
    const template = catalog.find((entry) => entry.type_name === typeName);
    if (template) addNode(template, at);
  }

  /**
   * Creates a Style node and wires it to one of a group's style sockets.
   *
   * The only way to give a chain its own style without laying out nodes by hand, which is
   * why the outliner's unassigned rows are clickable.
   */
  function assignStyle(chainId: string): void {
    addAndWire("style", chainId, "Style");
  }


  function drawProperties(): void {
    // The guard is about not pulling a widget out from under someone mid-edit, so it is
    // only text entry that blocks a redraw. It used to be *any* focus inside the panel,
    // which meant that after clicking a button there - a tab, an outliner row - the panel
    // stopped updating: clicking the viewport background then cleared the selection and
    // the panel went on showing the old one.
    const focused = document.activeElement;
    const editing =
      focused !== null &&
      properties.contains(focused) &&
      ["INPUT", "TEXTAREA", "SELECT"].includes(focused.tagName);
    if (interacting > 0 || editing) return;
    drawOutliner();
    drawNodeProperties();
  }

  // --- the properties panel --------------------------------------------------
  //
  // Blender's, as closely as this app can be: tabs down the left edge, a breadcrumb with a
  // pin, and a stack of collapsible panels. The Style tab is the Material tab - a slot
  // list, the library to pick from, and the style's own fields shown inline.

  /** The node the panel is about: the pinned one, or the selected one. */
  function propNode(): GraphNode | null {
    // A pin on a node that is gone - deleted, undone away, another world's graph loaded - is
    // a pin on nothing, and kept it would hold the panel on "nothing selected" whatever
    // was clicked next.
    if (pinned !== null && !nodeById(pinned)) pinned = null;
    const id = pinned ?? selected;
    return (id ? nodeById(id) : undefined) ?? null;
  }

  /** The molecule a node belongs to, following a chain's Molecule link. */
  function moleculeOf(node: GraphNode | null): GraphNode | null {
    if (!node) return null;
    if (node.type === "molecule") return node;
    const chain = chainOf(node);
    if (chain) {
      const wired = graph.links.find(
        (link) => link.to === chain.id && link.socket === "Molecule",
      );
      const molecule = wired ? nodeById(wired.from) : undefined;
      if (molecule?.type === "molecule") return molecule;
    }
    return null;
  }

  /** The chain a node is, or the chain a style network feeds. */
  function chainOf(node: GraphNode | null): GraphNode | null {
    if (!node) return null;
    if (node.type === "chain") return node;
    if (["style", "colouring", "solid-colour", "material"].includes(node.type)) {
      // Walk forward: a Colouring feeds a Style, which feeds a chain.
      const seen = new Set<string>();
      let front = [node.id];
      while (front.length > 0) {
        const next: string[] = [];
        for (const id of front) {
          for (const link of graph.links.filter((link) => link.from === id)) {
            if (seen.has(link.to)) continue;
            seen.add(link.to);
            const target = nodeById(link.to);
            if (target?.type === "chain") return target;
            if (target) next.push(target.id);
          }
        }
        front = next;
      }
    }
    return null;
  }

  /**
   * Keeps the active molecule and chain, the way Blender keeps the active object.
   *
   * Selecting a light does not clear the Style tab: the object stays active until another
   * object is picked. Falls back to the only molecule in the graph, because a panel that
   * is empty until you happen to click the right node is a panel people conclude is
   * broken.
   */
  function trackActive(): void {
    const node = propNode();
    const molecule = moleculeOf(node);
    if (molecule) activeMolecule = molecule.id;
    const chain = chainOf(node);
    if (chain) activeChain = chain.id;

    if (!activeMolecule || !nodeById(activeMolecule)) {
      activeMolecule = graph.nodes.find((entry) => entry.type === "molecule")?.id ?? null;
    }
    if (!activeChain || !nodeById(activeChain)) activeChain = null;
    // A chain from another molecule is not this molecule's chain.
    if (activeChain && moleculeOf(nodeById(activeChain) ?? null)?.id !== activeMolecule) {
      activeChain = null;
    }
  }

  function activeMoleculeNode(): GraphNode | null {
    return (activeMolecule ? nodeById(activeMolecule) : undefined) ?? null;
  }

  function activeChainNode(): GraphNode | null {
    return (activeChain ? nodeById(activeChain) : undefined) ?? null;
  }

  function tabContext(): TabContext {
    const node = propNode();
    return {
      selection: node?.type ?? null,
      hasMolecule: activeMoleculeNode() !== null,
      hasChain: activeChainNode() !== null,
      // Data tabs follow what is active, so a light's settings appear when a light is it.
      hasLight: node?.type === "light",
      hasRender: graph.nodes.some((entry) => entry.type === "render"),
      hasImage: graph.nodes.some((entry) => entry.type === "image"),
      hasWorld: graph.nodes.some((entry) => entry.type === "world"),
    };
  }

  /** What the panel was last drawn for, so a tab is only chosen for it when it changes. */
  let tabSubject: string | null = null;

  function drawNodeProperties(): void {
    trackActive();
    const node = propNode();

    const tabs = propertyTabs(tabContext());
    // The selection's own tab is gone to when the selection *changes* - pick a chain, get
    // the chain. Asked again on every redraw, it also undid every click on another tab
    // while something was selected: Render and World could not be opened at all with a
    // chain picked, and the tab strip looked dead.
    const subject = node?.id ?? null;
    const changed = subject !== tabSubject;
    tabSubject = subject;
    tab = resolveTab(tab, tabs, changed ? preferredTab(node?.type ?? null) : null);

    replace(
      propsTabs,
      ...tabs.map((spec) =>
        el("button", {
          class: `props-tab${spec.key === tab ? " active" : ""}`,
          text: spec.icon,
          title: `${spec.label}${tabHint(spec.label) ? ` - ${tabHint(spec.label)}` : ""}`,
          onclick: () => {
            tab = spec.key;
            drawNodeProperties();
          },
        }),
      ),
    );

    replace(propsCrumbs, ...crumbs(), pinButton());
    replace(propsBody, ...tabPanels(tab));

    drawSelectionCard();
  }

  /**
   * The viewport's corner card: what is selected, then what the pointer is over.
   *
   * Nothing at all when neither applies. Hover is the pale line at the bottom, so moving
   * the mouse across a structure reads it out without a click - and without touching the
   * selection above it.
   */
  function drawSelectionCard(): void {
    const node = propNode();
    const description = node ? descriptions.get(node.id) : undefined;
    if (!node && !hoverDetail) {
      selection.hidden = true;
      return;
    }
    replace(
      selection,
      node
        ? el(
            "div",
            { class: "selection-head" },
            el("span", {
              class: "selection-dot",
              style: `background: rgb(${(description?.accent ?? [140, 140, 140]).join(",")})`,
            }),
            el("span", { class: "selection-name", text: description?.title ?? node.type }),
            el("span", { class: "selection-kind", text: description?.category ?? "" }),
          )
        : null,
      // Informative and short: one line per fact, nothing the title already said.
      ...(node ? (selectionDetail ?? []) : []).map((line) =>
        el("div", { class: "selection-detail", text: line }),
      ),
      hoverDetail ? el("div", { class: "selection-hover", text: hoverDetail }) : null,
    );
    selection.hidden = false;
  }


  /** One step of the breadcrumb: an icon, a name, and the node it selects. */
  function crumb(icon: string, label: string, id: string | null): HTMLElement {
    return el(
      "button",
      {
        class: "props-crumb",
        title: id ? `${label} - select it` : label,
        disabled: id === null,
        onclick: () => {
          if (!id) return;
          selected = id;
          applySelection();
        },
      },
      el("span", { class: "crumb-icon", text: icon }),
      el("span", { class: "crumb-name", text: label }),
    );
  }

  /**
   * The path to what is being edited: `4HHB > Chain A > Cartoon`.
   *
   * Blender's is object > data > material, and it is the only thing in the editor that
   * says where the settings you are looking at actually live.
   */
  function crumbs(): HTMLElement[] {
    const molecule = activeMoleculeNode();
    const chain = activeChainNode();
    const steps: HTMLElement[] = [];

    const named = (node: GraphNode | null, fallback: string) =>
      node ? (descriptions.get(node.id)?.title ?? fallback) : fallback;

    if (tab === "render" || tab === "output" || tab === "world") {
      const node = tabNode(tab);
      steps.push(crumb(tab === "world" ? "○" : "◑", named(node, tab), node?.id ?? null));
    } else if (tab === "light") {
      const light = propNode();
      steps.push(
        crumb("☀", light?.type === "light" ? light.light.name : "Light", light?.id ?? null),
      );
    } else {
      if (molecule) steps.push(crumb("◆", named(molecule, "Molecule"), molecule.id));
      if (chain && (tab === "chain" || tab === "style")) {
        steps.push(crumb("●", named(chain, "Chain"), chain.id));
      }
      if (tab === "style") {
        const slot = activeSlot();
        const wired = chain ? wiredStyle(chain.id) : null;
        if (wired) steps.push(crumb("◕", named(wired, "Style"), wired.id));
        else if (slot) steps.push(crumb("◕", slot.name, molecule?.id ?? null));
      }
    }

    const path: HTMLElement[] = [];
    steps.forEach((step, index) => {
      if (index > 0) path.push(el("span", { class: "crumb-sep", text: "›" }));
      path.push(step);
    });
    if (path.length === 0) path.push(el("span", { class: "crumb-empty", text: "nothing selected" }));
    return path;
  }

  /**
   * The pin, which freezes the panel on what it is showing.
   *
   * The same reason Blender's exists: editing a style while clicking around the viewport
   * means the selection keeps changing under the panel.
   */
  function pinButton(): HTMLElement {
    return el("button", {
      class: `props-pin${pinned ? " on" : ""}`,
      text: "⌘",
      title: pinned ? "Unpin: follow the selection again" : "Pin: keep this in the panel",
      onclick: () => {
        pinned = pinned ? null : selected;
        drawNodeProperties();
      },
    });
  }

  /** The node a scene-wide tab edits: the selected one of its kind, or the only one. */
  function tabNode(key: TabKey): GraphNode | null {
    const kind = key === "render" ? "render" : key === "output" ? "image" : "world";
    const node = propNode();
    if (node?.type === kind) return node;
    // A Viewport selected while the Render tab is open still means the render: they are
    // two settings of one output.
    return graph.nodes.find((entry) => entry.type === kind) ?? null;
  }

  /**
   * One collapsible panel, with its open state remembered across redraws.
   *
   * `nested` is Blender's `bl_parent_id`: a sub-panel, drawn inside its parent's body and
   * indented under it. Draw and Colour are *part of* a style, not siblings of it, and a
   * panel stack that puts them at the same level says the wrong thing about what contains
   * what.
   */
  function panel(
    key: string,
    label: string,
    rows: (HTMLElement | null)[],
    trailing?: HTMLElement | null,
    nested = false,
  ): HTMLElement {
    const open = openPanels.has(key);
    return el(
      "div",
      { class: `props-panel${open ? " open" : ""}${nested ? " nested" : ""}` },
      el(
        "button",
        {
          class: "props-panel-head",
          title: `${label} - click to ${open ? "fold" : "unfold"}`,
          onclick: () => {
            if (open) openPanels.delete(key);
            else openPanels.add(key);
            drawNodeProperties();
          },
        },
        el("span", { class: "panel-twisty", text: open ? "▾" : "▸" }),
        el("span", { class: "panel-title", text: label }),
      ),
      open
        ? el(
            "div",
            { class: "props-panel-body" },
            ...rows.filter((row): row is HTMLElement => row !== null),
            trailing ?? null,
          )
        : null,
    );
  }

  /** A read-only value row, for what a molecule *is* rather than how it is drawn. */
  function readOnly(label: string, value: string): HTMLElement {
    return el(
      "div",
      { class: "node-field" },
      el("label", { text: label }),
      el("div", { class: "props-readonly", text: value }),
    );
  }

  function tabPanels(key: TabKey | null): HTMLElement[] {
    switch (key) {
      case "render":
      case "output":
      case "world": {
        const node = tabNode(key);
        if (!node) return [empty("nothing to configure")];
        // The lights and the cameras have no rows in the scene's list - a lamp is not a
        // thing in the scene - so they are reached here: lights with the world they light,
        // cameras with the render they are for.
        const lights = key === "world" ? graph.nodes.filter((entry) => entry.type === "light") : [];
        return [
          ...parameterGroups(node).map((group) => panel(group.key, group.label, group.rows)),
          ...lights.map((light) =>
            panel(
              `light-${light.id}`,
              light.type === "light" ? light.light.name : "Light",
              parameters(light),
              el("button", { class: "id-action", text: "×", title: "Remove this light", onclick: () => removeNode(light.id) }),
            ),
          ),
          key === "world" ? el("button", { class: "panel-add", text: "+ Light", onclick: () => addFromCatalog("light", centerOfView()) }) : null,
          ...(key === "render" ? (options.cameraRows?.() ?? []) : []),
          ...stateRows(node),
        ].filter((entry): entry is HTMLElement => entry !== null);
      }

      case "light": {
        const node = propNode();
        if (node?.type !== "light") return [empty("no light selected")];
        return [
          panel("light-data", "Light", parameters(node)),
          ...stateRows(node),
        ];
      }

      case "structure": {
        const molecule = activeMoleculeNode();
        if (molecule?.type !== "molecule") return [empty("no molecule in the scene")];
        const chains = graph.links
          .filter((link) => link.from === molecule.id && link.socket === "Molecule")
          .map((link) => nodeById(link.to))
          .filter((node): node is GraphNode => node?.type === "chain");
        return [
          panel("source", "Source", [
            readOnly("PDB id", molecule.pdb_id ? molecule.pdb_id.toUpperCase() : "-"),
            readOnly("File", molecule.path ?? "-"),
            readOnly("Chains", molecule.chains.join(" ") || "-"),
          ]),
          panel("contents", "Contents", parameters(molecule)),
          panel("group-colour", "Colour", groupColourRows(molecule)),
          // What the molecule is *drawn* with, which is a property of the molecule - the
          // Style tab is where styles are written, this is where they are in use.
          slotBlock(molecule, slotsOf(molecule), activeSlotIndex()),
          panel(
            "molecule-chains",
            "Chains",
            chains.map((chain) =>
              el(
                "button",
                {
                  class: `props-list-row${chain.id === activeChain ? " selected" : ""}`,
                  title:
                    chain.type === "chain"
                      ? `Chain ${chain.chain}, drawn with ${slotName(molecule, chain)} - click to select it`
                      : "",
                  onclick: () => {
                    selected = chain.id;
                    applySelection();
                  },
                },
                el("span", { class: "props-list-name", text: chain.type === "chain" ? chain.chain : "" }),
                el("span", {
                  class: "props-list-value",
                  text: chain.type === "chain" ? slotName(molecule, chain) : "",
                }),
              ),
            ),
          ),
          ...stateRows(molecule),
        ];
      }

      case "chain": {
        const chain = activeChainNode();
        if (chain?.type !== "chain") return [empty("no chain selected")];
        return [
          panel("chain-data", "Chain", [readOnly("Id", chain.chain)]),
          ...chainStylePanels(chain),
          // Only where the fields above cannot be: a chain driven by a Style node, or a
          // molecule with no slots yet.
          ...(wiredStyle(chain.id) || slotsOf(moleculeOf(chain)).length === 0 ? [panel("chain-slot", "Style", chainStyleRows(chain))] : []),
          ...stateRows(chain),
        ];
      }

      case "style":
        return styleTabPanels();

      default:
        return [empty("nothing selected")];
    }
  }

  /**
   * Colouring for the whole molecule group.
   *
   * Chainbow and a colour per chain are whole-molecule schemes: chainbow needs each chain
   * to know where its own ends are, and colouring by chain needs the chain list to hand
   * colours out from. A lone chain set to chainbow would just be a rainbow. So they are
   * set here, on the group, and written into every slot its chains draw with - which is
   * how PyMOL's `util.chainbow` and `util.cbc` behave: you run them on an object, not on
   * one chain.
   */
  function groupColourRows(molecule: GraphNode): (HTMLElement | null)[] {
    if (molecule.type !== "molecule") return [];
    const slots = slotsOf(molecule);
    if (slots.length === 0) {
      return [el("div", { class: "props-note" }, el("span", { text: "No style slots yet." }))];
    }

    // What every slot agrees on, or nothing when they differ.
    const schemes = new Set(slots.map((slot) => slot.style.scheme));
    const shared = schemes.size === 1 ? slots[0].style.scheme : null;
    const ramps = new Set(slots.map((slot) => slot.style.ramp));
    const sharedRamp = ramps.size === 1 ? slots[0].style.ramp : null;

    const writeAll = (change: (style: Style) => Style) => {
      if (molecule.type !== "molecule") return;
      setSlots(
        molecule,
        slotsOf(molecule).map((slot) => ({ ...slot, style: change(slot.style) })),
        molecule.active_slot ?? 0,
      );
    };

    return [
      field(
        "Colour by",
        el(
          "select",
          {
            onchange: (event: Event) => {
              const chosen = (event.target as HTMLSelectElement).value as ColorScheme;
              // A flat colour would sit on top of the scheme, so choosing one here drops
              // it - the group is being coloured by a rule now.
              writeAll((style) => ({ ...style, scheme: chosen, solid: null }));
            },
          },
          ...(shared === null
            ? [el("option", { value: "", text: "(chains differ)", selected: true })]
            : []),
          ...(Object.entries(COLOR_SCHEME_LABELS) as [ColorScheme, string][]).map(
            ([key, label]) =>
              el("option", {
                value: key,
                text: MOLECULE_WIDE_SCHEMES.includes(key) ? `${label} - whole molecule` : label,
                selected: key === shared,
              }),
          ),
        ),
      ),
      shared !== null && RAMP_SCHEMES.includes(shared)
        ? field(
            "Ramp",
            el(
              "select",
              {
                onchange: (event: Event) => {
                  const chosen = (event.target as HTMLSelectElement).value as Ramp;
                  writeAll((style) => ({ ...style, ramp: chosen }));
                },
              },
              ...(Object.entries(RAMP_LABELS) as [Ramp, string][]).map(([key, label]) =>
                el("option", { value: key, text: label, selected: key === sharedRamp }),
              ),
            ),
          )
        : null,
      el("div", {
        class: "help",
        text:
          slots.length > 1
            ? `Applies to all ${slots.length} style slots.`
            : "Applies to every chain in the group.",
      }),
    ];
  }

  function empty(text: string): HTMLElement {
    return el("div", { class: "properties-empty", text });
  }

  /** A node's own complaint, kept under its panels rather than in place of them. */
  function stateRows(node: GraphNode): HTMLElement[] {
    const state = states[node.id];
    if (state?.state === "failed" || state?.state === "warned") {
      return [el("div", { class: "node-error", text: state.message })];
    }
    if (state?.state === "incomplete") {
      return [el("div", { class: "node-error", text: state.reason })];
    }
    return [];
  }

  // --- style slots -----------------------------------------------------------

  function slotsOf(molecule: GraphNode | null): StyleSlot[] {
    return molecule?.type === "molecule" ? (molecule.slots ?? []) : [];
  }

  /** Which slot the panel is editing, clamped to the list that actually exists. */
  function activeSlotIndex(): number {
    const molecule = activeMoleculeNode();
    const slots = slotsOf(molecule);
    if (slots.length === 0) return -1;
    // A chain being active means its slot is the one on show, the way selecting a face in
    // Blender makes its material the active one.
    const chain = activeChainNode();
    if (chain?.type === "chain" && chain.slot !== null && chain.slot < slots.length) {
      return chain.slot;
    }
    const stored = molecule?.type === "molecule" ? (molecule.active_slot ?? 0) : 0;
    return Math.min(Math.max(stored, 0), slots.length - 1);
  }

  function activeSlot(): StyleSlot | null {
    const slots = slotsOf(activeMoleculeNode());
    const index = activeSlotIndex();
    return index >= 0 ? (slots[index] ?? null) : null;
  }

  /** The name of the slot a chain draws with, for a list row. */
  function slotName(molecule: GraphNode, chain: GraphNode): string {
    if (chain.type !== "chain") return "";
    const wired = wiredStyle(chain.id);
    if (wired) return "node";
    const slots = slotsOf(molecule);
    if (slots.length === 0) return chain.style ? (nameOf(chain.style) ?? "modified") : "default";
    return (slots[chain.slot ?? 0] ?? slots[0]).name;
  }

  /** The Style node wired to a chain, which overrides its slot. */
  function wiredStyle(chainId: string): GraphNode | null {
    const link = graph.links.find((entry) => entry.to === chainId && entry.socket === "Style");
    const node = link ? nodeById(link.from) : undefined;
    return node?.type === "style" ? node : null;
  }

  /** Writes the molecule's slot list back, then commits. */
  function setSlots(molecule: GraphNode, slots: StyleSlot[], active: number): void {
    if (molecule.type !== "molecule") return;
    molecule.slots = slots;
    molecule.active_slot = Math.min(Math.max(active, 0), Math.max(slots.length - 1, 0));
    commit();
  }

  /** A name no other slot has, so two slots are never both "Style". */
  function freeSlotName(slots: StyleSlot[], base: string): string {
    if (!slots.some((slot) => slot.name === base)) return base;
    for (let index = 2; ; index += 1) {
      const candidate = `${base} ${index}`;
      if (!slots.some((slot) => slot.name === candidate)) return candidate;
    }
  }

  /**
   * The Style tab.
   *
   * Two different things live here, and which one you get depends on what is selected,
   * because they answer different questions:
   *
   * - a Style node is *in the graph*, so editing it changes the picture as you type. That
   *   is what a node is for.
   * - otherwise this is the style library: where styles are written and kept. Editing here
   *   changes nothing on screen until the style is assigned to a chain or a molecule, which
   *   is the point - a workbench where you author a style, look at it, and then apply it,
   *   rather than one where every stray click on a slider repaints the scene.
   */
  function styleTabPanels(): HTMLElement[] {
    const node = propNode();
    if (node?.type === "style") return styleNodePanels(node);

    const chain = activeChainNode();
    const wired = chain ? wiredStyle(chain.id) : null;
    if (wired) return styleNodePanels(wired, chain);
    return libraryPanels();
  }

  /** A Style node's own settings: live, because the node is part of the graph. */
  function styleNodePanels(node: GraphNode, chain?: GraphNode | null): HTMLElement[] {
    if (node.type !== "style") return [];
    const groups = parameterGroups(node).filter((group) => group.key !== "style-library");
    return [
      panel(
        "styles",
        descriptions.get(node.id)?.title ?? "Style",
        [
          el(
            "div",
            { class: "props-note" },
            el("span", {
              text: chain?.type === "chain"
                ? `A node wired to chain ${chain.chain}. Edits here change the view.`
                : "A node in the graph. Edits here change the view.",
            }),
            el("button", {
              class: "btn",
              text: "Select",
              title: "Select this node in the graph editor",
              onclick: () => {
                selected = node.id;
                applySelection();
              },
            }),
          ),
          ...styleOrder(groups).map((group) => panel(group.key, group.label, group.rows, null, true)),
        ],
      ),
    ];
  }

  /**
   * The style library: a list of named styles, the one being edited, and where to put it.
   *
   * Draw, Colour and Surface are sub-panels *of* the style rather than panels beside it -
   * they are what a style is made of. Blender nests the same way with `bl_parent_id`.
   */
  function libraryPanels(): HTMLElement[] {
    const entries = library.styles;
    const current =
      entries.find((entry) => entry.name === styleDraft) ??
      entries.find((entry) => entry.name === draftFallback()) ??
      entries[0];
    if (!current) return [empty("the style library is empty")];
    const editable = !current.builtin;
    const draft = styleEdits.get(current.name) ?? current.style;

    const list = el(
      "div",
      { class: "slot-list" },
      ...entries.map((entry) =>
        el(
          "div",
          { class: `slot-row${entry.name === current.name ? " selected" : ""}` },
          el("span", { class: "style-swatch", style: swatchOf(entry.style) }),
          el("button", {
            class: "slot-open",
            text: entry.name,
            title: entry.builtin
              ? `${entry.name} - shipped with the app, so it is read-only`
              : `${entry.name} - edit it below, then assign it`,
            onclick: () => {
              styleDraft = entry.name;
              drawNodeProperties();
            },
          }),
          entry.builtin ? el("span", { class: "slot-badge", text: "shipped", title: "Read-only" }) : null,
        ),
      ),
    );

    const tools = el(
      "div",
      { class: "slot-buttons" },
      el("button", {
        class: "slot-btn",
        text: "+",
        title: "Register a new style, starting from the default cartoon",
        onclick: () => duplicateLibraryStyle("Cartoon", defaultStyle() as Style),
      }),
      el("button", {
        class: "slot-btn",
        text: "−",
        title: editable ? "Delete this style" : "Shipped styles cannot be deleted",
        disabled: !editable,
        onclick: () => {
          options.onStyleDelete?.(current.name);
          styleEdits.delete(current.name);
          styleDraft = null;
        },
      }),
      el("button", {
        class: "slot-btn",
        text: "⧉",
        title: "Duplicate this style under a new name",
        onclick: () => duplicateLibraryStyle(current.name, draft),
      }),
    );

    // Editing writes to the library, never to the graph: that is what makes this inert.
    const persist = () => {
      const next = styleEdits.get(current.name);
      if (next && editable) options.onStyleSave?.(current.name, structuredClone(next));
      drawNodeProperties();
    };
    const groups = styleGroups(
      draft,
      (next) => {
        // A shipped style is read-only, so no edit of it is held - Assign would otherwise
        // apply the edited copy under the shipped name.
        if (editable) styleEdits.set(current.name, next);
      },
      persist,
    );

    const body: (HTMLElement | null)[] = [
      el("div", { class: "slot-block" }, list, tools),
      el("div", {
        class: "help",
        text: editable
          ? "Editing a style here does not change the view. Assign it below."
          : "Shipped styles are read-only. Duplicate it to make one of your own.",
      }),
      // A fieldset disables every control inside it, which is what read-only means for a
      // shipped style: the values can be read, not quietly changed. It wraps each panel's
      // *rows* rather than the panels themselves - wrapped around the panels it disabled
      // their fold toggles too, so a shipped style could not even be opened and looked at.
      // `disabled` reaches form controls only; the drag fields are not form controls, and
      // without `inert` they went on dragging a shipped style's surface values.
      ...groups.map((group) =>
        panel(
          group.key,
          group.label,
          [
            el(
              "fieldset",
              { class: "props-fieldset", ...(editable ? {} : { disabled: "true", inert: "" }) },
              ...group.rows.filter((row): row is HTMLElement => row !== null),
            ),
          ],
          null,
          true,
        ),
      ),
      assignRows(current.name, draft),
    ];

    return [panel("styles", "Styles", body)];
  }

  /** Which library entry to edit when none has been chosen: what the active chain uses. */
  function draftFallback(): string | null {
    const chain = activeChainNode();
    const molecule = activeMoleculeNode();
    if (chain?.type === "chain" && molecule?.type === "molecule") {
      const slots = slotsOf(molecule);
      const slot = slots[chain.slot ?? 0] ?? slots[0];
      if (slot) return nameOf(slot.style) ?? slot.name;
    }
    return null;
  }

  async function duplicateLibraryStyle(from: string, style: Style): Promise<void> {
    const copy = structuredClone(style);
    const chosen = await askText({
      title: "Name for the new style",
      initial: `${from} copy`,
      accept: "Duplicate",
    });
    if (!chosen) return;
    options.onStyleSave?.(chosen, copy);
    styleDraft = chosen;
  }

  /**
   * Where a library style goes: onto the active chain, or onto every chain of the molecule.
   *
   * This is the only thing on this tab that changes the picture, and it says so. Assigning
   * writes the style into one of the molecule's slots - reusing the slot of the same name
   * if there is one, so assigning the same style twice does not grow the list.
   */
  function assignRows(name: string, style: Style): HTMLElement {
    const molecule = activeMoleculeNode();
    const chain = activeChainNode();
    const inUse = molecule && chain?.type === "chain" ? slotName(molecule, chain) : null;

    return el(
      "div",
      { class: "props-apply" },
      chain?.type === "chain"
        ? el("button", {
            class: "btn primary",
            text: `Assign to chain ${chain.chain}`,
            title: "Draw this chain with this style, from now on",
            onclick: () => assignStyleTo(name, style, "chain"),
          })
        : null,
      molecule
        ? el("button", {
            class: "btn",
            text: `Assign to ${descriptions.get(molecule.id)?.title ?? "the molecule"}`,
            title: "Draw every chain of this molecule with this style",
            onclick: () => assignStyleTo(name, style, "molecule"),
          })
        : null,
      inUse
        ? el("div", {
            class: "help",
            text: `Chain ${chain?.type === "chain" ? chain.chain : ""} currently draws with ${inUse}.`,
          })
        : null,
    );
  }

  function assignStyleTo(name: string, style: Style, scope: "chain" | "molecule"): void {
    const molecule = activeMoleculeNode();
    if (molecule?.type !== "molecule") return;
    const slots = slotsOf(molecule);
    const existing = slots.findIndex((slot) => slot.name === name);
    const index = existing >= 0 ? existing : slots.length;
    const next = [...slots];
    next[index] = { name, style: structuredClone(style) };

    const chains = graph.nodes.filter(
      (node) => node.type === "chain" && moleculeOf(node)?.id === molecule.id,
    );
    const targets =
      scope === "molecule" ? chains : chains.filter((node) => node.id === activeChain);
    for (const node of targets) {
      if (node.type !== "chain") continue;
      node.slot = index;
      // A style the chain carried from before slots existed would hide what the slot list
      // says, so assigning drops it.
      node.style = null;
    }
    setSlots(molecule, next, index);
  }

  /**
   * The slot list and the library row - Blender's `template_list` plus `template_ID`.
   *
   * The two together say what this molecule is drawn with: the list is its styles, and the
   * row underneath names the library style the active slot came from, with the browse
   * dropdown that swaps in another. Swapping one *is* an assignment, so it changes the
   * picture - unlike editing a style in the Style tab, which does not.
   */
  function slotBlock(molecule: GraphNode, slots: StyleSlot[], index: number): HTMLElement {
    const list = el(
      "div",
      { class: "slot-list" },
      ...slots.map((slot, at) =>
        el(
          "div",
          { class: `slot-row${at === index ? " selected" : ""}` },
          el("span", { class: "style-swatch", style: swatchOf(slot.style) }),
          // The name is edited in place, `emboss=False` in Blender's list.
          el("input", {
            class: "slot-name",
            value: slot.name,
            onfocus: () => {
              if (molecule.type === "molecule") molecule.active_slot = at;
            },
            onchange: (event: Event) => {
              const name = (event.target as HTMLInputElement).value.trim();
              const next = slots.map((entry, position) =>
                position === at ? { ...entry, name: name || entry.name } : entry,
              );
              setSlots(molecule, next, at);
            },
            onkeydown: (event: KeyboardEvent) => event.stopPropagation(),
          }),
          el("button", {
            class: "slot-pick",
            text: "",
            title: "Edit this slot",
            onclick: () => {
              if (molecule.type === "molecule") molecule.active_slot = at;
              // Selecting a slot must not silently restyle the chain on screen, so it
              // moves the panel's focus only - assigning is a button of its own.
              commit();
            },
          }),
        ),
      ),
    );

    const buttons = el(
      "div",
      { class: "slot-buttons" },
      el("button", {
        class: "slot-btn",
        text: "+",
        title: "Add a style slot, drawn as a cartoon until you assign something else",
        onclick: () => {
          // The app's default rather than a copy of the active slot: a new slot that
          // silently inherits spacefill because that is what slot 3 happens to be is a
          // new slot you have to undo. Duplicate is the button for a copy.
          const style = defaultStyle() as Style;
          const next = [
            ...slots,
            { name: freeSlotName(slots, "Cartoon"), style: structuredClone(style) },
          ];
          setSlots(molecule, next, next.length - 1);
        },
      }),
      el("button", {
        class: "slot-btn",
        text: "−",
        title: "Remove the active slot",
        disabled: index < 0,
        onclick: () => {
          const next = slots.filter((_, at) => at !== index);
          // Chains name slots by index, so taking one out has to move what pointed past
          // it, or every later chain silently takes its neighbour's style. A chain that
          // was on the removed slot falls back to the first, as it would have anyway.
          for (const node of graph.nodes) {
            if (node.type !== "chain" || node.slot === null) continue;
            if (moleculeOf(node)?.id !== molecule.id) continue;
            if (node.slot === index) node.slot = 0;
            else if (node.slot > index) node.slot -= 1;
          }
          setSlots(molecule, next, Math.max(index - 1, 0));
        },
      }),
      el("button", {
        class: "slot-btn",
        text: "▾",
        title: "Slot menu",
        onclick: (event: MouseEvent) => {
          const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
          openSlotMenu({ x: rect.left, y: rect.bottom + 4 }, molecule, index);
        },
      }),
      slots.length > 1
        ? el("button", {
            class: "slot-btn",
            text: "▴",
            title: "Move the slot up",
            disabled: index <= 0,
            onclick: () => moveSlot(molecule, index, -1),
          })
        : null,
      slots.length > 1
        ? el("button", {
            class: "slot-btn",
            text: "▾",
            title: "Move the slot down",
            disabled: index < 0 || index >= slots.length - 1,
            onclick: () => moveSlot(molecule, index, 1),
          })
        : null,
    );

    const slot = index >= 0 ? slots[index] : null;
    const matched = slot ? nameOf(slot.style) : null;
    const chain = activeChainNode();

    const idRow = el(
      "div",
      { class: "id-row" },
      // The browse dropdown: every style in the library, which is what Blender's
      // `template_ID` opens. One control, so "connect or choose" stays one question.
      el(
        "select",
        {
          class: "id-browse",
          disabled: !slot,
          onchange: (event: Event) => {
            const chosen = (event.target as HTMLSelectElement).value;
            const entry = library.styles.find((candidate) => candidate.name === chosen);
            if (!entry || !slot) return;
            const next = slots.map((existing, at) =>
              at === index
                ? { name: entry.name, style: structuredClone(entry.style) }
                : existing,
            );
            setSlots(molecule, next, index);
          },
        },
        el("option", { value: "", text: matched ?? (slot ? "(modified)" : "-"), selected: true }),
        ...library.styles.map((entry) => el("option", { value: entry.name, text: entry.name })),
      ),
      el("button", {
        class: "id-action",
        text: "⧉",
        title: "Duplicate this slot",
        disabled: !slot,
        onclick: () => {
          if (!slot) return;
          const next = [
            ...slots,
            { name: freeSlotName(slots, slot.name), style: structuredClone(slot.style) },
          ];
          setSlots(molecule, next, next.length - 1);
        },
      }),
      el("button", {
        class: "id-action",
        text: "×",
        title: "Reset this slot to the default style",
        disabled: !slot,
        onclick: () => {
          const next = slots.map((entry, at) =>
            at === index ? { ...entry, style: defaultStyle() as Style } : entry,
          );
          setSlots(molecule, next, index);
        },
      }),
    );

    // Blender's Assign, which is how a slot reaches the geometry. Ours assigns to the
    // active chain, since a chain is the unit a style applies to.
    const assign =
      chain?.type === "chain" && index >= 0
        ? el(
            "div",
            { class: "props-note" },
            el("span", {
              text:
                (chain.slot ?? 0) === index
                  ? `Chain ${chain.chain} draws with this slot`
                  : `Chain ${chain.chain} draws with ${slotName(molecule, chain)}`,
            }),
            el("button", {
              class: "btn",
              text: "Assign",
              disabled: (chain.slot ?? 0) === index && chain.style === null,
              onclick: () => {
                if (chain.type !== "chain") return;
                chain.slot = index;
                // A style the chain carried from before slots existed would win over
                // nothing, but it does hide what the slot list says, so assigning drops
                // it.
                chain.style = null;
                commit();
              },
            }),
          )
        : null;

    return panel("slots", "Style slots", [
      el("div", { class: "slot-block" }, list, buttons),
      idRow,
      assign,
    ]);
  }

  function moveSlot(molecule: GraphNode, index: number, by: number): void {
    const slots = slotsOf(molecule);
    const to = index + by;
    if (index < 0 || to < 0 || to >= slots.length) return;
    const next = [...slots];
    const [moved] = next.splice(index, 1);
    next.splice(to, 0, moved);
    // Chains name slots by index, so moving one has to move what points at it or the
    // molecule silently restyles itself.
    for (const node of graph.nodes) {
      if (node.type !== "chain") continue;
      if (moleculeOf(node)?.id !== molecule.id) continue;
      const current = node.slot ?? 0;
      if (current === index) node.slot = to;
      else if (current === to) node.slot = index;
    }
    setSlots(molecule, next, to);
  }

  /** The slot list's `▾` menu - Blender's `MATERIAL_MT_context_menu`. */
  function openSlotMenu(
    client: { x: number; y: number },
    molecule: GraphNode,
    index: number,
  ): void {
    const slots = slotsOf(molecule);
    const slot = index >= 0 ? slots[index] : null;
    const chains = graph.nodes.filter(
      (node) => node.type === "chain" && moleculeOf(node)?.id === molecule.id,
    );
    menus.open(client, [
      { kind: "title", label: "Styles" },
      {
        kind: "item",
        label: "Register in the library...",
        disabled: !slot,
        onPick: () => {
          if (!slot) return;
          const copy = structuredClone(slot.style);
          void askText({ title: "Name for this style", initial: slot.name, accept: "Register" }).then(
            (chosen) => {
              if (chosen) options.onStyleSave?.(chosen, copy);
            },
          );
        },
      },
      {
        kind: "item",
        label: "Assign to every chain",
        disabled: index < 0,
        onPick: () => {
          for (const chain of chains) {
            if (chain.type !== "chain") continue;
            chain.slot = index;
            chain.style = null;
          }
          commit();
        },
      },
      { kind: "separator" },
      {
        kind: "submenu",
        label: "Delete from the library",
        entries: () =>
          library.styles
            .filter((entry) => !entry.builtin)
            .map((entry) => ({
              kind: "item" as const,
              label: entry.name,
              onPick: () => options.onStyleDelete?.(entry.name),
            })),
      },
      {
        kind: "item",
        label: "Remove unused slots",
        disabled: slots.length < 2,
        onPick: () => {
          const used = new Set(
            chains.map((chain) => (chain.type === "chain" ? (chain.slot ?? 0) : 0)),
          );
          const kept = slots.filter((_, at) => used.has(at));
          if (kept.length === slots.length) return;
          // Indices shift, so every chain is repointed at the slot it was using.
          const moved = new Map<number, number>();
          let next = 0;
          slots.forEach((_, at) => {
            if (used.has(at)) {
              moved.set(at, next);
              next += 1;
            }
          });
          for (const chain of chains) {
            if (chain.type !== "chain") continue;
            chain.slot = moved.get(chain.slot ?? 0) ?? 0;
          }
          setSlots(molecule, kept, 0);
        },
      },
    ]);
  }

  /** The Style rows on the Chain tab: which slot it draws with, and what overrides it. */
  /** A style's parts in the order they are read: what colour, drawn how, with what surface. */
  function styleOrder(groups: PropGroup[]): PropGroup[] {
    const order = ["colour", "draw", "surface"];
    return [...groups].sort((a, b) => order.indexOf(a.key) - order.indexOf(b.key));
  }

  /**
   * The chain's style, as fields to change - how it is drawn and what it is coloured by.
   *
   * A chain draws with one of its molecule's slots, and the panel used to show only which:
   * to change how chain A looked you had to know that, find the slot on the molecule, and
   * work out whether other chains shared it. Here the fields are the chain's. Changing
   * one changes this chain: if other chains share its slot it is given a copy of its own
   * first, and they are left as they were.
   */
  function chainStylePanels(chain: GraphNode): HTMLElement[] {
    if (chain.type !== "chain" || wiredStyle(chain.id)) return [];
    const molecule = moleculeOf(chain);
    if (!molecule || molecule.type !== "molecule" || molecule.slots.length === 0) return [];
    const siblings = () =>
      graph.links
        .filter((link) => link.from === molecule.id && link.socket === "Molecule")
        .map((link) => nodeById(link.to))
        .filter((node): node is Extract<GraphNode, { type: "chain" }> => node?.type === "chain");
    const at = () => Math.min(chain.slot ?? molecule.active_slot, molecule.slots.length - 1);
    const sharedWith = () => siblings().filter((other) => other.id !== chain.id && !other.style && Math.min(other.slot ?? molecule.active_slot, molecule.slots.length - 1) === at()).length;
    const style = chain.style ?? molecule.slots[at()].style;
    const groups = styleGroups(style, (next, live) => {
      if (chain.style || sharedWith() > 0) {
        molecule.slots = [...molecule.slots, { name: freeSlotName(molecule.slots, `Chain ${chain.chain}`), style: next }];
        chain.slot = molecule.slots.length - 1;
        chain.style = null;
      } else {
        molecule.slots = molecule.slots.map((slot, index) => (index === at() ? { ...slot, style: next } : slot));
      }
      if (live) options.onChange(structuredClone(graph));
    });
    const shared = sharedWith();
    const current = molecule.slots[at()];
    /** Gives this chain a slot of its own holding `style`, under `name`. */
    const own = (name: string, made: Style) => {
      molecule.slots = [...molecule.slots, { name: freeSlotName(molecule.slots, name), style: structuredClone(made) }];
      chain.slot = molecule.slots.length - 1;
      chain.style = null;
      commit();
    };
    const head = panel("chain-style-id", "Style", [
      // Which style this chain wears: one of the molecule's, or one from the library.
      el(
        "div",
        { class: "id-row" },
        el(
          "select",
          {
            class: "id-browse",
            title: "The style this chain is drawn with",
            onchange: (event: Event) => {
              const value = (event.target as HTMLSelectElement).value;
              if (value.startsWith("slot:")) {
                chain.slot = Number(value.slice(5));
                chain.style = null;
                commit();
              } else if (value.startsWith("lib:")) {
                const entry = library.styles.find((candidate) => candidate.name === value.slice(4));
                if (entry) own(entry.name, entry.style);
              }
            },
          },
          el(
            "optgroup",
            { label: "This molecule's" },
            ...molecule.slots.map((slot, index) => el("option", { value: `slot:${index}`, text: slot.name, selected: index === at() ? "" : undefined })),
          ),
          el("optgroup", { label: "From the library" }, ...library.styles.map((entry) => el("option", { value: `lib:${entry.name}`, text: entry.name }))),
        ),
        el("button", {
          class: "id-action",
          text: "+",
          title: "New style for this chain, starting from how it looks now",
          onclick: () =>
            void askText({ title: "Name the new style", initial: `Chain ${chain.chain}`, accept: "Create" }).then((name) => {
              if (name) own(name, style);
            }),
        }),
        el("button", {
          class: "id-action",
          text: "✎",
          title: "Rename this style",
          onclick: () =>
            void askText({ title: "Rename the style", initial: current.name, accept: "Rename" }).then((name) => {
              if (!name) return;
              molecule.slots = molecule.slots.map((slot, index) => (index === at() ? { ...slot, name } : slot));
              commit();
            }),
        }),
        el("button", {
          class: "id-action",
          text: "⤓",
          title: "Keep this style in the library, to use on other molecules and in other worlds",
          onclick: () => {
            options.onStyleSave?.(current.name, structuredClone(current.style));
          },
        }),
      ),
      shared > 0
        ? el(
            "div",
            { class: "props-note" },
            el("span", { text: `Also worn by ${count(shared, "other chain")}. Editing below gives this chain its own copy.` }),
            el("button", { class: "btn", text: "Make its own now", onclick: () => own(`Chain ${chain.chain}`, style) }),
          )
        : el("div", { class: "props-note" }, el("span", { text: "This chain's own. Edit it below; + starts another." })),
      // What a style is made of, inside the style - sub-panels of it, the way Blender
      // hangs a panel under its parent (`bl_parent_id`), not three more panels beside it.
      ...styleOrder(groups).map((group) => panel(`chain-${group.key}`, group.label, group.rows, null, true)),
    ]);
    return [head];
  }

  function chainStyleRows(chain: GraphNode): (HTMLElement | null)[] {
    if (chain.type !== "chain") return [];
    const molecule = moleculeOf(chain);
    const slots = slotsOf(molecule);
    const wired = wiredStyle(chain.id);

    if (wired) {
      return [
        el(
          "div",
          { class: "props-note" },
          el("span", { text: "A wired Style node overrides the slot." }),
          el("button", {
            class: "btn",
            text: "Select",
            onclick: () => {
              selected = wired.id;
              applySelection();
            },
          }),
        ),
      ];
    }

    if (slots.length === 0) {
      return [
        el("div", { class: "props-note" }, el("span", { text: "This molecule has no style slots." })),
        molecule
          ? el("button", {
              class: "btn",
              text: "Add one from this chain",
              onclick: () => {
                const style = (chain.type === "chain" && chain.style
                  ? chain.style
                  : defaultStyle()) as Style;
                setSlots(
                  molecule,
                  [{ name: nameOf(style) ?? "Style", style: structuredClone(style) }],
                  0,
                );
              },
            })
          : null,
      ];
    }

    return [
      select(
        "Slot",
        slots.map((slot, at) => [String(at), slot.name] as [string, string]),
        String(Math.min(chain.slot ?? 0, slots.length - 1)),
        (value) => {
          if (chain.type !== "chain") return;
          chain.slot = Number(value);
          chain.style = null;
        },
      ),
      // The override, one click instead of laying out a node by hand: a chain that needs
      // a style nothing else has is what a Style node is for.
      el(
        "div",
        { class: "props-note" },
        el("span", { text: "Or drive it with a node of its own." }),
        el("button", {
          class: "btn",
          text: "Add a Style node",
          onclick: () => assignStyle(chain.id),
        }),
      ),
      chain.style
        ? el(
            "div",
            { class: "props-note" },
            el("span", { text: "This chain also carries a style of its own; the slot wins." }),
            el("button", {
              class: "btn",
              text: "Drop it",
              onclick: () => {
                if (chain.type === "chain") chain.style = null;
                commit();
              },
            }),
          )
        : null,
    ];
  }

  /** Selection is a class, so it can change without touching anything else. */  /** Selection is a class, so it can change without touching anything else. */
  function applySelection(): void {
    for (const [id, entry] of nodeElements) {
      entry.root.classList.toggle("selected", selected === id);
    }
    if (selected !== reported) {
      reported = selected;
      options.onSelect?.(selected);
    }
    drawProperties();
  }

  /**
   * Draws the socket rows.
   *
   * Real rows in the node's layout, not absolutely-positioned strips: the names used to
   * float over the body and land on top of whatever control was behind them. Outputs come
   * first and inputs after, as in Blender, and the dots hang half outside the node's edge
   * so a link visibly ends on the node rather than inside it.
   */
  function renderSockets(node: GraphNode, entry: NodeEntry): void {
    const description = descriptions.get(node.id);
    replace(entry.sockets);
    replace(entry.headerSockets);

    if (node.collapsed) {
      // Folded, the rows are gone and every link converges on one dot a side, level with
      // the header - which is the only part of the node still on screen.
      if (graph.links.some((link) => link.to === node.id)) {
        entry.headerSockets.append(
          el("span", {
            class: "socket bundle in",
            title: "Every input, bundled while the node is folded. Unfold it to rewire.",
          }),
        );
      }
      if (description?.output) {
        entry.headerSockets.append(
          el("span", {
            class: "socket bundle out",
            style: `background: rgb(${description.output.color.join(",")})`,
            title: socketHint(description.output.kind_label, description.output.kind_label, "out"),
            onpointerdown: (event: PointerEvent) => {
              event.stopPropagation();
              startLink(node.id, event);
            },
          }),
        );
      }
      return;
    }

    if (description?.output) {
      entry.sockets.append(
        el(
          "div",
          { class: "socket-row out" },
          el("span", { class: "socket-name", text: description.output.kind_label }),
          el("span", {
            class: "socket out",
            style: `background: rgb(${description.output.color.join(",")})`,
            title: socketHint(description.output.kind_label, description.output.kind_label, "out"),
            onpointerdown: (event: PointerEvent) => {
              event.stopPropagation();
              startLink(node.id, event);
            },
          }),
        ),
      );
    }

    for (const input of description?.inputs ?? []) {
      // A chain's molecule link is internal to its group: the frame around them says it,
      // so the row would be a socket that answers a question nobody asked.
      if (node.type === "chain" && input.name === "Molecule") continue;
      const connected = graph.links.some(
        (link) => link.to === node.id && link.socket === input.name,
      );
      entry.sockets.append(
        el(
          "div",
          { class: "socket-row" },
          el("span", {
            class: `socket in${connected ? " connected" : ""}${input.required && !connected ? " missing" : ""}`,
            dataset: { socket: input.name },
            style: `background: rgb(${input.color.join(",")})`,
            title: socketHint(input.name, input.kind_label, "in"),
            onpointerup: (event: PointerEvent) => {
              event.stopPropagation();
              if (dragging) finishLink(node.id, input.name);
            },
            onpointerdown: (event: PointerEvent) => {
              event.stopPropagation();
              // Pulling from a connected input detaches it, the way every node editor
              // lets you pick a link back up.
              graph.links = graph.links.filter(
                (link) => !(link.to === node.id && link.socket === input.name),
              );
              commit();
            },
          }),
          el("span", { class: "socket-name", text: input.name }),
        ),
      );
    }
  }

  function startNodeDrag(id: string, event: PointerEvent): void {
    if (event.button !== 0) return;
    event.stopPropagation();
    selected = id;
    applySelection();
    const node = nodeById(id);
    if (!node) return;
    const startX = event.clientX;
    const startY = event.clientY;
    // Dragging a group takes its members with it, which is what makes the frame a group
    // rather than a rectangle drawn near some nodes.
    const moving = node.type === "molecule" ? [node, ...membersOf(node)] : [node];
    const origins = moving.map((member) => [...member.position] as [number, number]);
    const move = (moveEvent: PointerEvent) => {
      const dx = (moveEvent.clientX - startX) / zoom;
      const dy = (moveEvent.clientY - startY) / zoom;
      moving.forEach((member, index) => {
        member.position = [origins[index][0] + dx, origins[index][1] + dy];
        const entry = nodeElements.get(member.id);
        if (entry) {
          entry.root.style.left = `${member.position[0]}px`;
          entry.root.style.top = `${member.position[1]}px`;
        }
      });
      drawFrames();
      drawLinks();
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      // Position is part of the graph, so moving a node is a change worth persisting -
      // but not one that needs re-evaluating.
      options.onChange(structuredClone(graph));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  // --- linking ---------------------------------------------------------------

  function startLink(from: string, event: PointerEvent): void {
    const start = surfacePoint(event);
    dragging = { from, x: start.x, y: start.y };
    const move = (moveEvent: PointerEvent) => {
      const point = surfacePoint(moveEvent);
      dragging = { from, x: point.x, y: point.y };
      drawLinks();
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      // A drop that did not land on a socket just cancels; the socket's own handler runs
      // first and clears this.
      if (dragging) {
        dragging = null;
        drawLinks();
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function finishLink(to: string, socket: string): void {
    if (!dragging) return;
    const from = dragging.from;
    dragging = null;

    const produced = descriptions.get(from)?.output;
    const accepted = descriptions.get(to)?.inputs.find((input) => input.name === socket);
    // The core decides what may connect; the editor only refuses what it already knows is
    // wrong, so a bad link never reaches evaluation.
    if (!produced || !accepted || produced.kind !== accepted.kind) {
      replace(
        status,
        el("span", {
          class: "bad",
          text: `${produced?.kind ?? "that"} cannot connect to ${socket} (${accepted?.kind ?? "?"})`,
        }),
      );
      drawLinks();
      return;
    }
    if (from === to) {
      drawLinks();
      return;
    }

    if (!accepted.multiple) {
      graph.links = graph.links.filter((link) => !(link.to === to && link.socket === socket));
    }
    // The same link twice is one link: a light dragged to Lights a second time would
    // otherwise be counted, and lit, twice.
    if (graph.links.some((link) => link.from === from && link.to === to && link.socket === socket)) {
      drawLinks();
      return;
    }
    graph.links.push({ from, to, socket });
    commit();
  }

  function surfacePoint(event: { clientX: number; clientY: number }): { x: number; y: number } {
    const rect = surface.getBoundingClientRect();
    return {
      x: (event.clientX - rect.left - pan.x) / zoom,
      y: (event.clientY - rect.top - pan.y) / zoom,
    };
  }

  function centerOfView(): { x: number; y: number } {
    const rect = surface.getBoundingClientRect();
    return {
      x: (rect.width / 2 - pan.x) / zoom - NODE_WIDTH / 2,
      y: (rect.height / 2 - pan.y) / zoom - 40,
    };
  }

  // --- menus -----------------------------------------------------------------

  function closeMenu(): void {
    menus.close();
  }

  /** One catalogue entry, greyed out when the graph cannot take another. */
  function templateEntry(template: NodeTemplate, dropAt: { x: number; y: number }): MenuEntry {
    // The core accepts one Viewport per graph, so offering a second would only produce a
    // graph that fails the moment it is evaluated. Render nodes have no such limit - two
    // of them is the point, one draft and one final.
    const taken =
      template.kind.type === "viewport" && graph.nodes.some((node) => node.type === "viewport");
    return {
      kind: "item",
      label: template.title,
      hint: taken ? "one only" : undefined,
      disabled: taken,
      onPick: () => addNode(template, dropAt),
    };
  }

  /**
   * The Add panel: a search field, then one submenu per group.
   *
   * Grouped rather than flat because the catalogue grows with every model and every kind
   * of output, and a list long enough to scroll is a list nobody reads. The search field
   * is there for when you already know the name.
   */
  function addEntries(dropAt: { x: number; y: number }, depth: number): MenuEntry[] {
    const matches = (query: string): NodeTemplate[] =>
      catalog.filter(
        (template) =>
          template.title.toLowerCase().includes(query) ||
          template.group.toLowerCase().includes(query),
      );

    const search = el("input", {
      class: "menu-search hint-placeholder",
      placeholder: "Search nodes",
      oninput: (event: Event) => {
        const field = event.target as HTMLInputElement;
        const query = field.value.trim().toLowerCase();
        const rect = field.getBoundingClientRect();
        if (query === "") {
          menus.close();
          openSurfaceMenu(lastMenuAt, dropAt);
          return;
        }
        const found = matches(query);
        // Rebuilt beside the field, which keeps the keyboard: the results panel is a
        // level of its own, so typing never destroys the input being typed into.
        menus.openLevel(
          depth + 1,
          { x: rect.right + 10, y: rect.top - 6 },
          found.length === 0
            ? [{ kind: "title", label: "no matching node" }]
            : found.map((template) => templateEntry(template, dropAt)),
        );
        field.focus();
      },
      onkeydown: (event: KeyboardEvent) => {
        event.stopPropagation();
        if (event.key === "Escape") {
          closeMenu();
          return;
        }
        if (event.key !== "Enter") return;
        const query = (event.target as HTMLInputElement).value.trim().toLowerCase();
        const first = matches(query).find(
          (template) =>
            template.kind.type !== "viewport" ||
            !graph.nodes.some((node) => node.type === "viewport"),
        );
        if (first) {
          closeMenu();
          addNode(first, dropAt);
        }
      },
    });

    const groups = new Map<string, NodeTemplate[]>();
    for (const template of catalog) {
      const entries = groups.get(template.group) ?? [];
      entries.push(template);
      groups.set(template.group, entries);
    }

    const entries: MenuEntry[] = [{ kind: "custom", node: search }];
    for (const [group, templates] of groups) {
      entries.push({
        kind: "submenu",
        label: group,
          entries: () => templates.map((template) => templateEntry(template, dropAt)),
      });
    }
    if (groups.size === 0) entries.push({ kind: "title", label: "loading the catalogue" });
    return entries;
  }

  /** Where the last menu was opened, so a search that empties can reopen in place. */
  let lastMenuAt = { x: 0, y: 0 };

  function openSurfaceMenu(client: { x: number; y: number }, dropAt: { x: number; y: number }): void {
    lastMenuAt = client;
    const anyExpanded = graph.nodes.some((node) => !node.collapsed);
    menus.open(client, [
      // Fetching is how a molecule gets into the graph, so it is the first thing in the
      // menu rather than a text field on a node - and it brings the whole group with it:
      // the molecule, a node per chain, and a style.
      {
        kind: "item",
        label: "Fetch from the PDB...",
        onPick: () => {
          void askText({
            title: "Fetch from the PDB",
            placeholder: "A four-character id, such as 1ubq",
            accept: "Fetch",
            check: (value) =>
              /^[0-9a-zA-Z]{4}$/.test(value) ? null : "A PDB id is four letters and digits.",
          }).then((id) => {
            if (id) options.onFetch?.(id, dropAt);
          });
        },
      },
      {
        kind: "item",
        label: "Open a file...",
        onPick: () => void options.onOpenFile?.(dropAt),
      },
      { kind: "separator" },
      {
        kind: "submenu",
        label: "Add node",
        entries: (depth) => addEntries(dropAt, depth),
      },
      { kind: "separator" },
      { kind: "item", label: "Arrange", hint: "L", onPick: () => arrange() },
      { kind: "item", label: "Fit view", hint: "Home", onPick: () => fitView() },
      {
        kind: "item",
        label: "Reset zoom",
        onPick: () => {
          zoom = 1;
          draw();
        },
      },
      { kind: "separator" },
      {
        kind: "item",
        label: anyExpanded ? "Collapse all" : "Expand all",
        hint: "Shift H",
        onPick: () => setAllCollapsed(anyExpanded),
      },
      { kind: "separator" },
      {
        kind: "item",
        label: "Reset graph",
        disabled: options.onReset === undefined,
        onPick: () => options.onReset?.(),
      },
    ]);
  }

  function openNodeMenu(client: { x: number; y: number }, id: string): void {
    const node = nodeById(id);
    if (!node) return;
    selected = id;
    applySelection();
    // The Viewport is what the graph is for: it can be moved and folded but not removed,
    // and a second one would leave the core with two outputs to choose between.
    const isViewport = node.type === "viewport";
    const connected = graph.links.some((link) => link.from === id || link.to === id);
    menus.open(client, [
      { kind: "title", label: descriptions.get(id)?.title ?? node.type },
      {
        kind: "item",
        label: node.collapsed ? "Expand" : "Collapse",
        hint: "H",
        onPick: () => toggleCollapse(id),
      },
      { kind: "item", label: "Duplicate", disabled: isViewport, onPick: () => duplicate(id) },
      { kind: "item", label: "Disconnect", disabled: !connected, onPick: () => disconnect(id) },
      { kind: "separator" },
      { kind: "item", label: "Frame", onPick: () => frameNode(id) },
      {
        kind: "item",
        label: "Delete",
        hint: "Del",
        disabled: isViewport,
        onPick: () => removeNode(id),
      },
    ]);
  }

  // --- editing the graph -----------------------------------------------------

  function addNode(template: NodeTemplate, at: { x: number; y: number }): void {
    const id = `${template.type_name}-${Date.now().toString(36)}`;
    const node = {
      ...structuredClone(template.kind),
      id,
      position: [at.x, at.y] as [number, number],
    } as GraphNode;
    if (node.type === "light") {
      // The light carries its own id inside the style; two lights sharing one would be
      // treated as the same light by everything downstream.
      node.light = { ...node.light, id, name: node.light.name || "Light" };
    }
    graph.nodes.push(node);
    selected = id;
    commit();
  }

  function duplicate(id: string): void {
    const source = nodeById(id);
    if (!source) return;
    const copy = structuredClone(source);
    const nextId = `${source.type}-${Date.now().toString(36)}`;
    copy.id = nextId;
    copy.position = [source.position[0] + 28, source.position[1] + 28];
    if (copy.type === "light") {
      copy.light = { ...copy.light, id: nextId, name: `${copy.light.name} copy` };
    }
    if (copy.type === "predict") {
      // A copy has not been run; carrying the job over would show the original's result.
      copy.job_id = null;
    }
    graph.nodes.push(copy);
    // Incoming links come along, which is what makes duplicating a Draw node useful: the
    // copy is already fed by the same structure and only its settings need changing.
    for (const link of graph.links.filter((candidate) => candidate.to === id)) {
      graph.links.push({ ...link, to: nextId });
    }
    selected = nextId;
    commit();
  }

  function disconnect(id: string): void {
    graph.links = graph.links.filter((link) => link.from !== id && link.to !== id);
    commit();
  }

  function removeNode(id: string): void {
    const node = nodeById(id);
    if (!node || node.type === "viewport") return;
    // A molecule takes its chains with it. Left behind they have nothing to be chains
    // of, the core refuses the whole plan over them, and their Molecule socket is hidden,
    // so they could not even be wired to something else.
    const gone = new Set<string>([id]);
    if (node.type === "molecule") {
      for (const member of membersOf(node)) {
        if (member.type === "chain") gone.add(member.id);
      }
    }
    graph.nodes = graph.nodes.filter((candidate) => !gone.has(candidate.id));
    graph.links = graph.links.filter((link) => !gone.has(link.from) && !gone.has(link.to));
    if (selected !== null && gone.has(selected)) selected = null;
    commit();
  }

  function removeSelected(): void {
    if (selected) removeNode(selected);
  }

  /**
   * Folds a node to its header.
   *
   * Once every step is a node, a working graph has more nodes than fit on a screen, and
   * most of them are set once and never touched. Folding is how the two or three being
   * worked on stay legible. Stored on the node, so it survives with the graph.
   */
  function toggleCollapse(id: string): void {
    const node = nodeById(id);
    if (!node) return;
    node.collapsed = !node.collapsed;
    commit();
  }

  function setAllCollapsed(collapsed: boolean): void {
    for (const node of graph.nodes) node.collapsed = collapsed;
    commit();
  }

  /**
   * Scrolls a node into view if it is off screen, and leaves the plane alone if it is not.
   *
   * Selecting a chain in the viewport should not yank the graph around, but a selection
   * highlighted somewhere off screen is a selection nobody can see.
   */
  function frameIntoView(id: string): void {
    const entry = nodeElements.get(id);
    if (!entry) return;
    const pane = surface.getBoundingClientRect();
    const node = entry.root.getBoundingClientRect();
    const outside =
      node.right < pane.left + 8 ||
      node.left > pane.right - 8 ||
      node.bottom < pane.top + 8 ||
      node.top > pane.bottom - 8;
    if (outside) frameNode(id);
  }

  function frameNode(id: string): void {
    const node = nodeById(id);
    if (!node) return;
    const rect = surface.getBoundingClientRect();
    zoom = 1;
    pan = {
      x: rect.width / 2 - (node.position[0] + NODE_WIDTH / 2),
      y: rect.height / 2 - (node.position[1] + nodeHeight(id) / 2),
    };
    draw();
  }

  function fitView(): void {
    // Only what is on screen. Folded members keep their positions, and framing those
    // frames a rectangle of empty plane around whatever is actually visible.
    const hidden = hiddenNodes();
    const visible = graph.nodes.filter((node) => !hidden.has(node.id));
    if (visible.length === 0) return;
    const left = Math.min(...visible.map((node) => node.position[0]));
    const top = Math.min(...visible.map((node) => node.position[1]));
    const right = Math.max(...visible.map((node) => node.position[0] + NODE_WIDTH));
    const bottom = Math.max(
      ...visible.map((node) => node.position[1] + nodeHeight(node.id)),
    );
    const rect = surface.getBoundingClientRect();
    const margin = 48;
    zoom = Math.min(
      1,
      Math.max(
        0.3,
        Math.min(
          (rect.width - margin * 2) / Math.max(1, right - left),
          (rect.height - margin * 2) / Math.max(1, bottom - top),
        ),
      ),
    );
    // Centred when it fits. When it does not - the zoom has a floor, and a short pane
    // cannot hold a tall graph at it - centring puts the top of the graph above the pane
    // and the bottom below it; starting from the top-left corner shows where it begins.
    const along = (size: number, low: number, high: number) =>
      (high - low) * zoom + margin * 2 <= size
        ? (size - (high - low) * zoom) / 2 - low * zoom
        : margin / 2 - low * zoom;
    pan = { x: along(rect.width, left, right), y: along(rect.height, top, bottom) };
    draw();
  }

  /**
   * Lays the graph out left to right and fits it on screen. The arithmetic lives in
   * ./layout, which knows nothing about the DOM; this half measures the nodes and writes
   * the result back.
   */
  function arrange(): void {
    if (graph.nodes.length === 0) return;
    // The groups go in as units, or their members scatter into whichever column their
    // links put them in and the frame drawn around them swallows the plane.
    // Members go in whether or not their group is folded. A folded group is condensed to
    // one node in the layout either way, so including its hidden members costs nothing on
    // the plane - and means unfolding it later shows a tidy interior rather than the
    // positions the chains happened to be created at.
    const groups: LayoutGroup[] = graph.nodes
      .filter((node) => node.type === "molecule")
      .map((node) => ({
        id: node.id,
        members: membersOf(node).map((member) => member.id),
      }))
      .filter((group) => group.members.length > 0);

    // A folded member's links are re-pointed at the group, or the group ends up with no
    // links at all in the condensed graph - which ranks it as a sink and puts it to the
    // *right* of the viewport it feeds.
    const links = graph.links
      .map((link) => ({ from: visibleAnchor(link.from), to: visibleAnchor(link.to) }))
      .filter((link) => link.from !== link.to);

    const positions = layoutGraph(
      graph.nodes.map((node) => ({
        id: node.id,
        y: node.position[1],
        height: nodeHeight(node.id),
      })),
      links,
      { width: NODE_WIDTH, gapX: 40, gapY: 14, groups, inset: 14, headerHeight: 34 },
    );
    for (const node of graph.nodes) {
      const placed = positions.get(node.id);
      if (placed) node.position = [placed[0], placed[1]];
    }
    commit();
    fitView();
  }

  // --- node parameters -------------------------------------------------------

  /**
   * A labelled row.
   *
   * The tooltip comes from the label, so a field is explained by existing rather than by
   * being registered twice - see `hints.ts`. It goes on the row rather than on the label
   * so that hovering the control shows it too.
   */
  function field(label: string, control: Node): HTMLElement {
    const hint = fieldHint(label);
    return el(
      "div",
      { class: "node-field", ...(hint ? { title: `${label} - ${hint}` } : {}) },
      el("label", { text: label }),
      control,
    );
  }

  /**
   * A Blender-style number field: drag across it to change the value, click it to type
   * one. Held shift slows the drag to a tenth for fine work, and arrow keys step it.
   *
   * Not an `<input type="range">`, which cannot be typed into, has no fine mode, and gives
   * a thumb far smaller than the row it sits in.
   */
  function dragField(
    label: string,
    spec: {
      value: number;
      min: number;
      max: number;
      step: number;
      onChange: (value: number) => void;
      /** Where the finished value is written; the graph, unless told otherwise. */
      persist?: () => void;
    },
  ): HTMLElement {
    const persist = spec.persist ?? commit;
    const decimals = Math.max(0, Math.ceil(-Math.log10(spec.step)));
    const span = spec.max - spec.min;
    const fill = el("div", { class: "drag-fill" });
    const readout = el("span", { class: "drag-value" });
    const root = el(
      "div",
      {
        class: "drag-field",
        tabindex: "0",
        title: `${label} - ${fieldHint(label) ?? ""}${fieldHint(label) ? "\n" : ""}Drag to change, click to type, arrows to step.`,
      },
      fill,
      el("span", { class: "drag-label", text: label }),
      readout,
    );
    let value = spec.value;

    // Dragging lands on multiples of the step; a typed number is taken as given, only
    // clamped and rounded to the field's precision. Snapping a typed 360 up to 368 because
    // the drag step happens to be 16 is the field overruling the person using it.
    const snap = (next: number, toStep: boolean): number => {
      const clamped = Math.min(spec.max, Math.max(spec.min, next));
      if (!toStep) return Number(clamped.toFixed(decimals));
      return Number((Math.round(clamped / spec.step) * spec.step).toFixed(6));
    };
    const show = () => {
      readout.textContent = value.toFixed(decimals);
      fill.style.width = `${((value - spec.min) / span) * 100}%`;
    };
    const set = (next: number, toStep = true) => {
      const snapped = snap(next, toStep);
      if (snapped === value) return;
      value = snapped;
      show();
      spec.onChange(value);
    };
    show();

    const startTyping = () => {
      if (root.querySelector(".drag-input")) return;
      const input = el("input", {
        class: "drag-input",
        type: "text",
        value: value.toFixed(decimals),
      });
      readout.hidden = true;
      root.classList.add("editing");
      root.appendChild(input);
      input.focus();
      input.select();
      const finish = (accept: boolean) => {
        if (!input.isConnected) return;
        // An emptied field is no number, not zero: `Number("")` is 0, which would quietly
        // set the field to its minimum.
        const typed = input.value.trim() === "" ? Number.NaN : Number(input.value);
        input.remove();
        readout.hidden = false;
        root.classList.remove("editing");
        if (accept && Number.isFinite(typed)) {
          set(typed, false);
          persist();
        }
      };
      input.addEventListener("keydown", (event) => {
        event.stopPropagation();
        if (event.key === "Enter") finish(true);
        else if (event.key === "Escape") finish(false);
      });
      input.addEventListener("blur", () => finish(true));
    };

    root.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      if (root.classList.contains("editing")) return;
      // Not stopPropagation: the node still wants to know it was clicked. preventDefault
      // keeps the drag from turning into a text selection.
      event.preventDefault();
      root.focus();
      const startX = event.clientX;
      const startValue = value;
      let travelled = 0;
      interacting += 1;
      root.classList.add("dragging");
      try {
        root.setPointerCapture(event.pointerId);
      } catch {
        // Capture is a nicety; the window listeners below carry the drag without it.
      }
      const move = (moveEvent: PointerEvent) => {
        const dx = moveEvent.clientX - startX;
        travelled = Math.max(travelled, Math.abs(dx));
        // A full sweep of the range is about 220 px of travel, at any zoom: the field is
        // read in screen pixels because that is what the hand moves in.
        const perPixel = span / 220;
        set(startValue + dx * perPixel * (moveEvent.shiftKey ? 0.1 : 1));
      };
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        window.removeEventListener("pointercancel", up);
        root.classList.remove("dragging");
        interacting -= 1;
        if (travelled < 3) startTyping();
        else persist();
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      window.addEventListener("pointercancel", up);
    });

    root.addEventListener("keydown", (event) => {
      const stride = spec.step * (event.shiftKey ? 10 : 1);
      if (event.key === "ArrowLeft" || event.key === "ArrowDown") set(value - stride);
      else if (event.key === "ArrowRight" || event.key === "ArrowUp") set(value + stride);
      else if (event.key === "Enter") {
        startTyping();
        event.preventDefault();
        return;
      } else return;
      event.preventDefault();
      event.stopPropagation();
      persist();
    });

    return root;
  }

  /**
   * A tick box.
   *
   * `persist` is where the change is written when the widget is done with it, and it
   * defaults to committing the graph. The style library editor passes its own, because
   * editing a style there must *not* touch the graph: nothing on screen changes until the
   * style is assigned to something.
   */
  function checkbox(
    label: string,
    value: boolean,
    onChange: (value: boolean) => void,
    persist: () => void = commit,
  ): HTMLElement {
    const hint = fieldHint(label);
    return el(
      "div",
      { class: "node-field inline", ...(hint ? { title: `${label} - ${hint}` } : {}) },
      el("input", {
        type: "checkbox",
        checked: value,
        onchange: (event: Event) => {
          onChange((event.target as HTMLInputElement).checked);
          persist();
        },
      }),
      el("label", { text: label }),
    );
  }

  function select(
    label: string,
    entries: [string, string][],
    value: string,
    onChange: (value: string) => void,
    persist: () => void = commit,
  ): HTMLElement {
    return field(
      label,
      el(
        "select",
        {
          onchange: (event: Event) => {
            onChange((event.target as HTMLSelectElement).value);
            persist();
          },
        },
        ...entries.map(([key, text]) =>
          el("option", { value: key, text, selected: key === value }),
        ),
      ),
    );
  }

  /**
   * One panel's worth of fields.
   *
   * The node body shows them all in a row and the properties panel puts each in its own
   * collapsible panel - the same fields either way, which is the point. Blender does this
   * with `template_node_view`: the Surface panel *is* the node, drawn in a panel.
   */
  interface PropGroup {
    key: string;
    label: string;
    rows: (HTMLElement | null)[];
  }

  function parameterGroups(node: GraphNode): PropGroup[] {
    switch (node.type) {
      case "style":
        return [
          { key: "style-library", label: "Style", rows: [stylePicker(node)] },
          ...styleGroups(node.style, (next, live) => {
            node.style = next;
            if (live) options.onChange(structuredClone(graph));
          }),
        ];
      case "render":
        return [{ key: "render-settings", label: "Render", rows: parameters(node) }];
      case "world":
        return [{ key: "world-data", label: "World", rows: parameters(node) }];
      case "image":
        return [{ key: "image-data", label: "Image", rows: parameters(node) }];
      default:
        return [
          {
            key: `${node.type}-data`,
            label: descriptions.get(node.id)?.title ?? node.type,
            rows: parameters(node),
          },
        ];
    }
  }

  /** The library row on a Style node: which registered style it is, or "(modified)". */
  function stylePicker(node: GraphNode & { type: "style" }): HTMLElement {
    const match = library.styles.find(
      (entry) => JSON.stringify(entry.style) === JSON.stringify(node.style),
    );
    return field(
      "Style",
      el(
        "select",
        {
          onchange: (event: Event) => {
            const chosen = (event.target as HTMLSelectElement).value;
            const entry = library.styles.find((candidate) => candidate.name === chosen);
            if (entry) node.style = structuredClone(entry.style);
            commit();
          },
        },
        el("option", { value: "", text: match ? match.name : "(modified)", selected: true }),
        ...library.styles.map((entry) => el("option", { value: entry.name, text: entry.name })),
      ),
    );
  }

  /**
   * A style's own fields, for whatever holds the style - a Style node or a molecule's slot.
   *
   * `apply` writes the new value; the widgets commit for themselves, and `live` is the
   * drag in progress that wants the viewport to keep up. Written once because a style
   * edited in the panel and the same style edited on the node have to be the same
   * controls, or one of them is the one you are not looking at.
   */
  function styleGroups(
    initial: Style,
    apply: (next: Style, live?: boolean) => void,
    persist: () => void = commit,
  ): PropGroup[] {
    // Kept current as it is edited. The widgets outlive the edit that made them - a body
    // is not rebuilt while the pointer or the focus is inside it - so one that wrote
    // "what I was built with, plus my change" would put back everything edited since:
    // drag roughness, then specular, and roughness returned to where it started.
    let style = initial;
    const set = (next: Partial<Style>, live?: boolean) => {
      style = { ...style, ...next };
      apply(style, live);
    };
    const surface = (label: string, key: "roughness" | "specular" | "metallic") =>
      dragField(label, {
        value: style.material[key],
        min: 0,
        max: 1,
        step: 0.01,
        onChange: (value) => set({ material: { ...style.material, [key]: value } }, true),
        persist,
      });

    return [
      {
        // Several at once, not one of a list: a cartoon *and* its side chains as sticks is
        // one style. The first one ticked decides the scene-wide settings.
        key: "draw",
        label: "Draw",
        rows: [
          ...(Object.entries(REPRESENTATION_LABELS) as [Representation, string][]).map(
            ([key, label]) =>
              checkbox(
                label,
                style.representations.includes(key),
                (value) => {
                  const next = style.representations.filter((entry) => entry !== key);
                  if (value) next.push(key);
                  set({ representations: next });
                },
                persist,
              ),
          ),
        ],
      },
      {
        key: "colour",
        label: "Colour",
        rows: [
          select(
            "Colour by",
            Object.entries(COLOR_SCHEME_LABELS),
            style.scheme,
            (value) => set({ scheme: value as Style["scheme"], solid: null }),
            persist,
          ),
          select(
            "Palette",
            [
              ["okabe-ito", "Okabe-Ito (safe)"],
              ["tol-bright", "Tol Bright (safe)"],
              ["tol-muted", "Tol Muted (safe)"],
              ["tableau10", "Tableau 10"],
              ["set2", "ColorBrewer Set2"],
              ["vivid", "Vivid"],
            ],
            style.categorical,
            (value) => set({ categorical: value as Style["categorical"] }),
            persist,
          ),
          checkbox(
            "Continuous scale",
            style.continuous,
            (value) => set({ continuous: value }),
            persist,
          ),
          // The palette a ramp runs through. Only where there is a ramp to run: on a
          // scheme that hands out one colour per chain it would be a control that does
          // nothing, which is worse than an absent one.
          RAMP_SCHEMES.includes(style.scheme) || (style.scheme === "confidence" && style.continuous)
            ? select(
                "Ramp",
                Object.entries(RAMP_LABELS),
                style.ramp,
                (value) => set({ ramp: value as Ramp }),
                persist,
              )
            : null,
          // A flat colour is not a scheme, so it is a switch and then a colour - the way
          // PyMOL's `orange` is one colour and `spectrum` is a rule.
          checkbox(
            "Flat colour",
            style.solid !== null,
            (value) => set({ solid: value ? [0.85, 0.45, 0.15] : null }),
            persist,
          ),
          style.solid !== null
            ? colorField("Colour", {
                value: unitToHex(style.solid),
                onChange: (hex) => {
                  set({ solid: hexToUnit(hex) });
                  persist();
                },
                sampleViewport: options.sampleViewport,
                setPickingCursor: options.setPickingCursor,
              })
            : null,
          checkbox(
            "Heteroatoms by element",
            style.heteroatoms_by_element,
            (value) => set({ heteroatoms_by_element: value }),
            persist,
          ),
        ],
      },
      {
        key: "surface",
        label: "Surface",
        rows: [surface("Roughness", "roughness"), surface("Specular", "specular"), surface("Metallic", "metallic")],
      },
    ];
  }

  function parameters(node: GraphNode): HTMLElement[] {
    switch (node.type) {
      case "molecule": {
        // No id field and no file button: a molecule enters the graph by being fetched or
        // opened, from the plane's menu, and that is what builds its chain nodes. A text
        // box here could only ever change the molecule out from under them.
        return [
          checkbox("Hydrogens", node.options.show_hydrogens, (value) => {
            node.options = { ...node.options, show_hydrogens: value };
          }),
          checkbox("Waters", node.options.show_waters, (value) => {
            node.options = { ...node.options, show_waters: value };
          }),
          checkbox("Ligands", node.options.show_ligands, (value) => {
            node.options = { ...node.options, show_ligands: value };
          }),
        ];
      }

      case "sequence-input":
        return [
          field(
            "Sequence",
            el("textarea", {
              rows: "3",
              class: "hint-placeholder",
              placeholder: "MKTAYIAKQRQ... or FASTA",
              value: node.fasta,
              oninput: (event: Event) => {
                node.fasta = (event.target as HTMLTextAreaElement).value;
              },
              onkeydown: (event: KeyboardEvent) => event.stopPropagation(),
              onchange: () => commit(),
            }),
          ),
        ];

      case "predict": {
        const eligible = models.filter((model) => model.eligible_providers.length > 0);
        return [
          select(
            "Model",
            eligible.map((model) => [model.id, model.name] as [string, string]),
            node.model_id,
            (value) => {
              node.model_id = value;
              // A different model means a different job; keeping the old one would show
              // the previous model's result under the new name.
              node.job_id = null;
            },
          ),
          select(
            "Run on",
            providers
              .filter((provider) => provider.enabled)
              .map((provider) => [provider.id, provider.name] as [string, string]),
            node.provider_id,
            (value) => {
              node.provider_id = value;
              node.job_id = null;
            },
          ),
          node.job_id
            ? el("div", { class: "help", text: `job ${node.job_id.slice(0, 8)}` })
            : null,
          node.job_id
            ? el("button", {
                class: "btn",
                text: "Run again",
                onclick: () => {
                  node.job_id = null;
                  commit();
                },
              })
            : null,
        ].filter((entry): entry is HTMLElement => entry !== null);
      }

      case "style":
        // The same fields the panel shows, in a row: `parameterGroups` is the one
        // definition of what a style has.
        return parameterGroups(node)
          .flatMap((group) => group.rows)
          .filter((row): row is HTMLElement => row !== null);

      case "chain": {
        // One control, not two. Wired, the socket *is* the style and the picker would be
        // a second answer to the same question; unwired, the picker is it. Connect or
        // choose, which is how a Blender material slot behaves.
        const wired = graph.links.some(
          (link) => link.to === node.id && link.socket === "Style",
        );
        if (wired) return [];
        const own = node.style ? nameOf(node.style) : null;
        return [
          field(
            "Style",
            el(
              "select",
              {
                onchange: (event: Event) => {
                  const chosen = (event.target as HTMLSelectElement).value;
                  const entry = library.styles.find((candidate) => candidate.name === chosen);
                  node.style = entry ? structuredClone(entry.style) : null;
                  commit();
                },
              },
              el("option", { value: "", text: own ?? "default", selected: true }),
              ...library.styles.map((entry) =>
                el("option", { value: entry.name, text: entry.name }),
              ),
            ),
          ),
        ];
      }

      case "colouring":
        return [
          select(
            "Colour by",
            Object.entries(COLOR_SCHEME_LABELS),
            node.scheme,
            (value) => (node.scheme = value as typeof node.scheme),
          ),
          select(
            "Palette",
            [
              ["okabe-ito", "Okabe-Ito (safe)"],
              ["tol-bright", "Tol Bright (safe)"],
              ["tol-muted", "Tol Muted (safe)"],
              ["tableau10", "Tableau 10"],
              ["set2", "ColorBrewer Set2"],
              ["vivid", "Vivid"],
            ],
            node.categorical,
            (value) => (node.categorical = value as typeof node.categorical),
          ),
          checkbox("Continuous scale", node.continuous, (value) => {
            node.continuous = value;
          }),
        ];

      case "solid-colour":
        return [
          // Named colours first, because "orange" is what people say. The picker is there
          // for the colour that has no name.
          field(
            "Named",
            el(
              "select",
              {
                onchange: (event: Event) => {
                  const chosen = (event.target as HTMLSelectElement).value;
                  const colour = library.colours.find((entry) => entry.name === chosen);
                  if (colour) node.color = colour.linear;
                  commit();
                },
              },
              el("option", { value: "", text: "-", selected: true }),
              ...library.colours.map((colour) =>
                el("option", { value: colour.name, text: colour.name }),
              ),
            ),
          ),
          colorField("Colour", {
            value: unitToHex(node.color),
            onChange: (hex) => {
              node.color = hexToUnit(hex);
              commit();
            },
            sampleViewport: options.sampleViewport,
            setPickingCursor: options.setPickingCursor,
          }),
        ];

      case "light": {
        const light = node.light;
        return [
          select("Type", LIGHT_KINDS, light.kind, (value) => {
            node.light = { ...node.light, kind: value as typeof light.kind };
          }),
          colorField("Colour", {
            value: unitToHex(light.color),
            onChange: (hex) => {
              node.light = { ...node.light, color: hexToUnit(hex) };
              commit();
            },
            sampleViewport: options.sampleViewport,
            setPickingCursor: options.setPickingCursor,
          }),
          dragField("Energy", {
            value: light.energy,
            min: 0,
            max: 10,
            step: 0.1,
            onChange: (value) => {
              node.light = { ...node.light, energy: value };
              options.onChange(structuredClone(graph));
            },
          }),
          dragField("Softness", {
            value: light.softness,
            min: 0.5,
            max: 45,
            step: 0.5,
            onChange: (value) => {
              node.light = { ...node.light, softness: value };
              options.onChange(structuredClone(graph));
            },
          }),
          // The difference between lighting an object you turn over in your hands and
          // lighting a scene you walk around in.
          checkbox("Follow the view", light.follow_view, (value) => {
            node.light = { ...node.light, follow_view: value };
          }),
        ];
      }

      case "world":
        return [
          colorField("Background", {
            value: srgbToHex(node.background),
            onChange: (hex) => {
              node.background = hexToSrgb(hex);
              commit();
            },
            sampleViewport: options.sampleViewport,
            setPickingCursor: options.setPickingCursor,
          }),
          dragField("Ambient", {
            value: node.ambient,
            min: 0,
            max: 2,
            step: 0.05,
            onChange: (value) => {
              node.ambient = value;
              options.onChange(structuredClone(graph));
            },
          }),
          checkbox("Occlusion", node.ambient_occlusion, (value) => {
            node.ambient_occlusion = value;
          }),
          checkbox("Outline", node.outline, (value) => (node.outline = value)),
        ];

      case "material":
        return (
          [
            ["Roughness", "roughness"],
            ["Specular", "specular"],
            ["Metallic", "metallic"],
          ] as [string, "roughness" | "specular" | "metallic"][]
        ).map(([label, key]) =>
          dragField(label, {
            value: node.material[key],
            min: 0,
            max: 1,
            step: 0.01,
            onChange: (value) => {
              node.material = { ...node.material, [key]: value };
              // Live, so the viewport tracks the drag; commit lands when it ends.
              options.onChange(structuredClone(graph));
            },
          }),
        );

      case "render": {
        const progress = renderContext.progress[node.id];
        const state = states[node.id];
        const unavailable = renderContext.unavailable;
        const ready =
          (state?.state === "ready" || state?.state === "warned") &&
          !progress?.rendering &&
          !unavailable;
        const viewAspect = options.viewportAspect?.() ?? null;
        const ownAspect = node.width / Math.max(node.height, 1);
        // Only worth saying when it actually changes the framing: a few percent of
        // difference is invisible, a factor of two is the whole complaint.
        const aspectMismatch =
          viewAspect && Math.abs(Math.log(ownAspect / viewAspect)) > 0.1 ? viewAspect : null;
        const integer = (
          label: string,
          value: number,
          min: number,
          max: number,
          step: number,
          apply: (value: number) => void,
        ) => dragField(label, { value, min, max, step, onChange: apply });

        return [
          select(
            "Engine",
            Object.entries(RENDER_ENGINE_LABELS),
            node.engine,
            (value) => (node.engine = value as typeof node.engine),
          ),
          integer("Samples", node.samples, 4, 1024, 4, (value) => (node.samples = value)),
          integer("Width", node.width, 128, 4096, 16, (value) => (node.width = value)),
          integer("Height", node.height, 128, 4096, 16, (value) => (node.height = value)),
          checkbox("Transparent", node.transparent, (value) => (node.transparent = value)),
          // A render never crops what the viewport showed - see `RenderSettings::
          // camera_fov_y` - which means a render shaped differently from the window shows
          // *more*, and the subject comes out smaller. Matching the shape is the way to
          // get exactly the framing on screen, so it is one button.
          aspectMismatch !== null
            ? el(
                "div",
                { class: "node-row" },
                el("button", {
                  class: "btn",
                  text: "Match viewport",
                  title: "Set the height so this render is framed exactly like the viewport",
                  onclick: () => {
                    const aspect = options.viewportAspect?.();
                    if (!aspect) return;
                    node.height = Math.min(4096, Math.max(128, Math.round(node.width / aspect)));
                    commit();
                  },
                }),
              )
            : null,

          el("button", {
            class: "btn primary",
            text: progress?.rendering ? "Rendering..." : "Render",
            disabled: !ready,
            title: unavailable ?? "",
            onclick: () => options.onRender?.(node.id),
          }),

          progress?.error ? el("div", { class: "node-error", text: progress.error }) : null,
          // The picture itself goes to the Image node this one feeds, so the result is a
          // thing in the graph that can be copied and saved rather than a preview stuck
          // to the settings that made it.
          progress?.seconds
            ? el("div", { class: "help", text: `${progress.seconds.toFixed(1)}s` })
            : null,
        ].filter((entry): entry is HTMLElement => entry !== null);
      }

      case "image": {
        if (!node.path) return [];
        const path = node.path;
        const rows: (HTMLElement | null)[] = [
          options.imageSource
            ? el(
                "div",
                { class: "node-thumb" },
                el("img", {
                  src: `${options.imageSource(path)}?v=${encodeURIComponent(path)}`,
                  alt: "render",
                  title: path,
                  onclick: () => options.onReveal?.(path),
                }),
              )
            : null,
          el(
            "div",
            { class: "node-row" },
            el("button", {
              class: "btn",
              text: "Copy",
              onclick: () => options.onImageCopy?.(path),
            }),
            el("button", {
              class: "btn",
              text: "Save as...",
              onclick: () => options.onImageSave?.(path),
            }),
          ),
        ];
        return rows.filter((entry): entry is HTMLElement => entry !== null);
      }

      default:
        return [];
    }
  }

  // --- surface interaction ---------------------------------------------------

  surface.addEventListener("pointermove", (event) => {
    pointerAt = { x: event.clientX, y: event.clientY };
  });

  surface.addEventListener("pointerdown", (event) => {
    closeMenu();
    if (event.target !== surface && event.target !== world && event.target !== links) return;
    if (event.button !== 0) return;
    selected = null;
    applySelection();
    const startX = event.clientX;
    const startY = event.clientY;
    const origin = { ...pan };
    const move = (moveEvent: PointerEvent) => {
      pan = {
        x: origin.x + (moveEvent.clientX - startX),
        y: origin.y + (moveEvent.clientY - startY),
      };
      world.style.transform = `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`;
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });

  // Double-clicking the plane frames the graph, the same gesture the viewport uses.
  surface.addEventListener("dblclick", (event) => {
    if (event.target !== surface && event.target !== world && event.target !== links) return;
    fitView();
  });

  surface.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    // A right-click on a node is the node's own menu; the node handler catches those, so
    // anything arriving here landed on the plane.
    openSurfaceMenu({ x: event.clientX, y: event.clientY }, surfacePoint(event));
  });

  surface.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      closeMenu();
      const rect = surface.getBoundingClientRect();
      const x = event.clientX - rect.left;
      const y = event.clientY - rect.top;
      const factor = Math.exp(-event.deltaY * 0.0015);
      const next = Math.min(Math.max(zoom * factor, 0.3), 2.5);
      // Zoom about the cursor, so the node under the pointer stays put.
      pan = {
        x: x - ((x - pan.x) * next) / zoom,
        y: y - ((y - pan.y) * next) / zoom,
      };
      zoom = next;
      draw();
    },
    { passive: false },
  );

  element.addEventListener("keydown", (event) => {
    const target = event.target as HTMLElement;
    // Never while a parameter has the keyboard: a node body owns its own keys, down to the
    // arrow steps on a drag field.
    if (["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;
    if (target.closest(".node-body")) return;
    // A chord belongs to the system or the window - Cmd+H hides the app, it does not
    // also fold a node.
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    if (event.key === "Delete" || event.key === "Backspace") {
      removeSelected();
    } else if (event.key === "Escape") {
      closeMenu();
    } else if (event.key === "Home") {
      fitView();
    } else if (event.key === "h" || event.key === "H") {
      if (event.shiftKey) setAllCollapsed(graph.nodes.some((node) => !node.collapsed));
      else if (selected) toggleCollapse(selected);
    } else if (event.key === "l" || event.key === "L") {
      arrange();
    } else if (event.key.toLowerCase() === "a" && event.shiftKey) {
      // With shift held the key arrives as "A", so asking for "a" never matched.
      const at = pointerAt.x === 0 && pointerAt.y === 0
        ? (() => {
            const rect = surface.getBoundingClientRect();
            return { x: rect.left + 40, y: rect.top + 40 };
          })()
        : pointerAt;
      openSurfaceMenu(at, surfacePoint({ clientX: at.x, clientY: at.y }));
    }
  });
  element.tabIndex = 0;

  return {
    element,
    properties,
    selection,
    setGraph(next) {
      graph = next;
      // A new graph object means new node objects, so every body's widgets are stale.
      for (const entry of nodeElements.values()) entry.root.remove();
      nodeElements.clear();
      draw();
    },
    setEvaluation(nodes, nextStates) {
      descriptions = new Map(nodes.map((node) => [node.id, node]));
      states = nextStates;
      draw();
    },
    setRenderContext(context) {
      renderContext = context;
      draw();
    },
    setStyleLibrary(next) {
      library = next;
      draw();
    },
    refresh: draw,
    graph: () => structuredClone(graph),
    fit: fitView,
    select(nodeId) {
      selected = nodeId && nodeById(nodeId) ? nodeId : null;
      applySelection();
      if (selected) frameIntoView(selected);
    },
    setSelectionDetail(lines) {
      selectionDetail = lines;
      drawNodeProperties();
    },
    setHoverDetail(line) {
      if (line === hoverDetail) return;
      hoverDetail = line;
      drawSelectionCard();
    },
  };
}
