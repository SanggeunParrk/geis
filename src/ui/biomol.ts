// The BioMol panel: the molecule as layers.
//
// A 2D space, panned and zoomed like the node graph, with one map per molecule laid out as
// an icicle partition: the molecule as a bar, its chains as cells spanning the bar, their
// residues as cells spanning each chain, and an opened residue's atoms as cells spanning
// it. A parent spans its children, so belonging is read at a glance; order runs left to
// right, so the residue layer is the sequence. Links that are not the backbone hang beneath
// as arcs - low between neighbours, high across the molecule - and an open residue's bonds
// are small arcs between its atoms. Zooming in changes what is shown, not just how big:
// cells become letters, letters get numbers. Every change of state is interpolated
// (`molmap/tween.ts`): a residue opens by widening and pushing its neighbours along, the
// camera glides, a selection pulses once, a hover glows in.
//
// It selects and shows selections through the same `View` the viewport and the outliner
// use: click a chain cell for the chain, a residue cell for the residue, an atom cell for
// the atom, drag along the residue layer for a range; shift adds, cmd/ctrl toggles. A
// selection made anywhere lights its cells here; hovering here outlines in the viewport.
//
// Drawn on a canvas, not with elements: a structure has thousands of residues, and the
// transitions have to hold sixty frames a second. The geometry is `molmap/layout.ts`, pure.

import { api, type ResidueAtoms, type StructureInfo, type StructureMap } from "../ipc";
import { el } from "./dom";
import {
  fitCamera,
  hitTest,
  layoutMap,
  mapOrigins,
  type Camera,
  type Cell,
  type MapLayout,
  type Openings,
} from "./molmap/layout";
import { easeInOutCubic, Pulse, Tween } from "./molmap/tween";
import { rangeBetween, type Combine, type View } from "./selection";

export interface BioMolHandle {
  element: HTMLElement;
  setStructures: (structures: StructureInfo[]) => Promise<void>;
  /**
   * Shows a selection: its cells are lit and pulse once. `residues` is the selection at
   * the residue level, which the screen has from the mask it built.
   */
  setSelection: (view: View | null, residues: Set<number>, chainIds: string[]) => void;
  /** Something hovered elsewhere - the viewport - to glow here too. */
  setHover: (view: View | null) => void;
}

export interface BioMolOptions {
  onSelect: (view: View, mode: Combine) => void;
  /** The pointer is over a chain, residue or atom here; null when it leaves. */
  onHover?: (view: View | null) => void;
}

interface PlacedMap {
  data: StructureMap;
  layout: MapLayout;
  origin: { x: number; y: number };
  /** Per residue, how far open, animated. */
  open: Map<number, Tween>;
  atoms: Map<number, ResidueAtoms>;
  /** Fetches in flight, so a double double-click does not fetch twice. */
  fetching: Set<number>;
}

type PlacedHit = { map: PlacedMap; cell: Cell };

const ORANGE = "#ff8a3d";
const ARC_COLOURS: Record<string, string> = {
  disulfide: "rgba(250, 176, 5, 0.85)",
  covalent: "rgba(76, 201, 176, 0.85)",
  metal: "rgba(190, 130, 255, 0.85)",
  other: "rgba(160, 170, 190, 0.6)",
  bond: "rgba(210, 220, 240, 0.7)",
};

