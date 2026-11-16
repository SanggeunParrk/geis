// The viewport: a camera, three draw passes, and whatever geometry it was handed.
//
// Deliberately thin. It knows nothing about proteins, representations, or colour
// schemes - those are decided in Rust, which builds the geometry the Blender renderer
// gets too. What is left here is the part that is genuinely about drawing: framing,
// interaction, and three shader programs.

import type { SceneStyle } from "../ipc";
import { capabilities, createContext, type GpuCapabilities } from "./gl";
import { cross, invert, normalize, subtract, type Vec3 } from "./mat";
import { PostPass } from "./post";
import { decodeScene, type Scene } from "./scene";
import {
  decodePick,
  mergeMasks,
  srgbToLinear,
  uploadSelection,
  SELECTION_TEXTURE_UNIT,
  type Picked,
  type ShadingUniforms,
  NO_ROW,
  type Row,
} from "./shading";
import {
  createRenderTarget,
  createSingleTarget,
  disposeRenderTarget,
  disposeSingleTarget,
  type RenderTarget,
  type SingleTarget,
} from "./target";
import { OrbitCamera } from "./camera";

/**
 * The colour a selection is outlined in, in linear RGB.
 *
 * One warm colour that no colour scheme uses, which is the whole requirement: a highlight
 * that could be mistaken for the structure's own colour is not a highlight.
 */
const SELECTION_COLOR: [number, number, number] = [1.0, 0.45, 0.06];
/** The hover outline: the same hue, much paler, so it reads as "this one, if you click". */
const HOVER_COLOR: [number, number, number] = [1.0, 0.78, 0.55];

/** How far the structure recedes under the graph view: mostly, not wholly. */
const GHOST_FADE = 0.62;

/**
 * The graph view's geometry, in the same layouts the structure's passes take.
 *
 * `nodeIds` are residue ids in the scene's id space, so a sphere is picked, hovered and
 * outlined by the same masks a ribbon's residues are.
 */
export interface GraphView {
  /** x, y, z, radius per component. */
  nodes: Float32Array;
  /** r, g, b per component. */
  nodeColors: Float32Array;
  nodeIds: Uint32Array;
  /** What is written on each component's sphere, packed by `packLabel`; 0 for nothing. */
  nodeLabels: Uint32Array;
  /** x1, y1, z1, x2, y2, z2, radius per bond. */
  edges: Float32Array;
  /** r1, g1, b1, r2, g2, b2 per bond. */
  edgeColors: Float32Array;
  /** What each bond is, in the order of `edges`. */
  edgeInfo: GraphEdge[];
}

/** One bond of the graph view: what kind it is and which two components it joins. */
export interface GraphEdge {
  /** `backbone`, `disulfide`, `metal` or `covalent`. */
  kind: string;
  /** Which structure on screen, and the two residues in it, by structure index. */
  structure: number;
  residues: [number, number];
  /** The two ends as they are called: `LEU 105`. */
  names: [string, string];
  /** The chains the two ends are in. */
  chains: [string, string];
  /** Centre to centre, in angstroms. */
  length: number;
  /** In words: `peptide bond`, `disulfide bridge`. */
  what: string;
  /** For a contact that was found: the closest approach of the atoms that make it. */
  measured?: number;
}

/** The keys held during a click, so a click can add to or toggle a selection. */
export interface PickModifiers {
  shift: boolean;
  toggle: boolean;
}

export class Viewer {
  readonly capabilities: GpuCapabilities;
  private gl: WebGL2RenderingContext;
  private camera = new OrbitCamera();
  /** Whether the background is left out, so the page shows through. */
  private transparent = false;
  /** Which part of the scene is turning, if any. */
  private rowState: Row = { ...NO_ROW };
  /** Whether the pointer drives the camera. The shelf of worlds says no. */
  private interactive = true;
  private spheres: import("./spheres").SpherePass | null = null;
  private cylinders: import("./cylinders").CylinderPass | null = null;
  private mesh: import("./mesh").MeshPass | null = null;
  private post: PostPass;
  private gbuffer: RenderTarget | null = null;
  private occlusion: SingleTarget | null = null;

  private scene: Scene | null = null;
  private style: SceneStyle | null = null;
  /**
   * The graph view: the structure as its components, one sphere each, joined by the bonds
   * between them, drawn over the structure itself - which recedes to a ghost, the way a
   * mesh does under X-ray, so the graph is seen through it and still seen in place.
   */
  private graphOn = false;
  private graphNodes: import("./spheres").SpherePass | null = null;
  private graphEdges: import("./cylinders").CylinderPass | null = null;
  private graphData: GraphView | null = null;
  /**
   * The selection, as masks keyed by atom and by residue.
   *
   * Two, because the spheres and sticks know which atom they are and a ribbon's vertices
   * know only which residue they belong to. The width doubles as the "is anything
   * selected" flag: zero means the shaders skip the lookup entirely.
   */
  private atomMask: WebGLTexture | null = null;
  private residueMask: WebGLTexture | null = null;
  private atomMaskWidth = 0;
  private residueMaskWidth = 0;
  /** The two masks as given, kept so either can change without the other being resent. */
  private selectedAtoms: Uint8Array | null = null;
  private selectedResidues: Uint8Array | null = null;
  private hoveredAtoms: Uint8Array | null = null;
  private hoveredResidues: Uint8Array | null = null;
  private dirty = true;
  private running = false;
  private animation: ((time: number) => boolean) | null = null;
  private disposed = false;
  private pixelRatioCap: number;

  constructor(private canvas: HTMLCanvasElement) {
    this.capabilities = capabilities();
    this.gl = createContext(canvas);
    // A software rasteriser at 2x device pixel ratio is four times the work for no visible
    // gain on a display it cannot drive anyway.
    this.pixelRatioCap = this.capabilities.software ? 1 : 2;

    const gl = this.gl;
    gl.enable(gl.DEPTH_TEST);
    gl.depthFunc(gl.LEQUAL);
    gl.disable(gl.CULL_FACE); // cylinder and ribbon interiors are both visible

    this.post = new PostPass(gl);
    this.attachInput();

    // A context can be taken away - the GPU switched, the driver reset, too many contexts
    // alive at once. Everything made in it is gone with it, and rebuilding each pass,
    // target and texture in place is a second copy of this constructor that would only
    // ever run on the worst day. So the viewer says it has died and its owner makes a new
    // one, which is the path that is exercised every time the app starts.
    canvas.addEventListener("webglcontextlost", (event) => {
      event.preventDefault();
      // `dispose` loses the context on purpose; that is not news to anyone.
      if (this.disposed) return;
      this.onContextLost?.();
    });
    this.followPixelRatio();
  }

