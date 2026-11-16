// The lighting model, shared by every pass that draws geometry.
//
// One GLSL function, pasted into the sphere, cylinder and mesh shaders, so a ribbon and a
// stick sitting next to each other are lit by the same lamps with the same falloff. When
// this lived separately in three shaders they drifted, and the fix was three edits.
//
// Shading happens in linear space and the composite pass encodes to sRGB at the very end.
// The core sends linear colour for the same reason: light does not add up in sRGB, and two
// lights over the same surface never sum correctly there.

import type { Light } from "../ipc";
import type { Mat4 } from "./mat";

/** Matches MAX_LIGHTS in the GLSL below. Four covers a studio rig with one to spare. */
export const MAX_LIGHTS = 4;

/** Uniform block and shading function, inserted into each geometry fragment shader. */
export const SHADING_GLSL = `
#define MAX_LIGHTS ${MAX_LIGHTS}

uniform vec3 uLightDirection[MAX_LIGHTS];  // view space, surface towards light
uniform vec3 uLightColor[MAX_LIGHTS];      // colour premultiplied by energy
uniform int uLightCount;
uniform vec3 uAmbient;
uniform float uRoughness;
uniform float uSpecular;
uniform float uGlow;

vec3 shade(vec3 baseColor, vec3 normal, vec3 viewDir) {
  // Ambient stands in for a uniform sky. Occlusion is applied later, in the composite,
  // where the depth buffer is available to compute it from.
  vec3 result = baseColor * uAmbient;

  // Roughness to a Blinn-Phong exponent. Not physically based, but matched to the same
  // roughness number Blender's Principled BSDF gets, so the preview tracks the render.
  float shininess = mix(96.0, 4.0, clamp(uRoughness, 0.0, 1.0));
  float specularScale = uSpecular * (1.0 - clamp(uRoughness, 0.0, 1.0) * 0.8);

  for (int index = 0; index < MAX_LIGHTS; index++) {
    if (index >= uLightCount) break;
    vec3 toLight = uLightDirection[index];
    float diffuse = max(dot(normal, toLight), 0.0);
    vec3 halfway = normalize(toLight + viewDir);
    float specular = pow(max(dot(normal, halfway), 0.0), shininess) * specularScale;
    result += uLightColor[index] * (baseColor * diffuse + vec3(specular));
  }
  // Under glass the ribbons are lit from within, as the cores of glass illustrations are:
  // their own colour, a little richer, brightest where the ribbon turns away - a glowing
  // tube rather than a painted one - so they read through the glass and its reflections
  // instead of greying under them.
  if (uGlow > 0.0) {
    float facing = clamp(dot(normal, viewDir), 0.0, 1.0);
    float luma = dot(baseColor, vec3(0.2126, 0.7152, 0.0722));
    vec3 rich = max(mix(vec3(luma), baseColor, 1.35), vec3(0.0));
    vec3 glow = rich * (0.75 + 0.35 * facing) + rich * pow(1.0 - facing, 2.0) * 1.1;
    result = mix(result, glow, uGlow);
  }
  return result;
}
`;

/** Which part of the scene is turning, and how far round it is. */
export interface Row {
  /** How far apart the worlds stand; 0 means this is not a row and nothing is posed. */
  pitch: number;
  /** How far along the row the camera is, in world units. */
  focus: number;
  /** How small a world a step away is drawn; 1 draws them all alike. */
  shrink: number;
  /** How far each world has turned, by its place in the row. */
  turns: number[];
}

export const NO_ROW: Row = { pitch: 0, focus: 0, shrink: 1, turns: [] };

/** Sets the uniforms every pass needs to pose a row of worlds. */
export function applyRow(
  gl: WebGL2RenderingContext,
  uniform: (name: string) => WebGLUniformLocation | null,
  row: Row,
): void {
  gl.uniform1f(uniform("uRowPitch"), row.pitch);
  gl.uniform1f(uniform("uRowFocus"), row.focus);
  gl.uniform1f(uniform("uRowShrink"), row.shrink);
  // Uploaded whole: an array uniform is set in one call, and the tail of a shorter row is
  // zero, which is a world that has not turned.
  const turns = new Float32Array(ROW_LIMIT);
  turns.set(row.turns.slice(0, ROW_LIMIT));
  gl.uniform1fv(uniform("uRowTurn[0]"), turns);
}

export interface ShadingUniforms {
  lights: Light[];
  ambient: number;
  /** sRGB 0..1, as picked. Converted to linear here for the ambient term. */
  background: [number, number, number];
  roughness: number;
  specular: number;
  /** How far the surfaces are lit from within (the glass view's ribbons), 0 to 1. */
  glow?: number;
}

