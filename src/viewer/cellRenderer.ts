// The cell the opening screens are set in, drawn live.
//
// Hundreds of molecules, each a rigid copy of a real structure's beads (one per residue,
// `assets/cell/molecules.json`), each drifting and turning on its own. Every bead of every
// copy is in one instanced draw; what moves is not the beads but each copy's position,
// size and rotation, which live in a float texture rewritten every frame - a few kilobytes
// for every copy at once, where moving the beads would be megabytes.
//
// Drawn as David Goodsell draws a cell: soft, flat-ish colours, a dark line where one
// molecule passes in front of another, and depth told by haze - the far side of the cell
// fading into the glow of the light behind it.

import { createContext, createProgram, type Program } from "./gl";
import { GLASS_GLSL } from "./glass";
import type { RibbonMesh } from "./cellRibbon";
import { lookAt, perspective, type Vec3 } from "./mat";

/** One molecule's beads, centred: x, y, z and radius in angstroms, four to a bead. */
export interface CellTemplate {
  beads: Float32Array;
  /** Which of the molecule's chains each bead is in, from 0. */
  chain: Uint16Array;
  /**
   * Drawn as its beads even near the camera in the glass view, where the others' beads give
   * way to ribbons: a small molecule or an ion has no backbone to draw.
   */
  solid?: boolean;
}

/** One copy in the scene: which molecule, and a colour for each of its chains. */
export interface CellBodySpec {
  template: number;
  colours: [number, number, number][];
}

export interface CellCamera {
  eye: Vec3;
  target: Vec3;
  up: Vec3;
  /** Vertical field of view, radians. */
  fovY: number;
}

/** Copies' transforms are laid out in the texture this many texels to a row. */
const ROW = 1024;

const BEADS_VERTEX = `#version 300 es
precision highp float;
precision highp sampler2D;
layout(location = 0) in vec4 aLocal;
layout(location = 1) in vec4 aInfo;
uniform sampler2D uBodies;
uniform mat4 uView;
uniform mat4 uProjection;
uniform float uAngstrom;
uniform float uSwell;
uniform float uHideNear;
uniform vec3 uEye;
out vec3 vQuad;
out vec3 vCentre;
out float vRadius;
out vec3 vColour;

vec3 turn(vec4 q, vec3 v) {
  vec3 t = 2.0 * cross(q.xyz, v);
  return v + q.w * t + cross(q.xyz, t);
}

ivec2 texel(int index) {
  return ivec2(index % ${ROW}, index / ${ROW});
}

void main() {
  int body = int(aInfo.w + 0.5);
  vec4 place = texelFetch(uBodies, texel(body * 2), 0);
  vec4 rotation = texelFetch(uBodies, texel(body * 2 + 1), 0);
  float size = place.w * uAngstrom;
  vec3 world = place.xyz + turn(rotation, aLocal.xyz) * size;
  // Swollen past their own size, so a molecule's beads run together into one soft lump
  // rather than a cluster of balls.
  // A bead's radius is negative when its molecule keeps its beads near the camera.
  float radius = abs(aLocal.w) * size * uSwell;
  vec3 centre = (uView * vec4(world, 1.0)).xyz;
  // A square facing the camera, a little larger than the sphere so its perspective
  // outline is inside it; the fragment shader finds the sphere in it.
  vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1)) * 2.0 - 1.0;
  vec3 quad = centre + vec3(corner * radius * 1.6, 0.0);
  vQuad = quad;
  vCentre = centre;
  vRadius = radius;
  vColour = aInfo.rgb;
  // Copies of size zero are hidden: they are put out of view entirely. So, in the glass
  // view, are the near ones' beads: those are drawn as ribbons instead.
  bool near = uHideNear > 0.0 && aLocal.w > 0.0 && distance(place.xyz, uEye) < uHideNear;
  gl_Position = place.w <= 0.0 || near ? vec4(2.0, 2.0, 2.0, 1.0) : uProjection * vec4(quad, 1.0);
}`;

