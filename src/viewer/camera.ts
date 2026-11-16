// Orbit camera.
//
// Orientation is kept as an explicit basis rather than Euler angles, so dragging never
// hits gimbal lock at the poles - the failure everyone has met in a viewer that rotates
// fine until you look straight down the Y axis and it starts to spin.

import {
  cross,
  dot,
  lookAt,
  normalize,
  perspective,
  rotateAbout,
  scale,
  subtract,
  type Mat4,
  type Vec3,
} from "./mat";

const FOV_Y = (40 * Math.PI) / 180;

export class OrbitCamera {
  target: Vec3 = [0, 0, 0];
  distance = 60;
  /** Bounding radius of what is being looked at, used to keep the depth range tight. */
  private sceneRadius = 30;
  /**
   * The middle of what `sceneRadius` is the radius of. The target starts there and a pan
   * moves it away, and the depth range has to follow the scene rather than the target.
   */
  private subject: Vec3 | null = null;
  private forward: Vec3 = [0, 0, 1];
  private up: Vec3 = [0, 1, 0];

  get eye(): Vec3 {
    return [
      this.target[0] + this.forward[0] * this.distance,
      this.target[1] + this.forward[1] * this.distance,
      this.target[2] + this.forward[2] * this.distance,
    ];
  }

  /** Bounding radius of the framed subject, for effects that must scale with it. */
  get radius(): number {
    return this.sceneRadius;
  }

  get right(): Vec3 {
    return normalize(cross(this.up, this.forward));
  }

  /** The camera basis, for callers that need to measure a scene in screen terms. */
  get basis(): { right: Vec3; up: Vec3; forward: Vec3 } {
    return { right: this.right, up: [...this.up] as Vec3, forward: [...this.forward] as Vec3 };
  }

  /** Half-extent tangents of the frustum at unit distance. */
  /** How tall the view is where the camera is looking, in the scene's units (angstroms). */
  viewHeight(): number {
    return 2 * this.distance * Math.tan(FOV_Y / 2);
  }

  fovTangents(aspect: number): { tanX: number; tanY: number } {
    const tanY = Math.tan(FOV_Y / 2);
    return { tanX: tanY * Math.max(aspect, 1e-3), tanY };
  }

  /**
   * Frames a scene whose extent the caller has measured.
   *
   * The camera cannot compute this itself: the tight answer depends on where the atoms
   * actually are, and every cheaper bound over-frames badly. A bounding sphere is set by
   * the single furthest atom, and a bounding box by corners a globular protein never
   * reaches - both leave the structure filling about half the window, which is the first
   * thing anyone notices about a viewer.
   */
  frame(center: Vec3, requiredDistance: number, sceneRadius: number): void {
    // One NaN coordinate in a structure reaches here as a NaN centre or distance, and a
    // NaN in the camera blanks the canvas for good - every later zoom and pan inherits it.
    // Such a request is ignored, or the part of it that can be read is kept.
    if (!center.every(Number.isFinite)) return;
    this.target = [...center] as Vec3;
    this.subject = [...center] as Vec3;
    this.sceneRadius = Number.isFinite(sceneRadius) ? Math.max(sceneRadius, 1) : this.sceneRadius;
    const wanted = Number.isFinite(requiredDistance) ? requiredDistance : this.sceneRadius * 2;
    this.distance = Math.max(wanted, this.sceneRadius * 0.05);
  }

  /** Says where the scene's middle is, when the camera is not being moved to it. */
  setSubject(center: Vec3): void {
    this.subject = [...center] as Vec3;
  }

  /** Screen-space drag to rotation, in radians per pixel. */
  rotate(deltaX: number, deltaY: number): void {
    const speed = 0.0075;
    const right = this.right;
    this.forward = normalize(rotateAbout(this.forward, this.up, -deltaX * speed));
    this.forward = normalize(rotateAbout(this.forward, right, -deltaY * speed));
    this.up = normalize(rotateAbout(this.up, right, -deltaY * speed));
    // Re-orthogonalise: repeated small rotations accumulate error that would slowly shear
    // the view.
    this.up = normalize(subtract(this.up, scale(this.forward, dot(this.up, this.forward))));
  }