  /**
   * Redraws when the display's pixel ratio changes - the window dragged from a retina
   * screen to an external one. The canvas is sized from the ratio on every draw, but a
   * move between displays is not a resize, so nothing asked for a draw and the picture
   * stayed at the old screen's sharpness until something else changed.
   */
  private followPixelRatio(): void {
    if (this.disposed || typeof matchMedia !== "function") return;
    const query = matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    query.addEventListener(
      "change",
      () => {
        this.requestFrame();
        this.followPixelRatio();
      },
      { once: true },
    );
  }

  /** Called when the GPU context is lost. The viewer is dead; make another. */
  onContextLost: (() => void) | null = null;

  /** Passes are created on first use, so a viewer with no structure costs two shaders. */
  private ensurePasses(): Promise<void> {
    // One creation, however many callers arrive while it is under way: two scenes set
    // back to back would otherwise each find no passes and each make a set, leaking one.
    this.passes ??= (async () => {
      const [{ SpherePass }, { CylinderPass }, { MeshPass }] = await Promise.all([
        import("./spheres"),
        import("./cylinders"),
        import("./mesh"),
      ]);
      this.spheres = new SpherePass(this.gl);
      this.cylinders = new CylinderPass(this.gl);
      this.mesh = new MeshPass(this.gl);
      this.graphNodes = new SpherePass(this.gl);
      this.graphNodes.pickKind = 2;
      this.graphEdges = new CylinderPass(this.gl);
      this.graphEdges.pickKind = 3;
      this.contactPass = new CylinderPass(this.gl);
      this.uploadContacts();
      if (this.graphData) this.uploadGraph(this.graphData);
    })();
    return this.passes;
  }
  private passes: Promise<void> | null = null;

  /**
   * Uploads a geometry payload from the core.
   *
   * `reframe` is false when only the colours or the visible subset changed, so switching
   * colour scheme does not throw away a camera the user has positioned.
   */
  /**
   * What is highlighted: masks keyed by atom and by residue, or nulls for nothing.
   *
   * Masks rather than a list of ids, because the shaders look them up per vertex, and
   * textures rather than attributes because the selection changes on a click while the
   * geometry does not - re-uploading a mask is kilobytes, re-uploading a spacefill scene
   * is tens of megabytes.
   */
  setSelection(atoms: Uint8Array | null, residues: Uint8Array | null): void {
    this.selectedAtoms = atoms;
    this.selectedResidues = residues;
    this.uploadMasks();
  }

  /** What the pointer is over, outlined more faintly than the selection. */
  setHover(atoms: Uint8Array | null, residues: Uint8Array | null): void {
    this.hoveredAtoms = atoms;
    this.hoveredResidues = residues;
    this.uploadMasks();
  }

  private uploadMasks(): void {
    const gl = this.gl;
    this.atomMask ??= gl.createTexture();
    this.residueMask ??= gl.createTexture();
    const atomCount = this.scene?.header.atom_count ?? 0;
    const residueCount = this.scene?.header.residue_count ?? 0;
    const atoms = mergeMasks(atomCount, this.selectedAtoms, this.hoveredAtoms);
    const residues = mergeMasks(residueCount, this.selectedResidues, this.hoveredResidues);
    if (this.atomMask) this.atomMaskWidth = uploadSelection(gl, this.atomMask, atoms);
    if (this.residueMask) {
      this.residueMaskWidth = uploadSelection(gl, this.residueMask, residues);
    }
    this.requestFrame();
  }

  /** Binds the mask a pass reads. Both share one texture unit; only one is bound at a time. */
  private bindSelection(mask: WebGLTexture | null): void {
    const gl = this.gl;
    gl.activeTexture(gl.TEXTURE0 + SELECTION_TEXTURE_UNIT);
    gl.bindTexture(gl.TEXTURE_2D, mask);
  }

  /** Turns the graph view on or off. What it shows is whatever `setGraph` last gave. */
  setGraphMode(on: boolean): void {
    this.graphOn = on;
    this.requestFrame();
  }

  get graphMode(): boolean {
    return this.graphOn;
  }

  /** Dashes for contacts that are not bonds - found, not part of the structure's geometry. */
  private contactPass: import("./cylinders").CylinderPass | null = null;
  private contactDashes: Float32Array = new Float32Array(0);
  private contactColor: [number, number, number] = [1, 0.82, 0.1];
  private contactColors: Float32Array | null = null;

  /**
   * Draws dashed lines: `segments` is x1, y1, z1, x2, y2, z2 per contact, in the scene's
   * coordinates. An empty list takes them away.
   */
  setContacts(segments: Float32Array, color: [number, number, number] | Float32Array = [1, 0.82, 0.1]): void {
    // One colour for all of them, or r, g, b per contact when several kinds are shown at once.
    const each = color instanceof Float32Array ? color : null;
    const dashColors: number[] = [];
    const dashes: number[] = [];
    // Five dashes to a contact, with gaps as long as the dashes: read as a dashed line at
    // any length a contact can have.
    const PARTS = 9;
    for (let at = 0; at + 5 < segments.length; at += 6) {
      for (let part = 0; part < PARTS; part += 2) {
        const a = part / PARTS;
        const b = (part + 1) / PARTS;
        for (const t of [a, b]) {
          for (let axis = 0; axis < 3; axis += 1) dashes.push(segments[at + axis] + (segments[at + 3 + axis] - segments[at + axis]) * t);
        }
        dashes.push(0.07);
        if (each) {
          const from = (at / 6) * 3;
          // Both ends of the dash.
          for (let end = 0; end < 2; end += 1) dashColors.push(each[from], each[from + 1], each[from + 2]);
        }
      }
    }
    this.contactDashes = new Float32Array(dashes);
    this.contactColors = each ? new Float32Array(dashColors) : null;
    if (!each) this.contactColor = color as [number, number, number];
    this.uploadContacts();
    this.requestFrame();
  }

  private uploadContacts(): void {
    if (!this.contactPass) return;
    const count = this.contactDashes.length / 7;
    this.contactPass.setInstances(this.contactDashes);
    let colors = this.contactColors;
    if (!colors || colors.length !== count * 6) {
      colors = new Float32Array(count * 6);
      for (let at = 0; at < colors.length; at += 3) colors.set(this.contactColor, at);
    }
    this.contactPass.setColors(colors);
    this.contactPass.setAtoms(new Uint32Array(count * 2));
  }

  private graphPick: "nodes" | "edges" = "nodes";

