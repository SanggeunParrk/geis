// The viewer screen.
//
// The viewport on top, the node graph below it. Everything the screen used to have in side
// panels - which structure, how it is drawn, how it is coloured and lit - is a node now,
// because a pipeline is a graph and side panels can only show one path through it.
//
// The screen itself holds no scene state. It holds a graph, asks the core to evaluate it,
// and draws whatever comes back.

import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import { openPath as openWithDefault } from "@tauri-apps/plugin-opener";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  api,
  events,
  type BlenderInfo,
  type Graph,
  type GraphResult,
  type RenderOutcome,
  type ColorScheme,
  type Ramp,
  type Representation,
  type StructureInfo,
  type StructureMap,
  type Feature,
  featureSpans,
  type SelectWhere,
  type ContactKind,
  type SceneRequest,
  type PresetId,
  type SavedCamera,
  type FilePreview,
  type Version,
  type WorldView,
  type StyleSlot,
  COLOR_SCHEME_LABELS,
  REPRESENTATION_LABELS,
  type ChainInfo,
  type GraphNode,
  type PickTarget,
  type Style,
} from "../ipc";
import { capabilities } from "../viewer/gl";
import type { Scene } from "../viewer/scene";
import { Viewer } from "../viewer/viewer";
import { clear, count, el, replace } from "./dom";
import { nodeEditor, type NodeEditorHandle, type RenderProgress, type SceneRow } from "./nodeEditor";
import { sequencePanel } from "./sequence";
import { askConfirm, askText } from "./ask";
import { setStatusNotes, showProgress, statusBar, statusNote } from "./progress";
import { extensionsPanel } from "./extensionsPanel";
import { pdfView } from "./pdfView";
import { openPalette, openShortcuts, paletteOpen, type PaletteItem, type ShortcutGroup } from "./palette";
import { PALETTES, preferences, savePreferences } from "../prefs";
import { COMMANDS } from "../commands";
import { colourName, parseColour } from "./colourNames";
import { iconMarkup, type StructureIcon } from "./icons";
import { menu, type MenuEntry } from "./menu";
import { buildGraphView, residueColours, type ContactEdge } from "../viewer/graphView";
import { decodePreset, EMPTY_CONTACTS, type ContactSet } from "../viewer/presets";
import type { GraphEdge, GraphView } from "../viewer/viewer";
import { agentsPanel } from "./agents";
import type { AgentHost, ResidueRef, StyleArgs } from "../agent/tools";
import { RAMPS, REPRESENTATIONS, SCHEMES } from "../agent/tools";
import { hexToSrgb, hexToUnit, normalizeHex, unitToHex } from "./colorPicker";
import { combine, layMask, viewKey, type Combine, type View } from "./selection";
import {
  activateTab,
  activeTab,
  addPane,
  addTab,
  ALL_SIDES,
  magnetise,
  movingSides,
  placePane,
  resistEdges,
  bringToFront,
  closeTab,
  cycleFit,
  defaultDesk,
  workDesk,
  detachTab,
  EDITOR_LABELS,
  movePane,
  moveTab,
  occupiedSnaps,
  parseScreen,
  removePane,
  resizePane,
  restorePane,
  setTabEditor,
  SNAP_LABELS,
  SNAPS,
  snapAt,
  snapPane,
  snapRect,
  tabShowing,
  toggleCollapse,
  toggleMaximise,
  type Desk,
  type Edge,
  type EditorKind,
  type Snap,
} from "./panes";
import { screenBar } from "./window";

export interface ViewerScreenHandle {
  element: HTMLElement;
  /** Opens a structure from a path by pointing the graph's source node at it. */
  openPath: (path: string) => Promise<void>;
  /** Opens a world: its graph becomes the one on screen, at the camera it was left at. */
  openWorld: (id: string) => Promise<void>;
  /** Keeps the screen dark until `assemble`, while a world loads. */
  hold: () => void;
  /** Plays the screen coming together: the viewport first, then each panel over it. */
  assemble: () => void;
  dispose: () => void;
}

/** Above this many atoms a software rasteriser cannot keep up with per-atom geometry. */
const SOFTWARE_ATOM_LIMIT = 20000;