const BEADS_FRAGMENT = `#version 300 es
precision highp float;
in vec3 vQuad;
in vec3 vCentre;
in float vRadius;
in vec3 vColour;
uniform mat4 uProjection;
uniform vec3 uRimColour;
uniform float uLift;
out vec4 colour;

void main() {
  // The ray from the eye through this pixel, against the sphere.
  vec3 ray = normalize(vQuad);
  float b = dot(ray, vCentre);
  float c = dot(vCentre, vCentre) - vRadius * vRadius;
  float h = b * b - c;
  if (h < 0.0) discard;
  float t = b - sqrt(h);
  vec3 hit = ray * t;
  vec3 normal = (hit - vCentre) / vRadius;
  vec4 clip = uProjection * vec4(hit, 1.0);
  gl_FragDepth = clip.z / clip.w * 0.5 + 0.5;
  // Lit mostly from behind, as the cell is: a dim warm light from the upper left, and the
  // cool glow behind everything catching each molecule's edge. Wrapped, so the dark side
  // is soft rather than black - the light scatters through the haze and into the protein.
  vec3 key = normalize(vec3(-0.5, 0.6, 0.55));
  float wrap = max((dot(normal, key) + 0.45) / 1.45, 0.0);
  float rim = pow(1.0 - max(normal.z, 0.0), 2.2);
  vec3 lit = vColour * (0.30 + 0.78 * wrap) + uRimColour * rim * 0.55;
  // In the glass view the chain inside glows a little, so it reads through the shell.
  vec3 vivid = mix(vec3(dot(vColour, vec3(0.3, 0.59, 0.11))), vColour, 1.4);
  colour = vec4(mix(lit, vivid * (0.9 + 0.5 * wrap), uLift), 1.0);
}`;

/** The glass view's ribbons: each copy's backbone, placed and turned as its beads are. */
const RIBBON_VERTEX = `#version 300 es
precision highp float;
precision highp sampler2D;
layout(location = 0) in vec3 aPosition;
layout(location = 1) in vec3 aNormal;
layout(location = 2) in vec3 aColour;
layout(location = 3) in float aBody;
uniform sampler2D uBodies;
uniform mat4 uView;
uniform mat4 uProjection;
uniform float uAngstrom;
out vec3 vNormal;
out vec3 vColour;

vec3 turn(vec4 q, vec3 v) {
  vec3 t = 2.0 * cross(q.xyz, v);
  return v + q.w * t + cross(q.xyz, t);
}

ivec2 texel(int index) {
  return ivec2(index % ${ROW}, index / ${ROW});
}

void main() {
  int body = int(aBody + 0.5);
  vec4 place = texelFetch(uBodies, texel(body * 2), 0);
  vec4 rotation = texelFetch(uBodies, texel(body * 2 + 1), 0);
  vec3 world = place.xyz + turn(rotation, aPosition) * place.w * uAngstrom;
  vNormal = mat3(uView) * turn(rotation, aNormal);
  vColour = aColour;
  gl_Position = place.w <= 0.0 ? vec4(2.0, 2.0, 2.0, 1.0) : uProjection * uView * vec4(world, 1.0);
}`;

const RIBBON_FRAGMENT = `#version 300 es
precision highp float;
in vec3 vNormal;
in vec3 vColour;
uniform vec3 uRimColour;
out vec4 colour;

void main() {
  vec3 normal = normalize(vNormal);
  if (!gl_FrontFacing) normal = -normal;
  vec3 key = normalize(vec3(-0.5, 0.6, 0.55));
  float wrap = max((dot(normal, key) + 0.45) / 1.45, 0.0);
  float rim = pow(1.0 - max(normal.z, 0.0), 2.2);
  float shine = pow(max(dot(normal, normalize(key + vec3(0.0, 0.0, 1.0))), 0.0), 40.0);
  vec3 vivid = mix(vec3(dot(vColour, vec3(0.3, 0.59, 0.11))), vColour, 1.35);
  colour = vec4(vivid * (0.62 + 0.75 * wrap) + uRimColour * rim * 0.3 + vec3(shine * 0.4), 1.0);
}`;

/**
 * The glass view's shell: the same beads swollen until they run into one surface, kept
 * as what that surface is (its normal, facing the eye, and the molecule's colour) for
 * the compose pass to draw as glass over what is inside.
 */
const SHELL_FRAGMENT = `#version 300 es
precision highp float;
in vec3 vQuad;
in vec3 vCentre;
in float vRadius;
in vec3 vColour;
uniform mat4 uProjection;
layout(location = 0) out vec4 normalOut;
layout(location = 1) out vec4 tintOut;

void main() {
  vec3 ray = normalize(vQuad);
  float b = dot(ray, vCentre);
  float c = dot(vCentre, vCentre) - vRadius * vRadius;
  float h = b * b - c;
  if (h < 0.0) discard;
  float t = b - sqrt(h);
  vec3 hit = ray * t;
  vec3 normal = (hit - vCentre) / vRadius;
  vec4 clip = uProjection * vec4(hit, 1.0);
  gl_FragDepth = clip.z / clip.w * 0.5 + 0.5;
  normalOut = vec4(normal * 0.5 + 0.5, 1.0);
  tintOut = vec4(vColour, 1.0);
}`;

