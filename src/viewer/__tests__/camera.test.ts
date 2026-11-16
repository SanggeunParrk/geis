import { describe, expect, it } from "vitest";
import { OrbitCamera } from "../camera";
import { cross, dot, identity, invert, lookAt, multiply, normalize, perspective, rotateAbout, type Vec3 } from "../mat";

const finite = (values: ArrayLike<number>) => Array.from(values).every(Number.isFinite);
const close = (a: ArrayLike<number>, b: ArrayLike<number>, digits = 5) =>
  Array.from(a).forEach((value, index) => expect(value).toBeCloseTo(b[index], digits));

describe("the matrices", () => {
  it("inverts a camera's view and projection back to the identity", () => {
    const view = lookAt([3, -4, 12], [1, 2, 0], [0, 1, 0]);
    const projection = perspective(0.7, 1.6, 0.5, 200);
    const both = multiply(projection, view);
    close(multiply(both, invert(both)), identity(), 4);
  });

  it("gives a usable matrix rather than NaN for a singular one or a degenerate look", () => {
    expect(invert(new Float32Array(16))).toEqual(identity());
    // Looking straight down the up vector, and from the target itself.
    expect(finite(lookAt([0, 10, 0], [0, 0, 0], [0, 1, 0]))).toBe(true);
    expect(finite(lookAt([1, 1, 1], [1, 1, 1], [0, 1, 0]))).toBe(true);
    expect(normalize([0, 0, 0])).toEqual([0, 0, 1]);
  });

  it("rotates about an axis without changing length", () => {
    const turned = rotateAbout([1, 0, 0], [0, 0, 1], Math.PI / 2);
    close(turned, [0, 1, 0]);
    expect(Math.hypot(...rotateAbout([1, 2, 3], normalize([1, 1, 0]), 1.3))).toBeCloseTo(Math.hypot(1, 2, 3));
  });
});

describe("the orbit camera", () => {
  it("frames what it is told and keeps the view finite", () => {
    const camera = new OrbitCamera();
    camera.frame([10, 0, 0], 80, 20);
    expect(camera.target).toEqual([10, 0, 0]);
    expect(camera.distance).toBe(80);
    expect(finite(camera.view())).toBe(true);
    expect(finite(camera.projection(1.5))).toBe(true);
  });

  it("ignores a NaN from a structure with a bad coordinate instead of blanking for good", () => {
    const camera = new OrbitCamera();
    camera.frame([0, 0, 0], 50, 10);
    camera.frame([Number.NaN, 0, 0], 50, 10);
    expect(camera.target).toEqual([0, 0, 0]);
    camera.frame([1, 1, 1], Number.NaN, Number.NaN);
    expect(Number.isFinite(camera.distance)).toBe(true);
    expect(Number.isFinite(camera.radius)).toBe(true);
    camera.zoom(Number.NaN);
    camera.zoom(0);
    camera.zoom(-2);
    expect(Number.isFinite(camera.distance)).toBe(true);
    expect(camera.distance).toBeGreaterThan(0);
    expect(finite(camera.view())).toBe(true);
    expect(finite(camera.projection(1))).toBe(true);
  });

  it("frames a single atom - a box of no size - at a distance greater than nothing", () => {
    const camera = new OrbitCamera();
    camera.frame([5, 5, 5], 0, 0);
    expect(camera.distance).toBeGreaterThan(0);
    expect(finite(camera.projection(1))).toBe(true);
  });

  it("keeps zoom within reach of the subject", () => {
    const camera = new OrbitCamera();
    camera.frame([0, 0, 0], 50, 10);
    for (let notch = 0; notch < 200; notch += 1) camera.zoom(0.5);
    expect(camera.distance).toBeCloseTo(0.5);
    for (let notch = 0; notch < 200; notch += 1) camera.zoom(2);
    expect(camera.distance).toBe(400);
  });

  it("pans in a zero-height viewport without dividing by zero", () => {
    const camera = new OrbitCamera();
    camera.pan(10, 10, 0);
    expect(camera.target.every(Number.isFinite)).toBe(true);
  });

  it("comes back from a saved state, the up made square to the view", () => {
    const camera = new OrbitCamera();
    camera.restore({ eye: [0, 0, 30], center: [0, 0, 0], up: [0, 2, 1], radius: 12 });
    const { up, forward } = camera.basis;
    expect(dot(up, forward)).toBeCloseTo(0);
    expect(Math.hypot(...up)).toBeCloseTo(1);
    expect(camera.distance).toBeCloseTo(30);
    expect(camera.radius).toBe(12);
  });

  it("survives a saved up that lies along the view, or is not a number", () => {
    for (const up of [[0, 0, 5], [0, 0, 0], [Number.NaN, 1, 0]] as Vec3[]) {
      const camera = new OrbitCamera();
      camera.restore({ eye: [0, 0, 30], center: [0, 0, 0], up, radius: Number.NaN });
      const { right, up: kept, forward } = camera.basis;
      expect(Math.hypot(...cross(kept, forward))).toBeCloseTo(1);
      expect(right.every(Number.isFinite)).toBe(true);
      expect(finite(camera.view())).toBe(true);
      expect(finite(camera.projection(1))).toBe(true);
    }
  });

  it("ignores a saved state whose eye is its centre", () => {
    const camera = new OrbitCamera();
    camera.frame([1, 2, 3], 40, 10);
    camera.restore({ eye: [1, 2, 3], center: [1, 2, 3], up: [0, 1, 0], radius: 10 });
    expect(camera.distance).toBe(40);
    expect(camera.target).toEqual([1, 2, 3]);
  });

  it("round-trips through describe", () => {
    const camera = new OrbitCamera();
    camera.frame([3, 1, 4], 45, 15);
    camera.rotate(120, -40);
    const saved = camera.describe();
    const again = new OrbitCamera();
    again.restore(saved);
    close(again.view(), camera.view(), 4);
  });
});