  /** Whether a click in the graph view lands on components or on the bonds between them. */
  setGraphPick(what: "nodes" | "edges"): void {
    this.graphPick = what;
  }

  /** The components and the bonds between them, or null for none. */
  setGraph(graph: GraphView | null): void {
    this.graphData = graph;
    if (this.graphNodes) this.uploadGraph(graph);
    this.requestFrame();
  }

  private uploadGraph(graph: GraphView | null): void {
    const none = new Float32Array(0);
    this.graphNodes!.setInstances(graph?.nodes ?? none);
    this.graphNodes!.setColors(graph?.nodeColors ?? none);
    this.graphNodes!.setAtoms(graph?.nodeIds ?? new Uint32Array(0));
    this.graphNodes!.setLabels(graph?.nodeLabels ?? new Uint32Array(0));
    this.graphEdges!.setInstances(graph?.edges ?? none);
    // Each bond is its own id at both ends, so a pick anywhere along it names the bond.
    const count = (graph?.edges.length ?? 0) / 7;
    const ids = new Uint32Array(count * 2);
    for (let edge = 0; edge < count; edge += 1) ids[edge * 2] = ids[edge * 2 + 1] = edge;
    this.graphEdges!.setAtoms(ids);
    this.edgeHover = null;
    this.edgeSelected = null;
    this.paintEdges();
  }

  private edgeHover: number | null = null;
  private edgeSelected: number | null = null;

  /**
   * Which bonds of the graph are pointed at and picked. A bond has no outline to light -
   * the selection mask is keyed by atom and residue - so it is recoloured instead, in the
   * same two colours the outline uses.
   */
  setGraphEdges(hover: number | null, selected: number | null): void {
    if (hover === this.edgeHover && selected === this.edgeSelected) return;
    this.edgeHover = hover;
    this.edgeSelected = selected;
    this.paintEdges();
    this.requestFrame();
  }

  private paintEdges(): void {
    const base = this.graphData?.edgeColors;
    if (!this.graphEdges || !base) {
      this.graphEdges?.setColors(new Float32Array(0));
      return;
    }
    const colors = new Float32Array(base);
    const paint = (edge: number | null, color: [number, number, number]) => {
      if (edge === null || edge * 6 + 5 >= colors.length) return;
      for (let end = 0; end < 2; end += 1) colors.set(color, edge * 6 + end * 3);
    };
    paint(this.edgeHover, HOVER_COLOR);
    paint(this.edgeSelected, SELECTION_COLOR);
    this.graphEdges.setColors(colors);
  }

  /** The lighting rig, background and material. Redraws on the next frame. */
  setStyle(style: SceneStyle): void {
    this.style = style;
    this.requestFrame();
  }

  /**
   * Returns whether the scene's ids were laid out afresh - in which case the selection
   * and hover masks were dropped, and whoever owns the selection should lay it again.
   */
  async setScene(payload: ArrayBuffer, reframe: boolean): Promise<boolean> {
    await this.ensurePasses();
    const scene = decodeScene(payload);
    // Masks are keyed by id, and ids are laid out by the scene: a molecule added, removed
    // or reordered moves every later one's offset. A mask made for the old layout would
    // outline whatever now sits at those numbers, so it goes, and whoever owns the
    // selection lays it again.
    const layout = (of: Scene | null) =>
      of
        ? `${of.header.atom_count}/${of.header.residue_count}/` +
          (of.header.parts ?? []).map((part) => `${part.atom_offset}:${part.residue_offset}`).join(",")
        : "";
    const relaid = layout(this.scene) !== layout(scene);
    this.scene = scene;
    if (relaid) {
      this.selectedAtoms = null;
      this.selectedResidues = null;
      this.hoveredAtoms = null;
      this.hoveredResidues = null;
      this.uploadMasks();
    }

    this.spheres!.setInstances(scene.sphereInstances);
    this.spheres!.setColors(scene.sphereColors);
    this.spheres!.setAtoms(scene.sphereAtoms);
    this.cylinders!.setInstances(scene.cylinderInstances);
    this.cylinders!.setColors(scene.cylinderColors);
    this.cylinders!.setAtoms(scene.cylinderAtoms);
    if (scene.header.triangle_count > 0) {
      this.mesh!.setMesh({
        positions: scene.meshPositions,
        normals: scene.meshNormals,
        colors: scene.meshColors,
        indices: scene.meshIndices,
        residues: scene.meshVertexResidue,
      });
    } else {
      this.mesh!.clear();
    }

    // Where the scene is, for the depth range - whether or not the camera moves to it.
    const { bounds_min: low, bounds_max: high } = scene.header;
    this.camera.setSubject([
      (low[0] + high[0]) / 2,
      (low[1] + high[1]) / 2,
      (low[2] + high[2]) / 2,
    ]);
    if (reframe) this.resetView();
    this.requestFrame();
    return relaid;
  }

  /**
   * Takes everything off the stage. A graph that draws nothing - its last chain unwired,
   * a world with no molecule yet - has to be able to say so; with no way to, the previous
   * molecule simply stayed on screen with nothing behind it.
   */
  clearScene(): void {
    this.scene = null;
    const none = new Float32Array(0);
    this.spheres?.setInstances(none);
    this.cylinders?.setInstances(none);
    this.mesh?.clear();
    this.selectedAtoms = null;
    this.selectedResidues = null;
    this.hoveredAtoms = null;
    this.hoveredResidues = null;
    this.uploadMasks();
  }

  get currentScene(): Scene | null {
    return this.scene;
  }

  /** The camera, for an offline render of exactly what is on screen. */
  /**
   * The camera, plus the shape of the canvas it is framed in.
   *
   * The aspect belongs here rather than in `Camera` because the camera does not own the
   * canvas - and a renderer given only a vertical field of view has no way to reproduce
   * this framing at a different aspect without cropping it.
   */
  describeCamera() {
    return { ...this.camera.describe(), view_aspect: this.aspect() };
  }

