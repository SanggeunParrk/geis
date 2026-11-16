// The space behind the opening screens: a cell on the home screen, and somewhere else -
// reached through it - where the worlds are.
//
// The home screen looks into a living cell: hundreds of real molecules drifting in the
// dark, lit from behind by a glow deep inside it. Choosing Worlds sends the camera rushing
// into that light, and at its brightest it comes out in another place: quiet, dark, dust in
// the air, and the worlds hanging in a row. The camera there is the shelf viewer's camera
// exactly (`ROW` is shared with `worlds.ts`), so the worlds the shelf draws stand in this
// space. Entering a world from there is a plain cut: the rush belongs to the one passage
// between the two places, and repeated on every world it was a wait.
//
// The molecules are real structures (`assets/cell/molecules.json`, made by
// `src-tauri/examples/cell_assets.rs` from PDB entries) - proteins, tRNA, nucleosomes and
// ribosomes - with DNA and messenger RNA laid along curves through them: chromatin as
// beads on a string, and polysomes, ribosomes strung along a message.

import molecules from "../assets/cell/molecules.json";
import { CellRenderer, type CellBodySpec, type CellCamera, type CellTemplate } from "../viewer/cellRenderer";
import { COARSE, dnaRibbon, FINE, moleculeRibbon, rnaRibbon, type RibbonDetail, type RibbonMesh } from "../viewer/cellRibbon";
import type { Vec3 } from "../viewer/mat";

/**
 * The shelf's row, which this has to agree with: see `worlds.ts`. `lift` raises the row on
 * screen, above the caption, by looking a little below it.
 */
export const ROW = { spacing: 21, distance: 38, size: 7, shrink: 0.62, fovY: (40 * Math.PI) / 180, lift: -2.6 };

/** Where the cell is: far from the worlds, so neither is seen from the other. */
const CELL: Vec3 = [0, 0, 900];

interface Molecule {
  id: string;
  name: string;
  nucleic: boolean;
  radius: number;
  chains: number;
  /** Per chain, 1 when it is DNA or RNA. */
  nucleic_chains?: number[];
  beads: number[];
  chain: number[];
  /** The backbone for the glass view's ribbons: trace atoms, secondary structure, chain slots. */
  trace?: number[];
  ss?: string;
  trace_chain?: number[];
  /** A small molecule: one bead per heavy atom, grouped by element rather than by chain. */
  small?: boolean;
}

/**
 * Mostly teal and blue, as the old render was, with a few warm molecules among them - the
 * eye finds the coral and amber because there are few of them.
 */
const PALETTE: Record<string, [number, number, number][]> = {
  "1ubq": [[0.22, 0.56, 0.64]],
  "1lyz": [[0.30, 0.44, 0.72]],
  "1ema": [[0.26, 0.60, 0.62]],
  "4hhb": [[0.80, 0.44, 0.38], [0.86, 0.52, 0.44], [0.72, 0.36, 0.32]],
  "1igt": [[0.32, 0.46, 0.76], [0.40, 0.54, 0.80], [0.28, 0.40, 0.70]],
  "3hhr": [[0.56, 0.46, 0.74], [0.48, 0.42, 0.70], [0.44, 0.38, 0.66]],
  "1aon": [[0.24, 0.52, 0.62], [0.30, 0.46, 0.70], [0.22, 0.58, 0.60]],
  "1ehz": [[0.86, 0.66, 0.34]],
  "1j6z": [[0.40, 0.56, 0.62], [0.70, 0.58, 0.34], [0.70, 0.58, 0.34]],
  "6vxx": [[0.80, 0.44, 0.38]],
  "1kx5": [[0.36, 0.50, 0.74], [0.42, 0.56, 0.78], [0.32, 0.46, 0.70], [0.40, 0.52, 0.76]],
  "4ybb": [[0.42, 0.46, 0.74], [0.36, 0.42, 0.70]],
};

/**
 * The nucleic acids' colours, warm among the cool proteins as Goodsell paints them: DNA a
 * golden amber, RNA a dusty rose. Backbone and bases differ a little, so a helix reads.
 */
const DNA: [number, number, number][] = [[0.80, 0.60, 0.32], [0.76, 0.55, 0.28], [0.86, 0.70, 0.44]];
const RNA: [number, number, number][] = [[0.84, 0.46, 0.50], [0.92, 0.60, 0.58]];

/**
 * A small molecule's colours, by the element groups `cell_assets.rs` writes - carbon,
 * oxygen, nitrogen, phosphorus and sulfur, metal - as chemistry colours them, a little
 * softened so they sit among the proteins.
 */
const ELEMENTS: [number, number, number][] = [
  [0.56, 0.62, 0.66],
  [0.88, 0.34, 0.30],
  [0.36, 0.52, 0.92],
  [0.95, 0.66, 0.28],
  [0.82, 0.46, 0.26],
];