export interface UniformSetter {
  uniform(name: string): WebGLUniformLocation | null;
}

export function srgbToLinear(value: number): number {
  return value <= 0.04045 ? value / 12.92 : Math.pow((value + 0.055) / 1.055, 2.4);
}

/**
 * Uploads the light rig, with directions rotated into view space.
 *
 * Directions are stored world-space in the style (that is what the user edits and what
 * Blender wants) and the shader needs them relative to the camera, so the rotation happens
 * once per frame here rather than per fragment.
 */
export function setShadingUniforms(
  gl: WebGL2RenderingContext,
  program: UniformSetter,
  view: Mat4,
  uniforms: ShadingUniforms,
): void {
  const enabled = uniforms.lights.filter((light) => light.enabled).slice(0, MAX_LIGHTS);

  const directions = new Float32Array(MAX_LIGHTS * 3);
  const colors = new Float32Array(MAX_LIGHTS * 3);

  enabled.forEach((light, index) => {
    const [x, y, z] = light.direction;
    // A light that follows the view is already written in the camera's frame - x right, y
    // up, z towards the viewer - so it needs no rotation at all. That is the whole trick:
    // the lit side stays the side you are looking at, however the structure is turned,
    // which is how PyMOL's lights behave. A world-space light gets the rotation, so it
    // stays put in the scene and the molecule turns within it.
    // view is column-major; its upper-left 3x3 is the rotation.
    const vx = light.follow_view ? x : view[0] * x + view[4] * y + view[8] * z;
    const vy = light.follow_view ? y : view[1] * x + view[5] * y + view[9] * z;
    const vz = light.follow_view ? z : view[2] * x + view[6] * y + view[10] * z;
    const length = Math.hypot(vx, vy, vz) || 1;
    directions[index * 3] = vx / length;
    directions[index * 3 + 1] = vy / length;
    directions[index * 3 + 2] = vz / length;

    // Energy is irradiance, as it is for a Blender sun. Dividing by pi turns it into the
    // radiance a lambertian surface reflects, which is what the shader adds up.
    const scale = light.energy / Math.PI;
    colors[index * 3] = light.color[0] * scale;
    colors[index * 3 + 1] = light.color[1] * scale;
    colors[index * 3 + 2] = light.color[2] * scale;
  });

  gl.uniform3fv(program.uniform("uLightDirection"), directions);
  gl.uniform3fv(program.uniform("uLightColor"), colors);
  gl.uniform1i(program.uniform("uLightCount"), enabled.length);

  // The sky's contribution is its colour, which is the background.
  const ambient = uniforms.background.map(
    (channel) => srgbToLinear(channel) * uniforms.ambient,
  );
  gl.uniform3fv(program.uniform("uAmbient"), new Float32Array(ambient));
  gl.uniform1f(program.uniform("uRoughness"), uniforms.roughness);
  gl.uniform1f(program.uniform("uSpecular"), uniforms.specular);
  gl.uniform1f(program.uniform("uGlow"), uniforms.glow ?? 0);
}

/** Fragment outputs shared by the geometry passes, for the G-buffer. */
export const GBUFFER_OUTPUTS = `
layout(location = 0) out vec4 outColor;
layout(location = 1) out vec4 outNormal;
`;

/**
 * Normals are written to an 8-bit target, so they are packed into 0..1.
 *
 * Eight bits per axis is coarse for a normal, but it is only used for occlusion and edge
 * detection, both of which are spatial rather than specular - the banding never shows.
 * A float target would need EXT_color_buffer_float, which is one more thing to be absent.
 */
export const PACK_NORMAL_GLSL = `
vec4 packNormal(vec3 normal) {
  return vec4(normal * 0.5 + 0.5, 1.0);
}
`;

/**
 * The selection mask, read in a vertex shader by atom or residue id.
 *
 * A texture rather than an attribute, because the selection changes when you click a chain
 * in the outliner and the geometry does not: re-uploading a mask the size of the atom count
 * is a few hundred kilobytes, re-uploading the geometry is tens of megabytes. Two
 * dimensional because a structure can have more atoms than `MAX_TEXTURE_SIZE` allows in one
 * row.
 */