  /**
   * Frames the geometry exactly.
   *
   * Measured over the drawn surface - sphere centres plus their radii, and every ribbon
   * vertex - rather than over a bounding volume. A bounding sphere is set by the single
   * furthest atom and a bounding box by corners a globular protein never reaches; both
   * leave the structure filling about half the window, which is the first thing anyone
   * notices about a viewer.
   */
  resetView(): void {
    const scene = this.scene;
    if (!scene) return;

    const { bounds_min: min, bounds_max: max } = scene.header;
    const center: Vec3 = [
      (min[0] + max[0]) / 2,
      (min[1] + max[1]) / 2,
      (min[2] + max[2]) / 2,
    ];

    const { right, up, forward } = this.camera.basis;
    const { tanX, tanY } = this.camera.fovTangents(this.aspect());
    let required = 0;
    let sceneRadius = 1;

    const consider = (x: number, y: number, z: number, radius: number) => {
      const rx = x - center[0];
      const ry = y - center[1];
      const rz = z - center[2];
      const screenX = Math.abs(rx * right[0] + ry * right[1] + rz * right[2]) + radius;
      const screenY = Math.abs(rx * up[0] + ry * up[1] + rz * up[2]) + radius;
      const depth = rx * forward[0] + ry * forward[1] + rz * forward[2] + radius;
      required = Math.max(required, depth + Math.max(screenX / tanX, screenY / tanY));
      sceneRadius = Math.max(sceneRadius, Math.hypot(rx, ry, rz) + radius);
    };

    for (let sphere = 0; sphere < scene.header.sphere_count; sphere += 1) {
      const base = sphere * 4;
      consider(
        scene.sphereInstances[base],
        scene.sphereInstances[base + 1],
        scene.sphereInstances[base + 2],
        scene.sphereInstances[base + 3],
      );
    }
    for (let vertex = 0; vertex < scene.header.vertex_count; vertex += 1) {
      consider(
        scene.meshPositions[vertex * 3],
        scene.meshPositions[vertex * 3 + 1],
        scene.meshPositions[vertex * 3 + 2],
        0,
      );
    }
    if (required === 0) return;

    // A few percent of air so nothing touches the edge of the window.
    this.camera.frame(center, required * 1.04, sceneRadius);
    this.requestFrame();
  }

  /**
   * Draws onto whatever is behind the canvas instead of onto a colour.
   *
   * For the shelf of worlds, where a preview with a white box around it would look like a
   * photograph of the app rather than the thing itself.
   */
  setTransparent(transparent: boolean): void {
    this.transparent = transparent;
    this.requestFrame();
  }

  /** Puts the camera back where a world left it. */
  setCamera(state: { eye: [number, number, number]; center: [number, number, number]; up: [number, number, number]; radius: number }): void {
    this.camera.restore(state);
    this.requestFrame();
  }

  /** Points the camera at a place in the scene, without changing which way it faces. */
  lookAt(center: [number, number, number], distance: number, radius: number): void {
    this.camera.frame(center, distance, radius);
    this.requestFrame();
  }

  /**
   * Whether the pointer drives the camera at all.
   *
   * The shelf of worlds is a picture to look at rather than a scene to fly through: a drag
   * there should do nothing, and the wheel belongs to the shelf for moving along the row.
   */
  setInteractive(interactive: boolean): void {
    this.interactive = interactive;
    this.canvas.style.cursor = interactive ? "grab" : "default";
  }

  /**
   * Stands the scene in a row of worlds a fixed distance apart, and poses it.
   *
   * `focus` is where along that row the camera is and `shrink` how small a world a whole
   * step away is drawn: the one being looked at is full size and its neighbours smaller,
   * the way a character select keeps the chosen one in front. Continuous in `focus`, so a
   * world grows as you travel to it rather than popping when you arrive. `turns` is how far
   * each world has turned, one angle per place in the row - a turn each rather than one
   * shared, so the world you leave stops where it was instead of snapping upright.
   *
   * A pitch of zero, the default, leaves every part where and how it was built.
   */
  setRow(pitch: number, focus: number, shrink: number, turns: number[]): void {
    this.rowState = { pitch, focus, shrink, turns };
    this.requestFrame();
  }

  private aspect(): number {
    if (this.fixedSize) return this.fixedSize.width / this.fixedSize.height;
    return Math.max(this.canvas.clientWidth, 1) / Math.max(this.canvas.clientHeight, 1);
  }

  private resizeToDisplay(): boolean {
    if (this.fixedSize) {
      const { width, height } = this.fixedSize;
      if (this.canvas.width === width && this.canvas.height === height) return false;
      this.canvas.width = width;
      this.canvas.height = height;
      return true;
    }
    const ratio = Math.min(window.devicePixelRatio || 1, this.pixelRatioCap);
    const width = Math.max(1, Math.round(this.canvas.clientWidth * ratio));
    const height = Math.max(1, Math.round(this.canvas.clientHeight * ratio));
    if (this.canvas.width === width && this.canvas.height === height) return false;
    this.canvas.width = width;
    this.canvas.height = height;
    return true;
  }

  requestFrame(): void {
    this.dirty = true;
    if (this.running || this.disposed) return;
    this.running = true;
    requestAnimationFrame((time) => {
      try {
        // Update the pose immediately before drawing it. A separate animation rAF that
        // requests another rAF renders the previous pose and adds a presentation interval.
        // Keep `running` set while updating so setters coalesce into this very draw.
        if (!this.disposed && this.animation && !this.animation(time)) this.animation = null;
        // Redrawing only when something changed keeps an idle viewer off the GPU entirely,
        // which matters on a laptop and matters more on a software renderer.
        if (this.dirty && !this.disposed) {
          this.dirty = false;
          this.draw();
        }
      } finally {
        // Whatever the draw did. A frame that throws - a lost context, a target that
        // could not be made - must not leave the viewer believing a frame is still on
        // its way, or every later request is ignored and the canvas never draws again.
        this.running = false;
      }
      if (!this.disposed && this.animation) this.requestFrame();
    });
  }

  /** Animate and draw in one display callback. Return false when the screen is detached. */
  animate(update: (time: number) => boolean): void {
    this.animation = update;
    this.requestFrame();
  }

  private shadingUniforms(): ShadingUniforms {
    const style = this.style;
    return {
      lights: style?.lights ?? [],
      ambient: style?.ambient ?? 0.55,
      background: style?.background ?? [1, 1, 1],
      roughness: style?.material.roughness ?? 0.28,
      specular: style?.material.specular ?? 0.5,
    };
  }

  private ensureTargets(width: number, height: number): void {
    const gl = this.gl;
    if (this.gbuffer && this.gbuffer.width === width && this.gbuffer.height === height) {
      return;
    }
    if (this.gbuffer) disposeRenderTarget(gl, this.gbuffer);
    if (this.occlusion) disposeSingleTarget(gl, this.occlusion);
    this.gbuffer = createRenderTarget(gl, width, height);
    // Occlusion at half resolution: it is low-frequency, the composite blurs it anyway,
    // and it is the most expensive pass here.
    this.occlusion = createSingleTarget(
      gl,
      Math.max(1, width >> 1),
      Math.max(1, height >> 1),
    );
  }