  /** Pans in the camera plane, scaled so a drag moves the same screen distance at any zoom. */
  pan(deltaX: number, deltaY: number, viewportHeight: number): void {
    const worldPerPixel = (2 * this.distance * Math.tan(FOV_Y / 2)) / Math.max(viewportHeight, 1);
    const right = this.right;
    for (let axis = 0; axis < 3; axis += 1) {
      this.target[axis] += -deltaX * worldPerPixel * right[axis] + deltaY * worldPerPixel * this.up[axis];
    }
  }

  /** Multiplicative zoom, so each wheel notch feels the same regardless of scale. */
  zoom(factor: number): void {
    // A wheel event with no delta, or a pinch reporting nothing, must not zoom to NaN.
    if (!Number.isFinite(factor) || factor <= 0) return;
    this.distance = Math.min(
      Math.max(this.distance * factor, this.sceneRadius * 0.05),
      this.sceneRadius * 40,
    );
  }

  view(): Mat4 {
    return lookAt(this.eye, this.target, this.up);
  }

  projection(aspect: number): Mat4 {
    // The depth range is derived from the scene rather than fixed. Impostors write
    // gl_FragDepth, so a range hundreds of times larger than the structure would quantise
    // depth badly enough to make touching spheres flicker against each other.
    //
    // Measured from the scene's own middle, not from the orbit target. After a pan the
    // two are different places: pan to a ligand on the surface, turn the structure a
    // quarter of the way round, and its bulk is now nearer than the target by the length
    // of the pan - inside a near plane set as if the target were still the centre.
    const strayed = this.subject
      ? Math.hypot(
          this.target[0] - this.subject[0],
          this.target[1] - this.subject[1],
          this.target[2] - this.subject[2],
        )
      : 0;
    const near = Math.max(
      this.distance - this.sceneRadius * 1.5 - strayed,
      this.sceneRadius * 0.01,
    );
    const far = this.distance + this.sceneRadius * 2.5 + strayed;
    return perspective(FOV_Y, aspect, near, far);
  }

  /**
   * The camera as an offline renderer needs it.
   *
   * Blender is given eye, target and up rather than a matrix: it builds its own basis, and
   * handing it a matrix in our conventions would need the handedness translated at the
   * boundary - one more place for a mirrored render to come from.
   */
  /**
   * Puts the camera back where it was, from what `describe` returned.
   *
   * A world remembers where it was left, and coming back to a structure at a different
   * angle from the one you left it at is disorienting in a way a saved session should not
   * be.
   */
  restore(state: { eye: Vec3; center: Vec3; up: Vec3; radius: number }): void {
    const away: Vec3 = [
      state.eye[0] - state.center[0],
      state.eye[1] - state.center[1],
      state.eye[2] - state.center[2],
    ];
    const distance = Math.hypot(away[0], away[1], away[2]);
    if (!Number.isFinite(distance) || distance < 1e-6) return;
    const forward: Vec3 = [away[0] / distance, away[1] / distance, away[2] / distance];
    // The saved up is made square to the view direction, as `rotate` keeps it. One left
    // leaning, or lying along the view, would shear the picture or collapse the basis.
    let up: Vec3 = [state.up[0], state.up[1], state.up[2]];
    if (!up.every(Number.isFinite)) up = [0, 1, 0];
    up = subtract(up, scale(forward, dot(up, forward)));
    if (Math.hypot(up[0], up[1], up[2]) < 1e-6) {
      // Looking straight along the saved up: any square direction will do.
      const other: Vec3 = Math.abs(forward[1]) < 0.9 ? [0, 1, 0] : [1, 0, 0];
      up = subtract(other, scale(forward, dot(other, forward)));
    }
    this.target = [state.center[0], state.center[1], state.center[2]];
    this.distance = distance;
    this.forward = forward;
    this.up = normalize(up);
    if (Number.isFinite(state.radius)) this.sceneRadius = Math.max(state.radius, 1e-3);
  }

  describe(): { eye: [number, number, number]; center: [number, number, number]; up: [number, number, number]; radius: number } {
    const eye = this.eye;
    return {
      eye: [eye[0], eye[1], eye[2]],
      center: [this.target[0], this.target[1], this.target[2]],
      up: [this.up[0], this.up[1], this.up[2]],
      radius: this.sceneRadius,
    };
  }

  /** Depth-cue bounds in view space, for fog that fades the far side of the structure. */
  fogRange(): [number, number] {
    return [
      Math.max(this.distance - this.sceneRadius, 0.1),
      this.distance + this.sceneRadius * 1.2,
    ];
  }
}