export function biomolPanel(options: BioMolOptions): BioMolHandle {
  const canvas = el("canvas", { class: "molmap-canvas" });
  const hint = el("div", {
    class: "molmap-hint",
    text: "drag to pan · wheel to zoom · double-click a residue to open its atoms, a chain to fit it",
  });
  const element = el("div", { class: "molmap" }, canvas, hint);
  const ctx = canvas.getContext("2d")!;

  let maps: PlacedMap[] = [];
  /** The structures drawn, so a re-evaluation that changed nothing does not refetch. */
  let drawnKey = "";
  /** Why the panel is empty, when it is empty for a reason worth saying. */
  let failure: string | null = null;
  let width = 0;
  let height = 0;
  let dpr = 1;
  let fontFamily = "-apple-system, BlinkMacSystemFont, 'Inter', sans-serif";
  let monoFamily = "ui-monospace, Menlo, monospace";

  const cam = { x: new Tween(0), y: new Tween(0), k: new Tween(1) };
  const hoverGlow = new Tween(0);
  const pulse = new Pulse();
  let hover: PlacedHit | null = null;
  let remoteHover: View | null = null;
  let selection: { view: View | null; residues: Set<number>; chains: Set<string> } = {
    view: null,
    residues: new Set(),
    chains: new Set(),
  };
  let frame = 0;
  let pointer: { x: number; y: number } | null = null;
  /** Where the last plain click landed per chain, for shift-click ranges of residues. */
  const anchors = new Map<string, number>();

  // --- sizing -----------------------------------------------------------------------

  const observer = new ResizeObserver(() => resize());
  observer.observe(element);

  function resize(): void {
    const rect = element.getBoundingClientRect();
    const first = width === 0 && rect.width > 0;
    width = Math.max(1, Math.floor(rect.width));
    height = Math.max(1, Math.floor(rect.height));
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.floor(width * dpr);
    canvas.height = Math.floor(height * dpr);
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    const style = getComputedStyle(element);
    fontFamily = style.fontFamily || fontFamily;
    monoFamily = style.getPropertyValue("--mono") || monoFamily;
    if (first || maps.length > 0) fitAll(first ? 0 : 350);
    request();
  }

  // --- data -------------------------------------------------------------------------

  function openings(map: PlacedMap, now: number): Openings {
    const open = new Map<number, number>();
    for (const [residue, tween] of map.open) {
      const value = tween.at(now);
      if (value > 0.001) open.set(residue, value);
      else if (tween.done(now)) map.open.delete(residue);
    }
    return { open, atoms: map.atoms };
  }

  function atomCounts(map: PlacedMap): Map<number, number> {
    const counts = new Map<number, number>();
    for (const [residue, fetched] of map.atoms) counts.set(residue, fetched.atoms.length);
    return counts;
  }

  /** Recomputes every map's cells for this instant, and where the maps sit. */
  function relayout(now: number): void {
    for (const map of maps) map.layout = layoutMap(map.data, openings(map, now), atomCounts(map));
    const origins = mapOrigins(maps.map((map) => map.layout.height));
    maps.forEach((map, index) => {
      map.origin = origins[index];
    });
  }

  let structuresAsked = 0;
  async function setStructures(structures: StructureInfo[]): Promise<void> {
    const key = structures.map((structure) => structure.id).join(",");
    // A failed load is not remembered as drawn: the panel would stay blank for as long as
    // the same molecules were open, with nothing on screen to say why. It was blank for a
    // whole session that way, because the running core was a build older than the command
    // the panel calls.
    if (key === drawnKey && failure === null) return;
    // The latest call wins. A large structure's map can still be loading when a smaller
    // one is asked for; finishing second, it would otherwise put the old molecule back.
    const asked = ++structuresAsked;
    const loaded: StructureMap[] = [];
    let problem: string | null = null;
    for (const structure of structures) {
      try {
        loaded.push(await api.structureMap(structure.id));
      } catch (error) {
        problem = String(error);
        void api.uiLog("warn", `biomol: could not read ${structure.source}: ${problem}`);
      }
    }
    if (asked !== structuresAsked) return;
    // Anything that failed is not remembered as drawn, so the next evaluation tries it
    // again; the message is only for when nothing at all could be read, since a panel with
    // one of two molecules on it says more by showing the one than by hiding both.
    failure = loaded.length === 0 && structures.length > 0 ? problem : null;
    drawnKey = problem === null ? key : "";
    maps = loaded.map((data) => ({
      data,
      layout: layoutMap(data, { open: new Map(), atoms: new Map() }, new Map()),
      origin: { x: 0, y: 0 },
      open: new Map(),
      atoms: new Map(),
      fetching: new Set(),
    }));
    relayout(performance.now());
    hint.hidden = maps.length === 0;
    fitAll(500);
    request();
  }

  function worldBox(): { x: number; y: number; w: number; h: number } {
    if (maps.length === 0) return { x: 0, y: 0, w: 100, h: 30 };
    const w = Math.max(...maps.map((map) => map.layout.width));
    const last = maps[maps.length - 1];
    return { x: 0, y: 0, w, h: last.origin.y + last.layout.height };
  }

  /**
   * Fits the maps to the view - but a molecule is wide and shallow, and fitting a long one
   * to a tall pane leaves the layers a few pixels high, which defeats the point. So the
   * layers never go below a readable height: when they would, the map is shown at that
   * height from its left edge, and the rest is reached by panning, as a sequence is.
   */
  function fitAll(duration: number): void {
    const box = worldBox();
    const target = fitCamera(box, width, height);
    const minK = 18 / 5; // the molecule bar is 5 units; keep it 18 px or more
    if (target.k < minK) {
      target.k = minK;
      target.x = 18;
      target.y = Math.max(18, height / 2 - (box.h / 2) * minK);
    }
    glideTo(target, duration);
  }

  function glideTo(target: Camera, duration: number): void {
    const now = performance.now();
    if (duration <= 0) {
      cam.x.set(target.x);
      cam.y.set(target.y);
      cam.k.set(target.k);
    } else {
      cam.x.go(target.x, now, duration, easeInOutCubic);
      cam.y.go(target.y, now, duration, easeInOutCubic);
      cam.k.go(target.k, now, duration, easeInOutCubic);
    }
    request();
  }

  /** Glides the camera so a cell and the layers under it fill the view. */
  function focusCell(map: PlacedMap, cell: Cell): void {
    const box = {
      x: map.origin.x + cell.x,
      y: map.origin.y,
      w: cell.w,
      h: map.layout.height,
    };
    // Letters need about 9 px per residue; never zoom past what a chain needs to read.
    const target = fitCamera(box, width, height, 24);
    target.k = Math.min(target.k, 22);
    target.x = width / 2 - (box.x + box.w / 2) * target.k;
    target.y = height / 2 - (box.y + box.h / 2) * target.k;
    glideTo(target, 600);
  }

  /** Opens a residue into its atoms, or closes it. Atoms are fetched the first time. */
  async function toggleResidue(map: PlacedMap, residue: number): Promise<void> {
    const now = performance.now();
    const existing = map.open.get(residue);
    if (existing && existing.target() > 0.5) {
      existing.go(0, now, 420, easeInOutCubic);
      request();
      return;
    }
    if (!map.atoms.has(residue)) {
      if (map.fetching.has(residue)) return;
      map.fetching.add(residue);
      try {
        map.atoms.set(residue, await api.residueAtoms(map.data.id, residue));
      } catch {
        return;
      } finally {
        map.fetching.delete(residue);
      }
    }
    const tween = existing ?? new Tween(0);
    tween.go(1, performance.now(), 480, easeInOutCubic);
    map.open.set(residue, tween);
    request();
  }

  // --- selection in --------------------------------------------------------------------

  function setSelection(view: View | null, residues: Set<number>, chainIds: string[]): void {
    const before = selection;
    selection = { view, residues, chains: new Set(chainIds) };
    const changed =
      before.view?.structureId !== view?.structureId ||
      before.view?.level !== view?.level ||
      before.residues.size !== residues.size ||
      before.chains.size !== selection.chains.size ||
      [...residues].some((index) => !before.residues.has(index));
    if (changed && view) pulse.fire(performance.now(), 650);
    request();
  }

  function setHover(view: View | null): void {
    remoteHover = view;
    request();
  }

  // --- frame loop ------------------------------------------------------------------------

  function request(): void {
    if (frame) return;
    frame = requestAnimationFrame(tick);
  }

  function tick(): void {
    frame = 0;
    const now = performance.now();
    const opening = maps.some((map) => [...map.open.values()].some((tween) => !tween.done(now)));
    if (opening || maps.some((map) => map.open.size > 0 && map.layout.atomCells.length === 0)) relayout(now);
    draw(now);
    const animating =
      opening || !cam.x.done(now) || !cam.y.done(now) || !cam.k.done(now) || !hoverGlow.done(now) || pulse.active(now);
    if (animating) frame = requestAnimationFrame(tick);
  }

  // --- drawing -----------------------------------------------------------------------

  function draw(now: number): void {
    const camera: Camera = { x: cam.x.at(now), y: cam.y.at(now), k: cam.k.at(now) };
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    const wash = ctx.createLinearGradient(0, 0, 0, height);
    wash.addColorStop(0, "#0f131d");
    wash.addColorStop(1, "#0a0d14");
    ctx.fillStyle = wash;
    ctx.fillRect(0, 0, width, height);
    for (const map of maps) drawMap(map, camera, now);
    if (maps.length === 0) drawEmpty();
    drawTooltip();
  }

  /** What an empty panel says. A blank canvas is indistinguishable from a broken one. */
  function drawEmpty(): void {
    const lines = failure
      ? ["The structure could not be read.", failure.slice(0, 120), "It will be tried again when the graph changes."]
      : ["No molecule.", "Add one in the node graph, or open a file."];
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    lines.forEach((line, index) => {
      ctx.fillStyle = index === 0 ? (failure ? "#ff8a8a" : "rgba(168, 178, 200, 0.9)") : "rgba(138, 147, 170, 0.8)";
      ctx.font = `${index === 0 ? 600 : 400} ${index === 0 ? 13 : 11}px ${fontFamily}`;
      ctx.fillText(line, width / 2, height / 2 - 10 + index * 18);
    });
  }

  function roundRect(x: number, y: number, w: number, h: number, r: number): void {
    const radius = Math.min(r, w / 2, h / 2);
    ctx.beginPath();
    ctx.roundRect(x, y, w, h, radius);
  }

  function selectedIn(map: PlacedMap): {
    here: boolean;
    level: View["level"] | null;
    indices: Set<number>;
  } {
    const here = selection.view?.structureId === map.layout.structureId;
    return { here, level: here ? selection.view!.level : null, indices: new Set(here ? selection.view!.indices : []) };
  }

  function drawMap(map: PlacedMap, camera: Camera, now: number): void {
    const { layout } = map;
    const k = camera.k;
    const px = (pixels: number) => pixels / k;
    ctx.save();
    ctx.translate(camera.x + map.origin.x * k, camera.y + map.origin.y * k);
    ctx.scale(k, k);

    const sel = selectedIn(map);
    const glow = hoverGlow.at(now);
    const isHover = (cell: Cell) => hover?.map === map && hover.cell === cell;
    const isRemote = (cell: Cell) =>
      remoteHover?.structureId === layout.structureId &&
      ((remoteHover.level === "chain" && cell.level === "chain") ||
        (remoteHover.level === "residue" && cell.level === "residue") ||
        (remoteHover.level === "atom" && cell.level === "atom")) &&
      remoteHover.indices.includes(cell.index);

    // Arcs first, behind the layers they rise from.
    for (const arc of layout.arcs) {
      const involved = sel.here && (selection.residues.has(arc.residues[0]) || selection.residues.has(arc.residues[1]));
      const hovered =
        hover?.map === map &&
        hover.cell.level === "residue" &&
        (hover.cell.index === arc.residues[0] || hover.cell.index === arc.residues[1]);
      ctx.beginPath();
      ctx.moveTo(arc.x1, arc.y);
      ctx.bezierCurveTo(arc.x1, arc.y + arc.height * 1.35, arc.x2, arc.y + arc.height * 1.35, arc.x2, arc.y);
      ctx.strokeStyle = involved || hovered ? ORANGE : ARC_COLOURS[arc.kind] ?? ARC_COLOURS.other;
      ctx.lineWidth = px(involved || hovered ? 2.4 : 1.4);
      ctx.globalAlpha = hover && !hovered && !involved ? 0.35 : 1;
      ctx.stroke();
      ctx.globalAlpha = 1;
      // Feet, so the arc's ends read as attached to their cells.
      for (const x of [arc.x1, arc.x2]) {
        ctx.beginPath();
        ctx.arc(x, arc.y, px(2), 0, Math.PI * 2);
        ctx.fillStyle = ctx.strokeStyle;
        ctx.fill();
      }
    }
    for (const bond of layout.bonds) {
      ctx.beginPath();
      ctx.moveTo(bond.x1, bond.y);
      ctx.bezierCurveTo(bond.x1, bond.y + bond.height * 1.3, bond.x2, bond.y + bond.height * 1.3, bond.x2, bond.y);
      ctx.strokeStyle = ARC_COLOURS.bond;
      ctx.lineWidth = px(1.2);
      ctx.stroke();
    }

    // The molecule bar.
    const molecule = layout.cells[0];
    if (molecule) {
      const bar = ctx.createLinearGradient(0, molecule.y, 0, molecule.y + molecule.h);
      bar.addColorStop(0, "#2c3853");
      bar.addColorStop(1, "#1d2537");
      roundRect(molecule.x, molecule.y, molecule.w, molecule.h, px(5));
      ctx.fillStyle = bar;
      ctx.fill();
      const allChains = sel.here && sel.level === "chain" && sel.indices.size === layout.chainCells.length;
      ctx.strokeStyle = allChains ? ORANGE : isHover(molecule) ? `rgba(255,255,255,${0.6 * glow})` : "rgba(140, 160, 200, 0.35)";
      ctx.lineWidth = px(allChains ? 2 : 1);
      ctx.stroke();
      const size = Math.min(px(13), molecule.h * 0.62);
      ctx.fillStyle = "#e6e9f2";
      ctx.font = `600 ${size}px ${fontFamily}`;
      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      ctx.fillText(molecule.label, molecule.x + px(8), molecule.y + molecule.h / 2);
      ctx.fillStyle = "rgba(168, 178, 200, 0.85)";
      ctx.font = `${size * 0.8}px ${fontFamily}`;
      ctx.textAlign = "right";
      if (molecule.w * k > 220) ctx.fillText(molecule.detail, molecule.x + molecule.w - px(8), molecule.y + molecule.h / 2);
    }

    // Chains.
    for (const cell of layout.chainCells) {
      const selected = sel.level === "chain" && sel.indices.has(cell.index);
      const contains = sel.here && !selected && selection.chains.has(cell.label);
      roundRect(cell.x, cell.y, cell.w, cell.h, px(4));
      ctx.fillStyle = cell.colour;
      ctx.globalAlpha = cell.entityType === "polymer" ? 0.9 : 0.75;
      ctx.fill();
      ctx.globalAlpha = 1;
      if (selected) {
        roundRect(cell.x, cell.y, cell.w, cell.h, px(4));
        ctx.fillStyle = "rgba(255,255,255,0.14)";
        ctx.fill();
        ctx.strokeStyle = ORANGE;
        ctx.lineWidth = px(2.2);
        ctx.stroke();
      } else if (contains) {
        ctx.beginPath();
        ctx.moveTo(cell.x, cell.y + cell.h - px(1));
        ctx.lineTo(cell.x + cell.w, cell.y + cell.h - px(1));
        ctx.strokeStyle = ORANGE;
        ctx.lineWidth = px(2);
        ctx.stroke();
      }
      if (isHover(cell) || isRemote(cell)) {
        roundRect(cell.x, cell.y, cell.w, cell.h, px(4));
        ctx.strokeStyle = `rgba(255,255,255,${0.85 * (isHover(cell) ? glow : 0.7)})`;
        ctx.lineWidth = px(1.6);
        ctx.stroke();
      }
      const size = Math.min(px(11.5), cell.h * 0.6);
      ctx.font = `600 ${size}px ${fontFamily}`;
      const label = cell.w * k > ctx.measureText(cell.label).width * k + px(10) * k ? cell.label : "";
      if (label) {
        ctx.fillStyle = "rgba(8, 10, 16, 0.9)";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(label, cell.x + cell.w / 2, cell.y + cell.h / 2);
      }
    }

    // Residues: cells, then letters once there is room for them.
    for (const cell of layout.residueCells) {
      const pixelWidth = cell.w * k;
      const selected = sel.here && sel.level !== "chain" && selection.residues.has(cell.index);
      const chainSelected = sel.level === "chain" && sel.indices.has(cell.parent);
      const gap = pixelWidth > 3 ? Math.min(cell.w * 0.12, px(1)) : 0;
      const open = cell.open ?? 0;
      roundRect(cell.x + gap / 2, cell.y, cell.w - gap, cell.h, px(open > 0 ? 3 : 1.5));
      ctx.fillStyle = selected ? ORANGE : cell.colour;
      ctx.globalAlpha = selected ? 1 : chainSelected ? 1 : 0.85;
      ctx.fill();
      ctx.globalAlpha = 1;
      if (open > 0) {
        // An open residue is lit from within, and a bracket below it holds its atoms.
        roundRect(cell.x + gap / 2, cell.y, cell.w - gap, cell.h, px(3));
        ctx.strokeStyle = `rgba(255,255,255,${0.45 * open})`;
        ctx.lineWidth = px(1.2);
        ctx.stroke();
      }
      if (isHover(cell) || isRemote(cell)) {
        roundRect(cell.x + gap / 2, cell.y, cell.w - gap, cell.h, px(1.5));
        ctx.strokeStyle = `rgba(255,255,255,${0.95 * (isHover(cell) ? glow : 0.7)})`;
        ctx.lineWidth = px(1.6);
        ctx.stroke();
      }
      if (pixelWidth >= 7) {
        const size = Math.min(px(12), cell.h * 0.55, cell.w * 0.9);
        ctx.fillStyle = selected ? "#1a0d05" : "rgba(8, 10, 16, 0.92)";
        ctx.font = `600 ${size}px ${monoFamily}`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(cell.label, cell.x + cell.w / 2, cell.y + cell.h * (pixelWidth >= 16 ? 0.62 : 0.5));
        if (pixelWidth >= 16) {
          // The residue number, small, along the top of the cell - a ruler.
          const number = cell.detail.split(" ").at(-1) ?? "";
          ctx.fillStyle = "rgba(8, 10, 16, 0.7)";
          ctx.font = `${Math.min(px(8), cell.h * 0.3)}px ${monoFamily}`;
          ctx.fillText(number, cell.x + cell.w / 2, cell.y + cell.h * 0.22);
        }
      }
    }

    // Atoms of open residues.
    for (const cell of layout.atomCells) {
      const pixelWidth = cell.w * k;
      const selected = sel.here && sel.level === "atom" && sel.indices.has(cell.index);
      const open = cell.open ?? 1;
      const gap = pixelWidth > 3 ? Math.min(cell.w * 0.14, px(1)) : 0;
      roundRect(cell.x + gap / 2, cell.y, cell.w - gap, cell.h, px(2));
      ctx.fillStyle = selected ? ORANGE : cell.colour;
      ctx.globalAlpha = 0.9 * open;
      ctx.fill();
      ctx.globalAlpha = 1;
      if (isHover(cell) || isRemote(cell)) {
        roundRect(cell.x + gap / 2, cell.y, cell.w - gap, cell.h, px(2));
        ctx.strokeStyle = `rgba(255,255,255,${0.95 * (isHover(cell) ? glow : 0.7)})`;
        ctx.lineWidth = px(1.6);
        ctx.stroke();
      }
      if (pixelWidth >= 12 && open > 0.6) {
        const size = Math.min(px(10), cell.h * 0.5, (cell.w * 1.6) / Math.max(cell.label.length, 1));
        ctx.fillStyle = "rgba(8, 10, 16, 0.92)";
        ctx.font = `600 ${size}px ${monoFamily}`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.globalAlpha = open;
        ctx.fillText(cell.label, cell.x + cell.w / 2, cell.y + cell.h / 2);
        ctx.globalAlpha = 1;
      }
    }

    // The selection's pulse: a glow that swells out from the selected cells and fades.
    if (sel.here && pulse.active(now)) {
      const cells =
        sel.level === "chain"
          ? layout.chainCells.filter((cell) => sel.indices.has(cell.index))
          : sel.level === "residue"
            ? layout.residueCells.filter((cell) => sel.indices.has(cell.index))
            : layout.atomCells.filter((cell) => sel.indices.has(cell.index));
      if (cells.length > 0) {
        const phase = pulse.phase(now);
        const x1 = Math.min(...cells.map((cell) => cell.x));
        const x2 = Math.max(...cells.map((cell) => cell.x + cell.w));
        const y1 = Math.min(...cells.map((cell) => cell.y));
        const y2 = Math.max(...cells.map((cell) => cell.y + cell.h));
        const grow = px(4 + phase * 22);
        roundRect(x1 - grow, y1 - grow, x2 - x1 + grow * 2, y2 - y1 + grow * 2, px(6));
        ctx.strokeStyle = `rgba(255, 138, 61, ${(1 - phase) * 0.7})`;
        ctx.lineWidth = px(2.5 * (1 - phase) + 0.5);
        ctx.stroke();
      }
    }

    ctx.restore();
  }

  function drawTooltip(): void {
    if (!hover || !pointer) return;
    const { cell } = hover;
    const lines =
      cell.level === "molecule"
        ? [cell.label, cell.detail]
        : cell.level === "chain"
          ? [cell.label, cell.detail]
          : cell.level === "residue"
            ? [cell.detail, `${hover.map.data.chains[cell.parent]?.id ?? ""}${cell.open ? " · open" : ""}`]
            : [`${cell.label} (${cell.detail})`, hover.map.layout.cellByResidue.get(cell.parent)?.detail ?? ""];
    ctx.font = `11px ${fontFamily}`;
    const w = Math.max(...lines.map((line) => ctx.measureText(line).width)) + 16;
    const h = 14 * lines.length + 10;
    let x = pointer.x + 14;
    let y = pointer.y + 14;
    if (x + w > width - 4) x = pointer.x - w - 10;
    if (y + h > height - 4) y = pointer.y - h - 10;
    ctx.fillStyle = "rgba(14, 17, 26, 0.94)";
    ctx.strokeStyle = "rgba(120, 140, 180, 0.4)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.roundRect(x, y, w, h, 5);
    ctx.fill();
    ctx.stroke();
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    lines.forEach((line, index) => {
      ctx.fillStyle = index === 0 ? "#e6e9f2" : "rgba(168, 178, 200, 0.9)";
      ctx.font = `${index === 0 ? 600 : 400} 11px ${fontFamily}`;
      ctx.fillText(line, x + 8, y + 6 + index * 14);
    });
  }

  // --- interaction -------------------------------------------------------------------

  function toWorld(clientX: number, clientY: number, now = performance.now()): { x: number; y: number } {
    const rect = canvas.getBoundingClientRect();
    const k = cam.k.at(now);
    return { x: (clientX - rect.left - cam.x.at(now)) / k, y: (clientY - rect.top - cam.y.at(now)) / k };
  }

  function hitAt(clientX: number, clientY: number): PlacedHit | null {
    const world = toWorld(clientX, clientY);
    for (const map of maps) {
      const cell = hitTest(map.layout, world.x - map.origin.x, world.y - map.origin.y);
      if (cell) return { map, cell };
    }
    return null;
  }

  function viewOf(placed: PlacedHit): View {
    const { map, cell } = placed;
    const structureId = map.layout.structureId;
    switch (cell.level) {
      case "molecule":
        return { structureId, level: "chain", indices: map.layout.chainCells.map((chain) => chain.index) };
      case "chain":
        return { structureId, level: "chain", indices: [cell.index] };
      case "residue":
        return { structureId, level: "residue", indices: [cell.index] };
      case "atom":
        return { structureId, level: "atom", indices: [cell.index] };
    }
  }

  function setLocalHover(next: PlacedHit | null): void {
    if (hover?.map === next?.map && hover?.cell === next?.cell) return;
    hover = next;
    if (next) {
      hoverGlow.set(0);
      hoverGlow.go(1, performance.now(), 180);
    }
    canvas.style.cursor = next ? "pointer" : "grab";
    options.onHover?.(next && next.cell.level !== "molecule" ? viewOf(next) : null);
    request();
  }

  let gesture:
    | { kind: "pan"; startX: number; startY: number; camX: number; camY: number }
    | { kind: "range"; map: PlacedMap; chain: number; anchor: Cell; last: Cell; moved: boolean }
    | { kind: "click"; hit: PlacedHit; startX: number; startY: number }
    | null = null;

  canvas.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    canvas.setPointerCapture(event.pointerId);
    const hit = hitAt(event.clientX, event.clientY);
    const now = performance.now();
    if (!hit) {
      gesture = { kind: "pan", startX: event.clientX, startY: event.clientY, camX: cam.x.at(now), camY: cam.y.at(now) };
      canvas.style.cursor = "grabbing";
    } else if (hit.cell.level === "residue") {
      gesture = { kind: "range", map: hit.map, chain: hit.cell.parent, anchor: hit.cell, last: hit.cell, moved: false };
    } else {
      gesture = { kind: "click", hit, startX: event.clientX, startY: event.clientY };
    }
  });

  function residuesOfChain(map: PlacedMap, chain: number): number[] {
    return map.layout.residueCells.filter((cell) => cell.parent === chain).map((cell) => cell.index);
  }

  canvas.addEventListener("pointermove", (event) => {
    const rect = canvas.getBoundingClientRect();
    pointer = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    if (gesture?.kind === "pan") {
      cam.x.set(gesture.camX + event.clientX - gesture.startX);
      cam.y.set(gesture.camY + event.clientY - gesture.startY);
      request();
      return;
    }
    if (gesture?.kind === "range") {
      const hit = hitAt(event.clientX, event.clientY);
      const cell = hit?.cell.level === "residue" && hit.map === gesture.map && hit.cell.parent === gesture.chain ? hit.cell : null;
      if (cell && cell !== gesture.last) {
        gesture.last = cell;
        gesture.moved = true;
        const range = rangeBetween(residuesOfChain(gesture.map, gesture.chain), gesture.anchor.index, cell.index);
        options.onSelect({ structureId: gesture.map.layout.structureId, level: "residue", indices: range }, "replace");
      }
      setLocalHover(hit);
      return;
    }
    setLocalHover(hitAt(event.clientX, event.clientY));
    request();
  });

  const finish = (event: PointerEvent) => {
    const active = gesture;
    gesture = null;
    canvas.style.cursor = hover ? "pointer" : "grab";
    if (!active || active.kind === "pan") return;
    const mode: Combine = event.shiftKey ? "range" : event.metaKey || event.ctrlKey ? "toggle" : "replace";
    if (active.kind === "range") {
      if (active.moved) return;
      const key = `${active.map.layout.structureId}:${active.chain}`;
      if (mode === "range") {
        const anchor = anchors.get(key) ?? active.anchor.index;
        const range = rangeBetween(residuesOfChain(active.map, active.chain), anchor, active.anchor.index);
        options.onSelect({ structureId: active.map.layout.structureId, level: "residue", indices: range }, "replace");
        return;
      }
      anchors.set(key, active.anchor.index);
      options.onSelect({ structureId: active.map.layout.structureId, level: "residue", indices: [active.anchor.index] }, mode);
      return;
    }
    if (Math.hypot(event.clientX - active.startX, event.clientY - active.startY) > 3) return;
    options.onSelect(viewOf(active.hit), mode === "range" ? "add" : mode);
  };
  canvas.addEventListener("pointerup", finish);
  canvas.addEventListener("pointercancel", () => {
    gesture = null;
  });
  canvas.addEventListener("pointerleave", () => {
    pointer = null;
    if (!gesture) setLocalHover(null);
    request();
  });

  canvas.addEventListener("dblclick", (event) => {
    const hit = hitAt(event.clientX, event.clientY);
    if (!hit) {
      fitAll(500);
      return;
    }
    if (hit.cell.level === "residue") void toggleResidue(hit.map, hit.cell.index);
    else if (hit.cell.level === "atom") void toggleResidue(hit.map, hit.cell.parent);
    else if (hit.cell.level === "chain") focusCell(hit.map, hit.cell);
    else fitAll(500);
  });

  canvas.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      const now = performance.now();
      const rect = canvas.getBoundingClientRect();
      const px = event.clientX - rect.left;
      const py = event.clientY - rect.top;
      const k = cam.k.at(now);
      const next = Math.min(80, Math.max(0.2, k * Math.exp(-event.deltaY * 0.0018)));
      const wx = (px - cam.x.at(now)) / k;
      const wy = (py - cam.y.at(now)) / k;
      cam.k.set(next);
      cam.x.set(px - wx * next);
      cam.y.set(py - wy * next);
      request();
    },
    { passive: false },
  );

  canvas.style.cursor = "grab";
  return { element, setStructures, setSelection, setHover };
}