  /**
   * Geometry into a G-buffer, then occlusion, then composite.
   *
   * Drawing straight to the screen is simpler and cannot do any of what makes this look
   * like a render: occlusion needs the depth buffer of the whole scene, and edges need its
   * normals. Both are only available once geometry has been captured rather than shown.
   */
  private draw(): void {
    const gl = this.gl;
    if (this.resizeToDisplay()) this.dirty = true;
    const width = this.canvas.width;
    const height = this.canvas.height;
    this.ensureTargets(width, height);
    const gbuffer = this.gbuffer!;
    const occlusion = this.occlusion!;

    const background = (this.style?.background ?? [1, 1, 1]).map(srgbToLinear) as [
      number,
      number,
      number,
    ];

    // --- geometry -------------------------------------------------------------
    gl.bindFramebuffer(gl.FRAMEBUFFER, gbuffer.framebuffer);
    gl.viewport(0, 0, width, height);
    // Clearing colour to the background means a fragment that is discarded leaves the
    // right value behind for the antialiasing taps to blend with.
    //
    // Alpha is cleared to zero, and that matters for a reason that is not about
    // transparency: the normal target's alpha carries the selection mask, so the
    // background has to read as *unselected*. Cleared to one, a molecule with every chain
    // selected had no boundary anywhere and got no outline at all.
    gl.clearBufferfv(gl.COLOR, 0, [background[0], background[1], background[2], 1]);
    // The same values the single clear used to leave in the normal target, with alpha
    // zeroed: the occlusion and outline passes read those channels, and clearing them to
    // black changed the halo around the silhouette.
    gl.clearBufferfv(gl.COLOR, 1, [background[0], background[1], background[2], 0]);
    gl.clearDepth(1);
    gl.clear(gl.DEPTH_BUFFER_BIT);

    if (this.scene) {
      const view = this.camera.view();
      const projection = this.camera.projection(this.aspect());
      const shading = this.shadingUniforms();

      // Under the graph the structure is a ghost, and a ghost is not outlined: what is
      // selected is shown on the graph's own spheres.
      const ghost = this.graphOn;
      this.bindSelection(this.residueMask);
      this.mesh?.draw(view, projection, shading, ghost ? 0 : this.residueMaskWidth, false, this.rowState);
      this.bindSelection(this.atomMask);
      this.cylinders?.draw(view, projection, shading, ghost ? 0 : this.atomMaskWidth, false, this.rowState);
      this.spheres?.draw(view, projection, shading, ghost ? 0 : this.atomMaskWidth, false, this.rowState);
      // Contacts, over the structure and under its outline: no mask, they are not selected.
      this.bindSelection(null);
      this.contactPass?.draw(view, projection, shading, 0, false, this.rowState);

      // --- occlusion ----------------------------------------------------------
      if (this.style?.ambient_occlusion !== false) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, occlusion.framebuffer);
        gl.viewport(0, 0, occlusion.width, occlusion.height);
        gl.disable(gl.DEPTH_TEST);
        this.post.drawOcclusion({
          depth: gbuffer.depth,
          normal: gbuffer.normal,
          projection,
          inverseProjection: invert(projection),
          // Scaled to the subject, and generously: the gaps that matter in a structure
          // are the ones between neighbouring strands and around a bound ligand, which
          // are five to ten angstroms across. A radius of one angstrom - which is what a
          // graphics-shaped guess produces - only finds the seams between touching atoms
          // and reads as no occlusion at all.
          radius: Math.min(Math.max(this.camera.radius * 0.2, 2.0), 25.0),
        });
        gl.enable(gl.DEPTH_TEST);
      }
    }

    // --- composite ------------------------------------------------------------
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, width, height);
    gl.disable(gl.DEPTH_TEST);
    this.post.drawComposite({
      color: gbuffer.color,
      normal: gbuffer.normal,
      depth: gbuffer.depth,
      occlusion: occlusion.texture,
      width,
      height,
      background,
      occlusionStrength: this.style?.ambient_occlusion === false ? 0 : 0.75,
      outlineStrength: this.style?.outline ? 1.0 : 0.0,
      antialias: 1.0,
      selectionColor: SELECTION_COLOR,
      hoverColor: HOVER_COLOR,
      // A line about two pixels wide, and nothing at all when nothing is selected.
      selectionWidth: this.atomMaskWidth > 0 || this.residueMaskWidth > 0 ? 2.0 : 0.0,
      backgroundAlpha: this.transparent ? 0 : 1,
      fade: this.graphOn ? GHOST_FADE : 0,
      pixelScale: this.pixelScale,
    });
    gl.enable(gl.DEPTH_TEST);

    if (this.graphOn && this.scene && this.graphData) this.drawGraph(width, height, background);
    gl.flush();
  }

  /**
   * The graph, over the picture already on screen.
   *
   * A second trip through the same G-buffer rather than more geometry in the first: the
   * graph has to be seen through the structure, so it cannot share a depth buffer with
   * it, and it wants the same edges and selection outline the composite already draws.
   * Composited with a transparent background and blended, so only the graph lands.
   */
  private drawGraph(width: number, height: number, background: [number, number, number]): void {
    const gl = this.gl;
    const gbuffer = this.gbuffer!;
    const view = this.camera.view();
    const projection = this.camera.projection(this.aspect());
    const shading = this.shadingUniforms();

    gl.bindFramebuffer(gl.FRAMEBUFFER, gbuffer.framebuffer);
    gl.viewport(0, 0, width, height);
    gl.clearBufferfv(gl.COLOR, 0, [background[0], background[1], background[2], 1]);
    gl.clearBufferfv(gl.COLOR, 1, [background[0], background[1], background[2], 0]);
    gl.clearDepth(1);
    gl.clear(gl.DEPTH_BUFFER_BIT);
    this.bindSelection(null);
    this.graphEdges?.draw(view, projection, shading, 0, false, this.rowState);
    this.bindSelection(this.residueMask);
    this.graphNodes?.draw(view, projection, shading, this.residueMaskWidth, false, this.rowState);

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, width, height);
    gl.disable(gl.DEPTH_TEST);
    gl.enable(gl.BLEND);
    // The composite writes premultiplied colour where it writes the background.
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    this.post.drawComposite({
      color: gbuffer.color,
      normal: gbuffer.normal,
      depth: gbuffer.depth,
      // Unused at strength zero, but a sampler has to be bound to something.
      occlusion: this.occlusion!.texture,
      width,
      height,
      background,
      occlusionStrength: 0,
      outlineStrength: 1.0,
      antialias: 1.0,
      selectionColor: SELECTION_COLOR,
      hoverColor: HOVER_COLOR,
      selectionWidth: this.residueMaskWidth > 0 ? 2.0 : 0.0,
      backgroundAlpha: 0,
    });
    gl.disable(gl.BLEND);
    gl.enable(gl.DEPTH_TEST);
  }

  /**
   * Forces a draw and reads the framebuffer back, reporting how much of it the structure
   * covers.
   *
   * This is the answer to "nothing is showing": a shader that fails to compile, an
   * attribute that never got bound, or a camera framed on empty space all produce a
   * plausible-looking canvas with nothing drawn on it, and none of them raise an error.
   */
  renderStats(): { coverage: number; pixels: number; distinctColors: number } {
    this.dirty = true;
    this.draw();
    const gl = this.gl;
    const width = this.canvas.width;
    const height = this.canvas.height;
    const pixels = new Uint8Array(width * height * 4);
    gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);

    // The background is a flat colour now, so anything that differs from it by more than
    // dithering noise was drawn by a pass.
    const expected = (this.style?.background ?? [1, 1, 1]).map((channel) => channel * 255);
    const colors = new Set<number>();
    let drawn = 0;
    for (let index = 0; index < pixels.length; index += 4) {
      let painted = false;
      for (let channel = 0; channel < 3; channel += 1) {
        if (Math.abs(pixels[index + channel] - expected[channel]) > 6) painted = true;
      }
      if (painted) {
        drawn += 1;
        colors.add((pixels[index] << 16) | (pixels[index + 1] << 8) | pixels[index + 2]);
      }
    }
    return {
      coverage: drawn / Math.max(width * height, 1),
      pixels: drawn,
      distinctColors: colors.size,
    };
  }

  /**
   * The colour of one pixel on screen, as #rrggbb.
   *
   * This is the eyedropper. The browser's `EyeDropper` API is absent in this webview, so
   * sampling the viewport is what is available - and it is the case that matters, taking a
   * colour off the structure itself.
   */
  samplePixel(client: { x: number; y: number }): string | null {
    const rect = this.canvas.getBoundingClientRect();
    if (
      client.x < rect.left ||
      client.x > rect.right ||
      client.y < rect.top ||
      client.y > rect.bottom
    ) {
      return null;
    }
    // The framebuffer's origin is bottom-left, the page's is top-left.
    const x = Math.min(
      this.canvas.width - 1,
      Math.floor(((client.x - rect.left) / rect.width) * this.canvas.width),
    );
    const y = Math.max(
      0,
      this.canvas.height - 1 - Math.floor(((client.y - rect.top) / rect.height) * this.canvas.height),
    );

    // Redraw first: without preserveDrawingBuffer the contents are undefined once the
    // frame ends, and a stale read returns black.
    this.dirty = true;
    this.draw();
    const pixel = new Uint8Array(4);
    this.gl.readPixels(
      Math.min(Math.max(x, 0), this.canvas.width - 1),
      Math.min(Math.max(y, 0), this.canvas.height - 1),
      1,
      1,
      this.gl.RGBA,
      this.gl.UNSIGNED_BYTE,
      pixel,
    );
    return `#${[pixel[0], pixel[1], pixel[2]]
      .map((channel) => channel.toString(16).padStart(2, "0"))
      .join("")}`;
  }

  /**
   * A PNG of the current view.
   *
   * The draw has to happen in this same task: without `preserveDrawingBuffer` the
   * framebuffer is undefined after the frame ends, and reading it later returns an empty
   * image rather than an error.
   */
  captureDataUrl(): string {
    this.dirty = true;
    this.draw();
    return this.canvas.toDataURL("image/png");
  }

  /** How tall the view is at the point it turns about, in angstroms: what a picture's height spans. */
  viewHeight(): number {
    return this.camera.viewHeight();
  }

  /** The largest image the graphics card will draw in one go, along each side. */
  maxCaptureSide(): number {
    const gl = this.gl;
    const viewport = gl.getParameter(gl.MAX_VIEWPORT_DIMS) as Int32Array;
    const limits = [gl.getParameter(gl.MAX_RENDERBUFFER_SIZE) as number, gl.getParameter(gl.MAX_TEXTURE_SIZE) as number, viewport[0], viewport[1]];
    return Math.min(...limits.filter((value) => Number.isFinite(value) && value > 0), 16384);
  }

  /**
   * The view drawn once at a size of its own, as a PNG data URL.
   *
   * The same view - the same camera and the same framing, at the viewport's own aspect -
   * at any number of pixels, rather than a copy of the pixels on screen. The canvas is
   * resized for one frame and put back; what is on screen is redrawn before it is seen.
   * `transparent` leaves the background out, for a picture to be laid on a page.
   */
  captureAt(width: number, height: number, transparent = false): string {
    const wasTransparent = this.transparent;
    this.fixedSize = { width: Math.max(1, Math.round(width)), height: Math.max(1, Math.round(height)) };
    this.transparent = transparent || wasTransparent;
    try {
      this.dirty = true;
      this.draw();
      return this.canvas.toDataURL("image/png");
    } finally {
      this.fixedSize = null;
      this.transparent = wasTransparent;
      this.dirty = true;
      this.draw();
    }
  }

  /** While a capture is drawn: the size it is drawn at, in place of the canvas's own. */
  private fixedSize: { width: number; height: number } | null = null;
  /** While a capture is drawn larger than it will be saved: by how much. */
  private pixelScale = 1;

  /**
   * The view as a picture `width` by `height`, smoothed by drawing it `supersample` times
   * larger each way and shrinking it.
   *
   * A picture the size of the screen is one sample a pixel: a ribbon thinner than a pixel
   * comes and goes along its length, and a capsid of them is speckle. Drawn four times as
   * large and averaged down, each pixel is sixteen samples - the size of the file is a
   * separate choice from how clean it is. The factor is lowered as far as it must be to
   * stay within what the graphics card draws in one go. `onStage` is told how far along it
   * is; the steps yield, so a gauge can be drawn between them.
   */
  async capturePicture(
    width: number,
    height: number,
    options: { supersample?: number; transparent?: boolean; onStage?: (fraction: number, stage: string) => void } = {},
  ): Promise<Blob> {
    const tell = options.onStage ?? (() => undefined);
    const yieldFrame = () => new Promise<void>((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
    const outWidth = Math.max(1, Math.round(width));
    const outHeight = Math.max(1, Math.round(height));
    const MOST_PIXELS = 40_000_000;
    const most = this.maxCaptureSide();
    let factor = Math.max(1, Math.floor(options.supersample ?? 1));
    while (factor > 1 && (Math.max(outWidth, outHeight) * factor > most || outWidth * outHeight * factor * factor > MOST_PIXELS)) {
      factor -= 1;
    }
    const drawWidth = outWidth * factor;
    const drawHeight = outHeight * factor;

    tell(0.05, `Drawing at ${drawWidth} \u00d7 ${drawHeight}`);
    await yieldFrame();
    // Drawn and copied out in one go: a WebGL canvas keeps its pixels only until the
    // frame it was drawn in is shown.
    const big = document.createElement("canvas");
    big.width = drawWidth;
    big.height = drawHeight;
    const wasTransparent = this.transparent;
    this.fixedSize = { width: drawWidth, height: drawHeight };
    this.pixelScale = factor;
    this.transparent = options.transparent || wasTransparent;
    try {
      this.dirty = true;
      this.draw();
      big.getContext("2d")?.drawImage(this.canvas, 0, 0);
    } finally {
      this.fixedSize = null;
      this.pixelScale = 1;
      this.transparent = wasTransparent;
      this.dirty = true;
      this.draw();
    }

    // Halved a step at a time: a single large shrink skips most of the samples it was
    // drawn large to have.
    let current = big;
    let steps = 0;
    while (current.width >= outWidth * 2 && current.height >= outHeight * 2) {
      tell(0.35 + Math.min(steps, 3) * 0.08, "Smoothing");
      await yieldFrame();
      const half = document.createElement("canvas");
      half.width = Math.max(outWidth, Math.round(current.width / 2));
      half.height = Math.max(outHeight, Math.round(current.height / 2));
      const context = half.getContext("2d");
      if (!context) break;
      context.imageSmoothingEnabled = true;
      context.imageSmoothingQuality = "high";
      context.drawImage(current, 0, 0, half.width, half.height);
      current = half;
      steps += 1;
    }
    if (current.width !== outWidth || current.height !== outHeight) {
      const last = document.createElement("canvas");
      last.width = outWidth;
      last.height = outHeight;
      const context = last.getContext("2d");
      if (context) {
        context.imageSmoothingEnabled = true;
        context.imageSmoothingQuality = "high";
        context.drawImage(current, 0, 0, outWidth, outHeight);
        current = last;
      }
    }

    tell(0.65, "Encoding the PNG");
    await yieldFrame();
    const picture = current;
    return await new Promise<Blob>((resolve, reject) =>
      picture.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("the picture could not be encoded"))), "image/png"),
    );
  }

  /**
   * The light being aimed, if one is selected in the graph.
   *
   * The viewer holds the direction rather than reading it back each frame so that a drag
   * accumulates smoothly: the graph is updated as the pointer moves, and waiting for that
   * round trip before the next move would make the light lag the cursor.
   */
  private lightAim: { id: string; direction: [number, number, number] } | null = null;

  /** Called while a light is being aimed. */
  onLightAim: ((id: string, direction: [number, number, number]) => void) | null = null;

  setLightAim(target: { id: string; direction: [number, number, number] } | null): void {
    this.lightAim = target ? { id: target.id, direction: [...target.direction] } : null;
  }

  /** Whether a light is selected, so the UI can say that alt-drag will aim it. */
  get aimingLight(): boolean {
    return this.lightAim !== null;
  }

  /**
   * Moves the aimed light with the pointer.
   *
   * Nudged along the camera's own right and up axes rather than rotated about them: the
   * light then follows the cursor whichever way the view is turned, with no sign to get
   * wrong, and renormalising keeps it on the sphere around the subject.
   */
  private aimLight(dx: number, dy: number): void {
    const aim = this.lightAim;
    if (!aim) return;
    const camera = this.camera.describe();
    const forward = normalize(subtract(camera.center, camera.eye));
    const right = normalize(cross(forward, camera.up));
    const up = cross(right, forward);

    const speed = 0.006;
    const next = normalize([
      aim.direction[0] + right[0] * dx * speed - up[0] * dy * speed,
      aim.direction[1] + right[1] * dx * speed - up[1] * dy * speed,
      aim.direction[2] + right[2] * dx * speed - up[2] * dy * speed,
    ]);
    aim.direction = next;
    this.onLightAim?.(aim.id, next);
  }

  /**
   * What is under a point on the canvas, or null for the background.
   *
   * Draws the scene once more with ids for colours, into a one-pixel scissor, and reads
   * that pixel back. `readPixels` stalls the pipeline, which is why this runs on a click
   * and never on a move.
   *
   * The alternative - an id channel kept in the G-buffer every frame - costs a render
   * target on every frame to serve a gesture that happens a few times a minute.
   */
  pickAt(client: { x: number; y: number }): Picked | null {
    if (!this.scene || !this.gbuffer) return null;
    const gl = this.gl;
    const rect = this.canvas.getBoundingClientRect();
    const scale = this.canvas.width / Math.max(rect.width, 1);
    // Floored, not rounded: a pixel is the half-open square its index starts, and rounding
    // reads the neighbour for half of every pixel - enough to miss a one-pixel edge.
    const x = Math.floor((client.x - rect.left) * scale);
    // GL's y runs up from the bottom, the pointer's down from the top.
    const y = this.canvas.height - 1 - Math.floor((client.y - rect.top) * scale);
    if (x < 0 || y < 0 || x >= this.canvas.width || y >= this.canvas.height) return null;

    const view = this.camera.view();
    const projection = this.camera.projection(this.aspect());
    const shading = this.shadingUniforms();

    gl.bindFramebuffer(gl.FRAMEBUFFER, this.gbuffer.framebuffer);
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(x, y, 1, 1);
    // Zero is the background, and zero decodes to "nothing".
    gl.clearBufferfv(gl.COLOR, 0, [0, 0, 0, 0]);
    gl.clearDepth(1);
    gl.clear(gl.DEPTH_BUFFER_BIT);
    gl.enable(gl.DEPTH_TEST);

    if (this.graphOn && this.graphData) {
      // In the graph view the graph is what is pointed at: the structure behind it is a
      // ghost, and a ghost that caught clicks meant for a sphere would be a nuisance.
      // One or the other, by what a click is selecting: bonds are thin and sit between
      // spheres, and with both on offer the spheres win nearly every click meant for one.
      if (this.graphPick === "edges") this.graphEdges?.draw(view, projection, shading, 0, true);
      else this.graphNodes?.draw(view, projection, shading, 0, true);
    } else {
      this.mesh?.draw(view, projection, shading, 0, true);
      this.cylinders?.draw(view, projection, shading, 0, true);
      this.spheres?.draw(view, projection, shading, 0, true);
    }

    const pixel = new Uint8Array(4);
    gl.readPixels(x, y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    gl.disable(gl.SCISSOR_TEST);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    // The G-buffer now holds one pixel of ids, so the next frame has to be a real one.
    this.requestFrame();
    return decodePick(pixel);
  }

  /** Called when the viewport is clicked without dragging: what was hit, or null. */
  onPick: ((picked: Picked | null, modifiers: PickModifiers) => void) | null = null;
  /**
   * Called as the pointer moves over the viewport: what is under it, or null.
   *
   * Throttled, because each call is a pick and a pick is a draw plus a `readPixels`; and
   * only while not dragging, because an orbit is not a hover.
   */
  onHover: ((picked: Picked | null) => void) | null = null;

  private attachInput(): void {
    const canvas = this.canvas;
    let dragging: "rotate" | "pan" | "zoom" | "light" | null = null;
    let lastX = 0;
    let lastY = 0;

    // Where the press started and how far it travelled, so a click can be told from a
    // drag: a viewport where releasing after an orbit also selects whatever is under the
    // cursor is a viewport that selects things by accident.
    let pressed: { x: number; y: number; button: number } | null = null;
    let travelled = 0;

    canvas.addEventListener("pointerdown", (event) => {
      if (!this.interactive) return;
      // Middle button, right button, or shift-drag all pan - between them they cover what
      // every viewer's users already have in their fingers. Alt-drag aims the light that
      // is selected in the graph, which is the one thing in the scene that is not the
      // camera and not the structure.
      // Control-drag zooms, as Blender's Ctrl with the middle button does - for a mouse
      // without a wheel and a trackpad without a pinch. A Control-click that does not
      // travel is still the toggle-select it always was.
      dragging =
        event.altKey && this.lightAim
          ? "light"
          : event.button === 0 && event.ctrlKey && !event.shiftKey
            ? "zoom"
            : event.button === 0 && !event.shiftKey
              ? "rotate"
              : "pan";
      lastX = event.clientX;
      lastY = event.clientY;
      pressed = { x: event.clientX, y: event.clientY, button: event.button };
      travelled = 0;
      event.preventDefault();
      // Capture is an optimisation - it keeps the drag alive when the pointer leaves the
      // canvas. It is set after `dragging`, and guarded, because a throw here would
      // otherwise leave the viewport unable to rotate at all.
      try {
        canvas.setPointerCapture(event.pointerId);
      } catch {
        /* the drag still works, it just stops at the canvas edge */
      }
    });

    let lastHoverAt = 0;
    let lastHoverX = -1;
    let lastHoverY = -1;
    canvas.addEventListener("pointermove", (event) => {
      if (!dragging) {
        // A hover pick at most every 60 ms, and only once the pointer has actually moved:
        // the draw behind a pick is the whole scene, scissored to one pixel, and running
        // it on every event a trackpad emits would compete with the frame.
        const now = performance.now();
        const moved =
          Math.abs(event.clientX - lastHoverX) + Math.abs(event.clientY - lastHoverY) >= 2;
        if (this.onHover && moved && now - lastHoverAt >= 60) {
          lastHoverAt = now;
          lastHoverX = event.clientX;
          lastHoverY = event.clientY;
          this.onHover(this.pickAt({ x: event.clientX, y: event.clientY }));
        }
        return;
      }
      const dx = event.clientX - lastX;
      const dy = event.clientY - lastY;
      lastX = event.clientX;
      lastY = event.clientY;
      travelled += Math.abs(dx) + Math.abs(dy);
      if (dragging === "rotate") this.camera.rotate(dx, dy);
      else if (dragging === "zoom") this.camera.zoom(Math.exp(dy * 0.006));
      else if (dragging === "light") this.aimLight(dx, dy);
      else this.camera.pan(dx, dy, canvas.clientHeight);
      this.requestFrame();
    });

    const stop = (event: PointerEvent) => {
      // A left press that went nowhere is a click, and a click asks what it hit. Three
      // pixels of slack, because a mouse moves a little under a finger.
      if (pressed && pressed.button === 0 && travelled < 3 && dragging !== "light") {
        const at = { x: event.clientX, y: event.clientY };
        this.onPick?.(this.pickAt(Number.isFinite(at.x) ? at : pressed), {
          shift: event.shiftKey,
          toggle: event.metaKey || event.ctrlKey,
        });
      }
      pressed = null;
      dragging = null;
      try {
        if (canvas.hasPointerCapture(event.pointerId)) {
          canvas.releasePointerCapture(event.pointerId);
        }
      } catch {
        /* already released */
      }
    };
    canvas.addEventListener("pointerup", stop);
    canvas.addEventListener("pointercancel", stop);
    // Leaving the canvas leaves nothing hovered.
    canvas.addEventListener("pointerleave", () => this.onHover?.(null));

    // Double-click frames everything. The camera is easy to lose - a pan and a zoom and
    // the structure is off screen - and hunting for a button to get back is worse than
    // the gesture every map and every 3D tool already uses.
    canvas.addEventListener("dblclick", (event) => {
      if (!this.interactive) return;
      event.preventDefault();
      this.resetView();
    });
    // A pointerup that lands outside the canvas would otherwise leave the drag stuck on.
    window.addEventListener("pointerup", stop);
    this.detachWindow = () => window.removeEventListener("pointerup", stop);
    canvas.addEventListener("contextmenu", (event) => event.preventDefault());

    canvas.addEventListener(
      "wheel",
      (event) => {
        // The shelf of worlds uses the wheel to move along the row, not to zoom.
        if (!this.interactive) return;
        event.preventDefault();
        // Exponential in the wheel delta, so a trackpad flick and a mouse notch both feel
        // proportional rather than one being unusable.
        this.camera.zoom(Math.exp(event.deltaY * 0.0015));
        this.requestFrame();
      },
      { passive: false },
    );

    window.addEventListener("resize", this.onResize);
  }

  private onResize = () => this.requestFrame();

  private detachWindow: (() => void) | null = null;

  dispose(): void {
    this.disposed = true;
    window.removeEventListener("resize", this.onResize);
    this.detachWindow?.();
    if (this.gbuffer) disposeRenderTarget(this.gl, this.gbuffer);
    if (this.occlusion) disposeSingleTarget(this.gl, this.occlusion);
    // Releasing the context matters: WebGL contexts are a limited resource and opening the
    // viewer repeatedly would eventually get the oldest one killed by the browser.
    this.gl.getExtension("WEBGL_lose_context")?.loseContext();
  }
}