/**
 * A row of worlds, posed: where the camera stands along it, how big each is drawn, and how
 * far each has turned.
 *
 * The shelf of worlds stands them in a row and turns the one you are looking at. Moving the
 * camera instead would swing the whole row about, and it is the world that should be
 * turning, not the shelf. Which world a point belongs to is decided by where it is - the
 * worlds stand a fixed distance apart, so its place in the row is its position divided by
 * that - which costs an attribute and a lookup table less than carrying a part id through
 * three kinds of geometry.
 *
 * A turn *each* rather than one turn and a band that has it, because one angle cannot
 * describe two worlds: moving the band meant the world arriving jumped to however far the
 * last one had got and the world leaving snapped back upright, on every press of an arrow.
 * With a turn apiece the one you leave simply stops where it was and the one you arrive at
 * carries on from where it stopped, and nothing moves that you did not ask to move.
 */
export const ROW_LIMIT = 32;

export const ROW_GLSL = `
const int ROW_LIMIT = ${ROW_LIMIT};

/** How far apart the worlds stand. Zero means this is not a row and nothing is posed. */
uniform float uRowPitch;
/** How far along the row the camera has got, in world units. */
uniform float uRowFocus;
/** How small a world a whole step away from the camera is drawn. */
uniform float uRowShrink;
/** How far each world has turned, by its place in the row. */
uniform float uRowTurn[ROW_LIMIT];

/** The middle of the world a point belongs to. */
float rowCentre(float x) {
  return uRowPitch > 0.0 ? floor(x / uRowPitch + 0.5) * uRowPitch : 0.0;
}

/**
 * Which world a point belongs to, as an index into the turns.
 *
 * Clamped, so a shelf longer than the array still stands and is still sized right - the
 * worlds past the end simply do not turn, which is a shelf that reads rather than a shelf
 * that breaks.
 */
int rowSlot(float x) {
  if (uRowPitch <= 0.0) return 0;
  return int(clamp(floor(x / uRowPitch + 0.5), 0.0, float(ROW_LIMIT - 1)));
}

/**
 * How big a world is drawn, by how far it is from the one being looked at.
 *
 * Continuous in the camera's position rather than stepped by which world is selected, so
 * the one you are arriving at grows as you travel instead of popping when you land.
 */
float rowScale(float x) {
  if (uRowPitch <= 0.0) return 1.0;
  float away = clamp(abs(rowCentre(x) - uRowFocus) / uRowPitch, 0.0, 1.0);
  // Smoothed rather than straight: a size that changes at a constant rate has a corner in
  // it at each end of the journey, and the eye reads that corner as the slide catching.
  return mix(1.0, uRowShrink, smoothstep(0.0, 1.0, away));
}

vec3 rowPoint(vec3 p) {
  if (uRowPitch <= 0.0) return p;
  vec3 centre = vec3(rowCentre(p.x), 0.0, 0.0);
  // Shrunk about its own place, so a world stays where it stands while it gets smaller.
  vec3 q = centre + (p - centre) * rowScale(p.x);
  float angle = uRowTurn[rowSlot(p.x)];
  vec3 d = q - centre;
  float c = cos(angle);
  float s = sin(angle);
  return centre + vec3(d.x * c + d.z * s, d.y, -d.x * s + d.z * c);
}

vec3 rowDirection(vec3 v, float x) {
  if (uRowPitch <= 0.0) return v;
  float angle = uRowTurn[rowSlot(x)];
  float c = cos(angle);
  float s = sin(angle);
  return vec3(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
}
`;

export const SELECTION_GLSL = `
uniform sampler2D uSelection;
uniform int uSelectionWidth;

float selectedById(int id) {
  if (uSelectionWidth <= 0) return 0.0;
  ivec2 at = ivec2(id % uSelectionWidth, id / uSelectionWidth);
  return texelFetch(uSelection, at, 0).r;
}
`;

/** Where the mask goes in the G-buffer: the alpha of the normal target, otherwise unused. */
export const SELECTION_OUTPUT_GLSL = `
void writeSelection(float selected) {
  outNormal.a = selected;
}
`;

/**
 * Picking: the same geometry, drawn once into a one-pixel buffer with ids for colours.
 *
 * A separate pass rather than a fourth channel in the G-buffer, because it only runs on a
 * click - and a separate *program* was not needed either: the geometry shaders already
 * carry the id they would have to write, so a uniform switches the fragment output over.
 * That keeps one implementation of the sphere impostor rather than two that can drift.
 *
 * Two bits of tag and thirty of index, because a sphere knows which *atom* it is and a
 * ribbon vertex knows only which *residue*, and the reader has to be able to tell which
 * kind of number came back. Encoded little-endian into RGBA8, which is the only render
 * target format guaranteed to be readable.
 */