const QUAD_VERTEX = `#version 300 es
out vec2 vUv;
void main() {
  vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
  vUv = corner;
  gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}`;

const COMPOSE_FRAGMENT = `#version 300 es
precision highp float;
in vec2 vUv;
uniform sampler2D uColour;
uniform sampler2D uDepth;
uniform vec2 uTexel;
uniform float uNear;
uniform float uFar;
uniform float uHaze;
uniform vec2 uGlow;
uniform float uFocus;
uniform float uBlur;
uniform vec3 uRimColour;
uniform vec3 uGlowColour;
uniform vec3 uDeepColour;
uniform float uLine;
uniform float uTime;
uniform bool uGlass;
uniform sampler2D uShellNormal;
uniform sampler2D uShellTint;
uniform sampler2D uShellDepth;
uniform float uSmooth;
out vec4 colour;
${GLASS_GLSL}

float linear(float depth) {
  float z = depth * 2.0 - 1.0;
  return 2.0 * uNear * uFar / (uFar + uNear - z * (uFar - uNear));
}

// The light behind the cell: brightest a little above the middle, deep blue at the edges.
vec3 backdrop(vec2 uv) {
  float d = distance(uv * vec2(1.6, 1.0), uGlow * vec2(1.6, 1.0));
  return mix(uGlowColour, uDeepColour, smoothstep(0.0, 1.05, d));
}

// A little film grain, so the dark is not flat and the gradients do not band.
float grain(vec2 uv) {
  return fract(sin(dot(uv * 1000.0 + uTime, vec2(12.9898, 78.233))) * 43758.5453) - 0.5;
}

vec3 finish(vec3 c, vec2 uv) {
  float vignette = smoothstep(1.15, 0.35, length((uv - 0.5) * vec2(1.25, 1.0)));
  return c * mix(0.72, 1.0, vignette) + grain(uv) * 0.012;
}

vec3 scene(vec2 uv, float raw, vec3 back);
vec3 glass(float coreRaw, vec3 back);

/** What is seen at a point without glass: the scene there, or the light behind. */
vec3 bare(vec2 uv) {
  float raw = texture(uDepth, uv).r;
  vec3 back = backdrop(uv);
  return raw >= 1.0 ? back : scene(uv, raw, back);
}

void main() {
  float raw = texture(uDepth, vUv).r;
  vec3 back = backdrop(vUv);
  if (raw >= 1.0 && !uGlass) {
    colour = vec4(finish(back, vUv), 1.0);
    return;
  }
  if (!uGlass) {
    colour = vec4(finish(raw >= 1.0 ? back : scene(vUv, raw, back), vUv), 1.0);
    return;
  }
  colour = vec4(finish(glass(raw, back), vUv), 1.0);
}

// The glass over what is inside, as the viewport's glass is (\`GLASS_GLSL\`): what is behind
// it bent along its slope and tinted by its molecule, the cell's light reflected at its
// rim, two highlights and a bright edge. The shell's normals are the swollen beads',
// smoothed over a few pixels, so the lumps where one bead meets the next read as one surface.
// What is seen here unbent is worked out only where it is needed - off the glass, at its
// edge, or where a chain is in front of it: it is a depth-of-field gather, and doing it as
// well as the bent one on every pixel of glass cost six frames a second.
vec3 glass(float coreRaw, vec3 back) {
  // How much of this pixel the shell covers, from a ring of taps round it, and the
  // nearest shell among them. The shell is drawn at half size and a depth texture cannot
  // be filtered, so read at one point its edge was a staircase two pixels a step.
  float shellRaw = texture(uShellDepth, vUv).r;
  float cover = shellRaw < 1.0 ? 1.0 : 0.0;
  // Where the shell's tint and normal are read: here, or the nearest tap that is shell.
  // Read here when only a neighbour is shell, they were the cleared texels' black, and the
  // rim came out stepped in dark.
  vec2 found = vUv;
  // Two rings, a texel and two out, so the edge is spread over the width of a step.
  for (int i = 0; i < 12; i++) {
    float a = float(i) * 0.523599 + 0.26;
    float r = i < 6 ? 1.7 : 3.4;
    vec2 at = vUv + vec2(cos(a * (i < 6 ? 2.0 : 1.0)), sin(a * (i < 6 ? 2.0 : 1.0))) * r * uTexel;
    float d = texture(uShellDepth, at).r;
    if (d < 1.0) {
      cover += 1.0;
      if (d < shellRaw) found = at;
      shellRaw = min(shellRaw, d);
    }
  }
  cover = smoothstep(0.08, 0.92, cover / 13.0);
  if (cover <= 0.0) return bare(vUv);
  float shell = linear(shellRaw);
  // Something's chain in front of another's shell: the chain is what is seen.
  if (coreRaw < 1.0 && linear(coreRaw) < shell - 0.3) return bare(vUv);
  vec3 n = vec3(0.0);
  float weight = 0.0;
  // Wider for what is near, which is drawn larger: about the same few angstroms of surface.
  // Wide enough that the lumps where one swollen bead meets the next are smoothed into one
  // shell: narrower, a molecule under glass read as a bunch of grapes.
  float reach = clamp(uSmooth / shell, 4.0, 30.0);
  for (int i = 0; i < 24; i++) {
    float a = float(i) * 2.39996;
    float r = reach * sqrt((float(i) + 0.5) / 24.0);
    vec2 at = vUv + vec2(cos(a), sin(a)) * r * uTexel;
    float d = texture(uShellDepth, at).r;
    if (d >= 1.0) continue;
    float w = exp(-abs(linear(d) - shell) * 1.5);
    n += (texture(uShellNormal, at).xyz * 2.0 - 1.0) * w;
    weight += w;
  }
  n = normalize(weight > 0.0 ? n : texture(uShellNormal, found).xyz * 2.0 - 1.0);
  vec3 tint = texture(uShellTint, found).rgb;
  tint = mix(vec3(dot(tint, vec3(0.3, 0.59, 0.11))), tint, 1.5);
  vec3 eye = vec3(0.0, 0.0, 1.0);
  float facing = clamp(n.z, 0.0, 1.0);
  // Its thickness, in angstroms, as a rounded shell's is: most where it faces the eye.
  float thickness = 34.0 * facing;
  // What is inside, seen through the glass: displaced along its slope, as the viewport's is.
  // Half the viewport's bend: the shell is the beads swollen, smooth to the eye but still
  // lumpy underneath, and bent at full strength every lump showed as a bubble.
  vec2 offset = 0.5 * n.xy * glassBend(facing, thickness) * vec2(uTexel.x / uTexel.y, 1.0);
  vec3 behind = bare(clamp(vUv - offset, vec2(0.001), vec2(0.999)));
  // The cell's own light is what it reflects - its glow, lighter above - never darker than
  // what is behind the glass: a dark floor reflected in every turned-away lump was a dark
  // patch on each.
  // Its highlights at a third: on a shell this lumpy underneath, full ones glinted on
  // every lump like snow.
  vec3 lit = glassLight(behind, n, eye, tint, thickness, uRimColour * 1.3, max(uGlowColour * 0.9, behind), 0.3);
  // The haze takes the glass as it takes everything, by how far away it is.
  float haze = 1.0 - exp(-shell * uHaze);
  vec3 glassy = mix(lit, back, haze);
  return cover < 0.999 ? mix(bare(vUv), glassy, cover) : glassy;
}

vec3 scene(vec2 uv, float raw, vec3 back) {
  float here = linear(raw);
  // Depth of field: the further from the focus, the wider the disc a point is spread over,
  // gathered from twenty taps around it. What lies behind a tap (the backdrop) is let in
  // too, so a blurred edge melts into the haze rather than into black.
  float blur = clamp(abs(here - uFocus) / here * uBlur, 0.0, 22.0);
  vec3 base = texture(uColour, uv).rgb;
  if (blur > 0.6) {
    vec3 sum = base;
    float weight = 1.0;
    for (int i = 0; i < 20; i++) {
      float a = float(i) * 2.39996;
      float r = blur * sqrt((float(i) + 0.5) / 20.0);
      vec2 at = uv + vec2(cos(a), sin(a)) * r * uTexel;
      float d = texture(uDepth, at).r;
      sum += d >= 1.0 ? backdrop(at) : texture(uColour, at).rgb;
      weight += 1.0;
    }
    base = sum / weight;
  }
  // A dark line where a molecule stands in front of something further away.
  float step = 0.0;
  for (int i = 0; i < 4; i++) {
    vec2 offset = vec2(i == 0 ? 1.0 : i == 1 ? -1.0 : 0.0, i == 2 ? 1.0 : i == 3 ? -1.0 : 0.0) * uTexel * 1.5;
    float there = texture(uDepth, uv + offset).r;
    float far = there >= 1.0 ? uFar : linear(there);
    step = max(step, (far - here) / here);
  }
  // The line fades as the blur grows: a drawn edge on a blurred shape looks pasted on.
  float line = smoothstep(0.015, 0.06, step) * (1.0 - smoothstep(0.8, 3.0, blur));
  base *= 1.0 - uLine * line;
  // Haze: the further, the more of the light behind it is between the eye and it.
  float haze = 1.0 - exp(-here * uHaze);
  return mix(base, back, haze);
}`;

