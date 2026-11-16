// Ribbons for the cell's glass view: a molecule's backbone as a cartoon, small enough to
// build in the window for every kind of molecule in the cell at once.
//
// From the trace atoms alone (alpha carbons, phosphorus) and each residue's secondary
// structure, as `cell_assets.rs` writes them. The same rules as the viewport's cartoon
// (`geometry/cartoon.rs`), cut down:
//
// - A helix is drawn as a helix about its own axis, not as a spline through its alpha
//   carbons. Those come 100 degrees apart, 3.6 to a turn, and a spline through them is a
//   rounded triangle - under glass, where nothing hides it, every helix looked faceted.
//   The axis is the mean of a turn's worth of alpha carbons, smoothed along the run; the
//   path slides along it and turns about it. It is eased in from the spline and back out to
//   it at the helix's ends, so the path has no corner where the two meet.
// - The section's breadth is blended over four residues by the cubic B-spline's weights,
//   so a helix narrows into its loop over two residues with no corner - blended over one,
//   every helix ended in a broad flat flag.
// - The face turns with the helix (it points out from the axis, so the ribbon is broad
//   along it) and elsewhere follows the bend, kept facing the way it faced before.

import type { Vec3 } from "./mat";

export interface RibbonMesh {
  /** Angstroms, about the molecule's middle, as its beads are. */
  positions: Float32Array;
  normals: Float32Array;
  colours: Float32Array;
  indices: Uint32Array;
}

type Rgb = [number, number, number];

/**
 * How finely a ribbon is built: points round each section, and sections a residue - more
 * in a helix, whose path turns a third of a circle each residue. Fine for the copies near
 * the camera, where a ribbon is large enough for an edge to show; coarse further off,
 * where the fine one cost a quarter of the frame rate for nothing anyone could see.
 */
export interface RibbonDetail {
  sides: number;
  samples: number;
  helixSamples: number;
}
export const FINE: RibbonDetail = { sides: 10, samples: 3, helixSamples: 5 };
export const COARSE: RibbonDetail = { sides: 6, samples: 2, helixSamples: 3 };

/** Half-width and half-thickness of the section, in angstroms, by secondary structure. */
const PROFILE: Record<string, [number, number]> = {
  H: [1.3, 0.3],
  E: [1.3, 0.3],
  C: [0.42, 0.42],
  N: [1.0, 1.0],
  // A base pair's rung, thinner than a backbone.
  R: [0.55, 0.55],
};
/** A strand's arrowhead: how wide it starts. */
const ARROW = 2.2;