/** The small molecules, and how common each is among them. */
const SMALL_ABUNDANCE: Record<string, number> = { atp: 3, adp: 2, nad: 2, hem: 1, glc: 3, sam: 1 };

/**
 * The ions: how big (a bead, in angstroms, before the glass swells it into a droplet - a
 * hydrated ion is about that), what colour (the usual element colours, softened), and how
 * many. Mostly potassium and chloride, as the inside of a cell is.
 */
const IONS: { name: string; radius: number; colour: [number, number, number]; count: number }[] = [
  { name: "K+", radius: 2.0, colour: [0.58, 0.40, 0.84], count: 150 },
  { name: "Na+", radius: 1.8, colour: [0.66, 0.52, 0.90], count: 50 },
  { name: "Cl-", radius: 1.9, colour: [0.42, 0.82, 0.48], count: 140 },
  { name: "Mg2+", radius: 1.5, colour: [0.52, 0.86, 0.62], count: 60 },
  { name: "Ca2+", radius: 1.7, colour: [0.82, 0.84, 0.86], count: 20 },
];

/** The molecules that drift past in the worlds' place: the smaller proteins and a few ligands. */
const DRIFTING = ["1ubq", "1lyz", "1ema", "4hhb", "1igt", "3hhr", "1ehz", "1j6z", "atp", "nad", "hem"];

/** How common each is in the crowd. */
const ABUNDANCE: Record<string, number> = {
  "1ubq": 10, "1lyz": 8, "1ema": 7, "4hhb": 9, "1igt": 4, "3hhr": 4, "1aon": 1.5, "1ehz": 6, "6vxx": 1, "1j6z": 0,
  "1kx5": 0, "4ybb": 0.5,
};

/**
 * One turn of B-DNA, ten base pairs, as beads in angstroms along z: the two backbones 9 A
 * out and 144 degrees apart (the minor groove between them), the bases across the middle.
 * Chains: 0 and 1 the backbones, 2 the bases. Ten pairs is exactly one turn, so copies laid
 * end to end on a curve continue the helix.
 */
function dnaTurn(): { beads: number[]; chain: number[] } {
  const beads: number[] = [];
  const chain: number[] = [];
  for (let i = 0; i < 10; i += 1) {
    const z = -17 + 3.4 * (i + 0.5);
    const a = (i * 36 * Math.PI) / 180;
    const b = a + (144 * Math.PI) / 180;
    const p1: Vec3 = [9 * Math.cos(a), 9 * Math.sin(a), z];
    const p2: Vec3 = [9 * Math.cos(b), 9 * Math.sin(b), z];
    beads.push(...p1, 3.3, ...p2, 3.3);
    chain.push(0, 1);
    for (const f of [0.33, 0.67]) {
      beads.push(p1[0] + (p2[0] - p1[0]) * f, p1[1] + (p2[1] - p1[1]) * f, z, 2.7);
      chain.push(2);
    }
  }
  return { beads, chain };
}

/** Ten nucleotides of single-stranded RNA, loosely helical, along z: backbone and base. */
function rnaStretch(): { beads: number[]; chain: number[] } {
  const beads: number[] = [];
  const chain: number[] = [];
  for (let i = 0; i < 10; i += 1) {
    const z = -29.5 + 5.9 * (i + 0.5);
    const a = (i * 33 * Math.PI) / 180;
    beads.push(4.5 * Math.cos(a), 4.5 * Math.sin(a), z, 3.3);
    beads.push(1.2 * Math.cos(a + 1.2), 1.2 * Math.sin(a + 1.2), z, 2.8);
    chain.push(0, 1);
  }
  return { beads, chain };
}

/** Darker and greyer, and a little towards the blue of the light: the cell's colours. */
function mute(rgb: [number, number, number], vary: number): [number, number, number] {
  const grey = 0.3 * rgb[0] + 0.59 * rgb[1] + 0.11 * rgb[2];
  const teal: [number, number, number] = [0.14, 0.30, 0.46];
  return rgb.map((c, i) => Math.min(1, (c * 0.7 + grey * 0.3) * vary * 0.92 + teal[i] * 0.12)) as [number, number, number];
}

type Quat = [number, number, number, number];

function quatAxisAngle(axis: Vec3, angle: number): Quat {
  const half = angle / 2;
  const s = Math.sin(half);
  const l = Math.hypot(axis[0], axis[1], axis[2]) || 1;
  return [(axis[0] / l) * s, (axis[1] / l) * s, (axis[2] / l) * s, Math.cos(half)];
}

function quatMultiply(a: Quat, b: Quat): Quat {
  return [
    a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
    a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
    a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
    a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
  ];
}