export class CellRenderer {
  private gl: WebGL2RenderingContext;
  private beads: Program;
  private shell: Program;
  private ribbon: Program;
  private ribbonVao: WebGLVertexArrayObject;
  private ribbonInstances: WebGLBuffer;
  /** Each template's ribbon in the shared buffers: first index and how many. */
  private ribbonRanges: { first: number; count: number }[] = [];
  private bodyTemplate = new Int32Array(0);
  private nearBodies = new Float32Array(0);
  /** In the glass view, copies nearer than this are drawn as ribbons, the rest as beads. */
  ribbonReach = 50;
  /** Within this, the ribbons are the fine ones (`cellRibbon.FINE`). */
  fineReach = 28;
  private compose: Program;
  private vao: WebGLVertexArrayObject;
  private quadVao: WebGLVertexArrayObject;
  private instances: WebGLBuffer;
  private info: WebGLBuffer;
  private bodies: WebGLTexture;
  private count = 0;
  private bodyCount = 0;
  /** Each copy's place and size, then its rotation: eight numbers a copy. */
  transforms = new Float32Array(0);
  private target: { framebuffer: WebGLFramebuffer; colour: WebGLTexture; depth: WebGLTexture; width: number; height: number } | null = null;
  /** The glass view's shell: its normal, its tint and its depth. Made when first needed. */
  private shellTarget: { framebuffer: WebGLFramebuffer; normal: WebGLTexture; tint: WebGLTexture; depth: WebGLTexture; width: number; height: number } | null = null;
  /**
   * Glass: each molecule a clear shell - its beads swollen into one surface - with a thin
   * chain of its beads inside, bright, as a ribbon is inside the viewport's glass view.
   */
  glass = false;
  /** How far the shell's beads are swollen, and how small the chain's are, in the glass view. */
  shellSwell = 2.8;
  coreSwell = 0.62;
  /**
   * Angstroms to scene units. Molecules are drawn larger than true beside the vesicles a
   * world lives in - a vesicle is thousands of angstroms across, and at true size the
   * proteins around it were specks - as an illustrator would draw them.
   */
  angstrom = 1 / 24;
  haze = 0.02;
  /** The distance in focus, and how quickly things leave it. */
  focus = 24;
  blur = 9;
  /** The light behind everything, the dark at the edges, and the edge light it puts on things. */
  glowColour: [number, number, number] = [0.20, 0.40, 0.48];
  deepColour: [number, number, number] = [0.006, 0.012, 0.024];
  rimColour: [number, number, number] = [0.30, 0.55, 0.62];
  /** How dark the line where one molecule passes in front of another is; none by default. */
  line = 0;
  /** How much the beads swell into each other. */
  swell = 1.35;
  glow: [number, number] = [0.5, 0.58];