const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const add = (a: Vec3, b: Vec3): Vec3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const scale = (a: Vec3, k: number): Vec3 => [a[0] * k, a[1] * k, a[2] * k];
const mix = (a: Vec3, b: Vec3, t: number): Vec3 => add(a, scale(sub(b, a), t));
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: Vec3, b: Vec3): Vec3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const length = (a: Vec3) => Math.hypot(a[0], a[1], a[2]);
function unit(a: Vec3, fallback: Vec3 = [1, 0, 0]): Vec3 {
  const l = length(a);
  return l > 1e-6 ? scale(a, 1 / l) : fallback;
}
function anyPerpendicular(v: Vec3): Vec3 {
  return unit(cross(v, Math.abs(v[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0]));
}
/** `v` with its part along `along` taken out, as a unit vector. */
function across(v: Vec3, along: Vec3, fallback?: Vec3): Vec3 {
  return unit(sub(v, scale(along, dot(v, along))), fallback ?? anyPerpendicular(along));
}
const smoothstep = (t: number) => {
  const x = Math.min(Math.max(t, 0), 1);
  return x * x * (3 - 2 * x);
};

function catmullRom(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: number): Vec3 {
  const t2 = t * t;
  const t3 = t2 * t;
  return [0, 1, 2].map(
    (i) => 0.5 * (2 * p1[i] + (p2[i] - p0[i]) * t + (2 * p0[i] - 5 * p1[i] + 4 * p2[i] - p3[i]) * t2 + (3 * p1[i] - p0[i] - 3 * p2[i] + p3[i]) * t3),
  ) as Vec3;
}

/** Turned from `a` towards `b` by fraction `t` of the angle between them. */
function slerp(a: Vec3, b: Vec3, t: number): Vec3 {
  const c = Math.min(Math.max(dot(a, b), -1), 1);
  const angle = Math.acos(c);
  if (angle < 1e-3) return unit(add(scale(a, 1 - t), scale(b, t)), a);
  const s = Math.sin(angle);
  return unit(add(scale(a, Math.sin((1 - t) * angle) / s), scale(b, Math.sin(t * angle) / s)), a);
}

/** The uniform cubic B-spline's four weights at `t`. */
function bspline(t: number): [number, number, number, number] {
  const u = 1 - t;
  return [(u * u * u) / 6, (3 * t * t * t - 6 * t * t + 4) / 6, (-3 * t * t * t + 3 * t * t + 3 * t + 1) / 6, (t * t * t) / 6];
}

/**
 * Each helix residue's place on its helix's axis, or null outside a helix. The axis
 * point between residues i and i+1 is the mean of the four alpha carbons round it - a
 * turn's worth, so the turning cancels - and a residue's is the mean of the two either
 * side of it, smoothed once more along the run.
 */
function helixAxes(points: Vec3[], kinds: string[]): (Vec3 | null)[] {
  const n = points.length;
  const axes: (Vec3 | null)[] = points.map(() => null);
  let start = 0;
  while (start < n) {
    if (kinds[start] !== "H") {
      start += 1;
      continue;
    }
    let end = start;
    while (end + 1 < n && kinds[end + 1] === "H") end += 1;
    // A run too short to fit stays a spline.
    if (end - start + 1 >= 5) {
      const mid: Vec3[] = [];
      for (let i = start; i < end; i += 1) {
        const a = Math.max(i - 1, start);
        const d = Math.min(i + 2, end);
        mid.push(scale(add(add(points[a], points[i]), add(points[i + 1], points[d])), 0.25));
      }
      const raw: Vec3[] = [];
      for (let i = start; i <= end; i += 1) {
        const before = mid[Math.max(i - start - 1, 0)];
        const after = mid[Math.min(i - start, mid.length - 1)];
        raw.push(scale(add(before, after), 0.5));
      }
      for (let i = start; i <= end; i += 1) {
        const k = i - start;
        const smooth = scale(add(add(raw[Math.max(k - 1, 0)], scale(raw[k], 2)), raw[Math.min(k + 1, raw.length - 1)]), 0.25);
        // Too near the axis to say which way is out - a residue pulled off towards a loop.
        axes[i] = length(sub(points[i], smooth)) > 1.2 ? smooth : null;
      }
    }
    start = end + 1;
  }
  return axes;
}

class Builder {
  positions: number[] = [];
  normals: number[] = [];
  colours: number[] = [];
  indices: number[] = [];

  constructor(private detail: RibbonDetail) {}

  /**
   * One run of connected residues. `kinds` is a letter a residue (H, E, C, N, R), `colour`
   * each residue's colour.
   */
  run(points: Vec3[], kinds: string[], colour: (index: number) => Rgb, samples = this.detail.samples): void {
    const n = points.length;
    if (n < 2) return;
    // A strand's pleat smoothed out of its path, as a cartoon draws it.
    const controls = points.map((p, i) =>
      kinds[i] === "E" && i > 0 && i < n - 1 ? scale(add(add(points[i - 1], scale(p, 2)), points[i + 1]), 0.25) : p,
    );
    const axes = helixAxes(points, kinds);
    const helical = (i: number) => i >= 0 && i + 1 < n && axes[i] !== null && axes[i + 1] !== null;

    const spline = (i: number, t: number) =>
      catmullRom(controls[Math.max(i - 1, 0)], controls[i], controls[i + 1], controls[Math.min(i + 2, n - 1)], t);

    /** On the helix between residues i and i+1: along the axis, round it. */
    const onHelix = (i: number, t: number): Vec3 => {
      const a = axes[i]!;
      const b = axes[i + 1]!;
      const centre = mix(a, b, t);
      const ra = sub(points[i], a);
      const rb = sub(points[i + 1], b);
      const along = unit(sub(b, a));
      const radial = slerp(across(ra, along), across(rb, along), t);
      return add(centre, scale(radial, length(ra) + (length(rb) - length(ra)) * t));
    };

    const pointAt = (i: number, t: number): Vec3 => {
      const curve = spline(i, t);
      if (!helical(i)) return curve;
      let weight = 1;
      if (!helical(i - 1)) weight = Math.min(weight, smoothstep(t));
      if (!helical(i + 1)) weight = Math.min(weight, smoothstep(1 - t));
      return mix(curve, onHelix(i, t), weight);
    };

    // The face at each residue: in a helix, out from its axis, so the ribbon is broad
    // along the helix; elsewhere the bend there, which points across a strand's pleat.
    const faces: Vec3[] = [];
    for (let i = 0; i < n; i += 1) {
      const tangent = unit(sub(points[Math.min(i + 1, n - 1)], points[Math.max(i - 1, 0)]));
      const axis = axes[i];
      if (axis) {
        faces.push(across(sub(points[i], axis), tangent));
        continue;
      }
      let bend: Vec3 = i === 0 || i === n - 1 ? [0, 0, 0] : add(sub(points[i - 1], points[i]), sub(points[i + 1], points[i]));
      bend = sub(bend, scale(tangent, dot(bend, tangent)));
      faces.push(across(length(bend) > 1e-3 ? unit(bend) : (faces[i - 1] ?? anyPerpendicular(tangent)), tangent));
    }
    // Each face made to agree with the one before, so the section never turns a half turn
    // between two residues - the tube would pinch. Not within a helix, whose faces turn a
    // hundred degrees a residue with it: a helix is matched once, where it starts, and keeps
    // that sense to its end.
    let sense = 1;
    for (let i = 1; i < n; i += 1) {
      if (helical(i - 1)) {
        faces[i] = scale(faces[i], sense);
      } else if (axes[i]) {
        sense = dot(faces[i], faces[i - 1]) < 0 ? -1 : 1;
        faces[i] = scale(faces[i], sense);
      } else if (dot(faces[i], faces[i - 1]) < 0) {
        faces[i] = scale(faces[i], -1);
      }
    }

    const profileAt = (i: number, t: number): [number, number] => {
      // The last residue of a strand is the arrow: broad where it starts, to a point.
      if (kinds[i] === "E" && kinds[i + 1] !== "E") {
        const tip = PROFILE.C[0];
        return [ARROW + (tip - ARROW) * t, PROFILE.E[1]];
      }
      const w = bspline(t);
      const around = [Math.max(i - 1, 0), i, Math.min(i + 1, n - 1), Math.min(i + 2, n - 1)];
      let width = 0;
      let height = 0;
      around.forEach((at, k) => {
        const [pw, ph] = PROFILE[kinds[at]] ?? PROFILE.C;
        width += w[k] * pw;
        height += w[k] * ph;
      });
      return [width, height];
    };

    const first = this.positions.length / 3;
    const sides = this.detail.sides;
    let rings = 0;
    for (let i = 0; i < n - 1; i += 1) {
      const per = helical(i) ? Math.max(samples, this.detail.helixSamples) : samples;
      const steps = i === n - 2 ? per + 1 : per;
      for (let s = 0; s < steps; s += 1) {
        const t = s / per;
        const centre = pointAt(i, t);
        const tangent = unit(sub(pointAt(i, Math.min(t + 0.01, 1)), pointAt(i, Math.max(t - 0.01, 0))), unit(sub(points[i + 1], points[i])));
        const face = across(slerp(faces[i], faces[i + 1], t), tangent);
        const wide = unit(cross(tangent, face));
        const [w, h] = profileAt(i, t);
        const rgb = colour(t < 0.5 ? i : i + 1);
        for (let j = 0; j < sides; j += 1) {
          const a = (j / sides) * Math.PI * 2;
          const c = Math.cos(a);
          const sn = Math.sin(a);
          const p = add(centre, add(scale(wide, c * w), scale(face, sn * h)));
          const normal = unit(add(scale(wide, c / w), scale(face, sn / h)));
          this.positions.push(p[0], p[1], p[2]);
          this.normals.push(normal[0], normal[1], normal[2]);
          this.colours.push(rgb[0], rgb[1], rgb[2]);
        }
        rings += 1;
      }
    }
    for (let r = 0; r < rings - 1; r += 1) {
      for (let j = 0; j < sides; j += 1) {
        const a = first + r * sides + j;
        const b = first + r * sides + ((j + 1) % sides);
        const c = a + sides;
        const d = b + sides;
        this.indices.push(a, c, b, b, c, d);
      }
    }
  }

  finish(): RibbonMesh {
    return {
      positions: new Float32Array(this.positions),
      normals: new Float32Array(this.normals),
      colours: new Float32Array(this.colours),
      indices: new Uint32Array(this.indices),
    };
  }
}

/**
 * A molecule's ribbons from its trace (x, y, z a residue), secondary structure letters and
 * chain slots; `colour` gives a chain's colour by slot and kind.
 */
export function moleculeRibbon(
  trace: number[],
  ss: string,
  chain: number[],
  colour: (chain: number, kind: string) => Rgb,
  detail: RibbonDetail = FINE,
): RibbonMesh {
  const builder = new Builder(detail);
  let points: Vec3[] = [];
  let kinds: string[] = [];
  let slots: number[] = [];
  const flush = () => {
    builder.run(points, kinds, (i) => colour(slots[i], kinds[i]));
    points = [];
    kinds = [];
    slots = [];
  };
  for (let i = 0; i < ss.length; i += 1) {
    const p: Vec3 = [trace[i * 3], trace[i * 3 + 1], trace[i * 3 + 2]];
    const kind = ss[i];
    const last = points[points.length - 1];
    // A new run at a new chain or a gap: further apart than consecutive residues can be.
    const gap = kind === "N" ? 8 : 5.5;
    if (last && (chain[i] !== slots[slots.length - 1] || length(sub(p, last)) > gap)) flush();
    points.push(p);
    kinds.push(kind);
    slots.push(chain[i]);
  }
  flush();
  return builder.finish();
}

/**
 * One turn of B-DNA as `dnaTurn` in `cellStage.ts` beads it: the two backbones as tubes,
 * running a step past each end so copies laid end to end join, and the pairs as rungs.
 */
export function dnaRibbon(colours: [Rgb, Rgb, Rgb], detail: RibbonDetail = FINE): RibbonMesh {
  const builder = new Builder(detail);
  const strand = (offset: number, i: number): Vec3 => {
    const a = ((i * 36 + offset) * Math.PI) / 180;
    return [9 * Math.cos(a), 9 * Math.sin(a), -17 + 3.4 * (i + 0.5)];
  };
  for (const [offset, rgb] of [[0, colours[0]], [144, colours[1]]] as [number, Rgb][]) {
    const points: Vec3[] = [];
    for (let i = -1; i <= 10; i += 1) points.push(strand(offset, i));
    builder.run(points, points.map(() => "N"), () => rgb, detail.samples + 1);
  }
  for (let i = 0; i < 10; i += 1) {
    const a = strand(0, i);
    const b = strand(144, i);
    builder.run([a, add(a, scale(sub(b, a), 0.5)), b], ["R", "R", "R"], () => colours[2], 1);
  }
  return builder.finish();
}

/** Ten nucleotides of single-stranded RNA as `rnaStretch` beads them: a backbone and its bases. */
export function rnaRibbon(colours: [Rgb, Rgb], detail: RibbonDetail = FINE): RibbonMesh {
  const builder = new Builder(detail);
  const backbone = (i: number): Vec3 => {
    const a = (i * 33 * Math.PI) / 180;
    return [4.5 * Math.cos(a), 4.5 * Math.sin(a), -29.5 + 5.9 * (i + 0.5)];
  };
  const points: Vec3[] = [];
  for (let i = -1; i <= 10; i += 1) points.push(backbone(i));
  builder.run(points, points.map(() => "N"), () => colours[0], detail.samples + 1);
  for (let i = 0; i < 10; i += 1) {
    const a = (i * 33 * Math.PI) / 180;
    const base: Vec3 = [1.2 * Math.cos(a + 1.2), 1.2 * Math.sin(a + 1.2), backbone(i)[2]];
    builder.run([backbone(i), base], ["R", "R"], () => colours[1], 1);
  }
  return builder.finish();
}