export const PICK_GLSL = `
uniform int uPicking;      // 0 draws the scene, 1 draws ids
uniform int uPickKind;     // 1 = atom, 2 = residue, 3 = a bond in the graph view

vec4 encodePick(uint id) {
  uint tagged = (uint(uPickKind) << 30) | (id & 0x3FFFFFFFu);
  return vec4(
    float(tagged & 0xFFu),
    float((tagged >> 8) & 0xFFu),
    float((tagged >> 16) & 0xFFu),
    float((tagged >> 24) & 0xFFu)
  ) / 255.0;
}
`;

/** The mask level of a selected atom or residue: the shader reads it as 1.0. */
export const MASK_SELECTED = 255;
/** The mask level of a hovered one: read as ~0.5, and drawn in the hover colour. */
export const MASK_HOVERED = 128;

/** What a picked pixel means. `null` is the background. */
export interface Picked {
  /** `edge` is a bond in the graph view, by its place in the graph's list of them. */
  kind: "atom" | "residue" | "edge";
  index: number;
}

/** Decodes the four bytes a picking read returns. */
export function decodePick(pixel: Uint8Array): Picked | null {
  const tagged =
    pixel[0] | (pixel[1] << 8) | (pixel[2] << 16) | ((pixel[3] & 0xff) << 24);
  const tag = (tagged >>> 30) & 0x3;
  const index = tagged & 0x3fffffff;
  if (tag === 1) return { kind: "atom", index };
  if (tag === 2) return { kind: "residue", index };
  if (tag === 3) return { kind: "edge", index };
  return null;
}

/** The texture unit the selection mask is bound to, in every pass. */
export const SELECTION_TEXTURE_UNIT = 4;

/**
 * A selection mask as a texture, kept between uploads.
 *
 * Its storage and its staging buffer are reused, so pointing at another residue of a
 * three-million-atom capsid writes the bytes again rather than allocating two buffers
 * of that size and a new texture each time - which was a frame's worth of work on every
 * hover. A kind whose inputs are the same arrays as last time is not touched at all.
 */
export class MaskTexture {
  private texture: WebGLTexture | null = null;
  private staging = new Uint8Array(0);
  private textureWidth = 0;
  private textureHeight = 0;
  private last: [number, Uint8Array | null, Uint8Array | null] | null = null;
  /** The width the shaders are given: 0 when nothing is marked. */
  width = 0;

  constructor(private gl: WebGL2RenderingContext) {}

  get handle(): WebGLTexture | null {
    return this.texture;
  }

  /** Merges the selection over the hover and uploads it, unless neither changed. */
  update(length: number, selected: Uint8Array | null, hovered: Uint8Array | null): void {
    const last = this.last;
    if (last && last[0] === length && last[1] === selected && last[2] === hovered) return;
    this.last = [length, selected, hovered];
    const gl = this.gl;
    this.texture ??= gl.createTexture();
    gl.activeTexture(gl.TEXTURE0 + SELECTION_TEXTURE_UNIT);
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    if (length === 0 || (!selected && !hovered)) {
      // The shader's `uSelectionWidth <= 0` path never samples, but the sampler still has
      // to be complete: one black pixel when there is no texture yet.
      if (this.textureWidth === 0) {
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, 1, 1, 0, gl.RED, gl.UNSIGNED_BYTE, new Uint8Array(1));
        this.parameters();
      }
      this.width = 0;
      return;
    }
    const width = Math.min(1024, length);
    const height = Math.ceil(length / width);
    if (this.staging.length !== width * height) this.staging = new Uint8Array(width * height);
    const merged = this.staging;
    merged.fill(0);
    // Levels, not flags: an R8 texture is normalised, so a mask of ones would arrive as
    // 1/255. Two levels because the one channel carries both the selection (255) and
    // the hover (128); see `MASK_SELECTED` and `MASK_HOVERED`.
    if (hovered) {
      const end = Math.min(length, hovered.length);
      for (let index = 0; index < end; index += 1) if (hovered[index] !== 0) merged[index] = MASK_HOVERED;
    }
    if (selected) {
      const end = Math.min(length, selected.length);
      for (let index = 0; index < end; index += 1) if (selected[index] !== 0) merged[index] = MASK_SELECTED;
    }
    if (width !== this.textureWidth || height !== this.textureHeight) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, width, height, 0, gl.RED, gl.UNSIGNED_BYTE, merged);
      this.parameters();
      this.textureWidth = width;
      this.textureHeight = height;
    } else {
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, width, height, gl.RED, gl.UNSIGNED_BYTE, merged);
    }
    this.width = width;
  }

  /** Forgets what was uploaded, so the next update writes again (a new scene, say). */
  invalidate(): void {
    this.last = null;
  }

  private parameters(): void {
    const gl = this.gl;
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  }
}