  constructor(private canvas: HTMLCanvasElement) {
    this.gl = createContext(canvas);
    const gl = this.gl;
    this.beads = createProgram(gl, "cell beads", BEADS_VERTEX, BEADS_FRAGMENT);
    this.shell = createProgram(gl, "cell shell", BEADS_VERTEX, SHELL_FRAGMENT);
    this.ribbon = createProgram(gl, "cell ribbon", RIBBON_VERTEX, RIBBON_FRAGMENT);
    this.ribbonVao = gl.createVertexArray()!;
    this.ribbonInstances = gl.createBuffer()!;
    this.compose = createProgram(gl, "cell compose", QUAD_VERTEX, COMPOSE_FRAGMENT);
    this.vao = gl.createVertexArray()!;
    this.quadVao = gl.createVertexArray()!;
    this.instances = gl.createBuffer()!;
    this.info = gl.createBuffer()!;
    this.bodies = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, this.bodies);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  }

  /** Builds the scene's beads: every copy's, each tagged with which copy it belongs to. */
  setScene(templates: CellTemplate[], bodies: CellBodySpec[]): void {
    const gl = this.gl;
    let total = 0;
    for (const body of bodies) total += (templates[body.template]?.beads.length ?? 0) / 4;
    const local = new Float32Array(total * 4);
    const info = new Float32Array(total * 4);
    let at = 0;
    bodies.forEach((body, index) => {
      const template = templates[body.template];
      if (!template) return;
      const beads = template.beads.length / 4;
      local.set(template.beads, at * 4);
      if (template.solid) for (let bead = 0; bead < beads; bead += 1) local[(at + bead) * 4 + 3] *= -1;
      for (let bead = 0; bead < beads; bead += 1) {
        const rgb = body.colours[template.chain[bead] % body.colours.length] ?? [0.7, 0.7, 0.7];
        const slot = (at + bead) * 4;
        info[slot] = rgb[0];
        info[slot + 1] = rgb[1];
        info[slot + 2] = rgb[2];
        info[slot + 3] = index;
      }
      at += beads;
    });
    this.count = total;
    this.bodyCount = bodies.length;
    this.bodyTemplate = Int32Array.from(bodies, (body) => body.template);
    this.nearBodies = new Float32Array(bodies.length);
    this.transforms = new Float32Array(bodies.length * 8);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.instances);
    gl.bufferData(gl.ARRAY_BUFFER, local, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 4, gl.FLOAT, false, 0, 0);
    gl.vertexAttribDivisor(0, 1);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.info);
    gl.bufferData(gl.ARRAY_BUFFER, info, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 4, gl.FLOAT, false, 0, 0);
    gl.vertexAttribDivisor(1, 1);
    gl.bindVertexArray(null);
  }

  /**
   * Each template's ribbons, by template index (null for one without), fine and then
   * coarse, into one set of buffers. Drawn only in the glass view, for the copies near the camera:
   * the fine ones within `fineReach`, the coarse ones out to `ribbonReach`.
   */
  setRibbons(fine: (RibbonMesh | null)[], coarse: (RibbonMesh | null)[]): void {
    const gl = this.gl;
    const meshes = [...fine, ...coarse];
    let vertices = 0;
    let indices = 0;
    for (const mesh of meshes) {
      vertices += (mesh?.positions.length ?? 0) / 3;
      indices += mesh?.indices.length ?? 0;
    }
    const positions = new Float32Array(vertices * 3);
    const normals = new Float32Array(vertices * 3);
    const colours = new Float32Array(vertices * 3);
    const elements = new Uint32Array(indices);
    let v = 0;
    let i = 0;
    this.ribbonRanges = meshes.map((mesh) => {
      if (!mesh) return { first: 0, count: 0 };
      positions.set(mesh.positions, v * 3);
      normals.set(mesh.normals, v * 3);
      colours.set(mesh.colours, v * 3);
      for (let k = 0; k < mesh.indices.length; k += 1) elements[i + k] = mesh.indices[k] + v;
      const range = { first: i, count: mesh.indices.length };
      v += mesh.positions.length / 3;
      i += mesh.indices.length;
      return range;
    });
    gl.bindVertexArray(this.ribbonVao);
    const attribute = (location: number, data: Float32Array) => {
      const buffer = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      gl.enableVertexAttribArray(location);
      gl.vertexAttribPointer(location, 3, gl.FLOAT, false, 0, 0);
    };
    attribute(0, positions);
    attribute(1, normals);
    attribute(2, colours);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.ribbonInstances);
    gl.bufferData(gl.ARRAY_BUFFER, this.nearBodies.byteLength || 4, gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(3);
    gl.vertexAttribDivisor(3, 1);
    const index = gl.createBuffer();
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, index);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, elements, gl.STATIC_DRAW);
    gl.bindVertexArray(null);
  }

  /** The near copies' ribbons, template by template, into what is bound. */
  private drawRibbons(view: Float32Array, projection: Float32Array, eye: Vec3): void {
    const gl = this.gl;
    // Fine then coarse, each by template.
    const slots = this.ribbonRanges.length;
    const templates = slots / 2;
    if (templates === 0) return;
    // The near copies, grouped by template and detail: counted, then laid out, then drawn
    // a group at a time.
    const counts = new Int32Array(slots + 1);
    const reach = this.ribbonReach * this.ribbonReach;
    const fine = this.fineReach * this.fineReach;
    const slot = new Int32Array(this.bodyCount).fill(-1);
    for (let body = 0; body < this.bodyCount; body += 1) {
      const at = body * 8;
      const dx = this.transforms[at] - eye[0];
      const dy = this.transforms[at + 1] - eye[1];
      const dz = this.transforms[at + 2] - eye[2];
      const template = this.bodyTemplate[body];
      const d2 = dx * dx + dy * dy + dz * dz;
      if (d2 < reach && template < templates && this.ribbonRanges[template].count > 0) {
        const s = d2 < fine ? template : template + templates;
        slot[body] = s;
        counts[s + 1] += 1;
      }
    }
    for (let t = 0; t < slots; t += 1) counts[t + 1] += counts[t];
    const fill = counts.slice();
    for (let body = 0; body < this.bodyCount; body += 1) {
      if (slot[body] >= 0) this.nearBodies[fill[slot[body]]++] = body;
    }
    const total = counts[slots];
    if (total === 0) return;
    this.ribbon.use();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.bodies);
    gl.uniform1i(this.ribbon.uniform("uBodies"), 0);
    gl.uniformMatrix4fv(this.ribbon.uniform("uView"), false, view);
    gl.uniformMatrix4fv(this.ribbon.uniform("uProjection"), false, projection);
    gl.uniform1f(this.ribbon.uniform("uAngstrom"), this.angstrom);
    gl.uniform3f(this.ribbon.uniform("uRimColour"), ...this.rimColour);
    gl.bindVertexArray(this.ribbonVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.ribbonInstances);
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, this.nearBodies, 0, total);
    for (let t = 0; t < slots; t += 1) {
      const instances = counts[t + 1] - counts[t];
      const range = this.ribbonRanges[t];
      if (instances === 0 || range.count === 0) continue;
      gl.vertexAttribPointer(3, 1, gl.FLOAT, false, 0, counts[t] * 4);
      gl.drawElementsInstanced(gl.TRIANGLES, range.count, gl.UNSIGNED_INT, range.first * 4, instances);
    }
    gl.bindVertexArray(null);
  }

  get beadCount(): number {
    return this.count;
  }

  private resize(): { width: number; height: number } {
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    const width = Math.max(1, Math.round(this.canvas.clientWidth * ratio));
    const height = Math.max(1, Math.round(this.canvas.clientHeight * ratio));
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }
    const gl = this.gl;
    if (!this.target || this.target.width !== width || this.target.height !== height) {
      if (this.target) {
        gl.deleteFramebuffer(this.target.framebuffer);
        gl.deleteTexture(this.target.colour);
        gl.deleteTexture(this.target.depth);
      }
      const make = (internal: number, format: number, type: number) => {
        const texture = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.texImage2D(gl.TEXTURE_2D, 0, internal, width, height, 0, format, type, null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        return texture;
      };
      const colour = make(gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE);
      const depth = make(gl.DEPTH_COMPONENT24, gl.DEPTH_COMPONENT, gl.UNSIGNED_INT);
      const framebuffer = gl.createFramebuffer();
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, colour, 0);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.TEXTURE_2D, depth, 0);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      this.target = { framebuffer, colour, depth, width, height };
    }
    // The shell at half size: its normals are smoothed over several pixels anyway.
    const shellWidth = Math.max(1, Math.round(width / 2));
    const shellHeight = Math.max(1, Math.round(height / 2));
    if (this.glass && (!this.shellTarget || this.shellTarget.width !== shellWidth || this.shellTarget.height !== shellHeight)) {
      if (this.shellTarget) {
        gl.deleteFramebuffer(this.shellTarget.framebuffer);
        gl.deleteTexture(this.shellTarget.normal);
        gl.deleteTexture(this.shellTarget.tint);
        gl.deleteTexture(this.shellTarget.depth);
      }
      const normal = this.texture(gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, shellWidth, shellHeight);
      const tint = this.texture(gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, shellWidth, shellHeight);
      const depth = this.texture(gl.DEPTH_COMPONENT24, gl.DEPTH_COMPONENT, gl.UNSIGNED_INT, shellWidth, shellHeight);
      const framebuffer = gl.createFramebuffer();
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, normal, 0);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT1, gl.TEXTURE_2D, tint, 0);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.TEXTURE_2D, depth, 0);
      gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1]);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      this.shellTarget = { framebuffer, normal, tint, depth, width: shellWidth, height: shellHeight };
    }
    return { width, height };
  }

  private texture(internal: number, format: number, type: number, width: number, height: number): WebGLTexture {
    const gl = this.gl;
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, internal, width, height, 0, format, type, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return texture;
  }

  /** The beads, once, with `program` and swollen by `swell`. */
  private drawBeads(program: Program, view: Float32Array, projection: Float32Array, swell: number, eye: Vec3 = [0, 0, 0], hideNear = 0): void {
    const gl = this.gl;
    program.use();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.bodies);
    gl.uniform1i(program.uniform("uBodies"), 0);
    gl.uniformMatrix4fv(program.uniform("uView"), false, view);
    gl.uniformMatrix4fv(program.uniform("uProjection"), false, projection);
    gl.uniform1f(program.uniform("uAngstrom"), this.angstrom);
    gl.uniform1f(program.uniform("uSwell"), swell);
    gl.uniform3f(program.uniform("uRimColour"), ...this.rimColour);
    gl.uniform1f(program.uniform("uLift"), this.glass ? 1 : 0);
    gl.uniform1f(program.uniform("uHideNear"), hideNear);
    gl.uniform3f(program.uniform("uEye"), ...eye);
    gl.bindVertexArray(this.vao);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, this.count);
    gl.bindVertexArray(null);
  }

  /** Draws a frame: the copies where `transforms` says they are, from `camera`. */
  render(camera: CellCamera, near: number, far: number): void {
    const gl = this.gl;
    const { width, height } = this.resize();
    const target = this.target!;

    // Each copy's transform into the texture: two texels a copy.
    const texels = Math.max(this.bodyCount * 2, 1);
    const rows = Math.ceil(texels / ROW);
    const padded = new Float32Array(ROW * rows * 4);
    padded.set(this.transforms);
    gl.bindTexture(gl.TEXTURE_2D, this.bodies);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA32F, ROW, rows, 0, gl.RGBA, gl.FLOAT, padded);

    const view = lookAt(camera.eye, camera.target, camera.up);
    const projection = perspective(camera.fovY, width / height, near, far);

    gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer);
    gl.viewport(0, 0, width, height);
    gl.clearColor(0, 0, 0, 0);
    gl.clearDepth(1);
    gl.enable(gl.DEPTH_TEST);
    gl.depthFunc(gl.LESS);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    const ribbons = this.glass && this.ribbonRanges.length > 0;
    if (this.count > 0) this.drawBeads(this.beads, view, projection, this.glass ? this.coreSwell : this.swell, camera.eye, ribbons ? this.ribbonReach : 0);
    if (ribbons) this.drawRibbons(view, projection, camera.eye);
    const shell = this.glass ? this.shellTarget : null;
    if (shell && this.count > 0) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, shell.framebuffer);
      gl.viewport(0, 0, shell.width, shell.height);
      gl.clearBufferfv(gl.COLOR, 0, [0.5, 0.5, 1, 0]);
      gl.clearBufferfv(gl.COLOR, 1, [0, 0, 0, 0]);
      gl.clear(gl.DEPTH_BUFFER_BIT);
      this.drawBeads(this.shell, view, projection, this.shellSwell);
    }

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, width, height);
    gl.disable(gl.DEPTH_TEST);
    this.compose.use();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, target.colour);
    gl.uniform1i(this.compose.uniform("uColour"), 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, target.depth);
    gl.uniform1i(this.compose.uniform("uDepth"), 1);
    gl.uniform2f(this.compose.uniform("uTexel"), 1 / width, 1 / height);
    gl.uniform1f(this.compose.uniform("uNear"), near);
    gl.uniform1f(this.compose.uniform("uFar"), far);
    gl.uniform1f(this.compose.uniform("uHaze"), this.haze);
    gl.uniform2f(this.compose.uniform("uGlow"), this.glow[0], this.glow[1]);
    const ratio = width / Math.max(this.canvas.clientWidth, 1);
    gl.uniform1f(this.compose.uniform("uFocus"), this.focus);
    gl.uniform3f(this.compose.uniform("uGlowColour"), ...this.glowColour);
    gl.uniform3f(this.compose.uniform("uRimColour"), ...this.rimColour);
    gl.uniform3f(this.compose.uniform("uDeepColour"), ...this.deepColour);
    gl.uniform1f(this.compose.uniform("uLine"), this.line);
    gl.uniform1f(this.compose.uniform("uTime"), (performance.now() / 1000) % 100);
    gl.uniform1f(this.compose.uniform("uBlur"), this.blur * ratio);
    gl.uniform1i(this.compose.uniform("uGlass"), shell ? 1 : 0);
    if (shell) {
      const bind = (unit: number, texture: WebGLTexture, name: string) => {
        gl.activeTexture(gl.TEXTURE0 + unit);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.uniform1i(this.compose.uniform(name), unit);
      };
      bind(2, shell.normal, "uShellNormal");
      bind(3, shell.tint, "uShellTint");
      bind(4, shell.depth, "uShellDepth");
      gl.uniform1f(this.compose.uniform("uSmooth"), 150 * ratio);
    }
    gl.bindVertexArray(this.quadVao);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.bindVertexArray(null);
    gl.activeTexture(gl.TEXTURE0);
  }

  dispose(): void {
    this.gl.getExtension("WEBGL_lose_context")?.loseContext();
  }
}