export function viewerScreen(onBack: () => void): ViewerScreenHandle {
  const stage = el("div", { class: "viewer-stage", tabindex: "-1" });
  const hud = el("div", { class: "viewer-hud" });
  /** What is open, said where it is asked for: in the world name's tooltip, not across the bar. */
  const titleSlot = { textContent: "" as string | null, title: "" };
  /**
   * Which world this is, where the screen's name used to be. Every world opens onto the
   * same screen, and with "Viewer" at the top of each there was nothing on it to say
   * which one you were in.
   */
  const worldTitle = el("h2", { class: "world-title", text: "Viewer" });

  let viewer: Viewer | null = null;
  let canvas: HTMLCanvasElement | null = null;
  let graph: Graph = { nodes: [], links: [] };
  let result: GraphResult | null = null;
  /** The molecules on screen, in the plan's order. */
  let structures: StructureInfo[] = [];
  /**
   * The molecules the viewer's scene was built from, part for part. `structures` changes
   * as soon as an evaluation lands, but the scene only once it has been built - a click in
   * between found its part in the old scene and its molecule in the new list.
   */
  let sceneStructures: StructureInfo[] = [];
  /** What a click selects, finest first - and so which number key chooses each. */
  type PickLevel = "atom" | "residue" | "chain" | "molecule" | "link";
  const LEVELS: { id: PickLevel; label: string; hint: string; icon: StructureIcon }[] = [
    { id: "atom", label: "Atom", hint: "A click selects one atom", icon: "atom" },
    { id: "residue", label: "Residue", hint: "A click selects one residue, or one ligand, ion or water", icon: "residue" },
    { id: "chain", label: "Chain", hint: "A click selects the chain it lands on", icon: "chain" },
    { id: "molecule", label: "Molecule", hint: "A click selects the whole molecule the thing clicked belongs to", icon: "molecule" },
    {
      id: "link",
      label: "Bond",
      // Said in full, because "edge" and "link" each mean several things: this is a bond
      // between two residues or components, not between two atoms inside one.
      hint: "A click selects a bond between two components - the backbone between neighbouring residues, a disulfide bridge, a metal contact, or a covalent link to a ligand. Shown in the graph view.",
      icon: "bond",
    },
  ];

  // Glass was a third mode here; it is a representation now ("Show > Glass"), so it can be
  // given to one molecule, one chain or a few residues, and is coloured as they are.
  type ViewMode = "structure" | "graph";
  const MODES: { id: ViewMode; label: string; about: string }[] = [
    { id: "structure", label: "Structure", about: "The molecule as it is styled" },
    { id: "graph", label: "Graph", about: "Each chemical component a sphere, over a ghost of the structure" },
  ];
  const modeNow = () => (graphMode ? MODES[1] : MODES[0]);

  /** Whether the viewport is showing the graph of components over a ghost of the structure. */
  let graphMode = false;
  /** Which load of the graph is the current one; a slower, older one must not land after it. */
  let graphAsked = 0;
  /**
   * What an action will act on: the selection, as the chains it lies in.
   *
   * Selection first, action second - the pies and the menu say what they are about to
   * change, and it is whatever was selected before they were opened. Chains, because a
   * style is a chain's: a selection of residues acts on the chains they belong to, and
   * the readout says so rather than letting it look finer than it is.
   */
  let actionTarget: { structureId: string; chains: string[]; level: "chain" | "residue" | "atom"; label: string } | null = null;

  /** The graph on screen, for what its bonds are; and the bond that was clicked, if one was. */
  let shownGraph: GraphView | null = null;
  let pickedEdge: number | null = null;

  let notice: string | null = null;
  /** The notice the status bar is timing out, and the timer doing it. */
  let noticeTimed: string | null = null;
  /** Molecules just switched to another version, whose chain nodes are to follow it. */
  const rechaining = new Set<string>();
  /** Whether the next scene drawn is framed afresh rather than kept where the camera is. */
  let reframeNext = false;
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  /** The level the caption last named, so that only a change of level flashes it. */
  let captionLevel: string | null = null;
  let captionTimer: ReturnType<typeof setTimeout> | undefined;

  let evaluating = false;
  let pendingEvaluation = false;
  let unlistenJobs: (() => void) | null = null;
  /**
   * Resolves once the starting graph has arrived.
   *
   * Opening a file means editing the graph, and the graph is fetched asynchronously. A
   * file passed on the command line raced that: the node was added to an empty graph, so
   * it was connected to nothing and nothing was drawn.
   */
  // Assigned once, further down, after the functions that wait on it are defined.
  // eslint-disable-next-line prefer-const
  let ready: Promise<void>;
  const gpu = capabilities();

  let blender: BlenderInfo | null | undefined;
  /** Per Render node, because a graph can hold more than one. */
  let renderProgress: Record<string, RenderProgress> = {};
  /** Whether the graph pane is folded away, for a full-window viewport. */
  /** Whether the environment's scripted view has been applied; it runs once. */
  let scripted = false;
  /** The left of the screen as a desk of floating panes - see `panes.ts`. */
  let desk: Desk = defaultDesk();
  /** The fixed right column's width, which the user drags. */
  let sideWidth = 272;
  /**
   * At which level a click in the viewport selects: a chain node, or one residue or atom
   * of it. The bar under the viewport sets it, like VS Code's status bar sets an editor's
   * mode.
   */
  let pickLevel: PickLevel = "chain";
  /** Pending graph write; see `save`. */
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  /** The world on screen, so leaving it can remember where it was left. */
  let activeWorld: string | null = null;
  /** Counts each time the graph is replaced wholesale, so work begun on one can tell. */
  let graphEpoch = 0;
  /** A camera to apply once there is a scene to apply it to. */
  let pendingCamera: { eye: [number, number, number]; center: [number, number, number]; up: [number, number, number]; radius: number } | null = null;

  // The molecule as sequence - see `sequence.ts` for why it is no longer a map.
  const biomol = sequencePanel({
    onSelect: (view, mode) => void applySelection(view, mode),
    onHover: (view) => void hoverView(view),
  });

  const editor: NodeEditorHandle = nodeEditor({
    onChange: (next) => {
      graph = next;
      save();
      void evaluate();
    },
    onRender: (nodeId) => void renderNode(nodeId),
    onReset: () => void resetGraph(),
    onFetch: (id, at) => void addMolecule({ pdb_id: id, path: null }, at),
    onOpenFile: (at) => void openMoleculeFile(at),
    onStyleSave: (name, style) => void saveStyle(name, style as never),
    cameraRows: () => cameraRows(),
    onObject: (moleculeId, name) => void selectObject(moleculeId, name),
    onObjectRemove: (moleculeId, name) => removeObject(moleculeId, name),
    versionRow: (moleculeId) => versionRow(moleculeId),
    presetRows: (moleculeId) => {
      const structure = structures.find((entry) => moleculeNodeFor(entry)?.id === moleculeId);
      const mine = structure ? analyses.get(structure.id) : undefined;
      if (!structure || !mine) return null;
      return PRESETS.map((preset) => {
        const entry = mine.get(preset.id);
        return {
          id: preset.id,
          label: preset.label,
          edges: preset.edges,
          state: entry?.state ?? "loading",
          count: preset.edges ? (entry?.contacts.count ?? 0) : (entry?.residues.length ?? 0),
          shown: preset.edges && shownEdges.has(preset.id as ContactKind),
          swatch: preset.edges ? unitToHex(CONTACTS[preset.id as ContactKind].color) : undefined,
          error: entry?.error,
        };
      });
    },
    onSceneMenu: (row, at) => openSceneMenu(row, at),
    onPreset: (moleculeId, id) => {
      const structure = structures.find((entry) => moleculeNodeFor(entry)?.id === moleculeId);
      const preset = PRESETS.find((entry) => entry.id === id);
      if (!structure || !preset) return;
      if (preset.edges) {
        // From the list it is the whole group that is meant, not the part by the selection.
        const kind = preset.id as ContactKind;
        if (shownEdges.has(kind)) void toggleContacts(kind, false);
        else {
          shownEdges.add(kind);
          contactScope = null;
          drawContacts();
          editor.refresh();
          const entry = analyses.get(structure.id)?.get(kind);
          editor.setHoverDetail(entry?.state === "ready" ? `${count(entry.contacts.count, CONTACTS[kind].one)} - ${CONTACTS[kind].rule}.` : "Still looking...");
        }
      } else void selectPreset(preset.id, structure);
    },
    onStyleDelete: (name) => void deleteStyle(name),
    onSelect: (nodeId) => {
      aimSelectedLight(nodeId);
      // Selecting a node from inside `applySelection` must not come back round as a new
      // chain-level selection and flatten the residue or atom that was picked.
      if (syncingSelection) return;
      void highlightSelection(nodeId);
      describeSelection(nodeId, null);
    },
    onReveal: (path) => void api.revealPath(path),
    onImageCopy: (path) => void copyImage(path),
    onImageSave: (path) => void saveImageAs(path),
    imageSource: (path) => imageUrl(path),
    viewportAspect: () => {
      if (!canvas || canvas.clientHeight === 0) return null;
      return canvas.clientWidth / canvas.clientHeight;
    },
    sampleViewport: async (client) => viewer?.samplePixel(client) ?? null,
    setPickingCursor: (picking) => stage.classList.toggle("picking", picking),
  });

  /**
   * The viewport area's own content: the canvas and the level bar below it. The sequence
   * strip that sat above the canvas became the residue ring of the BioMol panel's map.
   */
  const levelBar = el("div", { class: "level-bar mode-row", role: "radiogroup", "aria-label": "Select mode" });
  /** The viewport's own header, as Blender's 3D view has: which mode it is in, first. */
  const modeHeader = el("div", { class: "viewport-header" });
  /** What the next action acts on - see `drawTarget`. */
  const targetChip = el("div", { class: "target-chip" });
  /** Which select level is in use, in a word, under its buttons. */
  const levelCaption = el("div", { class: "level-caption" });
  const viewerMain = el("div", { class: "viewer-main" }, stage, hud, levelBar, levelCaption, targetChip, modeHeader);
  const viewportArea = el("div", { class: "viewport-area" }, viewerMain);
  const viewportMenu = menu(viewerMain);
  const biomolArea = biomol.element;
  const agents = agentsPanel(agentHost());
  const imageAreas = new Map<string, HTMLElement>();
  const textAreas = new Map<string, HTMLElement>();
  const deskRoot = el("div", { class: "desk" });
  /**
   * The right column: the outliner and the properties, fixed. Not an area, because it is
   * the one place that always says what is selected - it resizes, it does not move or
   * close. The divider beside it drags its width.
   */
  const sideDivider = el("div", { class: "side-divider", title: "Drag to resize the panel" });
  /**
   * Two fixed columns with the desk between them.
   *
   * On the left, the outliner and the properties: the one place that always says what is
   * selected. On the right, the conversation. Neither is on the desk, for the same reason -
   * a panel you can lose behind a viewport is a panel you stop using - and each has a
   * divider of its own. The conversation folds to a strip when the picture matters more.
   */
  let chatWidth = 320;
  let chatOpen = true;
  const chatFold = el("button", { class: "side-fold", text: "▸", title: "Fold the conversation away" });
  const chatHead = el(
    "div",
    { class: "side-head" },
    chatFold,
    el("span", { class: "side-title", text: "Agents" }),
    el("span", { class: "spacer" }),
    el("button", {
      class: "side-btn",
      text: "Settings",
      title: "Connections, local models, and what the agent may do",
      onclick: () => void agents.openSettings(),
    }),
  );
  const chatColumn = el("div", { class: "chat-column" }, chatHead, agents.element);
  const chatDivider = el("div", { class: "side-divider" });
  // --- the activity bar ------------------------------------------------------------------
  //
  // VS Code's: a narrow bar of icons at the far left that says what the side panel next
  // to it is showing. The scene is one of the things it can show and what can be added to
  // the app is another. The icon of what is showing, pressed again, folds the panel away.

  type Activity = "scene" | "extensions";
  const ACTIVITIES: { id: Activity; label: string; icon: string }[] = [
    {
      id: "scene",
      label: "Scene",
      icon: '<path d="M8 2 2 5l6 3 6-3-6-3Z"/><path d="m2 8 6 3 6-3"/><path d="m2 11 6 3 6-3"/>',
    },
    {
      id: "extensions",
      label: "Extensions",
      icon: '<rect x="2" y="8" width="5" height="5" rx="0.8"/><rect x="8" y="8" width="5" height="5" rx="0.8"/><rect x="2" y="2" width="5" height="5" rx="0.8"/><rect x="9.5" y="1.5" width="4.5" height="4.5" rx="0.8" transform="rotate(12 11.75 3.75)"/>',
    },
  ];
  let activity: Activity = "scene";
  let sideOpen = true;
  const extensions = extensionsPanel(() => window.dispatchEvent(new CustomEvent("geis:settings", { detail: "models" })));
  const sideColumn = el("div", { class: "side-column" }, editor.properties);
  const activityBar = el("nav", { class: "activity-bar", "aria-label": "Side panel" });
  /** Settings, at the foot of the activity bar: VS Code's gear, in VS Code's place. */
  const settingsButton = el("button", {
    class: "activity-btn settings-gear",
    title: "Settings - preferences, agents, compute and extensions (⌘,)",
    "aria-label": "Settings",
    onclick: () => window.dispatchEvent(new CustomEvent("geis:settings", { detail: "preferences" })),
  });
  settingsButton.innerHTML =
    '<svg viewBox="0 0 16 16" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" stroke-linecap="round" aria-hidden="true"><circle cx="8" cy="8" r="2.1"/><path d="M8 1.6v1.8M8 12.6v1.8M1.6 8h1.8M12.6 8h1.8M3.5 3.5l1.3 1.3M11.2 11.2l1.3 1.3M3.5 12.5l1.3-1.3M11.2 4.8l1.3-1.3"/><circle cx="8" cy="8" r="4.6"/></svg>';

  function drawActivity(): void {
    replace(
      activityBar,
      ...ACTIVITIES.map((entry) => {
        const on = sideOpen && activity === entry.id;
        const button = el("button", {
          class: `activity-btn${on ? " active" : ""}`,
          title: on ? `${entry.label} - press again to fold the panel away` : entry.label,
          "aria-label": entry.label,
          "aria-pressed": String(on),
          onclick: () => {
            if (activity === entry.id) sideOpen = !sideOpen;
            else {
              activity = entry.id;
              sideOpen = true;
            }
            drawActivity();
          },
        });
        // Fixed markup of this file's own, not text from anywhere else.
        button.innerHTML = `<svg viewBox="0 0 16 16" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" stroke-linecap="round" aria-hidden="true">${entry.icon}</svg>`;
        return button;
      }),
      el("span", { class: "spacer" }),
      settingsButton,
    );
    // The scene's panel is moved in and out rather than rebuilt: it holds the outliner's
    // state and the properties being edited.
    const wanted = activity === "scene" ? editor.properties : extensions.element;
    if (sideColumn.firstElementChild !== wanted) {
      sideColumn.replaceChildren(wanted);
      if (activity === "extensions") void extensions.refresh();
    }
    body.classList.toggle("side-folded", !sideOpen);
    viewer?.requestFrame();
  }

  function drawChat(): void {
    chatFold.textContent = chatOpen ? "▸" : "◂";
    chatColumn.classList.toggle("folded", !chatOpen);
    // Not hidden: an element with `display: none` is not placed in the grid at all, so the
    // column after it moved up into the divider's track and came out five pixels wide.
    chatDivider.classList.toggle("inert", !chatOpen);
    body.style.setProperty("--chat-width", chatOpen ? `${chatWidth}px` : "26px");
    viewer?.requestFrame();
  }
  chatFold.addEventListener("click", () => {
    chatOpen = !chatOpen;
    drawChat();
    saveLayout();
  });
  chatDivider.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    const rect = () => body.getBoundingClientRect();
    const move = (moveEvent: PointerEvent) => {
      chatWidth = Math.min(700, Math.max(220, rect().right - moveEvent.clientX));
      body.style.setProperty("--chat-width", `${chatWidth}px`);
      viewer?.requestFrame();
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      saveLayout();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });

  const body = el(
    "div",
    { class: "viewer-body" },
    activityBar,
    sideColumn,
    sideDivider,
    deskRoot,
    chatDivider,
    chatColumn,
  );
  sideDivider.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    const rect = body.getBoundingClientRect();
    const move = (moveEvent: PointerEvent) => {
      // Measured from the panel's own left edge, which is past the activity bar.
      sideWidth = Math.min(600, Math.max(180, moveEvent.clientX - rect.left - activityBar.offsetWidth));
      body.style.setProperty("--side-width", `${sideWidth}px`);
      viewer?.requestFrame();
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      editor.refresh();
      saveLayout();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });

  /** A small line icon for the bar, from fixed markup of this file's own. */
  function barIcon(paths: string): HTMLElement {
    const holder = el("span", { class: "bar-icon-glyph", "aria-hidden": "true" });
    holder.innerHTML = `<svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" stroke-linecap="round">${paths}</svg>`;
    return holder;
  }

  /**
   * The bar along the top, laid out as VS Code's title bar: where you are on the left, the
   * search for every command in the middle, and two quiet icons on the right for what is
   * reached for most - a picture of the view, and how the panes are arranged. Settings
   * are the gear at the foot of the activity bar, where VS Code keeps them.
   */
  function viewerBar(): HTMLElement {
    const back = el("button", {
      class: "bar-back",
      text: "‹ Worlds",
      title: "Back to the shelf. Where this world is left is remembered.",
      // The shelf comes up first and the camera is written after it. Waiting for a round
      // trip to the core before anything moves is a held breath on a button press, and
      // where the camera was left is not something the shelf reads.
      onclick: () => {
        // The picture first, while the scene is still the one being left.
        const picture = takeThumbnail();
        onBack();
        setTimeout(() => {
          void rememberWorld();
          if (picture) void api.worldThumbnailSave(picture.world, picture.data).catch(() => {});
        }, 0);
      },
    });
    const search = el(
      "button",
      { class: "bar-command", title: "Find any command by name (⌘K)", onclick: () => showPalette() },
      barIcon('<circle cx="7" cy="7" r="4.2"/><path d="m10.2 10.2 3.3 3.3"/>'),
      el("span", { class: "bar-command-text", text: "Search commands" }),
      el("kbd", { text: "⌘K" }),
    );
    const picture = el(
      "button",
      {
        class: "bar-icon",
        title: "Save image - the render, or the viewport as it is",
        "aria-label": "Save image",
        onclick: (event: Event) => {
          event.stopPropagation();
          saveImageMenu();
        },
      },
      barIcon('<path d="M2.5 5.5h2l1.2-1.8h4.6l1.2 1.8h2v7h-11z"/><circle cx="8" cy="8.8" r="2.2"/>'),
    );
    const layout = el(
      "button",
      {
        class: "bar-icon",
        title: "Layout - the viewport alone, or over the node graph and the sequence",
        "aria-label": "Layout",
        onclick: (event: Event) => {
          event.stopPropagation();
          const at = (event.currentTarget as HTMLElement).getBoundingClientRect();
          // Under the button, but never past the desk's right edge: the menu belongs to
          // the desk, and the part of it that reached over the agent panel was drawn
          // underneath it.
          const desk = deskRoot.getBoundingClientRect();
          deskMenu.open({ x: Math.max(desk.left + 8, Math.min(at.left, desk.right - 248)), y: at.bottom + 6 }, layoutEntries());
        },
      },
      barIcon('<rect x="2" y="2.5" width="12" height="11" rx="1.2"/><path d="M6.5 2.5v11M6.5 8H14"/>'),
    );
    const bar = screenBar(
      el("div", { class: "bar-side" }, back, worldTitle),
      search,
      el("div", { class: "bar-side end" }, picture, layout),
    );
    bar.classList.add("viewer-bar");
    return bar;
  }

  const element = el(
    "div",
    { class: "screen viewer-screen" },
    viewerBar(),
    body,
    statusBar(),
  );

  // Floating panels: the viewport is the whole screen and the panels stand over it, as a
  // game's HUD does, instead of beside it in columns. On trial, and remembered; the Layout
  // menu turns it off.
  let floating = true;
  try {
    floating = localStorage.getItem("geis.viewer.floating") !== "0";
  } catch {
    /* the default */
  }
  element.classList.toggle("floating", floating);
  function setFloating(on: boolean): void {
    floating = on;
    element.classList.toggle("floating", on);
    try {
      localStorage.setItem("geis.viewer.floating", on ? "1" : "0");
    } catch {
      /* not remembered */
    }
  }

  /**
   * The screen coming together as a world is entered: the viewport comes up first, then the
   * bar, the panels and the status line open over it one after another - each a bright
   * line drawn across and widening into the panel. Restarted on every entry.
   */
  let assembling: number | null = null;
  function hold(): void {
    element.classList.add("holding");
  }
  function assemble(): void {
    element.classList.remove("holding");
    element.classList.remove("assembling");
    // Read back, so the class going on again restarts the animations.
    void element.offsetWidth;
    element.classList.add("assembling");
    if (assembling !== null) window.clearTimeout(assembling);
    assembling = window.setTimeout(() => {
      assembling = null;
      element.classList.remove("assembling");
    }, 1800);
  }

  drawLevelBar();
  drawActivity();
  void api.layoutLoad().then((saved) => {
    const parsed = parseScreen(saved);
    if (parsed) {
      desk = parsed.desk;
      legacyDesk = JSON.parse(JSON.stringify(parsed.desk)) as Desk;
      sideWidth = parsed.sideWidth;
      chatWidth = parsed.chatWidth;
      chatOpen = parsed.chatOpen;
    }
    drawChat();
    body.style.setProperty("--side-width", `${sideWidth}px`);
    renderDesk();
  });

  // --- the desk -------------------------------------------------------------------
  //
  // Chrome's windows, not Blender's areas: each pane is a window with a title bar to drag
  // and edges to drag, panes overlap, and the one touched last is on top; and like a
  // browser window each holds tabs. Elements are kept per pane and restyled in place, so a
  // move is a style change, and a tab's editor is reparented only when the active tab
  // changes - a singleton's canvas is never rebuilt.

  interface PaneEntry {
    root: HTMLElement;
    strip: HTMLElement;
    content: HTMLElement;
    /** The tab whose editor is in `content`, and which editor it was at the time. */
    shown: { tab: string; editor: EditorKind } | null;
  }
  const paneElements = new Map<string, PaneEntry>();
  /** The outline shown where a dragged pane will land if released. */
  const snapGhost = el("div", { class: "snap-ghost", hidden: "" });
  deskRoot.append(snapGhost);
  const deskMenu = menu(deskRoot);
  /** The tab that follows the pointer while a tab is dragged, as Chrome's does. */
  const tabGhost = el("div", { class: "tab-ghost", hidden: "" });
  deskRoot.append(tabGhost);
  /** Where a dragged tab would go in a strip. */
  const tabSlot = el("div", { class: "tab-slot", hidden: "" });

  // The menu module leaves closing to its host. A press anywhere outside the menu, or
  // Escape, closes it - a menu that stays up after you click elsewhere is a stuck menu.
  document.addEventListener(
    "pointerdown",
    (event) => {
      if (!deskMenu.isOpen) return;
      if ((event.target as HTMLElement).closest(".node-menu")) return;
      deskMenu.close();
    },
    true,
  );
  // The viewport's own menu - right-click, Save image - is closed the same way: Escape
  // used to leave it standing.
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    deskMenu.close();
    viewportMenu.close();
  });
  window.addEventListener("blur", () => deskMenu.close());

  function commit(next: Desk): void {
    desk = next;
    renderDesk();
    saveLayout();
  }

  /** The Layout button's menu: the two arrangements, and a pane of each kind. */
  function layoutEntries(): MenuEntry[] {
    return [
      { kind: "title", label: "Arrange" },
      { kind: "item", label: "3D viewport only", onPick: () => commit(defaultDesk()) },
      { kind: "item", label: "3D viewport, sequence and node graph", onPick: () => commit(workDesk()) },
      {
        kind: "item",
        label: floating ? "Panels beside the viewport" : "Panels floating over the viewport",
        onPick: () => setFloating(!floating),
      },
      { kind: "separator" },
      { kind: "title", label: "Add a pane" },
      ...newTabEntries((kind) => commit(addPane(desk, kind))),
    ];
  }

  const newTabEntries = (pick: (kind: EditorKind) => void) =>
    (Object.entries(EDITOR_LABELS) as [EditorKind, string][]).map(([kind, label]) => ({
      kind: "item" as const,
      label,
      onPick: () => pick(kind),
    }));

  // The empty desk's right-click: make a pane here. The one way back from a desk with
  // nothing on it, and the natural way to add one where you want it.
  deskRoot.addEventListener("contextmenu", (event) => {
    if ((event.target as HTMLElement).closest(".pane")) return;
    event.preventDefault();
    const at = deskFraction(event.clientX, event.clientY);
    deskMenu.open({ x: event.clientX, y: event.clientY }, [
      { kind: "title", label: "New pane" },
      ...newTabEntries((kind) => commit(addPane(desk, kind, at))),
      { kind: "separator" as const },
      { kind: "item" as const, label: "Reset layout", onPick: () => commit(workDesk()) },
    ]);
  });

  function showSnapGhost(snap: Snap | null): void {
    snapGhost.hidden = snap === null;
    if (!snap) return;
    const rect = snapRect(snap);
    snapGhost.style.left = `${rect.x * 100}%`;
    snapGhost.style.top = `${rect.y * 100}%`;
    snapGhost.style.width = `${rect.w * 100}%`;
    snapGhost.style.height = `${rect.h * 100}%`;
  }

  function saveLayout(): void {
    // The columns' widths are the app's; which panes are open, and where, is the world's.
    // The app's copy keeps the arrangement from before layouts were per world, untouched,
    // for the worlds that have not been opened since and still expect it.
    void api.layoutSave(JSON.stringify({ desk: legacyDesk ?? desk, sideWidth, chatWidth, chatOpen }));
    // Only into the world whose arrangement is on the desk - otherwise whatever desk was up
    // when the screen first showed would be written over a world never restored.
    if (activeWorld && layoutOf === activeWorld) {
      void api.worldLayoutSave(activeWorld, JSON.stringify({ desk, sideWidth, chatWidth, chatOpen })).catch(() => {});
    }
  }

  /** Which world the desk was arranged from; see `saveLayout`. */
  let layoutOf: string | null = null;

  /** The one arrangement the app had when every world shared it. */
  let legacyDesk: Desk | null = null;

  // --- cameras ------------------------------------------------------------------------
  //
  // As Blender has them: a camera is a thing kept in the world - a place to look from,
  // with a name - not the view itself. The view moves freely; a camera is made from where
  // it is ("camera to view"), looked through later, and is what a figure is rendered from
  // twice the same.

  let cameras: SavedCamera[] = [];
  /** The camera `0` goes to: the one last made or looked through. */
  let activeCamera: string | null = null;
  /**
   * Which camera the viewport is looking through right now, or null for the main one.
   *
   * The main camera is the view itself: it goes wherever the view is dragged and is what
   * the world remembers. A made camera stays where it was put - for a figure taken twice
   * from the same place, and for the keyframes an animation will be cut between. Looking
   * through one moves the view to it; the moment the view is moved again it is the main
   * camera's once more, and the made one has not moved.
   */
  let lookingThrough: string | null = null;
  /** Where the main camera was before a made one was looked through, to go back to. */
  let mainBefore: Pick<SavedCamera, "eye" | "center" | "up" | "radius"> | null = null;

  function backToMain(restore: boolean): void {
    if (lookingThrough === null) return;
    lookingThrough = null;
    if (restore && mainBefore && viewer) viewer.setCamera(mainBefore);
    mainBefore = null;
    editor.refresh();
    if (restore) editor.setHoverDetail("Back to the main camera.");
  }

  /**
   * Which world `cameras` was read from. Written back only to that one: a list that was
   * never loaded is empty, and saving it over a world's row deleted the cameras it had.
   */
  let camerasOf: string | null = null;

  function saveCameras(): void {
    if (activeWorld && camerasOf === activeWorld) {
      void api.worldViewSave(activeWorld, { cameras, active: activeCamera }).catch(() => {});
    }
    editor.refresh();
  }

  async function loadCameras(world: string): Promise<void> {
    const read = await api.worldView(world).catch(() => null);
    // A list that could not be read is not known to be empty, so it is not written back.
    camerasOf = read ? world : null;
    const saved: WorldView = read ?? {};
    cameras = Array.isArray(saved.cameras) ? saved.cameras : [];
    lookingThrough = null;
    mainBefore = null;
    activeCamera = cameras.some((camera) => camera.id === saved.active) ? (saved.active ?? null) : null;
    editor.refresh();
  }

  /** Where the view is now, as a camera would hold it. */
  function viewNow(): Pick<SavedCamera, "eye" | "center" | "up" | "radius"> | null {
    if (!viewer) return null;
    const now = viewer.describeCamera();
    return { eye: now.eye, center: now.center, up: now.up, radius: now.radius };
  }

  function addCamera(): void {
    const here = viewNow();
    if (!here) return;
    const taken = new Set(cameras.map((camera) => camera.name));
    let number = cameras.length + 1;
    while (taken.has(`Camera ${number}`)) number += 1;
    const camera: SavedCamera = { id: `camera-${Date.now().toString(36)}`, name: `Camera ${number}`, ...here };
    cameras = [...cameras, camera];
    activeCamera = camera.id;
    saveCameras();
    editor.setHoverDetail(`${camera.name} added where the main camera is. The main camera goes on following the view.`);
  }

  function lookThrough(id: string | null): void {
    const camera = cameras.find((entry) => entry.id === id) ?? cameras.find((entry) => entry.id === activeCamera) ?? cameras[0];
    if (!camera || !viewer) {
      editor.setHoverDetail("This world has no camera yet. Add one from the View chain (V).");
      return;
    }
    // Where the main camera was, kept once - going from one made camera to another must
    // not make the first of them the place to come back to.
    if (lookingThrough === null) mainBefore = viewNow();
    viewer.setCamera(camera);
    activeCamera = camera.id;
    lookingThrough = camera.id;
    saveCameras();
    editor.setHoverDetail(`Looking through ${camera.name}. Move the view, or press 0, to leave it.`);
  }

  function cameraRows(): HTMLElement[] {
    return [
      el("div", { class: "camera-head", text: "Cameras" }),
      el(
        "div",
        { class: `camera-row${lookingThrough === null ? " active" : ""}` },
        el("button", { class: "camera-name", text: "Main camera", title: "The view itself: it follows wherever you look", onclick: () => backToMain(true) }),
        el("span", { class: "camera-note", text: "follows the view" }),
      ),
      ...cameras.map((camera) =>
        el(
          "div",
          { class: `camera-row${camera.id === lookingThrough ? " active" : ""}` },
          el("button", { class: "camera-name", text: camera.name, title: "Look through this camera (0 looks through the active one)", onclick: () => lookThrough(camera.id) }),
          el("button", {
            class: "id-action",
            text: "✎",
            title: "Rename",
            onclick: () =>
              void askText({ title: "Name this camera", initial: camera.name, accept: "Rename" }).then((name) => {
                if (!name) return;
                cameras = cameras.map((entry) => (entry.id === camera.id ? { ...entry, name } : entry));
                saveCameras();
              }),
          }),
          el("button", {
            class: "id-action",
            text: "⌖",
            title: "Move this camera to the current view",
            onclick: () => {
              const here = viewNow();
              if (!here) return;
              cameras = cameras.map((entry) => (entry.id === camera.id ? { ...entry, ...here } : entry));
              activeCamera = camera.id;
              saveCameras();
            },
          }),
          el("button", {
            class: "id-action",
            text: "×",
            title: "Remove this camera",
            onclick: () => {
              cameras = cameras.filter((entry) => entry.id !== camera.id);
              if (lookingThrough === camera.id) backToMain(false);
              if (activeCamera === camera.id) activeCamera = cameras[0]?.id ?? null;
              saveCameras();
            },
          }),
        ),
      ),
      el("button", { class: "panel-add", text: "+ Camera, from this view", onclick: () => addCamera() }),
    ];
  }

  /**
   * A world whose background is still one of the old defaults - never chosen, only
   * inherited - takes the preferred one. A background somebody picked is left alone.
   */
  function quietenBackground(): void {
    const backdrop = graph.nodes.find((node) => node.type === "world");
    const wanted = normalizeHex(preferences().background);
    if (!backdrop || backdrop.type !== "world" || !wanted) return;
    // The old default as the core had it, as it was once written into a world, and as a
    // build that converted it twice left it.
    const OLD: [number, number, number][] = [[0.925, 0.933, 0.945], [0.9386857, 0.9473065, 0.9559733], hexToUnit("#eceef1")];
    const now = backdrop.background;
    if (!OLD.some((old) => old.every((channel, axis) => Math.abs(channel - now[axis]) < 0.004))) return;
    backdrop.background = hexToSrgb(wanted);
    editor.setGraph(graph);
    save(false);
    void evaluate();
  }

  /** Arranges the panes the way this world was left - or, for a new one, the default. */
  async function restoreLayout(world: string): Promise<void> {
    const saved = await api.worldLayout(world).catch(() => null);
    if (saved === null) {
      // A world from before layouts were per world: it gets the arrangement the app had
      // then, and from here on that is its own.
      desk = legacyDesk ? (JSON.parse(JSON.stringify(legacyDesk)) as Desk) : workDesk();
      void api.worldLayoutSave(world, JSON.stringify({ desk, sideWidth, chatWidth, chatOpen })).catch(() => {});
    } else if (saved === "default") {
      // A world opened for the first time: laid out, selected in and lit the way the
      // preferences say a new one is - and from here on its own.
      const wanted = preferences();
      desk = wanted.newWorldLayout === "work" ? workDesk() : defaultDesk();
      setPickLevel(wanted.selectLevel);
      const backdrop = graph.nodes.find((node) => node.type === "world");
      const colour = normalizeHex(wanted.background);
      if (backdrop && backdrop.type === "world" && colour) {
        backdrop.background = hexToSrgb(colour);
        editor.setGraph(graph);
        save(false);
      }
      void api.worldLayoutSave(world, JSON.stringify({ desk, sideWidth, chatWidth, chatOpen })).catch(() => {});
    } else {
      desk = parseScreen(saved)?.desk ?? defaultDesk();
    }
    layoutOf = world;
    renderDesk();
  }

  /** The DOM an editor lives in. Singletons are moved, not rebuilt; images and text are per tab. */
  function editorElement(kind: EditorKind, tabId: string): HTMLElement {
    switch (kind) {
      case "viewport":
        return viewportArea;
      case "biomol":
        return biomolArea;
      case "graph":
        return editor.element;
      case "image": {
        let host = imageAreas.get(tabId);
        if (!host) {
          host = el("div", { class: "image-area" });
          imageAreas.set(tabId, host);
        }
        drawImageArea(host);
        return host;
      }
      case "text": {
        let host = textAreas.get(tabId);
        if (!host) {
          host = el("div", { class: "text-area" });
          textAreas.set(tabId, host);
        }
        void drawTextArea(host);
        return host;
      }
    }
  }

  /**
   * One way to move a pane by the pointer, whatever it was grabbed by - its title bar, or
   * the tab of a one-tab pane. The pane follows the pointer's travel from where the grab
   * started, and its edges are pulled onto nearby ones on the way; the grab point does not
   * matter to the rule, which is why it lives here and not in either handler.
   */
  function dragPaneTo(
    id: string,
    origin: { x: number; y: number },
    start: { x: number; y: number },
    at: { x: number; y: number },
  ): void {
    const pane = desk.panes.find((candidate) => candidate.id === id);
    if (!pane) return;
    // Where the pointer's travel alone would put it - before any clamp, so the band past
    // the desk's edge is known and the edge can resist.
    const asked = { x: origin.x + (at.x - start.x), y: origin.y + (at.y - start.y), w: pane.w, h: pane.h };
    const { tx, ty } = magnetReach();
    const resist = magnetReach(56);
    const held = resistEdges(asked, resist.tx, resist.ty);
    desk = placePane(desk, id, magnetise(desk, id, held, ALL_SIDES, tx, ty));
  }

  /** A few pixels, as desk fractions per axis: how close an edge has to be to be pulled. */
  function magnetReach(pixels = 10): { tx: number; ty: number } {
    const rect = deskRoot.getBoundingClientRect();
    return { tx: pixels / Math.max(rect.width, 1), ty: pixels / Math.max(rect.height, 1) };
  }

  /** Pointer coordinates to desk fractions. */
  function deskFraction(clientX: number, clientY: number): { x: number; y: number } {
    const rect = deskRoot.getBoundingClientRect();
    return {
      x: (clientX - rect.left) / Math.max(rect.width, 1),
      y: (clientY - rect.top) / Math.max(rect.height, 1),
    };
  }

  function renderDesk(): void {
    const alive = new Set(desk.panes.map((pane) => pane.id));
    for (const [id, entry] of paneElements) {
      if (!alive.has(id)) {
        entry.root.remove();
        paneElements.delete(id);
      }
    }
    for (const pane of desk.panes) {
      let entry = paneElements.get(pane.id);
      if (!entry) {
        entry = createPane(pane.id);
        paneElements.set(pane.id, entry);
        deskRoot.append(entry.root);
      }
      entry.root.style.left = `${pane.x * 100}%`;
      entry.root.style.top = `${pane.y * 100}%`;
      entry.root.style.width = `${pane.w * 100}%`;
      entry.root.style.height = pane.collapsed ? "" : `${pane.h * 100}%`;
      entry.root.style.zIndex = `${desk.order.indexOf(pane.id) + 1}`;
      entry.root.classList.toggle("front", desk.order.at(-1) === pane.id);
      entry.root.classList.toggle("filling", pane.x === 0 && pane.y === 0 && pane.w === 1 && pane.h === 1);
      entry.root.classList.toggle("collapsed", pane.collapsed === true);
      renderStrip(pane.id, entry);
      const active = activeTab(pane);
      if (entry.shown?.tab !== active.id || entry.shown.editor !== active.editor) {
        replace(entry.content, editorElement(active.editor, active.id));
        entry.shown = { tab: active.id, editor: active.editor };
      }
    }
    // Reparented editors have new boxes: the canvas is sized from its, the graph measures
    // its links from its.
    requestAnimationFrame(() => {
      viewer?.requestFrame();
      editor.refresh();
    });
  }

  /** The tab strip: one button per tab, the active one lit, and a `+` at the end. */
  function renderStrip(paneId: string, entry: PaneEntry): void {
    const pane = desk.panes.find((candidate) => candidate.id === paneId);
    if (!pane) return;
    const key = pane.tabs.map((tab) => `${tab.id}:${tab.editor}`).join(",") + "|" + pane.active;
    if (entry.strip.dataset.key === key) return;
    entry.strip.dataset.key = key;
    clear(entry.strip);
    for (const tab of pane.tabs) {
      const button = el(
        "div",
        {
          class: `pane-tab${tab.id === pane.active ? " active" : ""}`,
          dataset: { tab: tab.id },
          title: `${EDITOR_LABELS[tab.editor]}. Click to show; drag to another pane or off the strip; right-click to change.`,
        },
        el("span", { class: "pane-tab-label", text: EDITOR_LABELS[tab.editor] }),
        el("button", {
          class: "pane-tab-close",
          text: "×",
          title: "Close this tab",
          onclick: (event: MouseEvent) => {
            event.stopPropagation();
            commit(closeTab(desk, paneId, tab.id));
          },
        }),
      );
      button.addEventListener("pointerdown", (event) => beginTabDrag(event, paneId, tab.id));
      button.addEventListener("contextmenu", (event) => {
        event.preventDefault();
        event.stopPropagation();
        deskMenu.open({ x: event.clientX, y: event.clientY }, [
          { kind: "title", label: EDITOR_LABELS[tab.editor] },
          {
            kind: "submenu",
            label: "Show instead",
            entries: () => newTabEntries((kind) => commit(setTabEditor(desk, paneId, tab.id, kind))),
          },
          {
            kind: "item",
            label: "Move to its own pane",
            disabled: pane.tabs.length < 2,
            onPick: () => commit(detachTab(desk, paneId, tab.id, { x: pane.x + 0.05, y: pane.y + 0.05 })),
          },
          { kind: "separator" },
          { kind: "item", label: "Close tab", onPick: () => commit(closeTab(desk, paneId, tab.id)) },
        ]);
      });
      entry.strip.append(button);
    }
    entry.strip.append(
      el("button", {
        class: "pane-btn pane-tab-add",
        text: "+",
        title: "Open another tab in this pane",
        onclick: (event: MouseEvent) => {
          const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
          deskMenu.open({ x: rect.left, y: rect.bottom + 2 }, [
            { kind: "title", label: "New tab" },
            ...newTabEntries((kind) => commit(addTab(desk, paneId, kind))),
          ]);
        },
      }),
    );
  }

  /**
   * A tab is a click, a drag along the strip, a drag into another strip, or a drag off
   * onto the desk: the browser's gesture, and it has to look like it. While dragging, a
   * ghost of the tab follows the pointer and a slot opens in whichever strip it is over;
   * released over a strip it goes there, over a pane's body it joins that pane, over the
   * desk it becomes a pane of its own under the pointer. A pane with one tab is that tab:
   * dragging it drags the pane, and dropping it on another pane merges the two.
   */
  function beginTabDrag(event: PointerEvent, paneId: string, tabId: string): void {
    if ((event.target as HTMLElement).closest("button") || event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const pane = desk.panes.find((candidate) => candidate.id === paneId);
    const tab = pane?.tabs.find((candidate) => candidate.id === tabId);
    if (!pane || !tab) return;
    const pressed = { x: event.clientX, y: event.clientY };
    const start = deskFraction(event.clientX, event.clientY);
    const alone = pane.tabs.length === 1;
    const origin = { x: pane.x, y: pane.y };
    let dragging = false;
    /** The strip the ghost is over, and the index the slot marks in it. */
    let target: { pane: string; index: number } | null = null;
    let overPane: HTMLElement | null = null;

    const clearMarks = () => {
      tabSlot.hidden = true;
      tabSlot.remove();
      overPane?.classList.remove("tab-target");
      overPane = null;
      target = null;
    };

    const move = (moveEvent: PointerEvent) => {
      if (!dragging) {
        if (Math.hypot(moveEvent.clientX - pressed.x, moveEvent.clientY - pressed.y) < 6) return;
        dragging = true;
        document.body.classList.add("dragging-tab");
        if (!alone) {
          tabGhost.textContent = EDITOR_LABELS[tab.editor];
          tabGhost.hidden = false;
          paneElements.get(paneId)?.root.querySelector(`[data-tab="${tabId}"]`)?.classList.add("lifted");
        }
      }
      const deskBox = deskRoot.getBoundingClientRect();
      if (alone) {
        // The pane itself follows, edges magnetic as from the title bar; the ghost would be
        // a second copy of it.
        dragPaneTo(paneId, origin, start, deskFraction(moveEvent.clientX, moveEvent.clientY));
        renderDesk();
      } else {
        tabGhost.style.left = `${moveEvent.clientX - deskBox.left - 12}px`;
        tabGhost.style.top = `${moveEvent.clientY - deskBox.top - 10}px`;
      }
      // What is under the pointer, the ghost and the dragged pane not counting.
      tabGhost.style.pointerEvents = "none";
      const dragged = alone ? paneElements.get(paneId)?.root : null;
      if (dragged) dragged.style.pointerEvents = "none";
      const under = document.elementFromPoint(moveEvent.clientX, moveEvent.clientY) as HTMLElement | null;
      if (dragged) dragged.style.pointerEvents = "";
      const strip = under?.closest<HTMLElement>(".pane-tabs") ?? null;
      const overRoot = under?.closest<HTMLElement>(".pane") ?? null;
      clearMarks();
      if (strip) {
        const stripPane = strip.closest<HTMLElement>(".pane")!.dataset.pane!;
        // The slot goes before the first tab whose middle the pointer has not passed.
        const tabs = [...strip.querySelectorAll<HTMLElement>(".pane-tab")].filter((node) => node.dataset.tab !== tabId);
        let index = tabs.length;
        for (let i = 0; i < tabs.length; i += 1) {
          const box = tabs[i].getBoundingClientRect();
          if (moveEvent.clientX < box.left + box.width / 2) {
            index = i;
            break;
          }
        }
        target = { pane: stripPane, index };
        tabSlot.hidden = false;
        if (index < tabs.length) tabs[index].before(tabSlot);
        else strip.querySelector(".pane-tab-add")?.before(tabSlot);
      } else if (overRoot && overRoot.dataset.pane !== paneId) {
        overPane = overRoot;
        overPane.classList.add("tab-target");
        target = { pane: overRoot.dataset.pane!, index: Number.MAX_SAFE_INTEGER };
      }
    };

    const up = (upEvent: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("dragging-tab");
      tabGhost.hidden = true;
      paneElements.get(paneId)?.root.querySelector(`[data-tab="${tabId}"]`)?.classList.remove("lifted");
      const landing = target;
      clearMarks();
      if (!dragging) {
        commit(bringToFront(activateTab(desk, paneId, tabId), paneId));
        return;
      }
      if (landing) {
        const index = landing.index === Number.MAX_SAFE_INTEGER ? undefined : landing.index;
        commit(bringToFront(moveTab(desk, paneId, tabId, landing.pane, index), landing.pane));
        return;
      }
      if (alone) {
        // The pane was moved; that is the whole gesture.
        commit(bringToFront(desk, paneId));
        return;
      }
      // Off the strip and onto the desk: a pane of its own, its title under the pointer.
      const at = deskFraction(upEvent.clientX, upEvent.clientY);
      const source = desk.panes.find((candidate) => candidate.id === paneId)!;
      commit(detachTab(desk, paneId, tabId, { x: at.x - source.w * 0.1, y: at.y - 0.015 }));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function createPane(id: string): PaneEntry {
    const strip = el("div", { class: "pane-tabs" });
    const title = el(
      "div",
      { class: "pane-title", title: "Drag to move. Drag to an edge to snap. Double-click to fill the free space around; again to fill the desk; again to come back." },
      strip,
      el("span", { class: "spacer" }),
      el("button", {
        class: "pane-btn",
        text: "⊞",
        title: "Put this pane in a half or a quarter of the desk",
        onclick: (event: MouseEvent) => {
          const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
          deskMenu.open({ x: rect.left, y: rect.bottom + 2 }, [
            { kind: "title", label: "Snap to" },
            ...SNAPS.map((snap) => ({
              kind: "item" as const,
              label: SNAP_LABELS[snap],
              onPick: () => commit(snapPane(desk, id, snap)),
            })),
            { kind: "separator" as const },
            {
              kind: "item" as const,
              label: "Restore",
              disabled: !desk.panes.find((pane) => pane.id === id)?.before,
              onPick: () => commit(restorePane(desk, id)),
            },
          ]);
        },
      }),
      el("button", {
        class: "pane-btn",
        text: "–",
        title: "Fold this pane down to its title bar, or open it again",
        onclick: () => commit(toggleCollapse(desk, id)),
      }),
      el("button", {
        class: "pane-btn",
        text: "▢",
        title: "Fill the desk, or come back from filling it",
        onclick: () => commit(toggleMaximise(desk, id)),
      }),
      el("button", {
        class: "pane-btn",
        text: "×",
        title: "Close this pane. Right-click the empty desk for a new one.",
        onclick: () => commit(removePane(desk, id)),
      }),
    );
    const content = el("div", { class: "pane-content" });
    const root = el("div", { class: "pane", dataset: { pane: id } }, title, content);

    // Touching a pane brings it to the front, the way windows behave.
    root.addEventListener("pointerdown", () => {
      if (desk.order.at(-1) === id) return;
      commit(bringToFront(desk, id));
    });

    // Double-clicking the title grows the pane into the free space around it, touching no
    // other pane; a second double-click fills the desk; a third comes back.
    title.addEventListener("dblclick", (event) => {
      if ((event.target as HTMLElement).closest(".pane-tab, button")) return;
      commit(cycleFit(desk, id));
    });

    // The title bar moves the pane; its tabs and controls do not.
    title.addEventListener("pointerdown", (event) => {
      if ((event.target as HTMLElement).closest(".pane-tab, button") || event.button !== 0) return;
      event.preventDefault();
      const pane = desk.panes.find((candidate) => candidate.id === id);
      if (!pane) return;
      const start = deskFraction(event.clientX, event.clientY);
      const pressed = { x: event.clientX, y: event.clientY };
      // How far along the title bar the grab is, so a snapped pane un-snaps under the hand.
      const grabAlong = (start.x - pane.x) / Math.max(pane.w, 1e-6);
      let origin = { x: pane.x, y: pane.y };
      let dragging = false;
      let snap: Snap | null = null;
      const move = (moveEvent: PointerEvent) => {
        // A click is not a drag. Nothing moves - and a snapped pane is not un-snapped - until
        // the pointer has actually travelled; a click on a filled pane's title used to
        // shrink it, which read as a bug because it was one.
        if (!dragging) {
          if (Math.hypot(moveEvent.clientX - pressed.x, moveEvent.clientY - pressed.y) < 4) return;
          dragging = true;
          document.body.classList.add("dragging-pane");
          const current = desk.panes.find((candidate) => candidate.id === id);
          if (current?.before) {
            desk = restorePane(desk, id);
            const restored = desk.panes.find((candidate) => candidate.id === id)!;
            desk = movePane(desk, id, start.x - restored.w * grabAlong, start.y);
            const placed = desk.panes.find((candidate) => candidate.id === id)!;
            origin = { x: placed.x, y: placed.y };
          }
        }
        const at = deskFraction(moveEvent.clientX, moveEvent.clientY);
        dragPaneTo(id, origin, start, at);
        snap = snapAt(at, occupiedSnaps(desk, id));
        showSnapGhost(snap);
        renderDesk();
      };
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        document.body.classList.remove("dragging-pane");
        showSnapGhost(null);
        if (!dragging) return;
        if (snap) {
          desk = snapPane(desk, id, snap);
          renderDesk();
        }
        saveLayout();
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    });

    // Eight handles: four edges and four corners, as every window has.
    for (const edge of ["n", "s", "e", "w", "ne", "nw", "se", "sw"] as Edge[]) {
      const handle = el("div", { class: `pane-handle ${edge}` });
      handle.addEventListener("pointerdown", (event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        let last = deskFraction(event.clientX, event.clientY);
        document.body.classList.add("resizing-pane");
        const move = (moveEvent: PointerEvent) => {
          const at = deskFraction(moveEvent.clientX, moveEvent.clientY);
          desk = resizePane(desk, id, edge, at.x - last.x, at.y - last.y);
          last = at;
          // The dragged side is pulled onto a nearby edge; the opposite side stays put.
          const resized = desk.panes.find((candidate) => candidate.id === id)!;
          const { tx, ty } = magnetReach();
          desk = placePane(desk, id, magnetise(desk, id, resized, movingSides(edge), tx, ty));
          renderDesk();
        };
        const up = () => {
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", up);
          document.body.classList.remove("resizing-pane");
          saveLayout();
        };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", up);
      });
      root.append(handle);
    }
    return { root, strip, content, shown: null };
  }

  /**
   * Files opened into the file viewer, newest first: a paper, a figure, a slide deck kept
   * beside the structure. Renders are in its list as well, without being opened.
   */
  const viewedFiles: string[] = [];

  /** Opens a file in a file viewer, putting one on the desk if there is none. */
  function viewFile(path: string): void {
    const at = viewedFiles.indexOf(path);
    if (at >= 0) viewedFiles.splice(at, 1);
    viewedFiles.unshift(path);
    const showing = desk.panes.some((pane) => activeTab(pane).editor === "image");
    if (!showing) commit(addPane(desk, "image"));
    for (const host of imageAreas.values()) {
      host.dataset.path = path;
      if (host.isConnected) drawImageArea(host);
    }
  }

  async function chooseFileToView(): Promise<void> {
    const picked = await openFileDialog({ multiple: false, directory: false, title: "Open a file to view" });
    if (typeof picked === "string") viewFile(picked);
  }

  /**
   * The file viewer: a render, a picture, a PDF, a Word document, a slide deck or plain
   * text, whichever was chosen. What it can draw it draws; the rest it can hand to the
   * application the file belongs to.
   */
  function drawImageArea(host: HTMLElement): void {
    const renders = graph.nodes
      .filter((node): node is Extract<GraphNode, { type: "image" }> => node.type === "image" && node.path !== null)
      .map((node) => node.path as string);
    const files = [...viewedFiles, ...renders.filter((path) => !viewedFiles.includes(path))];
    const chosen = host.dataset.path && files.includes(host.dataset.path) ? host.dataset.path : (files[0] ?? null);
    const name = (path: string) => path.split("/").pop() ?? path;
    const bar = el(
      "div",
      { class: "file-bar" },
      files.length > 1
        ? el(
            "select",
            {
              class: "image-source",
              title: "Which file to show",
              onchange: (event: Event) => {
                host.dataset.path = (event.target as HTMLSelectElement).value;
                drawImageArea(host);
              },
            },
            ...files.map((path) => el("option", { value: path, text: name(path), selected: path === chosen })),
          )
        : el("span", { class: "file-name", text: chosen ? name(chosen) : "" , title: chosen ?? "" }),
      el("span", { class: "spacer" }),
      chosen
        ? el("button", {
            class: "file-btn",
            text: "Open in its app",
            title: "Open this file in the application it belongs to",
            onclick: () => void openWithSystem(chosen),
          })
        : null,
      el("button", { class: "file-btn", text: "Open file...", title: "Show another file here: a picture, PDF, Word, PowerPoint, Excel or text", onclick: () => void chooseFileToView() }),
    );
    const body = el("div", { class: "file-body" });
    replace(host, bar, body);
    if (!chosen) {
      replace(body, el("div", { class: "properties-empty", text: "No file yet. Render something, open a file, or drop one here." }));
      return;
    }
    // A render is shown straight away, and redrawn when it is written again.
    if (renders.includes(chosen)) {
      replace(body, el("img", { class: "image-view", src: imageUrl(chosen), alt: name(chosen), title: chosen }));
      return;
    }
    replace(body, el("div", { class: "properties-empty", text: "Opening..." }));
    void api
      .filePreview(chosen)
      .then((preview) => {
        if (host.dataset.path && host.dataset.path !== chosen) return;
        replace(body, filePreviewElement(preview, chosen));
      })
      .catch((error: unknown) => replace(body, el("div", { class: "properties-empty", text: String(error) })));
  }

  function filePreviewElement(preview: FilePreview, path: string): HTMLElement {
    switch (preview.kind) {
      case "image":
      case "picture":
        return el("img", { class: "image-view", src: convertFileSrc(preview.path), alt: path, title: path });
      case "pdf":
        return pdfView(preview.path);
      case "html": {
        // Sandboxed with nothing allowed: a document's page is shown, never run.
        const frame = el("iframe", { class: "file-frame page", sandbox: "", title: path });
        frame.srcdoc = preview.html;
        return frame;
      }
      case "text": {
        const view = el("textarea", { class: "text-view", readonly: "true", spellcheck: "false" });
        view.value = preview.cut ? `${preview.text}\n\n... (only the first megabyte is shown)` : preview.text;
        return view;
      }
      case "other":
        return el(
          "div",
          { class: "properties-empty" },
          el("div", { text: "This kind of file cannot be shown here." }),
          el("button", { class: "btn small", text: "Open in its app", onclick: () => void openWithSystem(path) }),
        );
    }
  }

  async function openWithSystem(path: string): Promise<void> {
    try {
      await openWithDefault(path);
    } catch (error) {
      notice = `could not open ${path.split("/").pop()}: ${String(error)}`;
      renderHud();
    }
  }

  /** A text area: the active molecule's sequences as FASTA. The start of a text editor. */
  async function drawTextArea(host: HTMLElement): Promise<void> {
    const structure = structures[0];
    if (!structure) {
      replace(host, el("div", { class: "properties-empty", text: "Nothing loaded." }));
      return;
    }
    try {
      const sequences = await api.structureSequence(structure.id);
      const fasta = sequences
        .map(
          (entry) =>
            `>${(structure.entry_id || structure.title).toUpperCase()}_${entry.chain}\n` +
            entry.codes.replace(/(.{60})/g, "$1\n"),
        )
        .join("\n");
      // A textarea's text is its content, not a `value` attribute - set as an attribute it
      // shows nothing, which is what the first version did.
      const view = el("textarea", {
        class: "text-view",
        readonly: "true",
        spellcheck: "false",
      });
      view.value = fasta;
      replace(host, view);
    } catch (error) {
      replace(host, el("div", { class: "properties-empty", text: String(error) }));
    }
  }

  /** Redraws the panes whose content follows the graph or the structures. */
  function refreshPanes(): void {
    for (const host of imageAreas.values()) if (host.isConnected) drawImageArea(host);
    for (const host of textAreas.values()) if (host.isConnected) void drawTextArea(host);
  }

  /**
   * The bar under the viewport: at which level a click selects.
   *
   * Thin, like VS Code's status bar, and about one thing. Chain is what the graph has
   * nodes for; residue and atom are the finer levels the same click resolves to - see
   * `pick_target` - and choosing one here changes what gets outlined and named.
   */
  // --- modes -------------------------------------------------------------------------
  //
  // After Blender's 3D view, read from its own source: the mode is the first thing in the
  // viewport's header (a menu, `object.mode_set`), and Ctrl+Tab opens the same choice as a
  // pie at the pointer (`VIEW3D_MT_object_mode_pie` - `pie.operator_enum`, which lays the
  // first item to the west and the second to the east). Alt+Z, its X-ray toggle, flips
  // straight between the two, since the graph view is this app's X-ray.

  /**
   * The mode buttons, at the right end of the viewport's header.
   *
   * Where Blender's 3D view keeps its shading buttons, and drawn the way it draws them -
   * `row.prop(shading, "type", text="", expand=True)`: one row of icon-only buttons, the
   * current one pressed in, the name in the tooltip.
   */
  function drawModeHeader(): void {
    replace(
      modeHeader,
      el(
        "div",
        { class: "mode-row", role: "radiogroup", "aria-label": "Viewport mode" },
        ...MODES.map((mode) =>
          el(
            "button",
            {
              class: `mode-btn${mode.id === modeNow().id ? " active" : ""}`,
              role: "radio",
              "aria-checked": String(mode.id === modeNow().id),
              "aria-label": mode.label,
              title: `${mode.label}: ${mode.about}. Press V over the viewport for the View chain.`,
              onclick: (event: Event) => {
                event.stopPropagation();
                setMode(mode.id);
              },
            },
            el("span", { class: `mode-glyph ${mode.id}` }),
          ),
        ),
      ),
    );
  }

  /** Where the pointer last was over the viewport, so a key can open something there. */
  let pointerAt: { x: number; y: number } | null = null;
  viewerMain.addEventListener("pointermove", (event) => (pointerAt = { x: event.clientX, y: event.clientY }));
  viewerMain.addEventListener("pointerleave", () => (pointerAt = null));

  /** How far the pointer has to leave the centre before it points at anything, in pixels. */
  const PIE_THRESHOLD = 12;
  /** A key let go sooner than this was tapped, not held, and the pie stays open. */
  const PIE_TAP_MS = 200;

  /** The key that opened the pie that is up; empty for a step further down a chain. */
  let pieKey = "";
  /** The open pie, if one is: where it is centred and which way the pointer has gone. */
  let pie: { element: HTMLElement; center: { x: number; y: number }; toward: number | null; close: () => void } | null = null;

  /** One choice on a pie. */
  interface PieItem {
    label: string;
    /** A mode's mark, where it has one. */
    glyph?: string;
    /** A colour to show beside the label, `#rrggbb`. */
    swatch?: string;
    current?: boolean;
    /** What choosing it does - unless it leads to another pie. */
    pick?: () => void;
    /** The pie it leads to: the next step of the chain, opened where this one was. */
    next?: () => Pie | null;
  }

  interface Pie {
    title: string;
    items: PieItem[];
  }

  /**
   * Where a pie's items go, in the order Blender fills one: west, east, south, north, then
   * the four diagonals. Each as a direction on screen (y down) and which way its box
   * grows from that point.
   */
  const PIE_PLACES: { x: number; y: number }[] = [
    { x: -1, y: 0 },
    { x: 1, y: 0 },
    { x: 0, y: 1 },
    { x: 0, y: -1 },
    { x: -0.72, y: -0.72 },
    { x: 0.72, y: -0.72 },
    { x: -0.72, y: 0.72 },
    { x: 0.72, y: 0.72 },
  ];
  const PIE_RADIUS = 96;

  function openPie(center: { x: number; y: number }, heldKey: string, title: string, choices: PieItem[]): void {
    const openedAt = performance.now();
    pie?.close();
    pieKey = heldKey;
    viewportMenu.close();
    const shown = choices.slice(0, PIE_PLACES.length);
    const items = shown.map((choice, index) => {
      const place = PIE_PLACES[index];
      // Anchored on the side facing the centre, so a long label grows away from it.
      const shift = place.x < -0.3 ? "-100%" : place.x > 0.3 ? "0" : "-50%";
      return el(
        "button",
        {
          class: `pie-item${choice.current ? " current" : ""}`,
          style: `left:${place.x * PIE_RADIUS}px;top:${place.y * PIE_RADIUS}px;transform:translate(${shift},-50%)`,
          onclick: (event: Event) => {
            event.stopPropagation();
            choose(index);
          },
        },
        choice.glyph ? el("span", { class: `mode-glyph ${choice.glyph}` }) : null,
        choice.swatch ? el("span", { class: "pie-swatch", style: `background:${choice.swatch}` }) : null,
        el("span", { text: choice.label }),
        choice.next ? el("span", { class: "pie-more", text: "›" }) : null,
      );
    });
    const ring = el("div", { class: "pie-ring" });
    const element_ = el("div", { class: "pie", style: `left:${center.x}px;top:${center.y}px` }, el("div", { class: "pie-title", text: title }), ring, ...items);
    document.body.append(element_);

    const choose = (index: number) => {
      close();
      const chosen = shown[index];
      const onward = chosen.next?.();
      // A step with more under it opens the next pie in the same place. That one stays up
      // for a click or a flick-and-click: the key that started the chain is already gone.
      if (onward) openPie(center, "", `${title} › ${onward.title}`, onward.items);
      else chosen.pick?.();
    };
    // Past a small dead zone the pointer's direction picks the item nearest that way, and
    // the ring turns to show it - a pie is chosen by where you go, not by what you hit.
    const move = (event: PointerEvent) => {
      const dx = event.clientX - center.x;
      const dy = event.clientY - center.y;
      const far = Math.hypot(dx, dy);
      let toward: number | null = null;
      if (far >= PIE_THRESHOLD) {
        let best = -Infinity;
        shown.forEach((_, index) => {
          const place = PIE_PLACES[index];
          const along = (dx * place.x + dy * place.y) / (far * Math.hypot(place.x, place.y));
          if (along > best) {
            best = along;
            toward = index;
          }
        });
      }
      if (pie) pie.toward = toward;
      items.forEach((item, index) => item.classList.toggle("toward", index === toward));
      ring.style.setProperty("--pie-angle", `${Math.atan2(dy, dx)}rad`);
      ring.classList.toggle("aimed", toward !== null);
    };
    const down = (event: PointerEvent) => {
      if ((event.target as HTMLElement | null)?.closest(".pie-item")) return;
      // A click away from the items takes the one being pointed towards, as Blender's does;
      // a click on the centre is a change of mind.
      event.preventDefault();
      event.stopPropagation();
      if (pie?.toward !== null && pie?.toward !== undefined) choose(pie.toward);
      else close();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
      }
    };
    // Two ways through a pie, told apart by how long the key was down. Held: press, move
    // towards an item, let go - letting go takes it. Tapped: the pie stays up and waits
    // for a click, on an item or anywhere in its direction.
    const keyup = (event: KeyboardEvent) => {
      if (event.code !== heldKey) return;
      const held = performance.now() - openedAt > PIE_TAP_MS;
      // Only a deliberate hold-and-flick is taken on release. A tap leaves the pie up,
      // whatever the pointer did in that moment, and so does a hold that went nowhere:
      // a pie that vanishes when the key comes up has to be held open to be read.
      if (held && pie?.toward !== null && pie?.toward !== undefined) choose(pie.toward);
    };
    function close(): void {
      window.removeEventListener("pointermove", move, true);
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("keyup", keyup, true);
      element_.remove();
      pie = null;
    }
    window.addEventListener("pointermove", move, true);
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("keyup", keyup, true);
    pie = { element: element_, center, toward: null, close };
  }

  /** The target, in a few words, for the chip and for a pie's title. */
  function targetWords(): string {
    if (!actionTarget) return "everything";
    const chains = actionTarget.chains;
    // Every chain of a molecule is the molecule, and is called that.
    const whole = structures.find((entry) => entry.id === actionTarget!.structureId);
    if (actionTarget.level === "chain" && whole && chains.length === whole.chains.length && chains.length > 1) {
      return `molecule ${(whole.entry_id || whole.title || "").toUpperCase()}`.trim();
    }
    const which = chains.length <= 3 ? chains.join(", ") : `${chains.length} chains`;
    // A selection finer than a chain is acted on as itself: its residues get a feature.
    return actionTarget.level === "chain" ? `chain ${which}` : actionTarget.label;
  }

  /**
   * The readout of what is selected for the next action, in the viewport's header.
   *
   * Always there, so there is never a question of what a pie is about to change: the
   * selection when there is one, "everything" when there is not, and a cross to go back
   * to everything.
   */
  function drawTarget(): void {
    replace(
      targetChip,
      el("span", { class: "target-key", text: "Acts on" }),
      el("span", { class: `target-what${actionTarget ? " chosen" : ""}`, text: targetWords(), title: actionTarget?.label ?? "Nothing is selected, so an action changes every molecule" }),
      actionTarget
        ? el("button", {
            class: "target-clear",
            text: "×",
            title: "Clear the selection: act on everything",
            onclick: () => void applySelection(null, "replace"),
          })
        : null,
    );
  }

  /**
   * Restyles what is selected - or, with nothing selected, everything.
   *
   * A selected chain gets a style slot of its own if it was sharing one, so changing it
   * leaves its neighbours as they were; the same thing `colour_chain` does for an agent.
   */
  /**
   * The selected residues as runs: a chain and a span of author numbers each, which is
   * how a feature names them.
   */
  async function selectedRuns(polymerOnly = true): Promise<{ chain: string; from: number; to: number }[]> {
    const view = currentView;
    if (!view) return [];
    const map = await api.structureMap(view.structureId);
    // Which residues the selection is, whatever level it was made at. A selection of
    // residues is its own indices - the summary only lists residues for an atom view,
    // and reading it for this one found none, so an action on picked residues did nothing.
    const wanted = new Set<number>();
    if (view.level === "residue") {
      for (const index of view.indices) wanted.add(index);
    } else if (view.level === "chain") {
      for (const index of view.indices) for (const residue of map.chains[index]?.residue_indices ?? []) wanted.add(residue);
    } else {
      const summary = await api.describeView(view.structureId, { level: view.level, indices: view.indices });
      for (const index of summary.residue_indices) wanted.add(index);
    }
    const runs: { chain: string; from: number; to: number }[] = [];
    for (const chain of map.chains) {
      let open: { chain: string; from: number; to: number } | null = null;
      chain.residue_indices.forEach((residue, row) => {
        const counts = !polymerOnly || chain.kinds[row] === "a" || chain.kinds[row] === "n";
        if (counts && wanted.has(residue)) {
          const number = chain.numbers[row];
          if (open && number === open.to + 1) open.to = number;
          else {
            if (open) runs.push(open);
            open = { chain: chain.id, from: number, to: number };
          }
        } else if (open) {
          runs.push(open);
          open = null;
        }
      });
      if (open) runs.push(open);
    }
    return runs;
  }

  // --- objects ---------------------------------------------------------------------------
  //
  // PyMOL's named selections. Whatever is selected can be given a name and kept: it is
  // then a row under its molecule in the scene's list, picked again with a click, and
  // acted on like anything else that is selected - shown, hidden, coloured, zoomed to,
  // searched around - as a thing of its own.

  async function makeObject(): Promise<void> {
    const view = currentView;
    const structure = view ? structures.find((entry) => entry.id === view.structureId) : null;
    const molecule = structure ? moleculeNodeFor(structure) : null;
    if (!view || !molecule || molecule.type !== "molecule") {
      editor.setHoverDetail("Select something first, then make an object of it.");
      return;
    }
    const runs = await selectedRuns(false);
    if (runs.length === 0) {
      editor.setHoverDetail("That selection has no residues to make an object of.");
      return;
    }
    const taken = new Set((molecule.objects ?? []).map((entry) => entry.name));
    let number = 1;
    while (taken.has(`obj${String(number).padStart(2, "0")}`)) number += 1;
    const name = await askText({ title: "Name this object", initial: `obj${String(number).padStart(2, "0")}`, accept: "Create" });
    if (!name) return;
    molecule.objects = [...(molecule.objects ?? []).filter((entry) => entry.name !== name), { name, runs }];
    editor.setGraph(graph);
    save();
    editor.setHoverDetail(`${name} made from the selection. It is under ${(structure?.entry_id || "its molecule").toUpperCase()} in the scene list.`);
  }

  /** Selects a named object's residues. */
  async function selectObject(moleculeId: string, name: string): Promise<void> {
    const entry = molecules().find(({ node }) => node.id === moleculeId);
    const found = entry && entry.node.type === "molecule" ? (entry.node.objects ?? []).find((object) => object.name === name) : undefined;
    if (!entry || !found) return;
    const map = await api.structureMap(entry.structure.id);
    const indices: number[] = [];
    for (const chain of map.chains) {
      const mine = found.runs.filter((run) => run.chain === chain.id);
      if (mine.length === 0) continue;
      chain.residue_indices.forEach((residue, row) => {
        const number = chain.numbers[row];
        if (mine.some((run) => number >= run.from && number <= run.to)) indices.push(residue);
      });
    }
    if (indices.length === 0) {
      editor.setHoverDetail(`${name} has no residues left in this molecule.`);
      return;
    }
    await applySelection({ structureId: entry.structure.id, level: "residue", indices }, "replace");
    editor.setHoverDetail(`${name}: ${count(indices.length, "residue")}.`);
  }

  function removeObject(moleculeId: string, name: string): void {
    const node = graph.nodes.find((candidate) => candidate.id === moleculeId);
    if (!node || node.type !== "molecule") return;
    node.objects = (node.objects ?? []).filter((entry) => entry.name !== name);
    editor.setGraph(graph);
    save();
  }

  /**
   * Says something about the selected residues: a feature of the molecule, laid over its
   * chains' styles. Where a selection is finer than a chain this is what an action does.
   */
  async function featureTarget(feature: Omit<Feature, "chain" | "from" | "to">, what: string): Promise<void> {
    const target = actionTarget;
    const structure = target ? structures.find((entry) => entry.id === target.structureId) : null;
    const molecule = structure ? moleculeNodeFor(structure) : null;
    if (!target || !molecule || molecule.type !== "molecule") return;
    const runs = await selectedRuns();
    if (runs.length === 0) {
      editor.setHoverDetail("That selection has no polymer residues to style.");
      return;
    }
    // One feature per chain, carrying all its runs: a selection scattered over a capsid -
    // its hydrophobic core, say - is a hundred thousand runs, and one feature for each was
    // most of what the world weighed.
    const byChain = new Map<string, { from: number; to: number }[]>();
    for (const run of runs) byChain.set(run.chain, [...(byChain.get(run.chain) ?? []), run]);
    const added: Feature[] = [...byChain].map(([chain, spans]) => ({
      chain,
      from: spans[0].from,
      to: spans[spans.length - 1].to,
      ...(spans.length > 1 ? { ranges: spans.map((span) => (span.from === span.to ? `${span.from}` : `${span.from}-${span.to}`)).join(",") } : {}),
      ...feature,
    }));
    molecule.features = [...(molecule.features ?? []), ...added];
    editor.setHoverDetail(`${what} - ${target.label}.`);
    await commitGraph();
  }

  /** Whether the next action is about residues rather than whole chains. */
  const residueTarget = () => actionTarget !== null && actionTarget.level !== "chain";

  /** A style in a few words, for a slot that was made rather than named. */
  function styleWords(style: Style): string {
    const shown = style.representations.length ? style.representations.map((entry) => REPRESENTATION_LABELS[entry] ?? entry).join(" + ") : "Hidden";
    const coloured = style.solid ? unitToHex(style.solid) : (COLOR_SCHEME_LABELS[style.scheme] ?? style.scheme);
    return `${shown} · ${coloured}`;
  }

  /**
   * Leaves a molecule with the slots it uses, once each.
   *
   * Slots that draw alike are one slot; a slot no chain draws with is dropped; and a slot
   * made by an action, or left named after one chain, is named for what it looks like. The
   * list in the properties panel is then the molecule's styles, not a log of what was done.
   */
  function tidySlots(molecule: Extract<GraphNode, { type: "molecule" }>, chains: Extract<GraphNode, { type: "chain" }>[]): void {
    const using = (at: number) => chains.filter((chain) => !chain.style && (chain.slot ?? molecule.active_slot) === at);
    const kept: StyleSlot[] = [];
    const moved = new Map<number, number>();
    molecule.slots.forEach((slot, at) => {
      const twin = kept.findIndex((entry) => JSON.stringify(entry.style) === JSON.stringify(slot.style));
      if (twin >= 0) {
        moved.set(at, twin);
        return;
      }
      // Kept if a chain draws with it - or if it is all there is.
      if (using(at).length === 0 && molecule.slots.length > 1 && !(kept.length === 0 && at === molecule.slots.length - 1)) return;
      moved.set(at, kept.length);
      kept.push(slot);
    });
    if (kept.length === 0) kept.push(molecule.slots[0]);
    for (const chain of chains) {
      if (chain.style) continue;
      chain.slot = moved.get(chain.slot ?? molecule.active_slot) ?? 0;
    }
    molecule.active_slot = moved.get(molecule.active_slot) ?? 0;
    const taken = new Set<string>();
    molecule.slots = kept.map((slot) => {
      const made = slot.name === "" || / own$/.test(slot.name) || /^[A-Za-z0-9_]+ #[0-9a-f]{6}$/.test(slot.name);
      let name = made ? styleWords(slot.style) : slot.name;
      // Two that look different can still read the same in three words.
      for (let again = 2; taken.has(name); again += 1) name = `${made ? styleWords(slot.style) : slot.name} ${again}`;
      taken.add(name);
      return { ...slot, name };
    });
  }

  function restyleTarget(change: (style: Style) => Style, what: string): void {
    const target = actionTarget;
    if (!target) {
      restyleAll(change, `${what} - everything.`);
      return;
    }
    const structure = structures.find((entry) => entry.id === target.structureId);
    const molecule = structure ? moleculeNodeFor(structure) : null;
    if (!structure || !molecule || molecule.type !== "molecule") return;
    const chainsOf = graph.nodes.filter(
      (candidate): candidate is Extract<GraphNode, { type: "chain" }> =>
        candidate.type === "chain" &&
        graph.links.some((link) => link.from === molecule.id && link.to === candidate.id && link.socket === "Molecule"),
    );
    const slotOf = (chain: Extract<GraphNode, { type: "chain" }>) => chain.slot ?? molecule.active_slot;
    const targeted = chainsOf.filter((chain) => target.chains.includes(chain.chain));
    // By the slot they draw with now: chains that shared a style and are changed together
    // go on sharing one. A slot per chain turned a twelve-chain molecule into twelve
    // copies of the same style, each with a chain's name on it.
    const bySlot = new Map<number, Extract<GraphNode, { type: "chain" }>[]>();
    for (const chain of targeted) {
      if (chain.style) {
        // A style the chain carried itself, from before slots: it gets a slot now.
        molecule.slots.push({ name: "", style: change(chain.style) });
        chain.slot = molecule.slots.length - 1;
        chain.style = null;
        continue;
      }
      bySlot.set(slotOf(chain), [...(bySlot.get(slotOf(chain)) ?? []), chain]);
    }
    for (const [at, group] of bySlot) {
      const current = molecule.slots[at]?.style ?? molecule.slots[0].style;
      const others = chainsOf.some((chain) => !chain.style && slotOf(chain) === at && !group.includes(chain));
      if (others) {
        molecule.slots.push({ name: "", style: change(current) });
        for (const chain of group) chain.slot = molecule.slots.length - 1;
      } else {
        molecule.slots[at] = { ...molecule.slots[at], style: change(current) };
      }
    }
    tidySlots(molecule, chainsOf);
    editor.setHoverDetail(`${what} - ${targetWords()}.`);
    void commitGraph();
  }

  /**
   * Restyles everything on screen: every slot of every molecule.
   *
   * The pies are the quick way to say "show it as sticks" or "colour by chain" about the
   * whole scene, the way one line in PyMOL does; a style for one chain is still made in
   * the node graph, where it can be seen and kept.
   */
  function restyleAll(change: (style: Style) => Style, what: string): void {
    const all = moleculeNodes();
    if (all.length === 0) {
      editor.setHoverDetail("Nothing is open to restyle.");
      return;
    }
    for (const node of all) node.slots = node.slots.map((slot) => ({ ...slot, style: change(slot.style) }));
    editor.setHoverDetail(what);
    void commitGraph();
  }

  /** Which representations and scheme every slot agrees on, for marking the current item. */
  function sharedStyle(): { representations: string | null; scheme: string | null } {
    const styles = moleculeNodes().flatMap((node) => node.slots.map((slot) => slot.style));
    const same = <T,>(values: T[]) => (values.length > 0 && values.every((value) => value === values[0]) ? values[0] : null);
    return {
      representations: same(styles.map((style) => [...style.representations].sort().join("+"))),
      scheme: same(styles.map((style) => (style.solid ? "solid" : style.scheme))),
    };
  }

  const SHOW_AS: { label: string; representations: Representation[] }[] = [
    { label: "Cartoon", representations: ["cartoon"] },
    { label: "Sticks", representations: ["sticks"] },
    { label: "Ball and stick", representations: ["ball-and-stick"] },
    { label: "Spacefill", representations: ["spacefill"] },
    { label: "Backbone", representations: ["backbone"] },
    { label: "Cartoon + sticks", representations: ["cartoon", "sticks"] },
    { label: "Cartoon in glass", representations: ["cartoon", "glass"] },
  ];
  const COLOUR_BY: { label: string; scheme: ColorScheme }[] = [
    { label: "Chain", scheme: "chain" },
    { label: "Secondary structure", scheme: "secondary" },
    { label: "Element", scheme: "element" },
    { label: "Rainbow", scheme: "rainbow" },
    { label: "Chainbow", scheme: "chainbow" },
    { label: "B-factor", scheme: "b-factor" },
    { label: "Confidence", scheme: "confidence" },
    { label: "Residue type", scheme: "residue-kind" },
  ];

  // --- the three chains ---------------------------------------------------------------
  //
  // S, A, V: Selection, Action, View. Each key opens a pie, and an item with more under
  // it opens the next pie where the first one was - a chain, walked a step at a time, the
  // way Blender's menus nest. Selection says what; Action does something to it; View is
  // about the viewport and nothing in it.

  /** Up to eight items; past that the last place is "More" and leads to the rest. */
  function paged(title: string, items: PieItem[]): Pie {
    if (items.length <= PIE_PLACES.length) return { title, items };
    const head = items.slice(0, PIE_PLACES.length - 1);
    const rest = items.slice(PIE_PLACES.length - 1);
    return { title, items: [...head, { label: "More", next: () => paged(title, rest) }] };
  }

  function selectionPie(): Pie {
    const open = molecules();
    const chainsOf = (structure: StructureInfo): Pie =>
      paged(structure.entry_id || "Molecule", [
        {
          label: "Whole molecule",
          pick: () => void applySelection({ structureId: structure.id, level: "chain", indices: structure.chains.map((_, index) => index) }, "replace"),
        },
        ...structure.chains
          .map((chain, index) => ({ chain, index }))
          .filter(({ chain }) => chain.entity_type === "polymer" || chain.entity_type === "branched")
          .map(({ chain, index }) => ({
            label: `Chain ${chain.id}`,
            current: actionTarget?.structureId === structure.id && actionTarget.level === "chain" && actionTarget.chains.length === 1 && actionTarget.chains[0] === chain.id,
            pick: () => void applySelection({ structureId: structure.id, level: "chain" as const, indices: [index] }, "replace"),
          })),
      ]);
    // The two that are always there come first, so they are always in the same place:
    // with the molecules ahead of them every item moved whenever a molecule was added.
    return paged("Selection", [
      { label: "Everything", current: actionTarget === null, pick: () => void applySelection(null, "replace") },
      {
        label: "Click selects",
        next: () => ({
          title: "Click selects",
          items: LEVELS.map((level) => ({ label: level.label, current: pickLevel === level.id, pick: () => setPickLevel(level.id) })),
        }),
      },
      ...open.map(({ structure }) => ({
        label: (structure.entry_id || structure.title || "Molecule").toUpperCase().slice(0, 18),
        next: () => chainsOf(structure),
      })),
    ]);
  }

  /** Selects what a condition matches in the molecule the target is in, or the only one. */
  async function find(what: string, where: SelectWhere, level: "chain" | "residue"): Promise<void> {
    const structure =
      (actionTarget ? structures.find((entry) => entry.id === actionTarget!.structureId) : null) ?? structures[0];
    if (!structure) {
      editor.setHoverDetail("Nothing is open to look in.");
      return;
    }
    try {
      const view = await api.selectView(structure.id, { level, where });
      if (view.indices.length === 0) {
        editor.setHoverDetail(`No ${what} in ${structure.entry_id || "this molecule"}.`);
        return;
      }
      await applySelection({ structureId: structure.id, level: view.level, indices: view.indices }, "replace");
      editor.setHoverDetail(`Found ${what}.`);
    } catch (error) {
      editor.setHoverDetail(String(error));
    }
  }

  function actionPie(): Pie {
    const shared = sharedStyle();
    const onResidues = residueTarget();
    const REPS: [Representation, string][] = [
      ["cartoon", "Cartoon"],
      ["sticks", "Sticks"],
      ["ball-and-stick", "Ball and stick"],
      ["spacefill", "Spacefill"],
      ["backbone", "Backbone"],
      ["glass", "Glass"],
    ];
    const drawn = (representation: Representation) => !onResidues && shared.representations !== null && shared.representations.split("+").includes(representation);
    // PyMOL's three verbs: `show` adds a representation, `hide` takes one away, `as`
    // leaves only the one named.
    const show: Pie = {
      title: "Show (add)",
      items: REPS.map(([representation, label]) => ({ label, current: drawn(representation), pick: () => showAlso(representation, label) })),
    };
    const hide: Pie = {
      title: "Hide",
      items: [
        ...REPS.map(([representation, label]) => ({ label, pick: () => hideOnly(representation, label) })),
        { label: "Everything", pick: () => RUN.hide() },
      ],
    };
    const only: Pie = {
      title: "Show as (only)",
      items: SHOW_AS.map((entry) => ({
        label: entry.label,
        current: !onResidues && shared.representations === [...entry.representations].sort().join("+"),
        pick: () => showAs([...entry.representations], entry.label.toLowerCase()),
      })),
    };
    // A scheme is a rule over a whole chain; a few residues are given a colour instead -
    // the preferred palette's, since those are the colours this user's figures are in.
    const palette = PALETTES.find((entry) => entry.id === preferences().palette) ?? PALETTES[0];
    /** Asks for a colour by name or code. */
    const custom = (apply: (hex: string) => void) =>
      void askText({
        title: "Custom colour",
        placeholder: "A name or a hex code: coral, teal, #ff7f50",
        accept: "Colour",
        check: (value) => (parseColour(value) ? null : "Not a colour I know. Try a name such as coral, or a code such as #ff7f50."),
      }).then((typed) => {
        const hex = typed ? parseColour(typed) : null;
        if (hex) apply(hex);
      });
    // South is the third place a pie fills: the bottom, where Custom goes on both.
    const atBottom = <T,>(items: T[], bottom: T): T[] => [...items.slice(0, 2), bottom, ...items.slice(2)];
    const colour: Pie = onResidues
      ? paged(
          "Colour",
          atBottom(
            palette.colors.slice(0, 7).map((hex) => ({
              label: colourName(hex),
              swatch: hex,
              pick: () => void featureTarget({ colour: hexToUnit(hex) }, `Coloured ${colourName(hex)}`),
            })) as PieItem[],
            { label: "Custom", pick: () => custom((hex) => void featureTarget({ colour: hexToUnit(hex) }, `Coloured ${colourName(hex)}`)) },
          ),
        )
      : paged(
          "Colour by",
          atBottom(
            COLOUR_BY.map((entry) => ({
              label: entry.label,
              current: shared.scheme === entry.scheme,
              pick: () =>
                restyleTarget(
                  (style) => ({ ...style, scheme: entry.scheme, solid: null, categorical: preferences().palette, ramp: preferences().ramp }),
                  `Coloured by ${entry.label.toLowerCase()}`,
                ),
            })) as PieItem[],
            // One flat colour for the whole target, in place of a rule.
            { label: "Custom", pick: () => custom((hex) => restyleTarget((style) => ({ ...style, solid: hexToUnit(hex) }), `Coloured ${colourName(hex)}`)) },
          ),
        );
    // Contacts on the four sides, sets of residues on the diagonals, and the rest a step
    // further: seven places and then More.
    const finds = paged("Find", [
      { label: "Hydrogen bonds", current: shownEdges.has("polar"), pick: () => void toggleContacts("polar") },
      { label: "Salt bridges", current: shownEdges.has("salt-bridge"), pick: () => void toggleContacts("salt-bridge") },
      { label: "Hydrophobic contacts", current: shownEdges.has("hydrophobic"), pick: () => void toggleContacts("hydrophobic") },
      { label: "Pi stacking", current: shownEdges.has("pi-stacking"), pick: () => void toggleContacts("pi-stacking") },
      { label: "Hydrophobic core", pick: () => void selectPreset("core") },
      { label: "Binding site", pick: () => void selectPreset("site") },
      { label: "Interface", pick: () => void selectPreset("interface") },
      ...(shownEdges.size > 0 ? [{ label: "Hide contacts", pick: () => hideContacts() }] : []),
      { label: "Ligands", pick: () => void selectPreset("ligands") },
      { label: "Helices", pick: () => void find("helices", { secondary: ["H"] }, "residue") },
      { label: "Strands", pick: () => void find("strands", { secondary: ["E"] }, "residue") },
      { label: "Waters", pick: () => void find("waters", { entity_type: ["water"] }, "chain") },
    ]);
    return {
      title: `Action · ${targetWords()}`,
      // Laid out as pairs across the centre, so a direction is learnt once: show to the
      // left and hide to the right, colour up and find down, and the less used four on
      // the diagonals with the one that undoes things furthest from the rest.
      items: [
        { label: "Show", next: () => show },
        { label: "Hide", next: () => hide },
        { label: "Find", next: () => finds },
        { label: "Colour", next: () => colour },
        { label: "Show as", next: () => only },
        { label: "Zoom", pick: () => zoomTo(actionTarget ? "selection" : "all") },
        { label: "Make object", pick: () => void makeObject() },
        { label: "Clear all", pick: () => void clearAll() },
      ],
    };
  }

  function viewPie(): Pie {
    const all = moleculeNodes();
    const any = (option: "show_waters" | "show_ligands" | "show_hydrogens") => all.some((node) => node.options[option]);
    const contents = (label: string, option: "show_waters" | "show_ligands" | "show_hydrogens"): PieItem => ({
      label: `${any(option) ? "Hide" : "Show"} ${label}`,
      pick: () => setShown(option, !any(option)),
    });
    const pane = (label: string, kind: "graph" | "biomol", name: string): PieItem => ({
      label: `${tabShowing(desk, kind) ? "Close" : "Open"} ${label}`,
      pick: () => void agentHost().showPane(name, !tabShowing(desk, kind)),
    });
    return {
      title: "View",
      items: [
        {
          label: "Mode",
          next: () => ({
            title: "Mode",
            items: MODES.map((mode) => ({ label: mode.label, glyph: mode.id, current: mode.id === modeNow().id, pick: () => setMode(mode.id) })),
          }),
        },
        {
          label: "Frame",
          next: () => ({
            title: "Frame",
            items: [
              { label: "Everything", pick: () => viewer?.resetView() },
              { label: "Selection", pick: () => void agentHost().focus("selection").catch((error) => editor.setHoverDetail(String(error))) },
            ],
          }),
        },
        {
          label: "Contents",
          next: () => ({ title: "Contents", items: [contents("solvent", "show_waters"), contents("ligands", "show_ligands"), contents("hydrogens", "show_hydrogens")] }),
        },
        {
          label: "Cameras",
          next: () =>
            paged("Cameras", [
              { label: "Add, from this view", pick: () => addCamera() },
              { label: "Main camera", current: lookingThrough === null, pick: () => backToMain(true) },
              ...cameras.map((camera) => ({ label: camera.name, current: camera.id === lookingThrough, pick: () => lookThrough(camera.id) })),
            ]),
        },
        {
          label: "Panes",
          next: () => ({ title: "Panes", items: [pane("node graph", "graph", "graph"), pane("sequence", "biomol", "sequence")] }),
        },
      ],
    };
  }

  const CHAINS: Record<string, () => Pie> = { KeyS: selectionPie, KeyA: actionPie, KeyV: viewPie };

  const onModeKey = (event: KeyboardEvent) => {
    if (!element.isConnected) return;
    // A dialog, the palette or the shortcuts sheet has the keyboard. Their own listeners
    // stop the key, but this one is on the window too and was added first, so it heard the
    // key anyway - and a pie opened, or the level changed, behind the sheet.
    if (paletteOpen() || document.querySelector(".ask-backdrop")) return;
    const target = event.target as HTMLElement | null;
    if (target && ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;
    const plain = !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;
    if (plain && CHAINS[event.code]) {
      if (!pointerAt) return;
      event.preventDefault();
      event.stopPropagation();
      // A held key repeats; the pie it opened is already up.
      if (event.repeat) return;
      // The key of the pie that is up puts it away. Any other of the three - or the same
      // one from a step further down its chain - opens its pie at once in place of what
      // is there: going from one pie to the next is one press, not a press to dismiss
      // and another to open.
      if (pie && pieKey === event.code) {
        pie.close();
        return;
      }
      const wanted = CHAINS[event.code]();
      // Kept far enough inside the window for every item to be on it: a pie opened near an
      // edge put the items on that side out of reach.
      const centre = {
        x: Math.min(Math.max(pointerAt.x, 200), Math.max(200, innerWidth - 200)),
        y: Math.min(Math.max(pointerAt.y, 130), Math.max(130, innerHeight - 130)),
      };
      openPie(centre, event.code, wanted.title, wanted.items);
    } else if (plain && pointerAt && (event.code === "Digit0" || event.code === "Numpad0")) {
      // Blender's numpad 0: into the active camera, and out of it again.
      event.preventDefault();
      if (lookingThrough !== null) backToMain(true);
      else lookThrough(null);
    } else if (plain && pointerAt && ["Digit1", "Digit2", "Digit3", "Digit4", "Digit5"].includes(event.code)) {
      event.preventDefault();
      setPickLevel(LEVELS[Number(event.code.slice(5)) - 1].id);
    } else if (plain && pointerAt && (event.key === "Delete" || event.key === "Backspace")) {
      // Blender's Delete key: no question asked, and ⌘Z to take it back.
      event.preventDefault();
      void deleteSelected();
    } else if (plain && pointerAt && event.code === "KeyX") {
      // Blender's X asks first, with the one word under the pointer.
      event.preventDefault();
      viewportMenu.open(pointerAt, [
        { kind: "title", label: `Delete ${targetWords()}?` },
        { kind: "item", label: "Delete", onPick: () => void deleteSelected() },
      ]);
    } else if (event.altKey && !event.metaKey && !event.ctrlKey && event.code === "KeyZ" && pointerAt) {
      event.preventDefault();
      setGraphMode(!graphMode);
    }
  };
  window.addEventListener("keydown", onModeKey, true);

  // --- the viewport's right-click menu ------------------------------------------------

  /** Every molecule in the graph, for the commands that are about all of them. */
  const moleculeNodes = () => graph.nodes.flatMap((node) => (node.type === "molecule" ? [node] : []));

  function setShown(option: "show_waters" | "show_ligands" | "show_hydrogens", value: boolean): void {
    for (const node of moleculeNodes()) node.options = { ...node.options, [option]: value };
    void commitGraph();
  }

  // --- commands ----------------------------------------------------------------------
  //
  // Everything the viewport can be told to do, by the names in `commands.ts`. The
  // right-click menu is a list of those names that the user chooses in the preferences;
  // a shortcut is a name, and this is what each name does.

  // --- what every structure is analysed for ----------------------------------------------
  //
  // What is done with a structure once it is open is much the same every time: find its
  // hydrogen bonds, its salt bridges, its core, what is around its ligands. So none of
  // that waits to be asked for. Each structure is analysed as it arrives - off the main
  // thread, each analysis its own call, so the quick ones are in before the slow one -
  // and what is found sits under the molecule in the scene list as groups: sets of
  // residues to select, and sets of edges to show.

  interface PresetInfo {
    id: PresetId;
    label: string;
    /** Edges between residues, or a set of residues. */
    edges: boolean;
    /** What puts a residue in the set, for the line that says what was selected. */
    rule: string;
  }
  const PRESETS: PresetInfo[] = [
    { id: "polar", label: "Hydrogen bonds", edges: true, rule: "" },
    { id: "salt-bridge", label: "Salt bridges", edges: true, rule: "" },
    { id: "pi-stacking", label: "Pi stacking", edges: true, rule: "" },
    { id: "hydrophobic", label: "Hydrophobic contacts", edges: true, rule: "" },
    { id: "core", label: "Hydrophobic core", edges: false, rule: "hydrophobic, with a fifth or less of their surface open to solvent" },
    { id: "site", label: "Binding site", edges: false, rule: "with an atom within 4 Å of a ligand" },
    { id: "interface", label: "Interface", edges: false, rule: "with an atom within 4 Å of another chain" },
    { id: "ligands", label: "Ligands", edges: false, rule: "neither polymer nor water" },
  ];

  interface PresetState {
    state: "loading" | "ready" | "failed";
    contacts: ContactSet;
    residues: number[];
    error?: string;
    /** Settles when the analysis does, so a command that needs it can wait. */
    done: Promise<void>;
  }
  /** By structure id, then by preset. Kept for as long as the app runs: a structure does not change. */
  const analyses = new Map<string, Map<PresetId, PresetState>>();

  /**
   * The structures waiting to be analysed, in turn. A structure's own analyses all run at
   * once - the neighbour searches behind them are on the graphics card, so eight at once
   * no longer means eight walks fighting over the processor - but two capsids at once
   * would still be twice the memory, so structures take turns.
   */
  let analysisQueue: Promise<void> = Promise.resolve();
  /** The quick ones first, so most of the list is filled while the slow ones run. */
  const ANALYSIS_ORDER: PresetId[] = ["ligands", "salt-bridge", "pi-stacking", "polar", "hydrophobic", "site", "interface", "core"];

  /**
   * Whether a structure is on screen, by its id. Not by the object: every evaluation hands
   * back new ones for the same structures, and asked by identity a structure that had
   * merely been drawn again looked closed - its analyses were skipped as if it had gone,
   * and the line counting them stopped where the redraw happened.
   */
  function isOpen(id: string): boolean {
    return structures.some((entry) => entry.id === id) || sceneStructures.some((entry) => entry.id === id);
  }

  /** Starts a structure's analyses, once. */
  function analyse(structure: StructureInfo): void {
    if (analyses.has(structure.id)) return;
    const mine = new Map<PresetId, PresetState>();
    analyses.set(structure.id, mine);
    // This structure's turn: it starts when the one before it has finished.
    const turn = analysisQueue;
    const runs: Promise<unknown>[] = [];
    for (const id of ANALYSIS_ORDER) {
      const preset = PRESETS.find((entry) => entry.id === id);
      if (!preset) continue;
      const entry: PresetState = { state: "loading", contacts: EMPTY_CONTACTS, residues: [], done: Promise.resolve() };
      const run = turn.then(() => {
        // A structure no longer open is not analysed, and is forgotten, so that it is
        // analysed afresh if it is opened again; its turn passes to the next.
        if (isOpen(structure.id)) return api.structurePreset(structure.id, preset.id);
        analyses.delete(structure.id);
        return Promise.reject(new Error("closed before it was analysed"));
      });
      runs.push(run);
      entry.done = run
        .then((buffer) => {
          const found = decodePreset(buffer);
          if ("contacts" in found) entry.contacts = found.contacts;
          else entry.residues = [...found.residues];
          entry.state = "ready";
        })
        .catch((error: unknown) => {
          entry.state = "failed";
          entry.error = String(error);
        })
        .then(() => {
          // Only what is still on screen is redrawn for; the result is kept either way.
          if (!isOpen(structure.id)) return;
          editor.refresh();
          renderHud();
          if (preset.edges && shownEdges.has(preset.id as ContactKind)) drawContacts();
        });
      mine.set(preset.id, entry);
    }
    analysisQueue = Promise.allSettled(runs).then(() => undefined);
  }

  /** How far along a structure's analyses are, for the scene list and the viewport. */
  function analysisProgress(structure: StructureInfo): { done: number; total: number } {
    const mine = analyses.get(structure.id);
    const done = mine ? [...mine.values()].filter((entry) => entry.state !== "loading").length : 0;
    return { done, total: PRESETS.length };
  }

  /** The kinds of contact being shown, across every molecule on screen. */
  const shownEdges = new Set<ContactKind>();
  /** Shown only where they touch these residues: what was selected when Find was asked. */
  let contactScope: { structureId: string; residues: Set<number> } | null = null;
  /** The shown contacts as edges, for the graph view; filled only while it is up. */
  let contactEdges: ContactEdge[] = [];
  /** How many of each kind are being shown, after the selection's scope. */
  const shownCounts = new Map<ContactKind, number>();

  /**
   * Shows the contacts the way the view in use shows things: dashed lines from atom to
   * atom over the structure, edges from component to component in the graph. Not both -
   * in the graph the atoms the dashes run between are a ghost.
   */
  function drawContacts(): void {
    const parts = viewer?.currentScene?.header.parts ?? [];
    // Sized first and filled in place: a capsid's hydrogen bonds are half a million, and
    // growing a list of numbers six at a time for each of them is seconds of the window.
    let total = 0;
    const shownSets: { structure: StructureInfo; index: number; kind: ContactKind; set: ContactSet }[] = [];
    structures.slice(0, parts.length).forEach((structure, index) => {
      for (const kind of shownEdges) {
        const set = analyses.get(structure.id)?.get(kind)?.contacts;
        if (!set || set.count === 0) continue;
        shownSets.push({ structure, index, kind, set });
        total += set.count;
      }
    });
    const segments = new Float32Array(total * 6);
    const colours = new Float32Array(total * 3);
    let used = 0;
    contactEdges = [];
    shownCounts.clear();
    for (const { structure, index, kind, set } of shownSets) {
      const about = CONTACTS[kind];
      const scoped = contactScope && contactScope.structureId === structure.id ? contactScope.residues : null;
      if (contactScope && !scoped) continue;
      for (let at = 0; at < set.count; at += 1) {
        const a = set.pairs[at * 2];
        const b = set.pairs[at * 2 + 1];
        if (scoped && !scoped.has(a) && !scoped.has(b)) continue;
        segments.set(set.ends.subarray(at * 6, at * 6 + 6), used * 6);
        colours.set(about.color, used * 3);
        used += 1;
        shownCounts.set(kind, (shownCounts.get(kind) ?? 0) + 1);
        if (graphMode) contactEdges.push({ structure: index, residues: [a, b], kind, what: about.one, distance: set.distance[at], color: about.color });
      }
    }
    viewer?.setContacts(graphMode ? new Float32Array(0) : segments.subarray(0, used * 6), colours.subarray(0, used * 3));
    if (graphMode) void loadGraph();
  }

  const CONTACTS: Record<ContactKind, { one: string; many: string; color: [number, number, number]; rule: string }> = {
    polar: { one: "hydrogen bond", many: "hydrogen bonds", color: [1, 0.82, 0.1], rule: "donor to acceptor within 3.5 Å, both angles at least 90° (HBPLUS, heavy atoms)" },
    "salt-bridge": { one: "salt bridge", many: "salt bridges", color: [1, 0.35, 0.75], rule: "Asp/Glu oxygen within 4.0 Å of a Lys/Arg/His nitrogen" },
    "pi-stacking": { one: "stacked ring pair", many: "stacked rings", color: [0.3, 0.85, 0.45], rule: "ring centres within 5.5 Å, parallel or T-shaped (PLIP)" },
    hydrophobic: { one: "hydrophobic contact", many: "hydrophobic contacts", color: [0.7, 0.72, 0.76], rule: "side-chain carbons within 4.0 Å (PLIP)" },
  };

  /** The residues the selection covers, in its structure - what Find is asked about. */
  async function selectedResidues(): Promise<{ structureId: string; residues: Set<number> } | null> {
    if (!currentView) return null;
    const view = { level: currentView.level, indices: currentView.indices };
    const [summary, mask] = await Promise.all([api.describeView(currentView.structureId, view), api.viewMask(currentView.structureId, view)]);
    const residues = new Set<number>(summary.residue_indices);
    mask.residues.forEach((flag, index) => {
      if (flag) residues.add(index);
    });
    return { structureId: currentView.structureId, residues };
  }

  /**
   * Shows a kind of contact, or takes it away if it is already shown.
   *
   * Nothing is computed here: the contacts were found when the structure was opened, and
   * this only says which of them to draw. With something selected, the ones that touch
   * the selection; otherwise all of them. Several kinds can be up at once.
   */
  async function toggleContacts(kind: ContactKind, on = !shownEdges.has(kind)): Promise<void> {
    if (structures.length === 0) {
      editor.setHoverDetail("Nothing is open to look for contacts in.");
      return;
    }
    const about = CONTACTS[kind];
    if (!on) {
      shownEdges.delete(kind);
      if (shownEdges.size === 0) contactScope = null;
      drawContacts();
      editor.refresh();
      editor.setHoverDetail(`${about.many[0].toUpperCase()}${about.many.slice(1)} hidden.`);
      return;
    }
    const epoch = graphEpoch;
    try {
      contactScope = await selectedResidues();
      if (epoch !== graphEpoch) return;
      shownEdges.add(kind);
      drawContacts();
      editor.refresh();
      const pending = structures.some((structure) => analyses.get(structure.id)?.get(kind)?.state === "loading");
      const shown = shownCounts.get(kind) ?? 0;
      editor.setHoverDetail(
        pending
          ? `Still looking for ${about.many} - they appear as they are found.`
          : shown === 0
            ? `No ${about.many} ${contactScope ? "around the selection" : "found"}.`
            : `${count(shown, about.one)} ${contactScope ? "around the selection" : "in all"} - ${about.rule}.`,
      );
    } catch (error) {
      editor.setHoverDetail(String(error));
    }
  }

  /** Selects one of a structure's preset sets of residues: its core, its binding site. */
  async function selectPreset(id: PresetId, of?: StructureInfo): Promise<void> {
    const structure = of ?? (actionTarget ? structures.find((entry) => entry.id === actionTarget!.structureId) : null) ?? structures[0];
    const preset = PRESETS.find((entry) => entry.id === id);
    if (!structure || !preset) {
      editor.setHoverDetail("Nothing is open to look in.");
      return;
    }
    analyse(structure);
    const entry = analyses.get(structure.id)?.get(id);
    if (!entry) return;
    if (entry.state === "loading") editor.setHoverDetail(`Still working out the ${preset.label.toLowerCase()}...`);
    const epoch = graphEpoch;
    await entry.done;
    if (epoch !== graphEpoch) return;
    if (entry.state === "failed") {
      editor.setHoverDetail(entry.error ?? "That could not be worked out.");
      return;
    }
    if (entry.residues.length === 0) {
      editor.setHoverDetail(`No ${preset.label.toLowerCase()} in this structure.`);
      return;
    }
    await applySelection({ structureId: structure.id, level: "residue", indices: entry.residues }, "replace");
    editor.setHoverDetail(`${preset.label}: ${count(entry.residues.length, "residue")} - ${preset.rule}.`);
  }

  function hideContacts(): void {
    shownEdges.clear();
    contactScope = null;
    drawContacts();
    editor.refresh();
  }

  const zoomTo = (on: "selection" | "all") => void agentHost().focus(on).catch((error) => editor.setHoverDetail(String(error)));
  const showAs = (representations: Representation[], words: string) =>
    residueTarget()
      ? void featureTarget({ show: [...representations] }, `Also shown as ${words}`)
      : restyleTarget((style) => ({ ...style, representations: [...representations] }), `Shown as ${words}`);
  /** Adds a representation to what is drawn, leaving the rest - PyMOL's `show`. */
  const showAlso = (representation: Representation, words: string) =>
    residueTarget()
      ? void featureTarget({ show: [representation] }, `${words} added`)
      : restyleTarget(
          (style) => (style.representations.includes(representation) ? style : { ...style, representations: [...style.representations, representation] }),
          `${words} added`,
        );
  /** Takes one representation away, leaving the rest - PyMOL's `hide`. */
  const hideOnly = (representation: Representation, words: string) => {
    if (!residueTarget()) {
      restyleTarget((style) => ({ ...style, representations: style.representations.filter((entry) => entry !== representation) }), `${words} hidden`);
      return;
    }
    // On residues it is the features that added it which are taken back - in the molecule
    // the selection is in. Matched by chain and number alone, every other open molecule
    // with a chain of that name lost its sticks there too.
    const selected = currentView ? structures.find((entry) => entry.id === currentView!.structureId) : undefined;
    const owner = selected ? moleculeNodeFor(selected) : null;
    void selectedRuns().then((runs) => {
      const hit = (feature: Feature) => runs.some((run) => run.chain === feature.chain && run.from <= feature.to && feature.from <= run.to);
      for (const node of moleculeNodes().filter((candidate) => candidate === owner)) {
        node.features = (node.features ?? [])
          // A feature of several ranges is taken apart first, so only the ranges the
          // selection reaches lose the representation.
          .flatMap((feature) =>
            feature.ranges && hit(feature)
              ? featureSpans(feature).map(([from, to]) => ({ ...feature, from, to, ranges: undefined }))
              : [feature],
          )
          .map((feature) => (hit(feature) && feature.show ? { ...feature, show: feature.show.filter((entry) => entry !== representation) } : feature))
          .filter((feature) => (feature.show?.length ?? 0) > 0 || feature.hide || feature.colour);
      }
      editor.setHoverDetail(`${words} hidden on the selection.`);
      void commitGraph();
    });
  };
  const colourBy = (scheme: ColorScheme, words: string) =>
    restyleTarget(
      (style) => ({ ...style, scheme, solid: null, categorical: preferences().palette, ramp: preferences().ramp }),
      `Coloured by ${words}`,
    );
  const spot = { x: 60, y: 60 };

  /** What each command does. A name with no entry here does nothing, and is not offered. */
  const RUN: Record<string, () => void> = {
    "show.sticks": () => showAlso("sticks", "Sticks"),
    "hide.sticks": () => hideOnly("sticks", "Sticks"),
    "show.cartoon": () => showAlso("cartoon", "Cartoon"),
    "hide.cartoon": () => hideOnly("cartoon", "Cartoon"),
    "show.ball-and-stick": () => showAlso("ball-and-stick", "Ball and stick"),
    "hide.ball-and-stick": () => hideOnly("ball-and-stick", "Ball and stick"),
    "show.spacefill": () => showAlso("spacefill", "Spacefill"),
    "hide.spacefill": () => hideOnly("spacefill", "Spacefill"),
    "show.glass": () => showAlso("glass", "Glass"),
    "hide.glass": () => hideOnly("glass", "Glass"),
    "as.cartoon": () => showAs(["cartoon"], "cartoon"),
    "as.sticks": () => showAs(["sticks"], "sticks"),
    hide: () =>
      residueTarget()
        ? void featureTarget({ hide: true }, "Hidden")
        : restyleTarget((style) => ({ ...style, representations: [] }), "Hidden"),
    "colour.chain": () => colourBy("chain", "chain"),
    "colour.element": () => colourBy("element", "element"),
    "colour.secondary": () => colourBy("secondary", "secondary structure"),
    "colour.rainbow": () => colourBy("rainbow", "rainbow"),
    "find.polar": () => void toggleContacts("polar"),
    "find.salt-bridges": () => void toggleContacts("salt-bridge"),
    "find.pi": () => void toggleContacts("pi-stacking"),
    "find.hydrophobic": () => void toggleContacts("hydrophobic"),
    "find.core": () => void selectPreset("core"),
    "find.site": () => void selectPreset("site"),
    "find.interface": () => void selectPreset("interface"),
    "find.clear": () => hideContacts(),
    "find.ligands": () => void selectPreset("ligands"),
    "zoom.all": () => viewer?.resetView(),
    "zoom.selection": () => zoomTo(actionTarget ? "selection" : "all"),
    "mode.toggle": () => setGraphMode(!graphMode),
    "solvent.toggle": () => setShown("show_waters", !moleculeNodes().some((node) => node.options.show_waters)),
    "ligands.toggle": () => setShown("show_ligands", !moleculeNodes().some((node) => node.options.show_ligands)),
    "hydrogens.toggle": () => setShown("show_hydrogens", !moleculeNodes().some((node) => node.options.show_hydrogens)),
    fetch: () => {
      // The molecule as it works, unless the deposited coordinates are what is wanted.
      // The choice is the one made last time.
      let assembly = preferences().fetchAssembly;
      void askText({
        title: "Fetch from the PDB",
        placeholder: "A four-character id, such as 1ubq",
        accept: "Fetch",
        check: (value) => (/^[0-9a-zA-Z]{4}$/.test(value) ? null : "A PDB id is four letters and digits."),
        toggle: {
          label: "Load the biological assembly",
          hint: "The whole molecule as it works, built from the entry's own assembly records. Off: only the deposited coordinates, the asymmetric unit.",
          value: assembly,
          onChange: (value) => (assembly = value),
        },
      }).then((id) => {
        if (!id) return;
        if (assembly !== preferences().fetchAssembly) void savePreferences({ ...preferences(), fetchAssembly: assembly }).catch(() => {});
        void addMolecule({ pdb_id: id, path: null, assembly }, spot);
      });
    },
    open: () => void openMoleculeFile(spot),
    "save.image": () => saveImageMenu(),
    "export.glb": () => void exportModel(),
    "object.make": () => void makeObject(),
    "clear.all": () => void clearAll(),
    delete: () => void deleteSelected(),
    "select.clear": () => void applySelection(null, "replace"),
    "styles.clear": () => {
      for (const node of moleculeNodes()) node.features = [];
      void commitGraph();
    },
  };

  /**
   * Takes molecules out of the world, as one step that ⌘Z puts back.
   *
   * Their chains go with them: left behind they are chains of nothing, and the core
   * refuses the whole plan over them. So does what was shown of them - their contacts,
   * and a selection in them.
   */
  async function removeMolecules(ids: string[]): Promise<void> {
    const gone = new Set<string>(ids);
    for (const link of graph.links) {
      if (gone.has(link.from) && link.socket === "Molecule") gone.add(link.to);
    }
    graph.nodes = graph.nodes.filter((candidate) => !gone.has(candidate.id));
    graph.links = graph.links.filter((link) => !gone.has(link.from) && !gone.has(link.to));
    shownEdges.clear();
    contactScope = null;
    drawContacts();
    await applySelection(null, "replace", true);
    await commitGraph();
  }

  /**
   * Deletes what is selected, as Blender's Delete does.
   *
   * A molecule is the unit a world holds, so a selection that is a whole molecule - or
   * several - takes those molecules out. A selection of part of one cannot take that part
   * out of the file it came from; it is hidden instead, and said to be.
   */
  async function deleteSelected(): Promise<void> {
    const target = actionTarget;
    if (!target) {
      editor.setHoverDetail("Nothing is selected to delete. Select a molecule, or use Clear all to empty the scene.");
      return;
    }
    const structure = structures.find((entry) => entry.id === target.structureId);
    const node = structure ? moleculeNodeFor(structure) : null;
    const whole = structure !== undefined && target.level === "chain" && target.chains.length >= structure.chains.length;
    if (whole && node) {
      await removeMolecules([node.id]);
      editor.setHoverDetail(`Deleted ${node.type === "molecule" ? (node.pdb_id || node.path?.split(/[\\/]/).pop() || "the molecule").toUpperCase() : "the molecule"}. \u2318Z brings it back.`);
      return;
    }
    if (target.level === "chain") {
      restyleTarget((style) => ({ ...style, representations: [] }), "Hidden - part of a molecule is hidden rather than deleted");
      return;
    }
    await featureTarget({ hide: true }, "Hidden - part of a molecule is hidden rather than deleted");
  }

  /** Empties the scene: every molecule, and all that was shown of them. One undo puts it back. */
  async function clearAll(): Promise<void> {
    const molecules = moleculeNodes().map((node) => node.id);
    if (molecules.length === 0) {
      editor.setHoverDetail("The scene is already empty.");
      return;
    }
    await removeMolecules(molecules);
    editor.refresh();
    editor.setHoverDetail(`Cleared the scene: ${count(molecules.length, "molecule")}. \u2318Z brings it back.`);
  }

  // --- the scene list's right-click menu ----------------------------------------------
  //
  // Blender's Outliner: a right-click on a row is a menu of what can be done to the thing
  // the row is. Each entry selects the thing first and then does what the same entry in
  // the viewport does to a selection, so the list and the viewport cannot disagree about
  // what an action means.

  const sceneMenu = menu(element);
  window.addEventListener(
    "pointerdown",
    (event) => {
      if (sceneMenu.isOpen && !(event.target as HTMLElement | null)?.closest(".node-menu")) sceneMenu.close();
    },
    true,
  );
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && sceneMenu.isOpen) sceneMenu.close();
  });

  const SCENE_REPRESENTATIONS: [Representation, string][] = [
    ["cartoon", "Cartoon"],
    ["sticks", "Sticks"],
    ["ball-and-stick", "Ball and stick"],
    ["spacefill", "Spacefill"],
    ["backbone", "Backbone"],
    ["glass", "Glass"],
  ];
  const SCENE_SCHEMES: [ColorScheme, string][] = [
    ["chain", "chain"],
    ["element", "element"],
    ["secondary", "secondary structure"],
    ["rainbow", "rainbow"],
  ];

  // --- what a molecule is read as ------------------------------------------------------
  //
  // An entry is more than one thing: its biological assembly (or several - a capsid's
  // whole shell, its pentamer, the crystal's own contents), the coordinates as deposited,
  // the models of an ensemble, and, residue by residue, alternate locations. The row under
  // each molecule says which it is, and a click on it chooses another.

  function assemblyWords(assembly: { id: string; details: string; count: number | null }): string {
    const what = assembly.details ? assembly.details.charAt(0).toUpperCase() + assembly.details.slice(1) : `Assembly ${assembly.id}`;
    return assembly.count ? `${what} (${count(assembly.count, "chain")})` : what;
  }

  function versionRow(moleculeId: string): { label: string; value: string; title: string } | null {
    const structure = structures.find((entry) => moleculeNodeFor(entry)?.id === moleculeId);
    const versions = structure?.versions;
    if (!structure || !versions) return null;
    const choices = versions.assemblies.length + (versions.models.length > 1 ? 1 : 0) + (versions.alternates.length > 0 ? 1 : 0);
    if (choices === 0) return null;
    const current = versions.assemblies.find((assembly) => assembly.id === versions.assembly);
    const label = current ? `Assembly ${current.id}` : "As deposited";
    const extras = [
      versions.models.length > 1 ? `model ${versions.model ?? versions.models[0]} of ${versions.models.length}` : "",
      versions.alternate ? `alt ${versions.alternate}` : "",
    ].filter(Boolean);
    return {
      label,
      value: extras.join(" · ") || (current?.count ? count(current.count, "chain") : ""),
      title: `${current ? assemblyWords(current) : "The coordinates as deposited: the asymmetric unit"}. Click to read it as something else.`,
    };
  }

  /** Reads a molecule again as another version of its entry. */
  async function setVersion(menuNode: Extract<GraphNode, { type: "molecule" }>, change: { assembly?: boolean; version?: Partial<Version> }, words: string): Promise<void> {
    sceneMenu.close();
    // The graph as it is now: it may have been read back since the menu was opened, and a
    // change made to the node the menu was opened on would be made to nothing.
    const node = graph.nodes.find((candidate) => candidate.id === menuNode.id);
    if (!node || node.type !== "molecule") return;
    if (change.assembly !== undefined) node.assembly = change.assembly;
    const next: Version = { ...(node.version ?? {}), ...change.version };
    for (const key of Object.keys(next) as (keyof Version)[]) if (next[key] === undefined || next[key] === "") delete next[key];
    node.version = Object.keys(next).length > 0 ? next : undefined;
    rechaining.add(node.id);
    notice = `Reading ${(node.pdb_id || "the molecule").toUpperCase()} as ${words}`;
    renderHud();
    await commitGraph();
  }

  function versionEntries(node: Extract<GraphNode, { type: "molecule" }>, structure: StructureInfo): MenuEntry[] {
    const versions = structure.versions;
    if (!versions) return [];
    const mark = (on: boolean) => (on ? "\u2713 " : "\u2003 ");
    const entries: MenuEntry[] = [{ kind: "title", label: "Read as" }];
    entries.push({
      kind: "item",
      label: `${mark(versions.assembly === null)}As deposited`,
      hint: "asymmetric unit",
      onPick: () => void setVersion(node, { assembly: false, version: { assembly_id: undefined } }, "deposited"),
    });
    for (const assembly of versions.assemblies) {
      entries.push({
        kind: "item",
        label: `${mark(versions.assembly === assembly.id)}${assembly.details ? assembly.details.charAt(0).toUpperCase() + assembly.details.slice(1) : `Assembly ${assembly.id}`}`,
        hint: assembly.count ? count(assembly.count, "chain") : `assembly ${assembly.id}`,
        onPick: () => void setVersion(node, { assembly: true, version: { assembly_id: assembly.id } }, `assembly ${assembly.id}`),
      });
    }
    if (versions.models.length > 1) {
      const chosen = versions.model ?? versions.models[0];
      entries.push({ kind: "separator" }, {
        kind: "submenu",
        label: `Model ${chosen} of ${versions.models.length}`,
        entries: () =>
          versions.models.map((model): MenuEntry => ({
            kind: "item",
            label: `${mark(model === chosen)}Model ${model}`,
            onPick: () => void setVersion(node, { version: { model: model === versions.models[0] ? undefined : model } }, `model ${model}`),
          })),
      });
    }
    if (versions.alternates.length > 0) {
      entries.push({ kind: "separator" }, {
        kind: "submenu",
        label: `Alternate locations${versions.alternate ? ` · ${versions.alternate}` : ""}`,
        entries: () => [
          { kind: "title", label: `${count(versions.alternate_residues, "residue")} with more than one` },
          {
            kind: "item",
            label: `${mark(versions.alternate === null)}Most occupied, residue by residue`,
            onPick: () => void setVersion(node, { version: { alternate: undefined } }, "each residue's most occupied conformer"),
          },
          ...versions.alternates.map((label): MenuEntry => ({
            kind: "item",
            label: `${mark(versions.alternate === label)}${label}`,
            hint: "where a residue has it",
            onPick: () => void setVersion(node, { version: { alternate: label } }, `alternate location ${label}`),
          })),
        ],
      });
    }
    return entries;
  }

  function openSceneMenu(row: SceneRow, at: { x: number; y: number }): void {
    const structure = structures.find((entry) => moleculeNodeFor(entry)?.id === row.moleculeId);
    const node = graph.nodes.find((candidate) => candidate.id === row.moleculeId);
    if (!structure || !node || node.type !== "molecule") return;
    const name = (node.pdb_id || node.path?.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") || "Molecule").toUpperCase();
    if (row.kind === "version") {
      sceneMenu.open(at, versionEntries(node, structure));
      return;
    }

    const wholeMolecule: View = { structureId: structure.id, level: "chain", indices: structure.chains.map((_, index) => index) };
    const chainIndex = row.kind === "chain" ? structure.chains.findIndex((chain) => chain.id === row.chain) : -1;
    const select = async (): Promise<void> => {
      if (row.kind === "molecule") await applySelection(wholeMolecule, "replace");
      else if (row.kind === "chain" && chainIndex >= 0) await applySelection({ structureId: structure.id, level: "chain", indices: [chainIndex] }, "replace");
      else if (row.kind === "object") await selectObject(row.moleculeId, row.name);
      else if (row.kind === "group" && !row.edges) await selectPreset(row.id as PresetId, structure);
    };
    /** Selects the row's thing, then does `then` to the selection. */
    const onIt = (label: string, then: () => void | Promise<void>): MenuEntry => ({
      kind: "item",
      label,
      onPick: () => {
        sceneMenu.close();
        void select().then(then);
      },
    });
    const looks = (): MenuEntry[] => [
      {
        kind: "submenu",
        label: "Show",
        entries: () => SCENE_REPRESENTATIONS.map(([representation, words]) => onIt(words, () => showAlso(representation, words))),
      },
      {
        kind: "submenu",
        label: "Hide",
        entries: () => SCENE_REPRESENTATIONS.map(([representation, words]) => onIt(words, () => hideOnly(representation, words))),
      },
      {
        kind: "submenu",
        label: "Show as",
        entries: () => SCENE_REPRESENTATIONS.map(([representation, words]) => onIt(`${words} only`, () => showAs([representation], words.toLowerCase()))),
      },
      {
        kind: "submenu",
        label: "Colour by",
        entries: () => SCENE_SCHEMES.map(([scheme, words]) => onIt(words[0].toUpperCase() + words.slice(1), () => colourBy(scheme, words))),
      },
    ];

    let entries: MenuEntry[];
    if (row.kind === "molecule") {
      const groups = analyses.get(structure.id);
      entries = [
        { kind: "title", label: name },
        onIt("Select", () => undefined),
        onIt("Zoom to", () => zoomTo("selection")),
        { kind: "separator" },
        ...looks(),
        {
          kind: "submenu",
          label: "Groups",
          entries: () =>
            PRESETS.map((preset): MenuEntry => {
              const entry = groups?.get(preset.id);
              const ready = entry?.state === "ready";
              const size = preset.edges ? (entry?.contacts.count ?? 0) : (entry?.residues.length ?? 0);
              return {
                kind: "item",
                label: preset.edges ? `${shownEdges.has(preset.id as ContactKind) ? "Hide" : "Show"} ${preset.label.toLowerCase()}` : `Select the ${preset.label.toLowerCase()}`,
                hint: ready ? size.toLocaleString() : "...",
                disabled: !ready || size === 0,
                onPick: () => {
                  sceneMenu.close();
                  if (preset.edges) void toggleContacts(preset.id as ContactKind);
                  else void selectPreset(preset.id, structure);
                },
              };
            }),
        },
        onIt("Make an object of it...", () => makeObject()),
        { kind: "separator" },
        {
          kind: "item",
          label: "Delete",
          hint: "\u2318Z undoes",
          onPick: () => {
            sceneMenu.close();
            void removeMolecules([row.moleculeId]).then(() => editor.setHoverDetail(`Deleted ${name}. \u2318Z brings it back.`));
          },
        },
      ];
    } else if (row.kind === "chain") {
      entries = [
        { kind: "title", label: `${name} \u00b7 chain ${row.chain}` },
        onIt("Select", () => undefined),
        onIt("Zoom to", () => zoomTo("selection")),
        { kind: "separator" },
        ...looks(),
        { kind: "separator" },
        onIt("Hide this chain", () => restyleTarget((style) => ({ ...style, representations: [] }), "Hidden")),
      ];
    } else if (row.kind === "object") {
      entries = [
        { kind: "title", label: row.name },
        onIt("Select", () => undefined),
        onIt("Zoom to", () => zoomTo("selection")),
        { kind: "separator" },
        ...looks(),
        { kind: "separator" },
        {
          kind: "item",
          label: "Remove the object",
          hint: "the molecule stays",
          onPick: () => {
            sceneMenu.close();
            removeObject(row.moleculeId, row.name);
          },
        },
      ];
    } else {
      const preset = PRESETS.find((entry) => entry.id === row.id);
      if (!preset) return;
      entries = row.edges
        ? [
            { kind: "title", label: preset.label },
            {
              kind: "item",
              label: shownEdges.has(row.id as ContactKind) ? "Hide them" : "Show them",
              onPick: () => {
                sceneMenu.close();
                void toggleContacts(row.id as ContactKind);
              },
            },
            {
              kind: "item",
              label: "Show only these",
              onPick: () => {
                sceneMenu.close();
                shownEdges.clear();
                contactScope = null;
                void toggleContacts(row.id as ContactKind, true);
              },
            },
          ]
        : [
            { kind: "title", label: preset.label },
            onIt("Select", () => undefined),
            onIt("Zoom to", () => zoomTo("selection")),
            { kind: "separator" },
            ...looks(),
            onIt("Make an object of it...", () => makeObject()),
          ];
    }
    viewportMenu.close();
    sceneMenu.open(at, entries);
  }

  /** The right-click menu: the commands the preferences name, in that order. */
  function contextEntries(): MenuEntry[] {
    const chosen = preferences().contextMenu.filter((id) => Object.hasOwn(RUN, id));
    const entries: MenuEntry[] = chosen.map((id) => ({
      kind: "item" as const,
      label: COMMANDS.find((command) => command.id === id)?.label ?? id,
      onPick: () => {
        viewportMenu.close();
        RUN[id]();
      },
    }));
    return [
      { kind: "title", label: `Acts on ${targetWords()}` },
      ...entries,
      // Always last, whatever is above it: where the list above is chosen.
      { kind: "separator" as const },
      {
        kind: "item" as const,
        label: "Preferences...",
        onPick: () => {
          viewportMenu.close();
          window.dispatchEvent(new CustomEvent("geis:settings", { detail: "preferences" }));
        },
      },
    ];
  }

  /** The keys that already run a command, shown beside it in the palette. */
  const COMMAND_KEYS: Record<string, string> = {
    open: "\u2318O",
    fetch: "\u21e7\u2318O",
    "save.image": "\u2318S",
    "mode.toggle": "\u2325Z",
  };

  /** What the keys and the mouse do, for the sheet that ? opens. */
  const SHORTCUTS: ShortcutGroup[] = [
    {
      title: "Everywhere",
      rows: [
        ["\u2318K", "Find any command"],
        ["\u2318O", "Open a file"],
        ["\u21e7\u2318O", "Fetch from the PDB"],
        ["\u2318S", "Save an image"],
        ["\u2318,", "Settings"],
        ["\u2318Z / \u21e7\u2318Z", "Undo / redo - edits, and moves of the view"],
        ["?", "This sheet"],
      ],
    },
    {
      title: "Viewport - mouse",
      rows: [
        ["Drag", "Rotate"],
        ["\u21e7 drag, or right or middle drag", "Move"],
        ["Scroll, or \u2303 drag", "Zoom"],
        ["Click", "Select; \u21e7 adds, \u2318 toggles"],
        ["Right-click", "Your shortcuts menu"],
      ],
    },
    {
      title: "Viewport - keys, pointer over it",
      rows: [
        ["S, A, V", "Tap for a pie that stays up; hold, move and let go to pick in one motion"],
        ["S", "Selection pie"],
        ["A", "Action pie: show, hide, colour, find"],
        ["V", "View pie: frame, contents, cameras, panes"],
        ["1 - 5", "Select atoms, residues, chains, molecules, bonds"],
        ["0", "Look through the camera, and back"],
        ["Delete / X", "Delete the selection (X asks first)"],
        ["\u2325Z", "Structure / graph view"],
        ["Tab", "Show or hide the node graph"],
      ],
    },
    {
      title: "Node graph",
      rows: [
        ["\u21e7A", "Add a node"],
        ["Delete", "Remove the selected node"],
        ["H / \u21e7H", "Fold the node / all nodes"],
        ["L", "Arrange"],
        ["Home", "Fit everything in view"],
      ],
    },
    {
      title: "Agent",
      rows: [
        ["\u21b5", "Send"],
        ["\u21e7\u21b5", "New line"],
        ["/", "Skills"],
      ],
    },
  ];

  /** Every command there is, for the palette: the viewport's, then the screen's own. */
  function paletteItems(): PaletteItem[] {
    const settings = (tab: string) => () => window.dispatchEvent(new CustomEvent("geis:settings", { detail: tab }));
    return [
      ...COMMANDS.filter((command) => RUN[command.id]).map((command) => ({
        label: command.label,
        group: command.group,
        keys: COMMAND_KEYS[command.id],
        run: RUN[command.id],
      })),
      { label: "Undo", group: "Edit", keys: "⌘Z", run: () => undo() },
      { label: "Redo", group: "Edit", keys: "⇧⌘Z", run: () => redo() },
      { label: "Layout: 3D viewport only", group: "Layout", run: () => commit(defaultDesk()) },
      { label: "Layout: 3D viewport, sequence and node graph", group: "Layout", run: () => commit(workDesk()) },
      ...newTabEntries((kind) => commit(addPane(desk, kind))).map((entry) => ({
        label: `Add a pane: ${entry.label}`,
        group: "Layout",
        run: entry.onPick,
      })),
      { label: "Add a camera, from this view", group: "View", run: () => addCamera() },
      { label: "Preferences...", group: "Settings", keys: "⌘,", run: settings("preferences") },
      { label: "Agents...", group: "Settings", run: settings("agents") },
      { label: "Compute...", group: "Settings", run: settings("compute") },
      { label: "Extensions...", group: "Settings", run: settings("models") },
      { label: "Jobs...", group: "Settings", run: settings("jobs") },
      { label: "Keyboard shortcuts", group: "Help", keys: "?", run: () => openShortcuts(SHORTCUTS) },
      { label: "Back to the worlds", group: "File", run: () => onBack() },
    ];
  }

  function showPalette(): void {
    viewportMenu.close();
    deskMenu.close();
    openPalette(paletteItems(), `Type a command - it acts on ${targetWords()}`);
  }

  /**
   * Runs a screen-wide command once.
   *
   * The same press can arrive twice: as a key in the page and, where there is a menu bar,
   * as the menu item with that key. Whichever comes second within a moment is the same
   * press, and two undos for one Cmd+Z would be a bug nobody could explain.
   */
  let lastFired = { id: "", at: 0 };
  function fire(id: string): void {
    const now = performance.now();
    if (lastFired.id === id && now - lastFired.at < 150) return;
    lastFired = { id, at: now };
    if (id === "palette") showPalette();
    else if (id === "shortcuts") openShortcuts(SHORTCUTS);
    else if (id === "undo") undo();
    else if (id === "redo") redo();
    else if (Object.hasOwn(RUN, id)) RUN[id]();
  }
  window.addEventListener("geis:command", (event) => {
    if (element.isConnected) fire((event as CustomEvent<string>).detail);
  });

  // The keys that belong to the whole screen rather than to the viewport under the pointer.
  window.addEventListener("keydown", (event) => {
    if (!element.isConnected || paletteOpen()) return;
    const command = (event.metaKey || event.ctrlKey) && !event.altKey;
    const key = event.key.toLowerCase();
    if ((command && !event.shiftKey && key === "k") || event.key === "F3") {
      event.preventDefault();
      fire("palette");
      return;
    }
    if (command && key === "o") {
      event.preventDefault();
      fire(event.shiftKey ? "fetch" : "open");
      return;
    }
    if (command && !event.shiftKey && key === "s") {
      event.preventDefault();
      fire("save.image");
      return;
    }
    const target = event.target;
    const typing = target instanceof Element && target.closest("input, textarea, select, [contenteditable]");
    if (!typing && !command && !event.altKey && event.key === "?") {
      event.preventDefault();
      openShortcuts(SHORTCUTS);
    }
  });

  // A right press that goes nowhere is a menu; one that travels is the pan it always was.
  let rightPress: { x: number; y: number } | null = null;
  // Dragging or zooming the view is the main camera moving: a made camera that was being
  // looked through is left where it is, and the view is the main camera's again.
  stage.addEventListener("wheel", () => backToMain(false), { capture: true, passive: true });
  stage.addEventListener("pointermove", (event) => {
    if (event.buttons !== 0) backToMain(false);
  }, true);

  stage.addEventListener("pointerdown", (event) => {
    // The viewer keeps a press from doing the browser's default, so that a drag orbits
    // rather than selects text - and the default is also what moves the keyboard. Without
    // this a click in the viewport left the caret in the agent's box, and S, A and V went
    // on being typed into it.
    const focused = document.activeElement as HTMLElement | null;
    if (focused && focused !== document.body && !stage.contains(focused)) focused.blur();
    stage.focus({ preventScroll: true });
    viewportMenu.close();
    rightPress = event.button === 2 ? { x: event.clientX, y: event.clientY } : null;
  }, true);
  stage.addEventListener("pointerup", (event) => {
    if (event.button !== 2 || !rightPress) return;
    const still = Math.abs(event.clientX - rightPress.x) + Math.abs(event.clientY - rightPress.y) < 4;
    rightPress = null;
    if (still) viewportMenu.open({ x: event.clientX, y: event.clientY }, contextEntries());
  });
  window.addEventListener("pointerdown", (event) => {
    if (!viewportMenu.isOpen) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest(".node-menu, .mode-menu") || stage.contains(target)) return;
    viewportMenu.close();
  }, true);

  /** Into one of the viewport's modes; the others are left. */
  function setMode(mode: ViewMode): void {
    if ((mode === "graph") !== graphMode) setGraphMode(mode === "graph");
    drawModeHeader();
  }

  function setGraphMode(on: boolean): void {
    graphMode = on;
    // A sphere is a component, so a click on one means that component: the level follows
    // the view in, and is left where it was put on the way out.
    if (on && pickLevel === "chain") pickLevel = "residue";
    viewer?.setGraphMode(on);
    viewer?.setGraphPick(pickLevel === "link" ? "edges" : "nodes");
    // Loads the graph on the way in, and swaps how the found contacts are shown.
    drawContacts();
    drawLevelBar();
  }

  /**
   * Reads the components of what is on screen and hands the viewer their graph.
   *
   * Only while the view is on: the map of a ribosome is a hundred thousand residues, and
   * nobody who never opens the view should wait for it.
   */
  // --- saving a picture ----------------------------------------------------------------
  //
  // There are two pictures to save: the viewport as it stands, and the render Blender
  // made. "Save image" used to mean the first without saying so - and to someone who has
  // just pressed Render, or has not yet, it is the second that is meant. So it asks, and
  // says which render it has: none, the current one, or one older than the scene.

  /** What the scene looked like when it was last rendered, to tell a stale render by. */
  let renderedAs: string | null = null;

  /** The scene and the view, as text: the same text means the same picture. */
  function sceneStamp(): string {
    const camera = viewer?.describeCamera();
    const round = (values: number[] | undefined) => (values ?? []).map((value) => Math.round(value * 100) / 100);
    return JSON.stringify({
      // Not the image nodes or where a render last went: those are what rendering changes.
      nodes: graph.nodes.filter((node) => node.type !== "image").map((node) => (node.type === "render" ? { ...node, output: null, position: null } : { ...node, position: null })),
      camera: camera ? [round(camera.eye), round(camera.center), round(camera.up), Math.round(camera.radius * 100) / 100] : null,
    });
  }

  /** The last render's file, if this world has one. */
  function lastRender(): string | null {
    const node = graph.nodes.find((candidate) => candidate.type === "render" && candidate.output);
    return node && node.type === "render" ? node.output : null;
  }

  async function renderThenSave(): Promise<void> {
    const node = graph.nodes.find((candidate) => candidate.type === "render");
    if (!node) {
      notice = "This world has no Render node - add one in the node graph.";
      renderHud();
      return;
    }
    editor.setHoverDetail("Rendering with Blender… the save dialog opens when it is done.");
    const outcome = await renderNode(node.id);
    if (!outcome) {
      // `renderNode` has said why in the notice; the promise of a dialog no longer holds.
      editor.setHoverDetail(null);
      return;
    }
    await saveImageAs(outcome.image);
  }

  function saveImageMenu(): void {
    const render = lastRender();
    const stale = render !== null && renderedAs !== sceneStamp();
    const frame = viewerMain.getBoundingClientRect();
    // The viewport can be closed, or be the hidden tab of its pane. Its menu lives inside
    // it, so opened there the menu went into a detached element and the button did nothing;
    // and there is no picture of it to save.
    const shown = viewerMain.isConnected && frame.width > 0 && frame.height > 0;
    const pick = (label: string, run: () => void, disabled = false): MenuEntry => ({
      kind: "item",
      label,
      disabled,
      onPick: () => {
        viewportMenu.close();
        run();
      },
    });
    // The viewport is drawn again at the size chosen rather than copied off the screen:
    // the pixels on screen are as many as the pane is wide, which is a small figure.
    const viewportSizes: MenuEntry = pick("Save the viewport...", () => openSavePanel(), !shown);
    const model: MenuEntry = pick("Export a 3D model for slides (.glb)...", () => void exportModel(), structures.length === 0);
    const entries: MenuEntry[] =
      render === null
        ? [
            { kind: "title", label: "Nothing has been rendered yet" },
            pick("Render now, then save...", () => void renderThenSave()),
            viewportSizes,
            model,
          ]
        : stale
          ? [
              { kind: "title", label: "The last render is older than the scene" },
              pick("Render again, then save...", () => void renderThenSave()),
              pick("Save the old render anyway...", () => void saveImageAs(render)),
              viewportSizes,
              model,
            ]
          : [
              { kind: "title", label: "Save image" },
              pick("Save the render...", () => void saveImageAs(render)),
              viewportSizes,
              model,
              pick("Render again, then save...", () => void renderThenSave()),
            ];
    if (shown) {
      viewportMenu.open({ x: frame.right - 260, y: frame.top + 8 }, entries);
    } else {
      const box = deskRoot.getBoundingClientRect();
      deskMenu.open({ x: box.right - 260, y: box.top + 8 }, entries);
    }
  }

  /** The scene as it was last sent to be drawn, for exporting it as it is. */
  let lastSceneRequest: SceneRequest | null = null;

  /** Writes the scene on screen as a .glb for slides, with its progress on the status bar. */
  async function exportModel(): Promise<void> {
    const request = lastSceneRequest;
    if (!request || structures.length === 0) {
      editor.setHoverDetail("There is nothing on screen to export.");
      return;
    }
    const target = await saveFileDialog({
      defaultPath: `${(structures[0].entry_id || "molecule").toLowerCase()}.glb`,
      filters: [{ name: "3D model (glTF binary)", extensions: ["glb"] }],
    });
    if (typeof target !== "string") return;
    const gauge = showProgress("Export 3D model");
    gauge.set(null, "Building the model");
    try {
      const done = await api.exportGlb(request, target);
      const megabytes = done.bytes / (1024 * 1024);
      gauge.finish(`Saved ${done.triangles.toLocaleString()} triangles, ${megabytes.toFixed(1)} MB`);
      notice = `saved the model to ${done.path} - in PowerPoint: Insert, 3D Models, This Device`;
    } catch (error) {
      gauge.fail(`Could not export: ${String(error)}`);
      notice = String(error);
    }
    renderHud();
  }

  /** How many pixels an angstrom gets when the picture fits the zoom: enough to see a bond. */
  const PIXELS_PER_ANGSTROM = 4;
  /**
   * The most a picture may be, in pixels: what the frame behind it can hold in one pass
   * on this machine's memory. Larger would need the picture drawn in tiles.
   */
  const MOST_PICTURE_PIXELS = 40_000_000;
  /** The width chosen last time, and whether it was chosen to match the zoom. */
  let pictureWidth: number | null = null;
  let pictureMatchesZoom = true;

  /**
   * Asks how large to save the viewport: a slider to drag, the size and what one pixel
   * covers in angstroms beside it, and a switch that sets the size from the zoom - so a
   * capsid seen whole gets the pixels its folds need, and a ligand seen close does not
   * get more than it can use.
   */
  function openSavePanel(): void {
    if (!viewer) return;
    const frame = viewerMain.getBoundingClientRect();
    const ratio = window.devicePixelRatio || 1;
    const screenWidth = Math.max(16, Math.round(frame.width * ratio));
    const aspect = Math.max(frame.width, 1) / Math.max(frame.height, 1);
    const card = viewer.maxCaptureSide();
    const most = Math.max(screenWidth, Math.floor(Math.min(card, card * aspect, Math.sqrt(MOST_PICTURE_PIXELS * aspect))));
    const least = Math.min(screenWidth, 480);
    const span = viewer.viewHeight();
    const zoomWidth = () => Math.min(most, Math.max(screenWidth, Math.round(span * PIXELS_PER_ANGSTROM * aspect)));

    const slider = el("input", { type: "range", class: "save-slider", min: String(least), max: String(most), step: "1" });
    const readout = el("div", { class: "save-readout" });
    const scale = el("div", { class: "save-scale" });
    const match = el("input", { type: "checkbox" });
    const fits = el("label", { class: "ask-toggle", title: `A size from how close the view is: ${PIXELS_PER_ANGSTROM} pixels an angstrom at the centre, and never less than the screen` }, match, el("span", { text: "Match the zoom" }));
    const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });
    const accept = el("button", { class: "btn primary", type: "button", text: "Save..." });

    const widthNow = () => (match.checked ? zoomWidth() : Number(slider.value));
    const draw = () => {
      const width = widthNow();
      const height = Math.round(width / aspect);
      slider.value = String(width);
      slider.disabled = match.checked;
      readout.textContent = `${width.toLocaleString()} \u00d7 ${height.toLocaleString()} pixels`;
      const perPixel = span / height;
      scale.textContent = `${perPixel < 0.1 ? perPixel.toFixed(3) : perPixel.toFixed(2)} \u00c5 a pixel \u00b7 ${(width * height / 1e6).toFixed(1)} megapixels`;
    };
    match.checked = pictureMatchesZoom;
    slider.value = String(Math.min(most, Math.max(least, pictureWidth ?? screenWidth * 2)));
    slider.addEventListener("input", draw);
    match.addEventListener("change", () => {
      if (!match.checked) slider.value = String(zoomWidth());
      draw();
    });
    draw();

    const panel = el(
      "div",
      { class: "ask-dialog save-panel", role: "dialog", "aria-modal": "true", "aria-label": "Save the viewport" },
      el("div", { class: "ask-title", text: "Save the viewport" }),
      slider,
      readout,
      scale,
      fits,
      el("div", { class: "ask-detail", text: "Drawn four times larger each way and shrunk to this size, so thin ribbons stay whole." }),
      el("div", { class: "ask-actions" }, cancel, accept),
    );
    const backdrop = el("div", { class: "ask-backdrop" }, panel);
    const close = () => {
      window.removeEventListener("keydown", onKey, true);
      backdrop.remove();
    };
    const onKey = (event: KeyboardEvent) => {
      event.stopPropagation();
      if (event.key === "Escape") close();
    };
    window.addEventListener("keydown", onKey, true);
    backdrop.addEventListener("pointerdown", (event) => {
      if (event.target === backdrop) close();
    });
    cancel.addEventListener("click", close);
    accept.addEventListener("click", () => {
      const width = widthNow();
      pictureWidth = width;
      pictureMatchesZoom = match.checked;
      close();
      void saveImage({ width, height: Math.round(width / aspect) });
    });
    document.body.append(backdrop);
    accept.focus();
  }

  async function saveImage(size?: { width: number; height: number }): Promise<void> {
    const target = await saveFileDialog({
      // "untitled.png", as an unnamed document is called everywhere else.
      defaultPath: "untitled.png",
      filters: [{ name: "PNG", extensions: ["png"] }],
    });
    if (typeof target !== "string") return;
    // Both ends said: a write that failed used to vanish into a dropped promise, and one
    // that worked looked exactly the same.
    if (!viewer) {
      notice = "There is no viewport to save.";
      renderHud();
      return;
    }
    const frame = viewerMain.getBoundingClientRect();
    const ratio = window.devicePixelRatio || 1;
    const wanted = size ?? { width: Math.round(frame.width * ratio), height: Math.round(frame.height * ratio) };
    const gauge = showProgress(`Saving ${target.split("/").pop() ?? "the image"} \u00b7 ${wanted.width} \u00d7 ${wanted.height}`);
    try {
      // Drawn four times larger each way and shrunk, whatever size is asked for: the size
      // of the file and how clean it is are separate. Lowered on its own if the card
      // cannot draw that much at once.
      const blob = await viewer.capturePicture(wanted.width, wanted.height, {
        supersample: 4,
        onStage: (fraction, stage) => gauge.set(fraction, stage),
      });
      gauge.set(0.85, "Writing the file");
      const data = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => (typeof reader.result === "string" ? resolve(reader.result) : reject(new Error("the picture could not be read back")));
        reader.onerror = () => reject(reader.error ?? new Error("the picture could not be read back"));
        reader.readAsDataURL(blob);
      });
      const saved = await api.saveImage(target, data);
      gauge.finish(`Saved to ${saved}`);
      notice = `saved to ${saved} (${wanted.width} \u00d7 ${wanted.height})`;
    } catch (error) {
      gauge.fail(`Could not save the image: ${String(error)}`);
      notice = `could not save the image: ${String(error)}`;
    }
    renderHud();
  }

  async function loadGraph(): Promise<void> {
    const instance = viewer;
    const parts = instance?.currentScene?.header.parts;
    if (!instance || !graphMode) return;
    if (!parts || structures.length === 0) {
      instance.setGraph(null);
      return;
    }
    const mine = ++graphAsked;
    const shown = structures.slice(0, parts.length);
    try {
      const maps = await Promise.all(shown.map((structure) => api.structureMap(structure.id)));
      if (mine !== graphAsked || viewer !== instance) return;
      // What the molecule leaves out of its atoms, the graph leaves out of its spheres.
      const show = (result?.evaluation.plan?.parts ?? []).map((part) => ({
        waters: part.options.show_waters,
        ligands: part.options.show_ligands,
      }));
      // Each component in the colour its residue is drawn in, and what Find found as edges.
      const scene = instance.currentScene;
      const colours = scene ? residueColours(scene, maps, parts) : undefined;
      shownGraph = buildGraphView(maps, parts.map((part) => part.residue_offset), show, { colours, contacts: contactEdges });
      const found = shownGraph.edgeInfo.filter((edge) => edge.measured !== undefined).length;
      void api.uiLog(
        "viewer",
        `graph: ${shownGraph.nodeIds.length} components (${colours?.size ?? 0} coloured as drawn), ` +
          `${shownGraph.edgeInfo.length - found} bonds, ${found} found contacts`,
      );
      pickedEdge = null;
      instance.setGraph(shownGraph);
      renderHud();
    } catch (error) {
      if (mine === graphAsked) editor.setHoverDetail(`Graph view: ${String(error)}`);
    }
  }

  /**
   * What a click selects, at the left end of the viewport's header.
   *
   * After Blender's edit mode, read from its source: three icon buttons in the header for
   * vertex, edge and face, and 1, 2, 3 on the keyboard for the same (`mesh.select_mode`
   * on `NUMBERS_1`) - finest first. Here the three are atom, residue and chain.
   */
  function setPickLevel(level: PickLevel): void {
    pickLevel = level;
    // Links are the graph's: there is nothing to pick as one in a ribbon. Choosing them
    // opens the graph view, and there a click lands on bonds and passes the spheres by -
    // as Blender's edge mode picks edges however close a vertex is.
    // Choosing a level never changes the mode. Bonds between components are drawn - and
    // so can be picked - in the graph view; chosen in the structure view, the level waits
    // until the view is switched, and says so.
    if (level === "link" && !graphMode) editor.setHoverDetail("Bonds are picked in the graph view (V, Mode, Graph).");
    viewer?.setGraphPick(level === "link" ? "edges" : "nodes");
    drawLevelBar();
  }

  /** Shows the caption under the level buttons for a moment, or while the pointer is on one. */
  function showCaption(text: string, forMs: number | null): void {
    levelCaption.textContent = text;
    levelCaption.classList.add("shown");
    clearTimeout(captionTimer);
    if (forMs !== null) captionTimer = setTimeout(() => levelCaption.classList.remove("shown"), forMs);
  }

  function drawLevelBar(): void {
    // The name of a level, under the buttons - but only when it is asked about: the
    // button under the pointer, or for a moment after a key changed the level, so a key
    // press is seen to have done something. Always there, it was one more word over the
    // structure.
    const current = LEVELS.find((level) => level.id === pickLevel)?.label ?? "";
    if (captionLevel !== null && captionLevel !== current) showCaption(current, 1200);
    captionLevel = current;
    replace(
      levelBar,
      ...LEVELS.map((level, index) => {
        const button = el("button", {
          class: `mode-btn${pickLevel === level.id ? " active" : ""}`,
          role: "radio",
          "aria-checked": String(pickLevel === level.id),
          "aria-label": level.label,
          title: `${level.label} select (${index + 1}): ${level.hint}`,
          onclick: () => setPickLevel(level.id),
          onpointerenter: () => showCaption(level.label, null),
          onpointerleave: () => showCaption(level.label, 0),
        });
        button.innerHTML = iconMarkup(level.icon);
        return button;
      }),
    );
    drawModeHeader();
    drawTarget();
  }

  // Tab folds the graph away, the way a viewport-plus-editor layout usually does it.
  element.addEventListener("keydown", (event) => {
    if (event.key !== "Tab" || event.metaKey || event.ctrlKey) return;
    const target = event.target as HTMLElement;
    if (["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;
    event.preventDefault();
    toggleGraph();
  });

  /**
   * Tab folds the graph pane away and brings it back, the gesture a viewport-plus-editor
   * layout always has. Back means a new pane along the bottom, where the default puts it.
   */
  function toggleGraph(): void {
    const shown = tabShowing(desk, "graph");
    if (shown) {
      desk = closeTab(desk, shown.pane.id, shown.tab.id);
    } else {
      desk = addPane(desk, "graph");
      const added = desk.panes.at(-1);
      if (added) desk = movePane(resizePane(desk, added.id, "se", 1 - added.w, 0.34 - added.h), added.id, 0, 0.66);
    }
    renderDesk();
    saveLayout();
  }

  /**
   * The canvas is created once and kept: a WebGL context is expensive, and browsers cap
   * how many can exist at a time.
   */
  function ensureStage(): Viewer | null {
    if (viewer) return viewer;
    if (!gpu.webgl2) {
      replace(
        stage,
        el(
          "div",
          { class: "empty" },
          el("div", { style: "font-size:15px;margin-bottom:10px", text: "No WebGL2 on this system." }),
          el("div", {
            style: "max-width:520px;margin:0 auto;line-height:1.7",
            text:
              "The interactive viewport needs WebGL2, which this system does not provide. " +
              "Offline Blender renders do not need it.",
          }),
        ),
      );
      return null;
    }
    canvas = el("canvas", { class: "viewer-canvas" });
    replace(stage, canvas);
    try {
      viewer = new Viewer(canvas);
      viewer.setGraphMode(graphMode);
      viewer.setGraphPick(pickLevel === "link" ? "edges" : "nodes");
      // A click in the viewport selects what it hit, and a click on the background clears
      // the selection - the two halves of the same gesture.
      viewer.onPick = (picked, modifiers) =>
        void resolvePick(picked, modifiers.shift ? "add" : modifiers.toggle ? "toggle" : "replace");
      viewer.onHover = (picked) => void resolveHover(picked);
      viewer.onContextLost = () => {
        // The camera is the one thing worth carrying over; the rest is rebuilt from the
        // last evaluation on a canvas of its own.
        const dead = viewer;
        if (!dead) return;
        try {
          const camera = dead.describeCamera();
          pendingCamera = { eye: camera.eye, center: camera.center, up: camera.up, radius: camera.radius };
        } catch {
          pendingCamera = null;
        }
        dead.dispose();
        viewer = null;
        canvas = null;
        void api.uiLog("warn", "the viewport lost its GPU context and was rebuilt");
        if (result) void drawScene(result).then(() => relaySelection());
      };
    } catch (error) {
      replace(stage, el("div", { class: "notice bad", text: String(error) }));
      return null;
    }
    return viewer;
  }

  /**
   * The style library, loaded once and refreshed whenever it changes.
   *
   * Held here rather than in the editor because it lives in the store: the editor draws it
   * and asks for changes, the screen owns the round trip.
   */
  async function loadStyles(): Promise<void> {
    try {
      editor.setStyleLibrary(await api.styleLibrary());
    } catch (error) {
      notice = `the style library could not be read: ${String(error)}`;
      renderHud();
    }
  }

  async function saveStyle(name: string, style: Style): Promise<void> {
    try {
      await api.styleSave(name, style);
      await loadStyles();
      notice = `saved the style "${name}"`;
    } catch (error) {
      notice = String(error);
    }
    renderHud();
  }

  async function deleteStyle(name: string): Promise<void> {
    const sure = await askConfirm({
      title: `Delete the style "${name}"?`,
      detail: "It is removed from the library for every world. Chains already drawn with it keep how they look.",
      accept: "Delete",
    });
    if (!sure) return;
    try {
      await api.styleDelete(name);
      await loadStyles();
    } catch (error) {
      notice = String(error);
    }
    renderHud();
  }

  /**
   * Puts a molecule in the graph, with a node for each of its chains.
   *
   * The structure is opened here rather than waiting for the graph to be evaluated,
   * because the chains are only known once it is open and a chain node is what puts the
   * molecule *in* the plan - so waiting is a deadlock. Fetching is one action for exactly
   * this reason: the molecule and its chains arrive together or not at all.
   */
  async function addMolecule(
    source: { pdb_id: string; path: string | null; assembly?: boolean },
    at: { x: number; y: number },
  ): Promise<StructureInfo | null> {
    let info: StructureInfo;
    // Said at once: this is the part that takes the time - a download, or a large file
    // being read - and it starts with a deliberate act, so there is no flicker to avoid.
    const name = source.path ? (source.path.split(/[\\/]/).pop() ?? "the file") : source.pdb_id.toUpperCase();
    loadingWhat = source.path ? `Reading ${name}` : `Fetching ${name} from the PDB`;
    loadingShown = true;
    notice = null;
    renderHud();
    try {
      info = source.path
        ? await api.openStructure(source.path, source.assembly ?? false)
        : await api.fetchStructure(source.pdb_id, source.assembly ?? true);
    } catch (error) {
      loadingShown = false;
      notice = String(error);
      renderHud();
      return null;
    }

    const id = `molecule-${Date.now().toString(36)}`;
    graph.nodes.push({
      type: "molecule",
      id,
      pdb_id: source.pdb_id,
      path: source.path,
      ...(source.assembly === undefined ? {} : { assembly: source.assembly }),
      chains: info.chains.map((chain) => chain.id),
      expanded: true,
      options: {
        show_hydrogens: preferences().showHydrogens,
        show_waters: preferences().showWaters,
        show_ligands: preferences().showLigands,
      },
      // One style slot, which every chain starts on - a Blender object arrives with one
      // material slot rather than one per face. The Style tab adds the rest.
      slots: [{ name: "Cartoon", style: startingStyle() }],
      active_slot: 0,
      position: [at.x, at.y],
    });
    buildChains(id, info.chains, at);
    editor.setGraph(graph);
    save();
    await evaluate();
    // Said after it is drawn, so it is what is left on screen: that this is not the
    // assembly that was asked for, and why.
    if (info.note) {
      notice = info.note;
      renderHud();
    }
    return info;
  }

  async function openMoleculeFile(at: { x: number; y: number }): Promise<void> {
    const picked = await openFileDialog({
      multiple: false,
      filters: [{ name: "Structure", extensions: ["pdb", "cif", "mmcif", "ent"] }],
    });
    if (typeof picked === "string") await addMolecule({ pdb_id: "", path: picked }, at);
  }

  /**
   * When each image was last written, by path, so a picture rendered again to the same
   * file is fetched again rather than shown from the cache under its old name.
   */
  const imageWritten = new Map<string, number>();
  function imageUrl(path: string): string {
    return `${convertFileSrc(path)}?v=${imageWritten.get(path) ?? 0}`;
  }

  /** The style a freshly fetched molecule is drawn with: a cartoon, coloured by fold. */
  function startingStyle(): Style {
    // As the preferences say a new molecule looks.
    const wanted = preferences();
    return {
      representations: [...wanted.representations],
      scheme: wanted.scheme,
      categorical: wanted.palette,
      continuous: false,
      solid: null,
      ramp: wanted.ramp,
      material: { roughness: 0.28, specular: 0.5, metallic: 0 },
      heteroatoms_by_element: wanted.heteroatomsByElement,
    };
  }

  /**
   * Builds a node per chain and the links to every output.
   *
   * Every output, because a render should draw what the viewport draws.
   */
  function buildChains(
    moleculeId: string,
    chains: { id: string; entity_type: string }[],
    at: { x: number; y: number },
  ): void {
    // The viewport only. A render follows the viewport's View, so wiring the chains to it
    // as well would give it two sources of the same scene.
    const outputs = graph.nodes
      .filter((node) => node.type === "viewport")
      .map((node) => node.id);

    // Every chain names the molecule's first slot rather than carrying a style of its
    // own. One slot used by four chains is one style to edit, which is what makes a
    // bundle of styles worth having - and a wired Style node is still the override.
    // Waters get no node. They are drawn or not by the molecule's own switch, and a
    // chain node per water asym - four of them in haemoglobin - would be four nodes with
    // nothing to say. Ligands do: a haem is a thing you style.
    const wanted = chains.filter((chain) => chain.entity_type !== "water");
    wanted.forEach((chain, index) => {
      const chainId = `chain-${chain.id}-${Date.now().toString(36)}-${index}`;
      graph.nodes.push({
        type: "chain",
        id: chainId,
        chain: chain.id,
        slot: 0,
        style: null,
        position: [at.x + 24, at.y + 44 + index * 78],
      });
      graph.links.push({ from: moleculeId, to: chainId, socket: "Molecule" });
      for (const output of outputs) {
        graph.links.push({ from: chainId, to: output, socket: "Geometry" });
      }
    });
  }

  /**
   * A molecule whose chains changed - a prediction finishing, say - gets nodes for them.
   *
   * Only when it has none: after that the chain nodes are the user's, to restyle, rewire
   * or delete, and one deleted on purpose must not come back on the next evaluation.
   */
  function chainsArrived(nodeId: string, chains: string[]): boolean {
    const molecule = graph.nodes.find((node) => node.id === nodeId);
    if (molecule?.type !== "molecule") return false;
    molecule.chains = chains;
    const known = new Map(
      structures.flatMap((entry) => entry.chains.map((chain) => [chain.id, chain.entity_type])),
    );
    // Read as another version - another assembly, another model - the molecule's chains
    // are another set: the ones still there keep their nodes and styles, the ones gone are
    // taken away, and the new ones get nodes. Only then: otherwise the chain nodes are the
    // user's, and one deleted on purpose stays deleted.
    if (rechaining.delete(nodeId)) {
      const keep = new Set(chains);
      const mine = graph.links
        .filter((link) => link.from === nodeId && link.socket === "Molecule")
        .map((link) => graph.nodes.find((node) => node.id === link.to))
        .filter((node): node is Extract<GraphNode, { type: "chain" }> => node?.type === "chain");
      const gone = new Set(mine.filter((node) => !keep.has(node.chain)).map((node) => node.id));
      graph.nodes = graph.nodes.filter((node) => !gone.has(node.id));
      graph.links = graph.links.filter((link) => !gone.has(link.from) && !gone.has(link.to));
      const have = new Set(mine.filter((node) => keep.has(node.chain)).map((node) => node.chain));
      buildChains(
        nodeId,
        chains.filter((id) => !have.has(id)).map((id) => ({ id, entity_type: known.get(id) ?? "polymer" })),
        { x: molecule.position[0], y: molecule.position[1] },
      );
      // The structure is another size and in another place: framed afresh.
      reframeNext = true;
      return true;
    }
    if (graph.links.some((link) => link.from === nodeId && link.socket === "Molecule")) {
      return true;
    }
    // The evaluation names the chains; their kinds come from the opened structure when it
    // is already known, and a chain nobody can classify yet is kept rather than dropped.
    buildChains(
      nodeId,
      chains.map((id) => ({ id, entity_type: known.get(id) ?? "polymer" })),
      { x: molecule.position[0], y: molecule.position[1] },
    );
    return true;
  }

  /** Throws the saved graph away and starts from the default pipeline. */
  async function resetGraph(): Promise<void> {
    if (saveTimer !== null) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    graph = await api.graphReset();
    graphEpoch += 1;
    forgetHistory();
    editor.setGraph(graph);
    await evaluate();
  }

  // --- undo ---------------------------------------------------------------------------
  //
  // The graph is the whole document, so undo is a stack of graphs. Every change already
  // passes through `save`, which makes that the one place to take a snapshot - there is no
  // list of commands to keep in step with the editor, and nothing an agent or a menu can
  // do that undo does not cover.

  const UNDO_LIMIT = 100;
  /** Changes this close together are one step: a slider dragged is not forty undos. */
  const UNDO_MERGE_MS = 700;
  /**
   * One step back: the graph as it was, or the view as it was. Moving the view is part
   * of the same history as editing the scene, in the order the two were done - "undo"
   * means the last thing, and someone who has just rotated away from the angle they
   * wanted means the rotation.
   */
  type ViewState = { eye: [number, number, number]; center: [number, number, number]; up: [number, number, number]; radius: number };
  type HistoryStep = string | { view: ViewState };
  let undoStack: HistoryStep[] = [];
  let redoStack: HistoryStep[] = [];
  /** The graph as of the last snapshot, which is what the next change is undone *to*. */
  let snapshot = "";
  let snapshotAt = 0;

  /** Starts history again from the graph as it stands - a world opened, a graph reset. */
  function forgetHistory(): void {
    undoStack = [];
    redoStack = [];
    settledView = null;
    snapshot = JSON.stringify(graph);
    snapshotAt = 0;
  }

  /**
   * Notes the graph as it now is. `undoable` is false for the app's own bookkeeping - the
   * chains a structure turned out to have, a job id - which moves the baseline without
   * making a step: undoing "the chains arrived" would only have them arrive again.
   */
  function remember(undoable: boolean): void {
    const now = JSON.stringify(graph);
    if (now === snapshot) return;
    if (undoable && snapshot !== "") {
      const at = performance.now();
      if (undoStack.length === 0 || at - snapshotAt > UNDO_MERGE_MS) {
        undoStack.push(snapshot);
        if (undoStack.length > UNDO_LIMIT) undoStack.shift();
      }
      snapshotAt = at;
      redoStack = [];
    }
    snapshot = now;
  }

  // --- the view's place in the history ---------------------------------------------------
  //
  // The view is moved from many places - a drag, the wheel, Zoom, a camera looked through,
  // the agent - so rather than each of them reporting it, the camera is looked at a few
  // times a second: when it has moved and then come to rest, where it rested before is a
  // step. A whole drag is therefore one undo, and so is a burst of scrolling.

  /** Where the view last came to rest, or null until it has (a world just opened). */
  let settledView: ViewState | null = null;
  /** What the last look saw, while the view is on the move. */
  let movingView: ViewState | null = null;
  /** How many looks in a row have seen the same thing. */
  let stillFor = 0;

  function currentView_(): ViewState | null {
    if (!viewer) return null;
    const camera = viewer.describeCamera();
    return { eye: camera.eye, center: camera.center, up: camera.up, radius: camera.radius };
  }

  function sameView(a: ViewState, b: ViewState): boolean {
    const near = (x: number[], y: number[]) => x.every((value, index) => Math.abs(value - y[index]) < 1e-3);
    return near(a.eye, b.eye) && near(a.center, b.center) && near(a.up, b.up) && Math.abs(a.radius - b.radius) < 1e-3;
  }

  const viewWatch = window.setInterval(() => {
    if (!element.isConnected) return;
    const now = currentView_();
    if (!now) return;
    const last = movingView ?? settledView;
    if (last && sameView(now, last)) stillFor += 1;
    else stillFor = 0;
    movingView = now;
    // A world that has just opened is framed and then put where it was left; neither is
    // the user moving the view, so nothing counts until it has been still for a second.
    if (settledView === null) {
      if (stillFor >= 4) settledView = now;
      return;
    }
    if (stillFor < 2 || sameView(now, settledView)) return;
    undoStack.push({ view: settledView });
    if (undoStack.length > UNDO_LIMIT) undoStack.shift();
    redoStack = [];
    settledView = now;
  }, 200);

  function stepHistory(from: HistoryStep[], to: HistoryStep[]): void {
    const target = from.pop();
    if (target === undefined) return;
    if (typeof target !== "string") {
      // A view step: the camera goes back, and where it was is what redo returns to.
      // Anything moved since the last look that had not yet been noted is that "where".
      const here = currentView_();
      if (here) to.push({ view: here });
      backToMain(false);
      viewer?.setCamera(target.view);
      settledView = target.view;
      movingView = null;
      editor.setHoverDetail("View restored.");
      return;
    }
    to.push(JSON.stringify(graph));
    graph = JSON.parse(target) as Graph;
    snapshot = target;
    // The next change after an undo is a new step, however soon it comes.
    snapshotAt = 0;
    editor.setGraph(graph);
    save(false);
    void evaluate();
  }

  const undo = () => stepHistory(undoStack, redoStack);
  const redo = () => stepHistory(redoStack, undoStack);

  window.addEventListener("keydown", (event) => {
    if (!element.isConnected) return;
    if (!(event.metaKey || event.ctrlKey) || event.altKey || event.key.toLowerCase() !== "z") return;
    // A text field has an undo of its own, and it is the one that is meant there.
    const target = event.target;
    if (target instanceof Element && target.closest("input, textarea, select, [contenteditable]")) return;
    event.preventDefault();
    fire(event.shiftKey ? "redo" : "undo");
  });

  /**
   * Writes the graph back to the store, coalesced.
   *
   * Every parameter drag produces dozens of changes a second and each one is a whole
   * graph; a short debounce turns a drag into one write without risking the last edit
   * being the one that is lost.
   */
  function save(undoable = true): void {
    remember(undoable);
    if (saveTimer !== null) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      void api.graphSave(graph).catch((error) => {
        notice = `the graph could not be saved: ${String(error)}`;
        renderHud();
      });
    }, 400);
  }

  /**
   * Evaluates the graph and draws the result.
   *
   * Serialised rather than concurrent: a slider drags out dozens of changes a second, and
   * each evaluation may fetch or open a structure. One in flight at a time with a single
   * pending re-run collapses a drag into the work it actually needs.
   */
  async function evaluate(): Promise<void> {
    if (evaluating) {
      pendingEvaluation = true;
      return;
    }
    evaluating = true;
    // Said only once it has gone on long enough to be noticed: a slider's re-evaluation is
    // over in a frame or two, and a line that flickers on and off for each is worse than
    // none. A capsid being fetched, read and drawn is seconds, and those must not be silent.
    const slow = window.setTimeout(() => {
      loadingShown = true;
      renderHud();
    }, 250);
    try {
      do {
        pendingEvaluation = false;
        const epoch = graphEpoch;
        loadingWhat = "Opening the structure";
        const next = await api.graphEvaluate(graph);
        // The graph was replaced while this was out - another world was opened. What came
        // back describes the old one: its chains must not be written onto nodes that
        // merely share an id, nor its scene drawn. Asked again, of the graph there is now.
        if (epoch !== graphEpoch) {
          pendingEvaluation = true;
          continue;
        }
        result = next;
        editor.setEvaluation(next.nodes, next.evaluation.node_states);

        // The chains the structure turned out to have go onto the group, which is what
        // gives it a style socket per chain. Recorded only when they change, so this does
        // not re-enter evaluation on every pass.
        if (next.chains.length > 0) {
          let touched = false;
          for (const discovered of next.chains) {
            touched = chainsArrived(discovered.node_id, discovered.chains) || touched;
          }
          if (touched) {
            editor.setGraph(graph);
            save(false);
            pendingEvaluation = true;
          }
        }

        // A prediction that was just submitted has to be recorded on its node, or the next
        // evaluation submits it again.
        if (next.submitted) {
          const submitted = next.submitted;
          const node = graph.nodes.find((candidate) => candidate.id === submitted.node_id);
          if (node && node.type === "predict") {
            node.job_id = submitted.job_id;
            editor.setGraph(graph);
            // Written down at once. A graph saved without the job id is a graph that
            // submits the same prediction again the next time it is opened.
            save(false);
          }
          watchJobs();
        }

        loadingWhat = "Drawing";
        if (loadingShown) renderHud();
        await drawScene(next);
      } while (pendingEvaluation);
    } catch (error) {
      notice = String(error);
      renderHud();
    } finally {
      evaluating = false;
      window.clearTimeout(slow);
      if (loadingShown) {
        loadingShown = false;
        renderHud();
      }
    }

    // After the guard has been released, deliberately. The scripted view changes the
    // graph and asks for a re-evaluation; called from inside an evaluation that
    // re-entrant call is swallowed by the guard above, and the frame it then captured was
    // the one already on screen. Every scripted screenshot was of the scene before the
    // change it had just made.
    if (!scripted && viewer && structures.length > 0) {
      scripted = true;
      await runScriptedView(viewer);
    }
  }

  async function drawScene(next: GraphResult): Promise<void> {
    const plan = next.evaluation.plan;
    structures = next.structures;
    refreshPanes();
    // New geometry means new ids; a cached hover mask would highlight the wrong atoms.
    hoverMasks.clear();
    hoverViewMasks.clear();
    hoveredKey = null;
    hoveredViewKey = null;
    if (!plan || next.structures.length === 0) {
      // Nothing to draw is a picture too. Returning with the last scene still up left a
      // molecule on screen that was no longer in the graph - unwire the last chain, or
      // open an empty world after a full one - with the panel and the title still
      // describing it and nothing under the pointer to pick.
      viewer?.clearScene();
      viewer?.setGraph(null);
      sceneStructures = [];
      // The graph view's bonds went with the scene; a picked one's card must go too.
      shownGraph = null;
      pickedEdge = null;
      void biomol.setStructures([]);
      titleSlot.textContent = "";
      if (currentView) await applySelection(null, "replace", true);
      renderHud();
      return;
    }

    const instance = ensureStage();
    if (!instance) return;
    instance.setStyle(plan.style);

    notice = null;
    const total = next.structures.reduce((sum, entry) => sum + entry.atom_count, 0);
    // The plan's parts and the opened structures are the same list, in the same order.
    let parts = plan.parts.map((part, index) => ({
      structure_id: next.structures[index].id,
      options: part.options,
      assignments: part.assignments,
      features: part.features ?? [],
    }));
    if (gpu.software && total > SOFTWARE_ATOM_LIMIT) {
      // Software rendering cannot carry a million impostors. Every style is forced to a
      // cartoon rather than only the unassigned one, or a chain assigned spacefill would
      // still sink it.
      parts = parts.map((part) => ({
        ...part,
        assignments: part.assignments.map((assignment) => ({
          ...assignment,
          // Anything that was drawn stays drawn, as a cartoon; a style drawing nothing
          // still draws nothing.
          style: {
            ...assignment.style,
            representations: assignment.style.representations.length
              ? (["cartoon"] as Representation[])
              : [],
          },
        })),
      }));
      notice =
        `No hardware-accelerated renderer was found (${gpu.renderer}), so this ` +
        `${total.toLocaleString()}-atom scene is drawn as a cartoon.`;
    }

    const firstLoad = !instance.currentScene || reframeNext;
    reframeNext = false;
    lastSceneRequest = { parts, style: plan.style };
    const payload = await api.buildScene({ parts, style: plan.style });
    // Reframe only when there was nothing on screen: re-evaluating after a slider move
    // must not throw away a camera the user positioned.
    const relaid = await instance.setScene(payload, firstLoad);
    sceneStructures = next.structures;
    // Each structure is analysed once, as it first comes on screen; what was shown of an
    // earlier one's contacts is redrawn against the scene there is now.
    for (const structure of next.structures) analyse(structure);
    if (shownEdges.size > 0) drawContacts();
    // The scene's ids moved - a molecule came or went - so the outline has to be laid
    // again where the selection now is, or dropped if what was selected is gone.
    if (relaid && currentView) await relaySelection();
    // Where the world was left, applied once there is a scene to apply it to. Done here
    // rather than by whoever opened the world: an evaluation already under way makes
    // theirs return at once, before anything is drawn, and the framing of the first
    // scene would then replace the camera they had just set.
    if (pendingCamera) {
      instance.setCamera(pendingCamera);
      pendingCamera = null;
      settledView = null;
    }
    // The first scene's own framing is not a move of the user's to undo.
    if (firstLoad) settledView = null;

    const chainCount = next.structures.reduce(
      (sum, entry) => sum + entry.chains.length,
      0,
    );
    void biomol.setStructures(next.structures);
    void loadGraph();
    titleSlot.textContent =
      next.structures.length === 1
        ? `${next.structures[0].title || next.structures[0].source} - ` +
          `${count(total, "atom")}, ${count(chainCount, "chain")}`
        : `${count(next.structures.length, "molecule")} - ${count(total, "atom")}, ` +
          `${count(chainCount, "chain")}`;
    titleSlot.title = titleSlot.textContent ?? "";
    worldTitle.title = titleSlot.title;
    renderHud();

    if (firstLoad) {
      const stats = instance.renderStats();
      // Read through `viewer` rather than `instance`: `firstLoad` was derived from
      // `instance.currentScene` being null, and the compiler keeps that narrowing on the
      // same expression even after the scene has been set.
      const scene: Scene | null | undefined = viewer?.currentScene;
      void api.uiLog(
        "viewer",
        `${next.structures.map((entry) => entry.source).join(" + ")}: ` +
          `${scene?.header.drawn_atoms ?? 0} atoms drawn, ` +
          `${scene?.header.triangle_count ?? 0} triangles, coverage ` +
          `${(stats.coverage * 100).toFixed(1)}%, renderer ${gpu.renderer}`,
      );
    }
  }

  /** Re-evaluates when a job the graph is waiting on changes state. */
  function watchJobs(): void {
    if (unlistenJobs) return;
    void events
      .onJobChanged((job) => {
        const waiting = graph.nodes.some(
          (node) => node.type === "predict" && node.job_id === job.id,
        );
        if (waiting) void evaluate();
      })
      .then((stop) => (unlistenJobs = stop));
  }

  /**
   * Path-traces one Render node with Blender.
   *
   * The node's inputs decide the scene and its own fields decide the quality; the camera
   * comes from the viewport, so a render is the view on screen rather than a second guess
   * at the same framing. The plan itself is resolved in the core from the same
   * `graph::resolve` the viewport used - resolving it here as well is how a render starts
   * to disagree with the preview it was set up in.
   */
  async function renderNode(nodeId: string, output?: string): Promise<RenderOutcome | null> {
    // Said rather than returned quietly: from Save image the only sign of a render that
    // never started was a "Rendering…" line that stayed up.
    if (!viewer) {
      notice = "There is no viewport to take the render's camera from.";
      renderHud();
      return null;
    }
    if (renderProgress[nodeId]?.rendering) {
      notice = "That render is still running.";
      renderHud();
      return null;
    }
    renderProgress = { ...renderProgress, [nodeId]: { rendering: true } };
    pushRenderContext();
    const world = activeWorld;
    // The scene as it is sent, not as it is when the render comes back minutes later: a
    // view turned meanwhile is not in the picture, and the render has to read as stale.
    const stamp = sceneStamp();
    // A render is minutes, so how far Blender has got is shown as it goes, from its own
    // status line; the gauge closes when the picture is in.
    const gauge = showProgress("Render", () => void api.renderCancel());
    const stopListening = events.onRenderProgress((progress) => gauge.set(progress.fraction, progress.stage));
    try {
      const camera = viewer.describeCamera();
      const outcome = await api.graphRender(
        graph,
        nodeId,
        {
          eye: camera.eye,
          center: camera.center,
          up: camera.up,
          radius: camera.radius,
          view_aspect: camera.view_aspect,
        },
        output ?? null,
      );
      if (activeWorld === world) {
        renderProgress = {
          ...renderProgress,
          [nodeId]: { rendering: false, image: outcome.image, seconds: outcome.seconds },
        };
      }
      // The node remembers where its image went, and the image itself becomes a node.
      const node = graph.nodes.find((candidate) => candidate.id === nodeId);
      // A render takes minutes and the world can be changed in that time. Node ids are
      // not unique across worlds - every default pipeline has a "render" - so a result
      // filed by id alone lands in whichever world happens to be open.
      if (activeWorld !== world) return outcome;
      if (node?.type === "render") node.output = outcome.image;
      imageWritten.set(outcome.image, Date.now());
      attachImage(nodeId, outcome.image);
      renderedAs = stamp;
      // Bookkeeping, not an edit: an undo straight after a render should take back the
      // last thing the user changed, not the render's record of where its file went.
      save(false);
      void evaluate();
      gauge.finish(`Rendered in ${outcome.seconds.toFixed(1)} s`);
      return outcome;
    } catch (error) {
      // Stopped on purpose is not a failure, and says so briefly.
      const cancelled = /cancelled/.test(String(error));
      if (cancelled) gauge.fail("Cancelled");
      else gauge.fail(`The render failed: ${String(error)}`);
      if (cancelled) {
        if (activeWorld === world) renderProgress = { ...renderProgress, [nodeId]: { rendering: false } };
        return null;
      }
      if (activeWorld === world) {
        renderProgress = {
          ...renderProgress,
          [nodeId]: { rendering: false, error: String(error) },
        };
        // On the node, and here too: the node graph may not be on the desk, and the agent's
        // render tool reports this notice as the reason.
        notice = `the render failed: ${String(error)}`;
      }
      return null;
    } finally {
      void stopListening.then((stop) => stop());
      pushRenderContext();
      renderHud();
    }
  }

  /**
   * Puts the image a render produced onto an Image node.
   *
   * Reuses the one already connected to that Render node rather than adding another, so
   * rendering repeatedly refines one result instead of littering the plane - but the first
   * render does create the node, which is where the picture becomes something in the graph
   * that can be copied and saved.
   */
  function attachImage(renderNodeId: string, image: string): void {
    const source = graph.nodes.find((node) => node.id === renderNodeId);
    if (!source) return;

    const existing = graph.links
      .filter((link) => link.from === renderNodeId && link.socket === "Image")
      .map((link) => graph.nodes.find((node) => node.id === link.to))
      .find((node) => node?.type === "image");

    if (existing?.type === "image") {
      existing.path = image;
    } else {
      const id = `image-${Date.now().toString(36)}`;
      graph.nodes.push({
        type: "image",
        id,
        path: image,
        position: [source.position[0] + 296, source.position[1]],
      });
      graph.links.push({ from: renderNodeId, to: id, socket: "Image" });
    }
    editor.setGraph(graph);
  }

  /**
   * Copies a rendered image to the clipboard.
   *
   * Handed to the core, because the webview's clipboard API only permits a write inside
   * the user gesture that started it - and reading the image's bytes first loses that
   * gesture, after which WebKit resolves the promise anyway. The first version of this
   * reported success and copied nothing.
   */
  async function copyImage(path: string): Promise<void> {
    try {
      await api.copyImage(path);
      notice = "image copied";
    } catch (error) {
      notice = `could not copy the image: ${String(error)}`;
    }
    renderHud();
  }

  async function saveImageAs(path: string): Promise<void> {
    const target = await saveFileDialog({
      // The render's own name when it has one worth keeping, `untitled.png` otherwise.
      defaultPath: (() => {
        const name = path.split("/").pop() ?? "";
        return name === "" || name.startsWith("untitled") ? "untitled.png" : name;
      })(),
      filters: [{ name: "PNG", extensions: ["png"] }],
    });
    if (typeof target !== "string") return;
    const gauge = showProgress(`Saving ${target.split("/").pop() ?? "the render"}`);
    gauge.set(null, "Copying the render");
    try {
      const saved = await api.exportImage(path, target);
      gauge.finish(`Saved to ${saved}`);
      notice = `saved to ${saved}`;
    } catch (error) {
      gauge.fail(`Could not save the render: ${String(error)}`);
      notice = String(error);
    }
    renderHud();
  }

  /**
   * Hands the viewport whichever light is selected, so alt-drag aims it.
   *
   * Selection lives in the graph and manipulation in the viewport: the graph is where you
   * say which thing you mean, and the viewport is where you can see what moving it does.
   */
  function aimSelectedLight(nodeId: string | null): void {
    const instance = viewer;
    if (!instance) return;
    const node = nodeId ? graph.nodes.find((candidate) => candidate.id === nodeId) : null;
    instance.setLightAim(
      node?.type === "light" ? { id: node.id, direction: node.light.direction } : null,
    );
    instance.onLightAim = (id, direction) => {
      const light = graph.nodes.find((candidate) => candidate.id === id);
      if (light?.type !== "light") return;
      light.light = { ...light.light, direction };
      editor.setGraph(graph);
      save();
      void evaluate();
    };
    renderHud();
  }

  /**
   * Turns a click in the viewport into a selection.
   *
   * The viewport reports an atom or a residue in the *scene's* id space; which molecule
   * that belongs to comes from the part offsets, and what it is called comes from the
   * core. For now the result selects the chain, which is the level the graph has nodes
   * for; the same call already returns the residue and the atom, for when those become
   * selectable.
   */
  async function resolvePick(
    picked: { kind: "atom" | "residue" | "edge"; index: number } | null,
    mode: Combine,
  ): Promise<void> {
    const instance = viewer;
    const scene = instance?.currentScene;
    if (!instance || !scene) return;

    if (picked?.kind === "edge") {
      // A bond is picked as itself and as what it joins: the bond turns the selection's
      // colour and gets a card saying what it is, and its two components are selected, so
      // the sequence shows where along the chains they are.
      const edge = shownGraph?.edgeInfo[picked.index];
      const structure = edge ? structures[edge.structure] : null;
      if (!edge || !structure) return;
      pickedEdge = picked.index;
      instance.setGraphEdges(null, pickedEdge);
      await applySelection({ structureId: structure.id, level: "residue", indices: [...edge.residues] }, "replace");
      renderHud();
      return;
    }
    if (pickedEdge !== null) {
      pickedEdge = null;
      instance.setGraphEdges(null, null);
      renderHud();
    }

    if (!picked) {
      // A click on the background clears - unless a modifier is held, since shift-clicking
      // past the edge of a molecule while building a selection is not a request to lose it.
      if (mode === "replace") void applySelection(null, "replace");
      return;
    }

    // Which merged part holds this id.
    const parts = scene.header.parts;
    const partIndex = parts.findIndex((part) =>
      picked.kind === "atom"
        ? picked.index >= part.atom_offset && picked.index < part.atom_offset + part.atom_count
        : picked.index >= part.residue_offset &&
          picked.index < part.residue_offset + part.residue_count,
    );
    const part = parts[partIndex];
    const structure = sceneStructures[partIndex];
    if (!part || !structure) return;

    const local =
      picked.kind === "atom" ? picked.index - part.atom_offset : picked.index - part.residue_offset;
    let target: PickTarget | null = null;
    try {
      target = await api.pickTarget(structure.id, picked.kind, local);
    } catch (error) {
      void api.uiLog("warn", `could not resolve a pick: ${String(error)}`);
      return;
    }
    if (!target) return;

    // At the bond level a click on anything that is not a bond is not a selection.
    if (pickLevel === "link") {
      editor.setHoverDetail("Bonds are picked in the graph view (V, Mode, Graph).");
      return;
    }
    // The level bar says what a click means: the chain, the residue, or the atom - and a
    // ribbon click at the atom level is the residue, since a ribbon has no atoms.
    const chainIndex = structure.chains.findIndex((chain) => chain.id === target.chain);
    const view: View =
      pickLevel === "molecule"
        ? // The whole molecule: every chain it has, polymer and not.
          { structureId: structure.id, level: "chain", indices: structure.chains.map((_, index) => index) }
        : pickLevel === "chain"
        ? { structureId: structure.id, level: "chain", indices: chainIndex >= 0 ? [chainIndex] : [] }
        : pickLevel === "residue" || target.atom_index === null
          ? { structureId: structure.id, level: "residue", indices: [target.residue_index] }
          : { structureId: structure.id, level: "atom", indices: [target.atom_index] };
    await applySelection(view, mode);
  }

  // --- the selection ----------------------------------------------------------------
  //
  // One selection, a BioMol `View`: a structure, a level, indices. Every way of selecting
  // - a click in the viewport, a row in the BioMol tree, a drag on the strip, a chain in
  // the outliner - ends in `applySelection`, and everything that shows a selection - the
  // outline, the card, the tree, the strip, the graph's selected node - is set there.

  let currentView: View | null = null;
  /** True while `applySelection` is selecting a graph node, so the outliner's callback stays quiet. */
  let syncingSelection = false;
  /** Which request is the latest, so a slow mask from mid-drag cannot overwrite the final one. */
  let selectionRequest = 0;

  /** Masks by view key, so hovering back and forth across the map does not refetch. */
  const hoverViewMasks = new Map<string, { atoms: Uint8Array; residues: Uint8Array }>();
  let hoveredViewKey: string | null = null;

  /** A hover from the BioMol map, outlined in the viewport like a hover in the viewport. */
  async function hoverView(view: View | null): Promise<void> {
    const instance = viewer;
    const scene = instance?.currentScene;
    if (!instance || !scene) return;
    const key = viewKey(view);
    if (key === hoveredViewKey) return;
    hoveredViewKey = key;
    hoveredKey = null;
    if (!view) {
      instance.setHover(null, null);
      editor.setHoverDetail(null);
      return;
    }
    const located = partFor(view.structureId);
    const part = located ? scene.header.parts[located.partIndex] : undefined;
    if (!located || !part) return;
    let masks = hoverViewMasks.get(key);
    if (!masks) {
      try {
        const mask = await api.viewMask(view.structureId, { level: view.level, indices: view.indices });
        masks = layMask(scene.header.atom_count, scene.header.residue_count, part, mask);
        hoverViewMasks.set(key, masks);
      } catch {
        return;
      }
    }
    if (hoveredViewKey === key) instance.setHover(masks.atoms, masks.residues);
  }

  /** The scene part and structure a view is about, by structure id. */
  function partFor(structureId: string): { partIndex: number; structure: StructureInfo } | null {
    const partIndex = sceneStructures.findIndex((entry) => entry.id === structureId);
    const structure = sceneStructures[partIndex];
    return structure ? { partIndex, structure } : null;
  }

  /**
   * `keepNode` clears what is selected in the structure and leaves the graph's own
   * selection alone - for when a node that is not part of any molecule was the thing
   * selected.
   */
  async function applySelection(pick: View | null, mode: Combine, keepNode = false): Promise<void> {
    const request = ++selectionRequest;
    const instance = viewer;
    const scene = instance?.currentScene;
    const view = pick ? combine(currentView, pick, mode) : null;
    currentView = view;

    if (!view || view.indices.length === 0 || !instance || !scene) {
      currentView = null;
      instance?.setSelection(null, null);
      biomol.setSelection(null, new Set(), []);
      if (!keepNode) {
        syncingSelection = true;
        editor.select(null);
        syncingSelection = false;
      }
      editor.setSelectionDetail(null);
      actionTarget = null;
      drawTarget();
      return;
    }

    const located = partFor(view.structureId);
    const part = located ? scene.header.parts[located.partIndex] : undefined;
    if (!located || !part) return;
    const { structure } = located;

    try {
      const core = { level: view.level, indices: view.indices };
      const [mask, summary] = await Promise.all([
        api.viewMask(structure.id, core),
        api.describeView(structure.id, core),
      ]);
      if (request !== selectionRequest) return;

      const laid = layMask(scene.header.atom_count, scene.header.residue_count, part, mask);
      instance.setSelection(laid.atoms, laid.residues);

      const residues = new Set<number>(summary.residue_indices);
      mask.residues.forEach((flag, index) => {
        if (flag) residues.add(index);
      });
      biomol.setSelection(view, residues, summary.chain_ids);
      actionTarget = {
        structureId: structure.id,
        chains: [...summary.chain_ids],
        level: view.level,
        label: summary.label,
      };
      drawTarget();

      // The graph follows: the chain's node when the selection is within one chain, the
      // molecule's when it spans several - so the properties panel shows the right thing.
      const molecule = moleculeNodeFor(structure);
      const chainNode =
        summary.chain_ids.length === 1 && molecule
          ? graph.nodes.find(
              (node) =>
                node.type === "chain" &&
                node.chain === summary.chain_ids[0] &&
                graph.links.some((link) => link.from === molecule.id && link.to === node.id && link.socket === "Molecule"),
            )
          : null;
      syncingSelection = true;
      editor.select(chainNode?.id ?? molecule?.id ?? null);
      syncingSelection = false;

      const entry = (structure.entry_id || structure.title).toUpperCase();
      const chain = summary.chain_ids.length === 1 ? structure.chains.find((c) => c.id === summary.chain_ids[0]) : null;
      // One line says where; a second says what, when the selection is finer than a chain.
      editor.setSelectionDetail([
        chain
          ? `${entry} · auth ${chain.auth_id} · ${chain.entity_type} · ${chain.residue_count} res`
          : `${entry} · ${count(summary.chains, "chain")} · ${count(summary.atoms, "atom")}`,
        ...(view.level === "chain" ? [] : [summary.label]),
      ]);
    } catch (error) {
      void api.uiLog("warn", `could not show the selection: ${String(error)}`);
    }
  }

  /**
   * Lays the current selection's outline into a scene whose ids have moved, touching
   * nothing else: not the graph's selected node, not the card. Drops the selection if the
   * structure it was in is no longer drawn.
   */
  async function relaySelection(): Promise<void> {
    const view = currentView;
    const instance = viewer;
    const scene = instance?.currentScene;
    if (!view || !instance || !scene) return;
    const located = partFor(view.structureId);
    const part = located ? scene.header.parts[located.partIndex] : undefined;
    if (!located || !part) {
      await applySelection(null, "replace", true);
      return;
    }
    const request = ++selectionRequest;
    try {
      const mask = await api.viewMask(located.structure.id, { level: view.level, indices: view.indices });
      if (request !== selectionRequest || instance.currentScene !== scene) return;
      const laid = layMask(scene.header.atom_count, scene.header.residue_count, part, mask);
      instance.setSelection(laid.atoms, laid.residues);
    } catch (error) {
      void api.uiLog("warn", `could not restore the selection: ${String(error)}`);
    }
  }

  /** The Molecule node a structure came from, matched by what it points at. */
  function moleculeNodeFor(structure: StructureInfo): GraphNode | null {
    const plan = result?.evaluation.plan;
    const partIndex = structures.indexOf(structure);
    const reference = plan?.parts[partIndex]?.structure;
    if (!reference) return null;
    const pointsAt = (node: GraphNode) => {
      if (node.type !== "molecule") return false;
      if (reference.source === "pdb") return reference.id.toLowerCase() === node.pdb_id.trim().toLowerCase();
      if (reference.source === "file") return reference.path === node.path;
      return false;
    };
    // The same entry can be in a world twice. The parts are in the order of the molecules,
    // so the second part that points at 1UF2 is the second molecule that does.
    const same = (plan?.parts ?? []).slice(0, partIndex).filter((part) => JSON.stringify(part.structure) === JSON.stringify(reference)).length;
    return graph.nodes.filter(pointsAt)[same] ?? graph.nodes.find(pointsAt) ?? null;
  }

  /** The mask for one chain, residue or atom of a structure, laid into the scene's ids. */
  async function levelMask(
    structureId: string,
    partIndex: number,
    target: PickTarget,
    level: "chain" | "residue" | "atom",
  ): Promise<{ atoms: Uint8Array; residues: Uint8Array } | null> {
    const scene = viewer?.currentScene;
    const part = scene?.header.parts[partIndex];
    if (!scene || !part) return null;
    const mask = await api.selectionMask(
      structureId,
      level === "chain" ? [target.chain] : [],
      level === "residue"
        ? { residue: target.residue_index }
        : level === "atom" && target.atom_index !== null
          ? { atom: target.atom_index }
          : level === "atom"
            ? { residue: target.residue_index }
            : undefined,
    );
    return layMask(scene.header.atom_count, scene.header.residue_count, part, mask);
  }

  /**
   * Fills in the corner card: the entry, the chain by both its names, what was clicked.
   *
   * Short by design. `4HHB · auth A · polymer · 141 res` says where you are; a second line
   * names the residue and atom when a click landed on one.
   */
  function describeSelection(nodeId: string | null, target: PickTarget | null): void {
    const node = nodeId ? graph.nodes.find((candidate) => candidate.id === nodeId) : null;
    if (!node) {
      editor.setSelectionDetail(null);
      return;
    }
    const located = locateMolecule(node);
    const lines: string[] = [];
    if (located) {
      const { structure, chain } = located;
      const entry = (structure.entry_id || structure.title).toUpperCase();
      if (chain) {
        lines.push(
          `${entry} · auth ${chain.auth_id} · ${chain.entity_type} · ${chain.residue_count} res`,
        );
      } else {
        lines.push(
          `${entry} · ${count(structure.chains.filter((c) => c.entity_type !== "water").length, "chain")} · ${count(structure.atom_count, "atom")}`,
        );
      }
    }
    if (target) {
      lines.push(
        target.atom_name
          ? `${target.residue_name} ${target.residue_number} · ${target.atom_name}${target.element ? ` (${target.element})` : ""}`
          : `${target.residue_name} ${target.residue_number}`,
      );
    }
    editor.setSelectionDetail(lines.length > 0 ? lines : null);
  }

  /** The structure a molecule or chain node stands for, and the chain when it is one. */
  function locateMolecule(
    node: GraphNode,
  ): { structure: StructureInfo; chain: ChainInfo | null } | null {
    const chainNode = node.type === "chain" ? node : null;
    const molecule =
      node.type === "molecule"
        ? node
        : chainNode
          ? graph.nodes.find(
              (candidate) =>
                candidate.type === "molecule" &&
                graph.links.some(
                  (link) =>
                    link.from === candidate.id &&
                    link.to === chainNode.id &&
                    link.socket === "Molecule",
                ),
            )
          : null;
    if (!molecule || molecule.type !== "molecule") return null;
    const plan = result?.evaluation.plan;
    const partIndex =
      plan?.parts.findIndex((part) => {
        const reference = part.structure;
        if (reference.source === "pdb") {
          return reference.id.toLowerCase() === molecule.pdb_id.trim().toLowerCase();
        }
        if (reference.source === "file") return reference.path === molecule.path;
        return false;
      }) ?? -1;
    const structure = structures[partIndex];
    if (!structure) return null;
    const chain = chainNode
      ? (structure.chains.find((candidate) => candidate.id === chainNode.chain) ?? null)
      : null;
    return { structure, chain };
  }

  /** The last chain the pointer was over, so a hover that stays put costs nothing. */
  let hoveredKey: string | null = null;

  /**
   * Outlines whatever the pointer is over, faintly, before it is clicked.
   *
   * The pick is throttled in the viewer; this side is throttled by identity - the mask is
   * only rebuilt when the pointer moves onto a *different* chain, and cached per chain
   * after that, so sweeping across one chain is one round trip.
   */
  const hoverMasks = new Map<string, { atoms: Uint8Array; residues: Uint8Array }>();
  /** One line for a bond: what it is, what it joins, how long. */
  const describeEdge = (edge: GraphEdge) =>
    `${edge.what} · ${edge.names[0]} (${edge.chains[0]}) – ${edge.names[1]} (${edge.chains[1]}) · ${edge.length.toFixed(1)} Å`;

  async function resolveHover(
    picked: { kind: "atom" | "residue" | "edge"; index: number } | null,
  ): Promise<void> {
    const instance = viewer;
    const scene = instance?.currentScene;
    if (!instance || !scene) return;
    if (picked?.kind === "edge") {
      const edge = shownGraph?.edgeInfo[picked.index];
      instance.setGraphEdges(picked.index, pickedEdge);
      instance.setHover(null, null);
      hoveredKey = `edge:${picked.index}`;
      biomol.setHover(null);
      editor.setHoverDetail(edge ? `⌖ ${describeEdge(edge)}` : null);
      return;
    }
    instance.setGraphEdges(null, pickedEdge);
    if (!picked) {
      if (hoveredKey !== null) {
        hoveredKey = null;
        instance.setHover(null, null);
        editor.setHoverDetail(null);
        biomol.setHover(null);
      }
      return;
    }
    const parts = scene.header.parts;
    const partIndex = parts.findIndex((part) =>
      picked.kind === "atom"
        ? picked.index >= part.atom_offset && picked.index < part.atom_offset + part.atom_count
        : picked.index >= part.residue_offset &&
          picked.index < part.residue_offset + part.residue_count,
    );
    const part = parts[partIndex];
    const structure = sceneStructures[partIndex];
    if (!part || !structure) return;
    const local =
      picked.kind === "atom" ? picked.index - part.atom_offset : picked.index - part.residue_offset;
    let target: PickTarget | null = null;
    try {
      target = await api.pickTarget(structure.id, picked.kind, local);
    } catch {
      return;
    }
    if (!target) return;
    // The map glows where the viewport is pointing.
    const hoverChainIndex = structure.chains.findIndex((chain) => chain.id === target.chain);
    // A molecule is pointed at by the chain under the pointer; the click takes all of it.
    const byChain = pickLevel === "chain" || pickLevel === "molecule";
    biomol.setHover(
      byChain
        ? { structureId: structure.id, level: "chain", indices: hoverChainIndex >= 0 ? [hoverChainIndex] : [] }
        : { structureId: structure.id, level: "residue", indices: [target.residue_index] },
    );
    // What the pointer is over, read out on the card before anything is clicked.
    editor.setHoverDetail(
      `⌖ ${target.residue_name} ${target.residue_number}${target.atom_name ? ` · ${target.atom_name}` : ""} · ${target.chain} (auth ${target.auth_chain})`,
    );
    const key =
      byChain
        ? `${structure.id}:${target.chain}`
        : pickLevel === "residue"
          ? `${structure.id}:r${target.residue_index}`
          : `${structure.id}:a${target.atom_index ?? `r${target.residue_index}`}`;
    if (key === hoveredKey) return;
    hoveredKey = key;

    let masks = hoverMasks.get(key);
    if (!masks) {
      try {
        masks = (await levelMask(structure.id, partIndex, target, byChain ? "chain" : pickLevel === "residue" ? "residue" : "atom")) ?? undefined;
        if (masks) hoverMasks.set(key, masks);
      } catch {
        return;
      }
      if (!masks) return;
    }
    // The pointer may have moved on while the mask was being built.
    if (hoveredKey === key) instance.setHover(masks.atoms, masks.residues);
  }

  /**
   * The outliner's selection as a view: a chain node is that chain, a molecule node is all
   * of its chains, anything else clears. From here on it is the same selection as a click.
   */
  async function highlightSelection(nodeId: string | null): Promise<void> {
    const node = nodeId ? graph.nodes.find((candidate) => candidate.id === nodeId) : null;
    const located = node ? locateMolecule(node) : null;
    if (!located) {
      // A light, a style, a render: not part of any molecule, so nothing in the structure
      // is selected - but the node itself still is. Clearing that as well deselected
      // every such node in the instant it was clicked, taking its properties tab, the
      // delete key and a light's alt-drag aim with it.
      await applySelection(null, "replace", nodeId !== null);
      return;
    }
    const { structure, chain } = located;
    const indices = chain
      ? [structure.chains.findIndex((candidate) => candidate.id === chain.id)].filter((index) => index >= 0)
      : structure.chains.map((_, index) => index);
    await applySelection({ structureId: structure.id, level: "chain", indices }, "replace");
  }

  // --- what an agent can reach ------------------------------------------------------
  //
  // The tools in `agent/tools.ts` say what an agent may do; this is how each of them is
  // done, and it is deliberately the same code paths the user's own clicks take - a
  // restyle by an agent goes through the graph, the save and the re-evaluation exactly as
  // a restyle by hand does, so there is one way for the scene to change and one way for it
  // to be undone.

  /** A molecule node and the structure it opened, paired. */
  function molecules(): { node: GraphNode; structure: StructureInfo }[] {
    return structures
      .map((structure) => ({ node: moleculeNodeFor(structure), structure }))
      .filter((entry): entry is { node: GraphNode; structure: StructureInfo } => entry.node !== null);
  }

  /** The molecule an argument names, or the only one there is. */
  function resolveMolecule(id?: string): { node: GraphNode; structure: StructureInfo } {
    const open = molecules();
    if (open.length === 0) throw new Error("no molecule is open");
    if (!id) {
      if (open.length === 1) return open[0];
      throw new Error(`say which molecule: ${open.map((entry) => entry.node.id).join(", ")}`);
    }
    const found = open.find((entry) => entry.node.id === id);
    if (!found) throw new Error(`no molecule called ${id}; open ones are ${open.map((entry) => entry.node.id).join(", ")}`);
    return found;
  }

  /** A chain by either of its names - the label id `A_1` or the author's letter. */
  function resolveChain(structure: StructureInfo, chain: string): ChainInfo {
    const wanted = chain.trim().toLowerCase();
    const found =
      structure.chains.find((entry) => entry.id.toLowerCase() === wanted) ??
      structure.chains.find((entry) => entry.auth_id.toLowerCase() === wanted);
    if (!found) throw new Error(`no chain ${chain}; this molecule has ${structure.chains.map((entry) => entry.id).join(", ")}`);
    return found;
  }

  /** Which slot an argument names: a position, a name, or the active one. */
  function resolveSlot(node: GraphNode, slot: number | string | undefined): number {
    if (node.type !== "molecule") throw new Error("not a molecule");
    if (slot === undefined) return node.active_slot;
    if (typeof slot === "number") {
      if (slot < 0 || slot >= node.slots.length) throw new Error(`no slot ${slot}; there are ${node.slots.length}`);
      return slot;
    }
    const index = node.slots.findIndex((entry) => entry.name.toLowerCase() === String(slot).trim().toLowerCase());
    if (index < 0) throw new Error(`no slot called ${slot}; there are ${node.slots.map((entry) => entry.name).join(", ")}`);
    return index;
  }

  /** The style fields an agent may set, applied to a style. */
  /**
   * A model's arguments are text it wrote, not values the editor's own controls produced,
   * and they are checked as such. Anything unrecognised is refused with the list of what
   * would have been accepted - an error the loop hands straight back to the model. Written
   * into the graph unchecked, `"sticks"` for a list or `"magenta"` for a colour made a
   * graph the core would not evaluate or save again, while the tool reported success.
   */
  function patchStyle(style: Style, args: StyleArgs): Style {
    const next: Style = { ...style };
    const oneOf = (what: string, value: unknown, allowed: string[]): string => {
      if (typeof value !== "string" || !allowed.includes(value)) {
        throw new Error(`${what} must be one of: ${allowed.join(", ")} - not ${JSON.stringify(value)}`);
      }
      return value;
    };
    if (args.representations !== undefined && args.representations !== null) {
      if (!Array.isArray(args.representations)) {
        throw new Error(`representations must be a list, such as ["cartoon", "sticks"]`);
      }
      next.representations = args.representations.map(
        (entry) => oneOf("a representation", entry, REPRESENTATIONS) as Representation,
      );
    }
    if (args.scheme !== undefined && args.scheme !== null) {
      next.scheme = oneOf("scheme", args.scheme, SCHEMES) as ColorScheme;
    }
    if (args.ramp !== undefined && args.ramp !== null) {
      next.ramp = oneOf("ramp", args.ramp, RAMPS) as Ramp;
    }
    if (args.colour !== undefined) {
      if (args.colour === null) {
        next.solid = null;
      } else {
        const hex = typeof args.colour === "string" ? normalizeHex(args.colour) : null;
        if (!hex) throw new Error(`colour must be a hex colour such as #ff00ff, or null - not ${JSON.stringify(args.colour)}`);
        next.solid = hexToUnit(hex);
      }
    }
    if (args.heteroatoms_by_element !== undefined) {
      next.heteroatoms_by_element = flag("heteroatoms_by_element", args.heteroatoms_by_element);
    }
    if (args.glass_colour !== undefined) {
      if (args.glass_colour === null) {
        next.glass = null;
      } else {
        const hex = typeof args.glass_colour === "string" ? normalizeHex(args.glass_colour) : null;
        if (!hex) throw new Error(`glass_colour must be a hex colour such as #6fa8ff, or null - not ${JSON.stringify(args.glass_colour)}`);
        next.glass = hexToUnit(hex);
      }
    }
    return next;
  }

  /** A true or false that is really one, not the word for it. */
  function flag(name: string, value: unknown): boolean {
    if (typeof value !== "boolean") {
      throw new Error(`${name} must be true or false - not ${JSON.stringify(value)}`);
    }
    return value;
  }

  /** Commits a change an agent made, the same way a change by hand is committed. */
  async function commitGraph(): Promise<void> {
    editor.setGraph(graph);
    save();
    await evaluate();
  }

  /** Each residue as it is cited - `HIS 87 (A_1)` - by its index in the structure. */
  function residueNames(map: StructureMap): Map<number, string> {
    const names = new Map<number, string>();
    for (const chain of map.chains) {
      chain.residue_indices.forEach((residue, row) => names.set(residue, `${chain.names[row]} ${chain.numbers[row]} (${chain.id})`));
    }
    return names;
  }

  function agentHost(): AgentHost {
    const describeStyle = (style: Style) => ({
      representations: style.representations,
      scheme: style.scheme,
      ramp: style.ramp,
      solid: style.solid,
    });
    return {
      listDocs: () => api.agentDocs(),
      readDoc: (name) => api.agentDoc(name),
      listMolecules: async () =>
        molecules().map(({ node, structure }) => ({
          molecule: node.id,
          entry: structure.entry_id || structure.source,
          title: structure.title,
          atoms: structure.atom_count,
          slots: node.type === "molecule" ? node.slots.map((slot, index) => ({ index, name: slot.name, ...describeStyle(slot.style) })) : [],
          chains: structure.chains.map((chain) => ({
            id: chain.id,
            auth: chain.auth_id,
            entity: chain.entity_type,
            residues: chain.residue_count,
          })),
        })),
      readGraph: async () => ({
        nodes: graph.nodes.map((node) => {
          const common = { id: node.id, type: node.type };
          if (node.type === "molecule") {
            return { ...common, pdb_id: node.pdb_id, path: node.path, options: node.options, slots: node.slots.map((slot) => slot.name) };
          }
          if (node.type === "chain") return { ...common, chain: node.chain, slot: node.slot };
          if (node.type === "style") return { ...common, ...describeStyle(node.style) };
          return common;
        }),
        links: graph.links,
      }),
      chainSequence: async (chain, molecule) => {
        const { structure } = resolveMolecule(molecule);
        const wanted = resolveChain(structure, chain);
        const sequences = await api.structureSequence(structure.id);
        const found = sequences.find((entry) => entry.chain === wanted.id);
        if (!found) throw new Error(`chain ${wanted.id} has no sequence - it is ${wanted.entity_type}`);
        return { chain: found.chain, first_number: found.first_number, codes: found.codes, secondary: found.secondary };
      },
      currentSelection: async () => {
        if (!currentView) return { selected: null };
        const summary = await api.describeView(currentView.structureId, {
          level: currentView.level,
          indices: currentView.indices,
        });
        return { level: summary.level, label: summary.label, chains: summary.chain_ids, residues: summary.residues, atoms: summary.atoms };
      },

      openPdb: async (id) => {
        const info = await addMolecule({ pdb_id: id, path: null }, { x: 40, y: 40 });
        if (!info) throw new Error(notice ?? `could not open ${id}`);
        return {
          opened: info.entry_id || id,
          title: info.title,
          chains: info.chains.map((chain) => chain.id),
          atoms: info.atom_count,
        };
      },
      select: async (spec, molecule) => {
        const { structure } = resolveMolecule(molecule);
        const view = await api.selectView(structure.id, spec);
        if (view.indices.length === 0) throw new Error("nothing matched those conditions");
        await applySelection({ structureId: structure.id, level: view.level, indices: view.indices }, "replace");
        const summary = await api.describeView(structure.id, view);
        return { selected: summary.label, chains: summary.chain_ids, residues: summary.residues, atoms: summary.atoms };
      },
      clearSelection: async () => {
        await applySelection(null, "replace");
        return { selected: null };
      },
      setStyle: async (args) => {
        const { node } = resolveMolecule(args.molecule);
        if (node.type !== "molecule") throw new Error("not a molecule");
        const index = resolveSlot(node, args.slot);
        node.slots[index] = { ...node.slots[index], style: patchStyle(node.slots[index].style, args) };
        await commitGraph();
        const chains = graph.nodes.filter((entry) => entry.type === "chain" && entry.slot === index).length;
        return { slot: node.slots[index].name, ...describeStyle(node.slots[index].style), chains_drawn_with_it: chains };
      },
      addSlot: async (args) => {
        const { node } = resolveMolecule(args.molecule);
        if (node.type !== "molecule") throw new Error("not a molecule");
        const base = node.slots[node.active_slot]?.style ?? node.slots[0].style;
        node.slots.push({ name: args.name, style: patchStyle(base, args) });
        node.active_slot = node.slots.length - 1;
        await commitGraph();
        return { slot: args.name, index: node.slots.length - 1, note: "assign it to a chain with assign_slot" };
      },
      assignSlot: async (chain, slot, molecule) => {
        const { node, structure } = resolveMolecule(molecule);
        if (node.type !== "molecule") throw new Error("not a molecule");
        const wanted = resolveChain(structure, chain);
        const index = resolveSlot(node, slot);
        const chainNode = graph.nodes.find(
          (candidate) =>
            candidate.type === "chain" &&
            candidate.chain === wanted.id &&
            graph.links.some((link) => link.from === node.id && link.to === candidate.id && link.socket === "Molecule"),
        );
        if (!chainNode || chainNode.type !== "chain") throw new Error(`chain ${wanted.id} has no node - it is not drawn`);
        chainNode.slot = index;
        chainNode.style = null;
        await commitGraph();
        return { chain: wanted.id, slot: node.slots[index].name };
      },
      colourChain: async (chain, colour, molecule) => {
        // The whole slot dance in one call, because "make chain A magenta" is one thought
        // and a model that has to work out slots for it usually gets it wrong.
        if (!/^#[0-9a-fA-F]{6}$/.test(colour.trim())) {
          throw new Error(`"${colour}" is not a colour - they look like #ff00ff`);
        }
        const { node, structure } = resolveMolecule(molecule);
        if (node.type !== "molecule") throw new Error("not a molecule");
        const wanted = resolveChain(structure, chain);
        const chainNode = graph.nodes.find(
          (candidate) =>
            candidate.type === "chain" &&
            candidate.chain === wanted.id &&
            graph.links.some((link) => link.from === node.id && link.to === candidate.id && link.socket === "Molecule"),
        );
        if (!chainNode || chainNode.type !== "chain") throw new Error(`chain ${wanted.id} has no node - it is not drawn`);
        const solid = hexToUnit(colour.trim());
        const name = `${wanted.id} ${colour.trim().toLowerCase()}`;
        // A slot per chain-and-colour, reused when the same request comes twice.
        let index = node.slots.findIndex((slot) => slot.name === name);
        if (index < 0) {
          const base = node.slots[chainNode.slot ?? 0]?.style ?? node.slots[0].style;
          node.slots.push({ name, style: { ...base, solid } });
          index = node.slots.length - 1;
        } else {
          node.slots[index] = { ...node.slots[index], style: { ...node.slots[index].style, solid } };
        }
        chainNode.slot = index;
        chainNode.style = null;
        await commitGraph();
        return { chain: wanted.id, colour: colour.trim().toLowerCase(), slot: name };
      },
      setMoleculeOptions: async (args) => {
        const { node } = resolveMolecule(args.molecule);
        if (node.type !== "molecule") throw new Error("not a molecule");
        const option = (name: string, value: unknown, current: boolean) =>
          value === undefined || value === null ? current : flag(name, value);
        node.options = {
          show_hydrogens: option("hydrogens", args.hydrogens, node.options.show_hydrogens),
          show_waters: option("waters", args.waters, node.options.show_waters),
          show_ligands: option("ligands", args.ligands, node.options.show_ligands),
        };
        await commitGraph();
        return node.options;
      },
      resetView: async () => {
        viewer?.resetView();
        return { framed: true };
      },
      styleResidues: async (args) => {
        const { node, structure } = resolveMolecule(args.molecule);
        if (node.type !== "molecule") throw new Error("not a molecule");
        const chain = resolveChain(structure, String(args.chain ?? ""));
        const from = Number(args.from);
        const to = Number(args.to ?? args.from);
        if (!Number.isInteger(from) || !Number.isInteger(to) || to < from) throw new Error("from and to are residue numbers, from <= to");
        const feature: Feature = { chain: chain.id, from, to };
        if (args.show !== undefined) {
          if (!Array.isArray(args.show) || !args.show.every((entry) => REPRESENTATIONS.includes(entry as string))) {
            throw new Error(`show must be a list from: ${REPRESENTATIONS.join(", ")}`);
          }
          feature.show = args.show as Representation[];
        }
        if (args.colour !== undefined && args.colour !== null) {
          const hex = typeof args.colour === "string" ? normalizeHex(args.colour) : null;
          if (!hex) throw new Error(`colour must be a hex colour such as #ff8800 - not ${JSON.stringify(args.colour)}`);
          feature.colour = hexToUnit(hex);
        }
        if (args.hide !== undefined) feature.hide = flag("hide", args.hide);
        if (!feature.show && !feature.colour && !feature.hide) throw new Error("say what to do with them: show, colour or hide");
        node.features = [...(node.features ?? []), feature];
        await commitGraph();
        return { chain: chain.id, from, to, features: node.features.length };
      },
      clearResidueStyles: async (molecule) => {
        const { node } = resolveMolecule(molecule);
        if (node.type !== "molecule") throw new Error("not a molecule");
        const cleared = (node.features ?? []).length;
        node.features = [];
        await commitGraph();
        return { cleared };
      },
      listLinks: async (molecule) => {
        const { structure } = resolveMolecule(molecule);
        const map = await api.structureMap(structure.id);
        const named = residueNames(map);
        return map.links.map((link) => ({
          kind: link.kind,
          from: named.get(link.residues[0]) ?? null,
          to: named.get(link.residues[1]) ?? null,
        }));
      },
      listLigands: async (molecule) => {
        const { structure } = resolveMolecule(molecule);
        const map = await api.structureMap(structure.id);
        return map.chains.flatMap((chain) =>
          chain.residue_indices.flatMap((_, row) =>
            chain.kinds[row] === "l" || chain.kinds[row] === "i"
              ? [{ component: chain.names[row], kind: chain.kinds[row] === "i" ? "ion" : "ligand", chain: chain.id, auth_chain: chain.auth_id, number: chain.numbers[row] }]
              : [],
          ),
        );
      },
      measureDistance: async (from, to, molecule) => {
        const { structure } = resolveMolecule(molecule);
        const map = await api.structureMap(structure.id);
        const place = (ref: ResidueRef) => {
          const chain = resolveChain(structure, String(ref?.chain ?? ""));
          const rows = map.chains.find((entry) => entry.id === chain.id);
          const row = rows?.numbers.indexOf(Number(ref?.number)) ?? -1;
          if (!rows || row < 0) throw new Error(`chain ${chain.id} has no residue numbered ${ref?.number}`);
          return { name: `${rows.names[row]} ${rows.numbers[row]} (${chain.id})`, p: rows.centers.slice(row * 3, row * 3 + 3) };
        };
        const a = place(from);
        const b = place(to);
        const distance = Math.hypot(a.p[0] - b.p[0], a.p[1] - b.p[1], a.p[2] - b.p[2]);
        return { from: a.name, to: b.name, angstroms: Number(distance.toFixed(2)), between: "component centres (CA, C4', or centroid)" };
      },
      setViewMode: async (mode) => {
        if (mode !== "structure" && mode !== "graph") throw new Error("mode must be structure or graph");
        setGraphMode(mode === "graph");
        return { mode };
      },
      setSelectLevel: async (level) => {
        if (level !== "atom" && level !== "residue" && level !== "chain") throw new Error("level must be atom, residue or chain");
        setPickLevel(level);
        return { level };
      },
      focus: async (on) => {
        if (on === "all") {
          viewer?.resetView();
          return { framed: "all" };
        }
        if (on !== "selection") throw new Error("on must be selection or all");
        if (!currentView) throw new Error("nothing is selected - call select first");
        const structure = structures.find((entry) => entry.id === currentView!.structureId);
        if (!structure || !viewer) throw new Error("the selection's molecule is not on screen");
        if (currentView.level === "atom") throw new Error("focus works on a selection of residues or chains - select at one of those levels");
        const map = await api.structureMap(structure.id);
        const points: number[][] = [];
        map.chains.forEach((chain, chainIndex) => {
          chain.residue_indices.forEach((residue, row) => {
            const inside = currentView!.level === "chain" ? currentView!.indices.includes(chainIndex) : currentView!.indices.includes(residue);
            if (inside) points.push(chain.centers.slice(row * 3, row * 3 + 3));
          });
        });
        if (points.length === 0) throw new Error("the selection has no residues to look at");
        const centre = [0, 1, 2].map((axis) => points.reduce((sum, p) => sum + p[axis], 0) / points.length) as [number, number, number];
        const radius = Math.max(6, ...points.map((p) => Math.hypot(p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]) + 4));
        viewer.lookAt(centre, radius * 2.6, radius);
        return { framed: "selection", residues: points.length };
      },
      setWorld: async (args) => {
        const world = graph.nodes.find((node) => node.type === "world");
        if (!world || world.type !== "world") throw new Error("this graph has no World node - add one in the node graph");
        if (args.background !== undefined) {
          const hex = typeof args.background === "string" ? normalizeHex(args.background) : null;
          if (!hex) throw new Error(`background must be a hex colour such as #ffffff - not ${JSON.stringify(args.background)}`);
          world.background = hexToSrgb(hex);
        }
        if (args.ambient !== undefined) {
          if (typeof args.ambient !== "number" || !(args.ambient >= 0 && args.ambient <= 2)) throw new Error("ambient must be a number from 0 to 2");
          world.ambient = args.ambient;
        }
        if (args.ambient_occlusion !== undefined) world.ambient_occlusion = flag("ambient_occlusion", args.ambient_occlusion);
        if (args.outline !== undefined) world.outline = flag("outline", args.outline);
        await commitGraph();
        return { background: world.background, ambient: world.ambient, ambient_occlusion: world.ambient_occlusion, outline: world.outline };
      },
      setLight: async (light, energy) => {
        const lights = graph.nodes.flatMap((node) => (node.type === "light" ? [node] : []));
        const wanted = String(light).trim().toLowerCase();
        const found =
          lights.find((node) => node.id.toLowerCase() === wanted || node.light.name.toLowerCase() === wanted) ??
          (Number.isInteger(Number(light)) ? lights[Number(light)] : undefined);
        if (!found) throw new Error(`no light called ${light}; there are ${lights.map((node) => node.light.name).join(", ")}`);
        if (!(energy >= 0 && energy <= 100)) throw new Error("energy must be a number from 0 to 100");
        found.light = { ...found.light, energy };
        await commitGraph();
        return { light: found.light.name, energy };
      },
      removeMolecule: async (molecule) => {
        const { node } = resolveMolecule(molecule);
        await removeMolecules([node.id]);
        return { removed: node.id };
      },
      openFile: async (path) => {
        if (!/\.(pdb|cif|mmcif|ent)$/i.test(path.trim())) throw new Error("that is not a structure file - .pdb, .cif, .mmcif or .ent");
        const info = await addMolecule({ pdb_id: "", path: path.trim() }, { x: 40, y: 40 });
        if (!info) throw new Error(notice ?? `could not open ${path}`);
        return { opened: info.entry_id || info.source, title: info.title, chains: info.chains.map((chain) => chain.id), atoms: info.atom_count };
      },
      showPane: async (pane, open) => {
        const kind = pane === "sequence" ? "biomol" : pane;
        if (kind !== "graph" && kind !== "biomol" && kind !== "text") throw new Error("pane must be graph, sequence or text");
        const shown = tabShowing(desk, kind);
        if (open && !shown) {
          desk = addPane(desk, kind);
          const added = desk.panes.at(-1);
          if (added) desk = snapPane(desk, added.id, kind === "graph" ? "bottom" : "right");
        } else if (!open && shown) {
          desk = closeTab(desk, shown.pane.id, shown.tab.id);
        }
        renderDesk();
        saveLayout();
        return { pane, open };
      },
      render: async () => {
        const node = graph.nodes.find((candidate) => candidate.type === "render");
        if (!node) throw new Error("this graph has no Render node - add one in the node graph");
        const outcome = await renderNode(node.id);
        if (!outcome) throw new Error(notice ?? "the render did not finish");
        return { image: outcome.image, seconds: outcome.seconds };
      },
      saveViewportImage: async (path, width) => {
        if (!viewer) throw new Error("there is no viewport to save");
        if (!path.trim().startsWith("/") || !/\.png$/i.test(path.trim())) throw new Error("give a full path ending in .png");
        const frame = viewerMain.getBoundingClientRect();
        const aspect = Math.max(frame.width, 1) / Math.max(frame.height, 1);
        const most = viewer.maxCaptureSide();
        const screen = Math.round(frame.width * (window.devicePixelRatio || 1));
        const wide = Math.max(16, Math.min(Math.round(width ?? screen), most, Math.round(most * aspect)));
        const high = Math.round(wide / aspect);
        // Smoothed the same way the Save image menu does it.
        const blob = await viewer.capturePicture(wide, high, { supersample: 4 });
        const data = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => (typeof reader.result === "string" ? resolve(reader.result) : reject(new Error("could not read the picture back")));
          reader.onerror = () => reject(reader.error ?? new Error("could not read the picture back"));
          reader.readAsDataURL(blob);
        });
        await api.saveImage(path.trim(), data);
        return { saved: path.trim(), width: wide, height: high };
      },
      undo: async () => {
        undo();
        return { undone: true };
      },
      redo: async () => {
        redo();
        return { redone: true };
      },
      showImage: async (url, title) => {
        const path = await api.fetchImage(url);
        // An image pane shows the graph's Image nodes, so the picture becomes one: it is
        // then in the world, saved with it, and can be picked again from any image pane.
        const known = graph.nodes.find((node) => node.type === "image" && node.path === path);
        if (!known) {
          const images = graph.nodes.filter((node) => node.type === "image").length;
          graph.nodes.push({
            type: "image",
            id: `image-${Date.now().toString(36)}`,
            path,
            position: [640, 40 + images * 90],
          });
          editor.setGraph(graph);
          save();
        }
        // Beside the viewport, in the half of the desk to its right, on top of whatever is there.
        desk = addPane(desk, "image");
        const added = desk.panes.at(-1);
        if (added) desk = snapPane(desk, added.id, "right");
        renderDesk();
        const host = added ? imageAreas.get(activeTab(added).id) : undefined;
        if (host) {
          host.dataset.path = path;
          drawImageArea(host);
        }
        saveLayout();
        return { shown: title ?? url, saved_as: path };
      },
    };
  }

  /** The first Render node, for the scripted path that has no node to point at. */
  function firstRenderNode(): string | null {
    return graph.nodes.find((node) => node.type === "render")?.id ?? null;
  }

  function pushRenderContext(): void {
    editor.setRenderContext({
      unavailable:
        blender === null ? "Blender was not found. Set GEIS_BLENDER to point at it." : null,
      progress: renderProgress,
    });
  }

  /** Folds the properties panel away, for when the viewport wants the width. */

  /** Applies whatever the environment asked for, then takes the outputs it asked for. */
  async function runScriptedView(instance: Viewer): Promise<void> {
    try {
      const request = await api.scriptedView();
      if (!request) return;

      // The scripted view speaks in representation and colour. Both are fields of a style,
      // and a style lives in a molecule's slot or on a Style node, so both are written -
      // otherwise a graph with no Style node in it (which is now the normal case) ignores
      // the script and screenshots whatever it was already showing.
      let changed = false;
      const edit = (style: Style): Style => {
        let next = style;
        if (request.representation) {
          // The environment names one representation; a style holds a set, so this is
          // "draw exactly this" rather than "add this".
          next = { ...next, representations: [request.representation as Representation] };
        }
        if (request.color_scheme) {
          next = { ...next, scheme: request.color_scheme as Style["scheme"] };
        }
        return next;
      };
      for (const node of graph.nodes) {
        if (node.type === "style") {
          node.style = edit(node.style);
          changed = true;
        }
        if (node.type === "molecule" && (node.slots ?? []).length > 0) {
          node.slots = node.slots.map((slot) => ({ ...slot, style: edit(slot.style) }));
          changed = true;
        }
      }
      if (changed) {
        editor.setGraph(graph);
        await evaluate();
      }

      if (request.screenshot) {
        const saved = await api.saveImage(request.screenshot, instance.captureDataUrl());
        void api.uiLog("viewer", `frame written to ${saved}`);
      }
      if (request.blender_render) {
        const target = firstRenderNode();
        const outcome = target ? await renderNode(target, request.blender_render) : null;
        if (outcome) {
          void api.uiLog(
            "render",
            `blender wrote ${outcome.image} - ${outcome.triangles} triangles in ` +
              `${outcome.seconds.toFixed(1)}s with ${outcome.engine}`,
          );
        } else {
          const target = firstRenderNode();
          void api.uiLog(
            "error",
            target
              ? `blender render failed: ${renderProgress[target]?.error ?? "unknown"}`
              : "the graph has no Render node",
          );
        }
      }
    } catch (error) {
      void api.uiLog("error", `scripted view failed: ${String(error)}`);
    }
  }

  /**
   * The overlay on the viewport: legend, what is being shown, and render status.
   *
   * Floating over the viewport rather than in a side panel, so the graph gets the width it
   * needs and the viewport keeps the rest.
   */
  /**
   * The viewport's overlay.
   *
   * Only what was asked for: what is selected, and a problem when there is one. The
   * legend, the atom counts and the triangle counts are gone - a viewport should show the
   * molecule, and the counts are in the graph's status line where they belong.
   */
  /** What the picked bond is, spelled out. */
  function edgeCard(): HTMLElement | null {
    const edge = graphMode && pickedEdge !== null ? shownGraph?.edgeInfo[pickedEdge] : null;
    if (!edge) return null;
    const row = (label: string, value: string) =>
      el("div", { class: "edge-row" }, el("span", { class: "edge-key", text: label }), el("span", { text: value }));
    return el(
      "div",
      { class: "hud-card edge-card" },
      el("div", { class: "edge-title", text: edge.what }),
      row("From", `${edge.names[0]} · chain ${edge.chains[0]}`),
      row("To", `${edge.names[1]} · chain ${edge.chains[1]}`),
      // A found contact is between particular atoms, and that distance is the one the
      // rule it was found by is about; the centres' is given beside it.
      edge.measured !== undefined ? row("Distance", `${edge.measured.toFixed(2)} Å between the atoms`) : null,
      row(edge.measured !== undefined ? "Centres" : "Length", `${edge.length.toFixed(2)} Å between component centres`),
    );
  }

  /** Whether an evaluation has been running long enough to say so, and what it is doing. */
  let loadingShown = false;
  let loadingWhat = "";

  /**
   * What is still being worked on: the scene itself while it is opened and drawn, then a
   * line for each structure whose analyses are still running, with how far along.
   */
  function analysing(): HTMLElement[] {
    const loading = loadingShown ? [statusNote(loadingWhat, "busy")] : [];
    // One line per structure, however many molecules show it.
    const distinct = structures.filter((structure, index) => structures.findIndex((entry) => entry.id === structure.id) === index);
    return loading.concat(distinct.flatMap((structure) => {
      const { done, total } = analysisProgress(structure);
      if (done >= total) return [];
      return [statusNote(`Analysing ${structure.entry_id || "the structure"} - ${done} of ${total}`, "busy")];
    }));
  }

  function renderHud(): void {
    // A world with no molecule in it is not a broken graph, and "no chain connected" in
    // red is the wrong first thing for a new world to say. It gets told what to do next
    // instead; the problem list is for a graph that has something in it to be wrong.
    const empty = !graph.nodes.some((node) => node.type === "molecule");
    const problems = empty ? [] : (result?.evaluation.problems ?? []);
    replace(
      hud,
      editor.selection,
      edgeCard(),
      empty
        ? el(
            "div",
            { class: "hud-card notice info" },
            el("div", { class: "empty-title", text: "This world is empty." }),
            el(
              "div",
              { class: "empty-actions" },
              el("button", { class: "btn primary", text: "Open a file...", onclick: () => RUN.open() }),
              el("button", { class: "btn", text: "Fetch from the PDB...", onclick: () => RUN.fetch() }),
            ),
            el("div", {
              class: "help",
              text: "Or drop a .pdb or .cif file anywhere on the window. ⌘K finds every command; ? lists the keys.",
            }),
          )
        : null,
      problems.length > 0
        ? el(
            "div",
            { class: "hud-card notice bad" },
            ...problems.slice(0, 2).map((problem) => el("div", { text: problem })),
          )
        : null,
    );
    // What happened and what is under way go on the status bar at the foot of the window,
    // as Blender says them, not over the structure in the corner of the viewport.
    if (notice !== noticeTimed) {
      noticeTimed = notice;
      clearTimeout(noticeTimer);
      if (notice) {
        const said = notice;
        noticeTimer = setTimeout(() => {
          if (notice === said) {
            notice = null;
            renderHud();
          }
        }, 10_000);
      }
    }
    setStatusNotes(
      ...analysing(),
      Object.values(renderProgress).some((entry) => entry.rendering) ? statusNote("Rendering", "busy") : null,
      notice ? statusNote(notice.charAt(0).toUpperCase() + notice.slice(1)) : null,
    );

    if (blender === undefined) {
      blender = null;
      void api.blenderInfo().then((info) => {
        blender = info;
        // Told to the Render nodes rather than to a button in the bar: the reason a render
        // cannot happen belongs on the node that would have done it.
        pushRenderContext();
      });
    }
  }

  /** The editor's height is a working preference, so it is draggable. */

  async function openPath(path: string): Promise<void> {
    await ready;
    // A file given on the command line, or from the jobs list, can bring the screen up
    // without a world ever being opened from the shelf. The world it lands in still has
    // panes and cameras of its own; read them now, or the first change to either would be
    // saved over that world's with the empty list this screen starts with. A world opened
    // the usual way has had them read already.
    if (activeWorld && layoutOf !== activeWorld) await restoreLayout(activeWorld);
    if (activeWorld && camerasOf !== activeWorld) await loadCameras(activeWorld);
    // A file on the command line is the same action as opening one from the menu: it puts
    // a molecule in the graph and its chains follow.
    const existing = graph.nodes.find((node) => node.type === "molecule");
    if (existing?.type === "molecule" && existing.chains.length === 0) {
      existing.path = path;
      existing.pdb_id = "";
      editor.setGraph(graph);
      save();
      await evaluate();
      return;
    }
    await addMolecule({ pdb_id: "", path }, { x: 40, y: 60 });
  }

  // Dropping a file on the window is how anyone with a structure on disk expects to open
  // it. The webview takes drops itself and hands over paths - a page's own `drop` event
  // never fires here, and a `File` has no path if it did.
  const STRUCTURE_FILE = /\.(pdb|ent|cif|mmcif)$/i;
  const dropVeil = el("div", { class: "drop-veil", hidden: "" }, el("div", { text: "Drop to open: structures into this world, other files in the file viewer" }));
  element.append(dropVeil);
  const stopDrops = getCurrentWebview()
    .onDragDropEvent((event) => {
      if (!element.isConnected) return;
      const drop = event.payload;
      if (drop.type === "enter") {
        dropVeil.hidden = drop.paths.length === 0;
      } else if (drop.type === "leave") {
        dropVeil.hidden = true;
      } else if (drop.type === "drop") {
        dropVeil.hidden = true;
        const files = drop.paths.filter((path) => STRUCTURE_FILE.test(path));
        // Anything else - a paper, a figure, slides - is for reading beside the structure.
        for (const path of drop.paths.filter((path) => !STRUCTURE_FILE.test(path)).reverse()) viewFile(path);
        void (async () => {
          // One after another: each takes its place in the graph beside the last.
          for (const path of files) await openPath(path);
        })();
      }
    })
    // Outside the app's own window - a browser tab in development - there is no webview to ask.
    .catch(() => null);

  ready = api.graphLoad().then(async (initial) => {
    graph = initial;
    forgetHistory();
    // The world this graph is - the one that was open last - so that what belongs to a
    // world has one to belong to even before any is opened from the shelf.
    activeWorld ??= await api.worldActive().catch(() => null);
    void agents.setWorld(activeWorld);
    if (activeWorld) {
      const here = await api.worldRead(activeWorld).catch(() => null);
      if (here) worldTitle.textContent = here.name;
    }
    editor.setGraph(initial);
    await loadStyles();
    await evaluate();
    // Nothing is put into the graph on the app's own initiative. A development launch
    // used to add 4HHB to whichever world was open and had no molecule - so a world made
    // empty was, the next time the app started, not empty, and every world ended up
    // holding the same haemoglobin nobody had asked for.
  }).catch((error) => {
    // Said, and then let go of: everything else waits on `ready`, and a promise left
    // rejected made every world clicked on afterwards fail to open without a word.
    notice = `the last graph could not be read: ${String(error)}`;
    renderHud();
  });

  renderHud();

  /**
   * Opens a world.
   *
   * The graph is the world's, and the camera goes back where it was left - a session you
   * come back to at a different angle from the one you left it at is disorienting in a way
   * a saved session should not be. The picture on its card is taken when you leave.
   */
  async function openWorld(id: string): Promise<void> {
    await ready;
    if (saveTimer !== null) {
      // Whatever the world before this one changed goes to *its* row, not the new one's.
      clearTimeout(saveTimer);
      saveTimer = null;
      await api.graphSave(graph).catch(() => {});
    }
    await rememberWorld();
    let world;
    try {
      world = await api.worldOpen(id);
    } catch (error) {
      notice = String(error);
      renderHud();
      return;
    }
    activeWorld = id;
    worldTitle.textContent = world.name;
    worldTitle.title = world.name;
    notice = null;
    graph = world.graph;
    graphEpoch += 1;
    // A different world is a different scene: the stage is emptied so the first thing
    // drawn in it is framed as a first load, not left wherever the last world's camera
    // was, and a render still running belongs to the world that started it.
    viewer?.clearScene();
    forgetHistory();
    void agents.setWorld(id);
    await restoreLayout(id);
    await loadCameras(id);
    // A render from an earlier session is not known to be of the scene as it is now.
    renderedAs = null;
    quietenBackground();
    // The selection and what hangs off it were the other world's. The target especially:
    // left set, the chip went on naming a chain from there and the pies acted on it - on
    // nothing, or on this world's chain of the same name when the same file is open here.
    // None of this is the graph, so nothing is saved and no undo step is made.
    currentView = null;
    actionTarget = null;
    drawTarget();
    biomol.setSelection(null, new Set(), []);
    hideContacts();
    shownGraph = null;
    pickedEdge = null;
    renderProgress = {};
    pushRenderContext();
    pendingCamera = world.camera;
    editor.setGraph(graph);
    // What was selected was selected in the other world. Node ids repeat between worlds -
    // every one has a "fill" light - so left alone the selection carried across, and a new
    // world opened with the last world's light already picked.
    syncingSelection = true;
    editor.select(null);
    syncingSelection = false;
    await evaluate();
    // The plane's pan and zoom are the screen's, not the world's: arriving at a different
    // graph with the last one's view shows a corner of a node, or nothing at all.
    editor.fit();
    // `drawScene` puts the camera back once there is a scene to put it on. While an
    // evaluation is still out, the `evaluate` above returned at once and the scene has not
    // arrived: applying the camera here put it on an empty stage, and the first scene's
    // framing then replaced it. Only a world with nothing to draw is left holding one,
    // and that one is dropped rather than applied to whatever is opened into it later.
    if (!evaluating) pendingCamera = null;
  }

  /** Writes back where this world was left, and what it looked like. */
  /**
   * Writes down where the world was left, on the way out.
   *
   * Only the camera. A picture of the world used to be taken here too, for the card on the
   * shelf - but the shelf draws the worlds themselves now, so nobody ever looked at it, and
   * taking it cost a redraw, a read-back and a PNG of a retina canvas: about half a second
   * of stillness every time you pressed Back, to make something nothing reads.
   */
  /**
   * A picture of the world as it is being left, for the shelf to show while it builds its
   * own. Drawn at a modest size and without the background, so it stands on the shelf the
   * way the built world will; none for an empty world or a viewport that is not shown.
   */
  function takeThumbnail(): { world: string; data: string } | null {
    if (!viewer || !activeWorld || structures.length === 0) return null;
    const frame = viewerMain.getBoundingClientRect();
    if (!viewerMain.isConnected || frame.width < 10 || frame.height < 10) return null;
    const LONG_SIDE = 720;
    const scale = LONG_SIDE / Math.max(frame.width, frame.height);
    try {
      return { world: activeWorld, data: viewer.captureAt(frame.width * scale, frame.height * scale, true) };
    } catch {
      return null;
    }
  }

  async function rememberWorld(): Promise<void> {
    if (!activeWorld || !viewer) return;
    const camera = viewer.describeCamera();
    await api
      .worldCamera(activeWorld, {
        eye: camera.eye,
        center: camera.center,
        up: camera.up,
        radius: camera.radius,
      })
      .catch(() => {});
  }


  return {
    element,
    openPath,
    openWorld,
    hold,
    assemble,
    dispose() {
      window.clearInterval(viewWatch);
      void stopDrops.then((stop) => stop?.());
      // A pending debounced write would be lost with the window, so it goes now.
      if (saveTimer !== null) {
        clearTimeout(saveTimer);
        saveTimer = null;
        void api.graphSave(graph);
      }
      unlistenJobs?.();
      for (const entry of structures) void api.closeStructure(entry.id);
      viewer?.dispose();
      viewer = null;
      structures = [];
      canvas = null;
    },
  };
}