/** The rotation that takes direction `from` to direction `to`. */
function quatBetween(from: Vec3, to: Vec3): Quat {
  const f = normal(from);
  const t = normal(to);
  const d = f[0] * t[0] + f[1] * t[1] + f[2] * t[2];
  if (d < -0.9999) return quatAxisAngle(Math.abs(f[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0], Math.PI);
  const c: Vec3 = [f[1] * t[2] - f[2] * t[1], f[2] * t[0] - f[0] * t[2], f[0] * t[1] - f[1] * t[0]];
  const q: Quat = [c[0], c[1], c[2], 1 + d];
  const l = Math.hypot(...q);
  return [q[0] / l, q[1] / l, q[2] / l, q[3] / l];
}

function normal(v: Vec3): Vec3 {
  const l = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / l, v[1] / l, v[2] / l];
}

/** A seeded random number generator, so the cell is the same cell every time. */
function seeded(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface Mover {
  template: number;
  base: Vec3;
  /** How far and how fast it wanders from where it was put. */
  drift: number;
  phases: [number, number, number];
  axis: Vec3;
  spin: number;
  start: Quat;
}

interface Pose {
  eye: Vec3;
  target: Vec3;
  fovY: number;
}

type Mode = "home" | "shelf" | "hidden";

/** The two places' light: the glow behind, the dark around it, and the edge light it gives. */
const LIGHT = {
  cell: { glow: [0.28, 0.50, 0.70] as Vec3, deep: [0.016, 0.036, 0.078] as Vec3, rim: [0.44, 0.66, 0.86] as Vec3, at: [0.5, 0.52] as [number, number], haze: 0.016 },
  // The same blue as the cell: one space, so the tone does not change on the way through.
  realm: { glow: [0.28, 0.50, 0.70] as Vec3, deep: [0.016, 0.036, 0.078] as Vec3, rim: [0.44, 0.66, 0.86] as Vec3, at: [0.5, 0.62] as [number, number], haze: 0.0055 },
}

/** A flash of light over the whole window: up quickly, then down slowly over what is next. */
function flashScreen(hold = 520): void {
  const veil = document.createElement("div");
  veil.className = "dive-flash";
  document.body.append(veil);
  requestAnimationFrame(() => veil.classList.add("up"));
  setTimeout(() => {
    veil.classList.remove("up");
    veil.classList.add("down");
    setTimeout(() => veil.remove(), 900);
  }, hold);
}

class Stage {
  private canvas = document.createElement("canvas");
  private renderer: CellRenderer | null = null;
  private movers: Mover[] = [];
  private built = false;
  private mode: Mode = "hidden";
  /** Which of the two places the camera is in now. */
  private place: "cell" | "realm" = "cell";
  private along = 0;
  private pose: Pose = { eye: [0, 0, 60], target: [0, 0, 0], fovY: ROW.fovY };
  private flight: { from: Pose; to: () => Pose; start: number; length: number; ease: (t: number) => number; done?: () => void } | null = null;
  private lean: [number, number] = [0, 0];
  private leanWanted: [number, number] = [0, 0];
  private running = false;
  private started = performance.now();
  /** Whether the glass view's ribbons have been made: once, the first time it is shown. */
  private ribbons = false;
  /** Who is waiting for the camera to stop flying (`settled`). */
  private waiting: (() => void)[] = [];

  private release(): void {
    const waiting = this.waiting;
    this.waiting = [];
    for (const resolve of waiting) resolve();
  }

  constructor() {
    this.canvas.className = "cell-stage";
    this.canvas.setAttribute("aria-hidden", "true");
    this.canvas.hidden = true;
    window.addEventListener("pointermove", (event) => {
      this.leanWanted = [event.clientX / Math.max(window.innerWidth, 1) - 0.5, event.clientY / Math.max(window.innerHeight, 1) - 0.5];
    }, { passive: true });
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) this.loop();
    });
  }

  /**
   * The cell's molecules are drawn as glass: each a clear shell with its fold inside. The
   * opaque Goodsell look is kept in the renderer, but nothing asks for it.
   */
  readonly glass = true;

  /** The canvas, for the shell to put behind its screens. */
  get element(): HTMLCanvasElement {
    return this.canvas;
  }

  /**
   * Builds both places once. Nothing in either depends on how many worlds there are, so
   * it is never built again - a rebuild was a stall as the screen changed.
   */
  private build(): void {
    if (this.built) return;
    if (!this.renderer) {
      try {
        this.renderer = new CellRenderer(this.canvas);
      } catch {
        return;
      }
    }
    this.built = true;
    const renderer = this.renderer;
    const list = (molecules as { molecules: Molecule[] }).molecules;
    const templates: CellTemplate[] = list.map((molecule) => ({
      beads: new Float32Array(molecule.beads),
      chain: new Uint16Array(molecule.chain),
      solid: molecule.small === true,
    }));
    const dnaPiece = templates.length;
    const dna = dnaTurn();
    templates.push({ beads: new Float32Array(dna.beads), chain: new Uint16Array(dna.chain) });
    const rnaPiece = templates.length;
    const rna = rnaStretch();
    templates.push({ beads: new Float32Array(rna.beads), chain: new Uint16Array(rna.chain) });
    const ionFirst = templates.length;
    for (const ion of IONS) templates.push({ beads: new Float32Array([0, 0, 0, ion.radius]), chain: new Uint16Array([0]), solid: true });

    const random = seeded(1961);
    const index = (id: string) => list.findIndex((molecule) => molecule.id === id);
    const pickable = list.map((molecule) => ABUNDANCE[molecule.id] ?? 0);
    const totalWeight = pickable.reduce((sum, weight) => sum + weight, 0);
    const pick = () => {
      let roll = random() * totalWeight;
      for (let i = 0; i < pickable.length; i += 1) {
        roll -= pickable[i];
        if (roll <= 0) return i;
      }
      return 0;
    };
    const turnAnywhere = (): Quat => quatAxisAngle(normal([random() - 0.5, random() - 0.5, random() - 0.5]), random() * Math.PI * 2);
    const movers: Mover[] = [];
    const bodies: CellBodySpec[] = [];
    const add = (mover: Mover, colours: [number, number, number][]) => {
      movers.push(mover);
      bodies.push({ template: mover.template, colours });
    };
    // Per chain: the molecule's palette for its proteins, DNA or RNA for its nucleic chains
    // (DNA only in the nucleosome; every other nucleic chain here is RNA).
    const colourOf = (id: string, vary: number) => {
      const molecule = list.find((entry) => entry.id === id);
      if (molecule?.small) return ELEMENTS.map((rgb) => mute(rgb, vary));
      const palette = PALETTE[id] ?? [[0.6, 0.6, 0.6]];
      const nucleic = molecule?.nucleic_chains;
      if (!molecule || !nucleic?.some(Boolean)) return palette.map((rgb) => mute(rgb, vary));
      let protein = 0;
      return nucleic.map((flag, chain) =>
        mute(flag ? (id === "1kx5" ? DNA : RNA)[chain % 2] : palette[protein++ % palette.length], vary),
      );
    };

    // The cell's crowd, round the path the home screen's camera takes and clear of it.
    const path = Array.from({ length: 48 }, (_, k) => this.homeEye((k / 48) * (2 * Math.PI) / 0.018));
    // And the space straight ahead of it, where the title stands: nothing within the first
    // thirty units of its line of sight.
    const sight = path.map((eye) => {
      const target: Vec3 = [CELL[0] + 12, CELL[1], CELL[2] - 40];
      return { eye, ahead: normal([target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]]) };
    });
    const offPath = (p: Vec3, size: number) =>
      path.every((eye) => Math.hypot(p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]) > size + 4) &&
      sight.every(({ eye, ahead }) => {
        const d: Vec3 = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
        const along = d[0] * ahead[0] + d[1] * ahead[1] + d[2] * ahead[2];
        if (along < 0 || along > 30) return true;
        const across = Math.hypot(d[0] - ahead[0] * along, d[1] - ahead[1] * along, d[2] - ahead[2] * along);
        return across > 3 + along * 0.16 + size;
      });
    let placed = 0;
    while (placed < 1500) {
      const template = pick();
      const p: Vec3 = [CELL[0] - 90 + random() * 180, CELL[1] - 32 + random() * 64, CELL[2] - 75 + random() * 95];
      const size = list[template].radius * renderer.angstrom;
      if (!offPath(p, size)) continue;
      add(
        { template, base: p, drift: 0.5 + random() * 1.2, phases: [random() * 6.28, random() * 6.28, random() * 6.28], axis: normal([random() - 0.5, random() - 0.5, random() - 0.5]), spin: (0.04 + random() * 0.18) * (random() < 0.5 ? -1 : 1), start: turnAnywhere() },
        colourOf(list[template].id, 0.85 + random() * 0.3),
      );
      placed += 1;
    }

    // Actin filaments: the monomer, stacked on the filament's helix - each next one 27.5 A
    // further along and turned -166.7 degrees about the axis.
    const actin = index("1j6z");
    if (actin >= 0) {
      for (let f = 0; f < 5; f += 1) {
        const start: Vec3 = [CELL[0] - 90 + random() * 120, CELL[1] - 28 + random() * 56, CELL[2] - 70 + random() * 50];
        const axis = normal([0.6 + random() * 0.4, (random() - 0.5) * 0.8, (random() - 0.5) * 0.6]);
        const across = normal([-axis[1], axis[0], 0.2]);
        const align = quatBetween([0, 0, 1], axis);
        for (let k = 0; k < 70; k += 1) {
          const twist = quatAxisAngle(axis, (k * -166.7 * Math.PI) / 180);
          const radial = rotate(twist, across);
          const along = k * 27.5 * renderer.angstrom;
          const out = 25 * renderer.angstrom;
          const p: Vec3 = [start[0] + axis[0] * along + radial[0] * out, start[1] + axis[1] * along + radial[1] * out, start[2] + axis[2] * along + radial[2] * out];
          if (!offPath(p, 1)) continue;
          add({ template: actin, base: p, drift: 0.15, phases: [f, f * 2, f * 3], axis, spin: 0, start: quatMultiply(twist, align) }, colourOf("1j6z", k % 2 ? 0.95 : 1.05));
        }
      }
    }

    // Strands: a smooth random walk through the cell, turning a little at each step and
    // steered back when it nears the edge. Each piece is turned from the last by the bend
    // between them alone (parallel transport), so a helix laid along it does not twist
    // where the curve bends.
    const inside = (p: Vec3) => Math.abs(p[0] - CELL[0]) < 85 && Math.abs(p[1] - CELL[1]) < 28 && p[2] - CELL[2] > -72 && p[2] - CELL[2] < 16;
    const strand = (start: Vec3, heading: Vec3, steps: number, step: number, bend: number) => {
      const pieces: { centre: Vec3; turn: Quat; tangent: Vec3; side: Vec3 }[] = [];
      let p = start;
      let t = normal(heading);
      let turn = quatBetween([0, 0, 1], t);
      for (let k = 0; k < steps; k += 1) {
        const towards = normal([CELL[0] - p[0], CELL[1] - p[1], CELL[2] - 28 - p[2]]);
        const pull = inside(p) ? 0.02 : 0.3;
        const next = normal([
          t[0] + (random() - 0.5) * bend + towards[0] * pull,
          t[1] + (random() - 0.5) * bend + towards[1] * pull,
          t[2] + (random() - 0.5) * bend + towards[2] * pull,
        ]);
        turn = quatMultiply(quatBetween(t, next), turn);
        t = next;
        const centre: Vec3 = [p[0] + t[0] * step * 0.5, p[1] + t[1] * step * 0.5, p[2] + t[2] * step * 0.5];
        pieces.push({ centre, turn, tangent: t, side: rotate(turn, [1, 0, 0]) });
        p = [p[0] + t[0] * step, p[1] + t[1] * step, p[2] + t[2] * step];
      }
      return pieces;
    };
    const anywhere = (): Vec3 => [CELL[0] - 80 + random() * 160, CELL[1] - 24 + random() * 48, CELL[2] - 68 + random() * 70];
    const heading = (): Vec3 => normal([random() - 0.5, (random() - 0.5) * 0.6, random() - 0.5]);
    const still = (seed: number): Pick<Mover, "drift" | "phases" | "axis" | "spin"> => ({ drift: 0.12, phases: [seed, seed * 2, seed * 3], axis: [0, 1, 0], spin: 0 });

    // Chromatin: DNA wound round nucleosomes, beads on a string - fifty-odd base pairs of
    // bare linker between each, and the next nucleosome where the string comes out.
    const nucleosome = index("1kx5");
    const turnLength = 34 * renderer.angstrom;
    for (let fibre = 0; fibre < 3; fibre += 1) {
      const pieces = strand(anywhere(), heading(), 110, turnLength, 0.22);
      const seed = 10 + fibre;
      for (let k = 0; k < pieces.length; k += 1) {
        const piece = pieces[k];
        if (nucleosome >= 0 && k % 8 === 7) {
          // The nucleosome sits beside the string, the DNA running round its edge.
          const out = 45 * renderer.angstrom;
          const p: Vec3 = [piece.centre[0] + piece.side[0] * out, piece.centre[1] + piece.side[1] * out, piece.centre[2] + piece.side[2] * out];
          if (offPath(p, list[nucleosome].radius * renderer.angstrom * 0.6)) {
            add({ template: nucleosome, base: p, ...still(seed), start: quatMultiply(piece.turn, quatAxisAngle([1, 0, 0], random() * 6.28)) }, colourOf("1kx5", 0.9 + random() * 0.2));
          }
          continue;
        }
        if (!offPath(piece.centre, 1)) continue;
        add({ template: dnaPiece, base: piece.centre, ...still(seed), start: piece.turn }, DNA.map((rgb) => mute(rgb, 0.8)));
      }
    }
    // A long stretch of bare DNA, looping through the crowd.
    for (let loop = 0; loop < 1; loop += 1) {
      for (const piece of strand(anywhere(), heading(), 160, turnLength, 0.2)) {
        if (offPath(piece.centre, 1)) add({ template: dnaPiece, base: piece.centre, ...still(20 + loop), start: piece.turn }, DNA.map((rgb) => mute(rgb, 0.78)));
      }
    }
    // Polysomes: messenger RNA with ribosomes strung along it, each reading its own stretch.
    const ribosome = index("4ybb");
    const rnaLength = 59 * renderer.angstrom;
    for (let polysome = 0; polysome < 4; polysome += 1) {
      const pieces = strand(anywhere(), heading(), 70, rnaLength, 0.35);
      const seed = 30 + polysome;
      pieces.forEach((piece, k) => {
        if (ribosome >= 0 && k % 9 === 4) {
          const out = 70 * renderer.angstrom;
          const p: Vec3 = [piece.centre[0] + piece.side[0] * out, piece.centre[1] + piece.side[1] * out, piece.centre[2] + piece.side[2] * out];
          if (offPath(p, list[ribosome].radius * renderer.angstrom * 0.7)) {
            add({ template: ribosome, base: p, ...still(seed), start: quatMultiply(piece.turn, quatAxisAngle([0, 0, 1], random() * 6.28)) }, colourOf("4ybb", 0.9 + random() * 0.2));
          }
        }
        if (offPath(piece.centre, 1)) add({ template: rnaPiece, base: piece.centre, ...still(seed), start: piece.turn }, RNA.map((rgb) => mute(rgb, 1)));
      });
    }

    // Small molecules, tumbling faster than the proteins, as small things do; and ions,
    // which are single beads the glass swells into droplets.
    const smallIds = Object.keys(SMALL_ABUNDANCE).filter((id) => index(id) >= 0);
    const smallWeight = smallIds.reduce((sum, id) => sum + SMALL_ABUNDANCE[id], 0);
    let small = 0;
    while (smallIds.length > 0 && small < 320) {
      let roll = random() * smallWeight;
      const id = smallIds.find((entry) => (roll -= SMALL_ABUNDANCE[entry]) <= 0) ?? smallIds[0];
      const p: Vec3 = [CELL[0] - 90 + random() * 180, CELL[1] - 32 + random() * 64, CELL[2] - 75 + random() * 95];
      if (!offPath(p, list[index(id)].radius * renderer.angstrom)) continue;
      add(
        { template: index(id), base: p, drift: 0.8 + random() * 1.6, phases: [random() * 6.28, random() * 6.28, random() * 6.28], axis: normal([random() - 0.5, random() - 0.5, random() - 0.5]), spin: (0.3 + random() * 0.5) * (random() < 0.5 ? -1 : 1), start: turnAnywhere() },
        colourOf(id, 0.9 + random() * 0.2),
      );
      small += 1;
    }
    IONS.forEach((ion, k) => {
      let made = 0;
      while (made < ion.count) {
        const p: Vec3 = [CELL[0] - 90 + random() * 180, CELL[1] - 32 + random() * 64, CELL[2] - 75 + random() * 95];
        if (!offPath(p, 0.5)) continue;
        add({ template: ionFirst + k, base: p, drift: 1 + random() * 2, phases: [random() * 6.28, random() * 6.28, random() * 6.28], axis: [0, 1, 0], spin: 0, start: [0, 0, 0, 1] }, [mute(ion.colour, 0.9 + random() * 0.2)]);
        made += 1;
      }
    });

    // The worlds' place: a few molecules drifting in the dark, far behind the row and to
    // either side of it, never between it and the camera. Dust was here before - grey balls
    // floating about, which read as nothing in particular.
    const drifting = DRIFTING.map(index).filter((at) => at >= 0);
    let drifted = 0;
    while (drifting.length > 0 && drifted < 110) {
      const template = drifting[Math.floor(random() * drifting.length)];
      const p: Vec3 = [-90 + random() * 380, -45 + random() * 85, -100 + random() * 115];
      // Clear of the row and of the space between it and the camera: behind it, or well
      // above or below it.
      if (p[2] > -28 && Math.abs(p[1] - ROW.lift) < 20) continue;
      add(
        { template, base: p, drift: 3 + random() * 4, phases: [random() * 6.28, random() * 6.28, random() * 6.28], axis: normal([random() - 0.5, random() - 0.5, random() - 0.5]), spin: (0.04 + random() * 0.12) * (random() < 0.5 ? -1 : 1), start: turnAnywhere() },
        colourOf(list[template].id, 0.75 + random() * 0.3),
      );
      drifted += 1;
    }
    this.movers = movers;
    renderer.setScene(templates, bodies);
  }

  /**
   * Each template's ribbons, in the template order `build` made: the molecules (none for a
   * small one), then a turn of DNA and a stretch of RNA; the ions after them have none. Coloured as the beads are, a
   * little lighter for a helix, so the folds read inside the glass.
   */
  private makeRibbons(detail: RibbonDetail): (RibbonMesh | null)[] {
    const list = (molecules as { molecules: Molecule[] }).molecules;
    const meshes: (RibbonMesh | null)[] = list.map((molecule) => {
      if (!molecule.trace || !molecule.ss || !molecule.trace_chain) return null;
      const palette = PALETTE[molecule.id] ?? [[0.6, 0.6, 0.6]];
      const nucleic = molecule.nucleic_chains ?? [];
      let protein = 0;
      const byChain = Array.from({ length: Math.max(molecule.chains, nucleic.length, 1) }, (_, chain) =>
        nucleic[chain] ? (molecule.id === "1kx5" ? DNA : RNA)[chain % 2] : palette[protein++ % palette.length],
      );
      return moleculeRibbon(molecule.trace, molecule.ss, molecule.trace_chain, (chain, kind) => {
        const rgb = byChain[chain % byChain.length];
        const lift = kind === "H" ? 1.12 : kind === "E" ? 1.04 : 0.9;
        return rgb.map((c) => Math.min(1, c * lift)) as [number, number, number];
      }, detail);
    });
    meshes.push(dnaRibbon([DNA[0], DNA[1], DNA[2]], detail));
    meshes.push(rnaRibbon([RNA[0], RNA[1]], detail));
    return meshes;
  }

  /** Every copy's place and turn at time `t`, into the renderer's transforms. */
  private poseBodies(t: number): void {
    const renderer = this.renderer;
    if (!renderer) return;
    const out = renderer.transforms;
    this.movers.forEach((mover, i) => {
      const d = mover.drift;
      // Each wanders about where it was put.
      const p: Vec3 = [
        mover.base[0] + Math.sin(t * 0.11 + mover.phases[0]) * d,
        mover.base[1] + Math.sin(t * 0.09 + mover.phases[1]) * d,
        mover.base[2] + Math.sin(t * 0.07 + mover.phases[2]) * d,
      ];
      const q = mover.spin === 0 ? mover.start : quatMultiply(quatAxisAngle(mover.axis, t * mover.spin), mover.start);
      const at = i * 8;
      out[at] = p[0];
      out[at + 1] = p[1];
      out[at + 2] = p[2];
      out[at + 3] = 1;
      out[at + 4] = q[0];
      out[at + 5] = q[1];
      out[at + 6] = q[2];
      out[at + 7] = q[3];
    });
  }

  /** Where the camera would be in this mode, now. */
  private wanted(t: number): Pose {
    if (this.mode === "shelf" || this.mode === "hidden") return this.shelfPose();
    // Home: a slow orbit through the crowd, leaning with the pointer.
    const eye = this.homeEye(t);
    const target: Vec3 = [CELL[0] + 12 + this.lean[0] * 10, CELL[1] - this.lean[1] * 6, CELL[2] - 40];
    return { eye, target, fovY: (50 * Math.PI) / 180 };
  }

  private shelfPose(): Pose {
    const x = this.along * ROW.spacing;
    return { eye: [x, ROW.lift, ROW.distance], target: [x, ROW.lift, 0], fovY: ROW.fovY };
  }

  /** Where the home screen's camera is at time `t`: wandering through the crowd. */
  private homeEye(t: number): Vec3 {
    const angle = t * 0.018;
    return [CELL[0] + Math.sin(angle) * 12, CELL[1] + 4 + Math.sin(t * 0.05) * 2.5, CELL[2] + 14 + Math.cos(angle) * 7];
  }

  private loop(): void {
    if (this.running || this.mode === "hidden" || document.hidden) return;
    this.running = true;
    const frame = () => {
      if (this.mode === "hidden" || document.hidden || !this.canvas.isConnected) {
        this.running = false;
        return;
      }
      this.draw();
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  }

  private draw(): void {
    const renderer = this.renderer;
    if (!renderer) return;
    const now = performance.now();
    const t = (now - this.started) / 1000;
    this.lean[0] += (this.leanWanted[0] - this.lean[0]) * 0.04;
    this.lean[1] += (this.leanWanted[1] - this.lean[1]) * 0.04;
    let pose = this.wanted(t);
    if (this.flight) {
      const progress = Math.min((now - this.flight.start) / this.flight.length, 1);
      const k = this.flight.ease(progress);
      const to = this.flight.to();
      const mix = (a: Vec3, b: Vec3): Vec3 => [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
      pose = { eye: mix(this.flight.from.eye, to.eye), target: mix(this.flight.from.target, to.target), fovY: this.flight.from.fovY + (to.fovY - this.flight.from.fovY) * k };
      if (progress >= 1) {
        const done = this.flight.done;
        this.flight = null;
        done?.();
        if (!this.flight) this.release();
      }
    }
    this.pose = pose;
    this.poseBodies(t);
    const light = LIGHT[this.place];
    renderer.glowColour = light.glow;
    renderer.deepColour = light.deep;
    renderer.rimColour = light.rim;
    renderer.glow = light.at;
    renderer.haze = light.haze;
    // Glass in both places: the worlds' place has molecules drifting in it too.
    renderer.glass = this.glass;
    if (renderer.glass && !this.ribbons) {
      this.ribbons = true;
      renderer.setRibbons(this.makeRibbons(FINE), this.makeRibbons(COARSE));
    }
    renderer.focus = this.place === "realm" ? Math.hypot(pose.eye[0] - pose.target[0], pose.eye[1] - pose.target[1], pose.eye[2] - pose.target[2]) : 26;
    const camera: CellCamera = { eye: pose.eye, target: pose.target, up: [0, 1, 0], fovY: pose.fovY };
    renderer.render(camera, 0.3, 400);
  }

  private fly(to: () => Pose, length: number, ease: (t: number) => number, done?: () => void): void {
    this.flight = { from: { ...this.pose, eye: [...this.pose.eye] as Vec3, target: [...this.pose.target] as Vec3 }, to, start: performance.now(), length, ease, done };
  }

  /**
   * Through to the other place: the camera rushes forward into the light, the window goes
   * to light at the fastest moment, and the camera comes out in `place` - at `arrive` - and
   * settles to `settle`.
   */
  private warp(place: "cell" | "realm", arrive: () => Pose, settle: () => Pose, done?: () => void): void {
    const from = this.pose;
    const ahead = normal([from.target[0] - from.eye[0], from.target[1] - from.eye[1], from.target[2] - from.eye[2]]);
    const rush: Pose = {
      eye: [from.eye[0] + ahead[0] * 70, from.eye[1] + ahead[1] * 70, from.eye[2] + ahead[2] * 70],
      target: [from.target[0] + ahead[0] * 70, from.target[1] + ahead[1] * 70, from.target[2] + ahead[2] * 70],
      fovY: (85 * Math.PI) / 180,
    };
    setTimeout(() => flashScreen(380), 420);
    this.fly(() => rush, 760, easeIn, () => {
      this.place = place;
      this.pose = arrive();
      this.fly(settle, 1100, easeOut, done);
    });
  }

  /** The home screen: in the cell. */
  home(): void {
    this.build();
    const from = this.mode;
    this.mode = "home";
    this.canvas.hidden = false;
    if (from === "shelf" && this.place === "realm") {
      const t = () => (performance.now() - this.started) / 1000;
      this.warp("cell", () => {
        const pose = this.wanted(t());
        const back = normal([pose.eye[0] - pose.target[0], pose.eye[1] - pose.target[1], pose.eye[2] - pose.target[2]]);
        return { ...pose, eye: [pose.eye[0] + back[0] * 25, pose.eye[1] + back[1] * 25, pose.eye[2] + back[2] * 25], fovY: (70 * Math.PI) / 180 };
      }, () => this.wanted(t()));
    } else {
      this.place = "cell";
    }
    this.loop();
  }

  /**
   * The shelf: in the worlds' place, in front of the row at `along`. Whether the camera
   * is coming through from the cell, so the shelf can keep its worlds back until it is there.
   */
  shelf(along: number): boolean {
    this.build();
    this.along = along;
    const from = this.mode;
    this.mode = "shelf";
    this.canvas.hidden = false;
    this.loop();
    if (from === "home" && this.place === "cell") {
      this.warp("realm", () => {
        const pose = this.shelfPose();
        return { ...pose, eye: [pose.eye[0], pose.eye[1] + 4, pose.eye[2] + 40], fovY: (70 * Math.PI) / 180 };
      }, () => this.shelfPose());
      return true;
    }
    this.place = "realm";
    return false;
  }

  /**
   * Resolves once the camera has stopped flying - at once when it is not. The shelf waits
   * for this before it hands the row to the graphics card, so the upload's stall falls on
   * a still camera rather than in the middle of the passage.
   */
  settled(): Promise<void> {
    if (!this.flight) return Promise.resolve();
    return new Promise((resolve) => this.waiting.push(resolve));
  }

  /** Where the shelf's camera is along the row, every frame, so the two agree. */
  follow(along: number): void {
    this.along = along;
  }

  /** Inside a world, or in the settings, nothing behind the page is drawn. */
  hide(): void {
    this.mode = "hidden";
    this.canvas.hidden = true;
    // Nothing is drawn now, so no flight will finish; whoever waits for one goes on.
    this.flight = null;
    this.release();
  }
}

function rotate(q: Quat, v: Vec3): Vec3 {
  const [x, y, z, w] = q;
  const t: Vec3 = [2 * (y * v[2] - z * v[1]), 2 * (z * v[0] - x * v[2]), 2 * (x * v[1] - y * v[0])];
  return [v[0] + w * t[0] + (y * t[2] - z * t[1]), v[1] + w * t[1] + (z * t[0] - x * t[2]), v[2] + w * t[2] + (x * t[1] - y * t[0])];
}

const easeIn = (t: number) => t * t * t;
const easeOut = (t: number) => 1 - Math.pow(1 - t, 3);

/** The one stage, shared by the home screen and the shelf. */
export const cellStage = new Stage();
