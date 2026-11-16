// Sphere impostors.
//
// Each atom is one camera-facing quad; the fragment shader intersects the view ray with
// the sphere and writes the true depth. The alternative - a tessellated icosphere per
// atom - is either faceted at close range or millions of triangles for a whole protein.
// Impostors are pixel-exact at any zoom, and 200k atoms is 200k quads.

import { createBuffer, createProgram, type Program } from "./gl";
import {
  GBUFFER_OUTPUTS,
  PACK_NORMAL_GLSL,
  PICK_GLSL,
  SELECTION_GLSL,
  SELECTION_OUTPUT_GLSL,
  SELECTION_TEXTURE_UNIT,
  setShadingUniforms,
  SHADING_GLSL,
  type ShadingUniforms,
  ROW_GLSL,
  NO_ROW,
  applyRow,
  type Row,
} from "./shading";

const VERTEX = `#version 300 es
precision highp float;

in vec2 aCorner;      // per-vertex quad corner in [-1, 1]
in vec3 aCenter;      // per-instance
in float aRadius;     // per-instance
in vec3 aColor;       // per-instance
in uint aAtom;        // per-instance, for the selection mask
in uint aLabel;       // per-instance: up to three glyphs, six bits each; 0 for none

uniform mat4 uView;
uniform mat4 uProj;

${SELECTION_GLSL}
${ROW_GLSL}

out vec3 vCenterView;
out vec3 vRayPoint;
out vec3 vColor;
out float vRadius;
out float vSelected;
flat out uint vId;
flat out uint vLabel;

void main() {
  vec3 centerWorld = rowPoint(aCenter);
  // The radius shrinks with the world it belongs to; a full-size atom on a small world
  // would swallow it.
  float radius = aRadius * rowScale(aCenter.x);
  vec3 centerView = (uView * vec4(centerWorld, 1.0)).xyz;
  float d = length(centerView);

  // The quad has to contain the sphere's silhouette, and the silhouette is not centred on
  // the sphere's projected centre: it is the circle where the tangent cone from the eye
  // touches the sphere, which lies closer to the eye and, for an atom away from the middle
  // of the screen, off to one side of the centre's projection. A quad built in the image
  // plane around that projection therefore cuts the sphere's left and right edges - worse
  // the further off-axis the atom is, which is why zooming out made it show up on the
  // atoms around the rim.
  //
  // So the quad is built in the silhouette's own plane, perpendicular to the eye-to-centre
  // axis rather than to the view axis. The circle has centre |C| - r^2/|C| along that axis
  // and radius r*sqrt(|C|^2 - r^2)/|C|, and a square around it in that plane projects to a
  // region containing the whole silhouette. Exactly, not approximately.
  vec3 pointView;
  if (d > radius * 1.02) {
    vec3 axis = centerView / d;
    float silhouetteDistance = d - radius * radius / d;
    float silhouetteRadius = radius * sqrt(max(d * d - radius * radius, 1e-4)) / d;
    // Any pair of directions across the axis will do; picking the helper by the axis's
    // smallest component keeps the cross product away from zero.
    vec3 helper = abs(axis.z) < 0.9 ? vec3(0.0, 0.0, 1.0) : vec3(1.0, 0.0, 0.0);
    vec3 right = normalize(cross(helper, axis));
    vec3 up = cross(axis, right);
    pointView = axis * silhouetteDistance
        + (right * aCorner.x + up * aCorner.y) * silhouetteRadius;
  } else {
    // The eye is inside this atom, or as good as: there is no silhouette to bound, and the
    // fragment shader discards these anyway.
    pointView = centerView + vec3(aCorner * radius * 4.0, 0.0);
  }

  vCenterView = centerView;
  vRayPoint = pointView;
  vColor = aColor;
  vRadius = radius;
  vSelected = selectedById(int(aAtom));
  vId = aAtom;
  vLabel = aLabel;
  gl_Position = uProj * vec4(pointView, 1.0);
}
`;

const FRAGMENT = `#version 300 es
precision highp float;

in vec3 vCenterView;
in vec3 vRayPoint;
in vec3 vColor;
in float vRadius;
in float vSelected;
flat in uint vId;
flat in uint vLabel;

uniform mat4 uProj;
uniform sampler2D uGlyphs;

// How much of this point on the sphere's face is ink, for the label written on it.
//
// The face is read in view space - across the sphere as the eye sees it - so the letters
// stay upright and face the camera however the scene is turned. Up to three glyphs side
// by side, each six bits of the label; the atlas is eight cells a side.
float labelInk(vec2 face) {
  if (vLabel == 0u) return 0.0;
  float count = (vLabel >> 12) != 0u ? 3.0 : ((vLabel >> 6) != 0u ? 2.0 : 1.0);
  float cellHeight = count == 1.0 ? 1.15 : (count == 2.0 ? 0.85 : 0.64);
  float cellWidth = cellHeight * 0.66;
  vec2 at = vec2(face.x / cellWidth + count * 0.5, face.y / cellHeight + 0.5);
  if (at.x < 0.0 || at.x >= count || at.y < 0.0 || at.y > 1.0) return 0.0;
  uint glyph = (vLabel >> (6u * uint(floor(at.x)))) & 63u;
  if (glyph == 0u) return 0.0;
  // A cell holds its glyph centred with room either side; the middle of it is the letter.
  vec2 inCell = vec2(0.5 + (fract(at.x) - 0.5) * 0.66, 1.0 - at.y);
  vec2 cell = vec2(float(glyph % 8u), float(glyph / 8u));
  return texture(uGlyphs, (cell + inCell) / 8.0).r;
}

${SHADING_GLSL}
${PACK_NORMAL_GLSL}
${GBUFFER_OUTPUTS}
${SELECTION_OUTPUT_GLSL}
${PICK_GLSL}

void main() {
  // The camera sits at the origin in view space, so the ray is just the fragment's
  // direction.
  vec3 dir = normalize(vRayPoint);
  float b = dot(dir, vCenterView);
  float c = dot(vCenterView, vCenterView) - vRadius * vRadius;
  float disc = b * b - c;
  if (disc < 0.0) discard;             // the quad's corners fall outside the sphere

  float t = b - sqrt(disc);
  if (t < 0.0) discard;                // the camera is inside this atom

  vec3 hit = dir * t;
  vec3 normal = normalize(hit - vCenterView);

  vec4 clip = uProj * vec4(hit, 1.0);
  gl_FragDepth = (clip.z / clip.w) * 0.5 + 0.5;

  if (uPicking == 1) {
    outColor = encodePick(vId);
    outNormal = vec4(0.0);
    return;
  }

  vec3 shaded = shade(vColor, normal, -dir);
  float ink = labelInk(normal.xy);
  if (ink > 0.0) {
    // Dark letters on a light sphere and light on a dark one.
    float luma = dot(vColor, vec3(0.2126, 0.7152, 0.0722));
    vec3 inkColor = luma > 0.16 ? vec3(0.015) : vec3(0.95);
    shaded = mix(shaded, inkColor, ink * 0.92);
  }
  outColor = vec4(shaded, 1.0);
  outNormal = packNormal(normal);
  writeSelection(vSelected);
}
`;

/** The glyphs a label can use. The first is "none", so a label of zero is blank. */
const GLYPHS = "\0ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-?";
const GLYPH_TEXTURE_UNIT = 5;

/**
 * Packs up to three characters into one label: six bits each, first character lowest.
 * Anything the atlas has no glyph for is written `?`, and a longer name is cut to three.
 */
export function packLabel(text: string): number {
  let packed = 0;
  const letters = text.toUpperCase().slice(0, 3);
  for (let index = 0; index < letters.length; index += 1) {
    const found = GLYPHS.indexOf(letters[index]);
    packed |= (found > 0 ? found : GLYPHS.indexOf("?")) << (6 * index);
  }
  return packed >>> 0;
}

/** The atlas: eight cells a side, a glyph centred in each, white on black. */
function glyphAtlas(gl: WebGL2RenderingContext): WebGLTexture | null {
  const size = 512;
  const cell = size / 8;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext("2d");
  if (!context) return null;
  context.fillStyle = "#000";
  context.fillRect(0, 0, size, size);
  context.fillStyle = "#fff";
  context.font = `700 ${Math.round(cell * 0.86)}px -apple-system, "Helvetica Neue", Arial, sans-serif`;
  context.textAlign = "center";
  context.textBaseline = "middle";
  for (let index = 1; index < GLYPHS.length; index += 1) {
    context.fillText(GLYPHS[index], (index % 8) * cell + cell / 2, Math.floor(index / 8) * cell + cell / 2 + cell * 0.04);
  }
  const texture = gl.createTexture();
  gl.activeTexture(gl.TEXTURE0 + GLYPH_TEXTURE_UNIT);
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, gl.RED, gl.UNSIGNED_BYTE, canvas);
  // Mipmapped, so a letter on a sphere a few pixels across fades out instead of sparkling.
  gl.generateMipmap(gl.TEXTURE_2D);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return texture;
}

export class SpherePass {
  private program: Program;
  private vao: WebGLVertexArrayObject;
  private instanceBuffer: WebGLBuffer;
  private colorBuffer: WebGLBuffer;
  private atomBuffer: WebGLBuffer;
  private labelBuffer: WebGLBuffer | null = null;
  private labelLocation = -1;
  private glyphs: WebGLTexture | null = null;
  private count = 0;
  private capacity = 0;
  /**
   * What the ids on these spheres are: 1 for atoms, which is what a structure's spheres
   * carry, 2 for residues, which is what the graph view's do - one sphere a component.
   */
  pickKind: 1 | 2 = 1;

  constructor(private gl: WebGL2RenderingContext) {
    this.program = createProgram(gl, "spheres", VERTEX, FRAGMENT);

    const vao = gl.createVertexArray();
    if (!vao) throw new Error("could not create sphere VAO");
    this.vao = vao;
    gl.bindVertexArray(vao);

    const corners = new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]);
    createBuffer(gl, gl.ARRAY_BUFFER, corners);
    const cornerLocation = this.program.attribute("aCorner");
    gl.enableVertexAttribArray(cornerLocation);
    gl.vertexAttribPointer(cornerLocation, 2, gl.FLOAT, false, 0, 0);

    // Interleaved centre + radius: one buffer, one upload, and the four floats land in a
    // single cache line per atom.
    this.instanceBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array(0), gl.DYNAMIC_DRAW);
    const centerLocation = this.program.attribute("aCenter");
    gl.enableVertexAttribArray(centerLocation);
    gl.vertexAttribPointer(centerLocation, 3, gl.FLOAT, false, 16, 0);
    gl.vertexAttribDivisor(centerLocation, 1);
    const radiusLocation = this.program.attribute("aRadius");
    gl.enableVertexAttribArray(radiusLocation);
    gl.vertexAttribPointer(radiusLocation, 1, gl.FLOAT, false, 16, 12);
    gl.vertexAttribDivisor(radiusLocation, 1);

    // Colour is separate because it changes on its own: switching scheme re-uploads
    // colours without touching a byte of geometry.
    this.colorBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array(0), gl.DYNAMIC_DRAW);
    const colorLocation = this.program.attribute("aColor");
    gl.enableVertexAttribArray(colorLocation);
    gl.vertexAttribPointer(colorLocation, 3, gl.FLOAT, false, 0, 0);
    gl.vertexAttribDivisor(colorLocation, 1);

    // Which atom each sphere is, so the selection mask can be read by id. Uploaded with
    // the geometry and never again; the mask itself is the thing that changes.
    this.atomBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Uint32Array(0), gl.DYNAMIC_DRAW);
    const atomLocation = this.program.attribute("aAtom");
    gl.enableVertexAttribArray(atomLocation);
    gl.vertexAttribIPointer(atomLocation, 1, gl.UNSIGNED_INT, 0, 0);
    gl.vertexAttribDivisor(atomLocation, 1);

    // No label buffer until one is given: the attribute then reads its constant, zero,
    // which is "nothing written on it" - an atom's sphere has no letter.
    this.labelLocation = this.program.attribute("aLabel");
    gl.disableVertexAttribArray(this.labelLocation);
    gl.vertexAttribI4ui(this.labelLocation, 0, 0, 0, 0);

    gl.bindVertexArray(null);
  }

  /**
   * What is written on each sphere, one packed label per instance - see `packLabel`.
   * The atlas is drawn the first time anything is labelled.
   */
  setLabels(labels: Uint32Array): void {
    const gl = this.gl;
    gl.bindVertexArray(this.vao);
    if (!this.labelBuffer) {
      this.labelBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Uint32Array(0), gl.DYNAMIC_DRAW);
      gl.bufferData(gl.ARRAY_BUFFER, labels, gl.DYNAMIC_DRAW);
      gl.enableVertexAttribArray(this.labelLocation);
      gl.vertexAttribIPointer(this.labelLocation, 1, gl.UNSIGNED_INT, 0, 0);
      gl.vertexAttribDivisor(this.labelLocation, 1);
    } else {
      gl.bindBuffer(gl.ARRAY_BUFFER, this.labelBuffer);
      gl.bufferData(gl.ARRAY_BUFFER, labels, gl.DYNAMIC_DRAW);
    }
    gl.bindVertexArray(null);
    this.glyphs ??= glyphAtlas(gl);
  }

  /** `instances` is [x, y, z, radius] per sphere. */
  setInstances(instances: Float32Array): void {
    const gl = this.gl;
    this.count = instances.length / 4;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.instanceBuffer);
    if (this.count > this.capacity) {
      gl.bufferData(gl.ARRAY_BUFFER, instances, gl.DYNAMIC_DRAW);
      this.capacity = this.count;
    } else {
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, instances);
    }
  }

  setColors(colors: Float32Array): void {
    const gl = this.gl;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.colorBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, colors, gl.DYNAMIC_DRAW);
  }

  /** The atom each sphere stands for, for the selection mask. */
  setAtoms(atoms: Uint32Array): void {
    const gl = this.gl;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.atomBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, atoms, gl.DYNAMIC_DRAW);
  }

  draw(
    view: Float32Array,
    projection: Float32Array,
    shading: ShadingUniforms,
    selectionWidth = 0,
    picking = false,
    row: Row = NO_ROW,
  ): void {
    if (this.count === 0) return;
    const gl = this.gl;
    this.program.use();
    gl.bindVertexArray(this.vao);
    gl.uniformMatrix4fv(this.program.uniform("uView"), false, view);
    gl.uniformMatrix4fv(this.program.uniform("uProj"), false, projection);
    applyRow(gl, (name) => this.program.uniform(name), row);
    gl.uniform1i(this.program.uniform("uSelection"), SELECTION_TEXTURE_UNIT);
    gl.uniform1i(this.program.uniform("uSelectionWidth"), selectionWidth);
    gl.uniform1i(this.program.uniform("uPicking"), picking ? 1 : 0);
    // A sphere and a stick end know their atom; a ribbon vertex knows its residue.
    gl.uniform1i(this.program.uniform("uPickKind"), this.pickKind);
    gl.activeTexture(gl.TEXTURE0 + GLYPH_TEXTURE_UNIT);
    gl.bindTexture(gl.TEXTURE_2D, this.glyphs);
    gl.uniform1i(this.program.uniform("uGlyphs"), GLYPH_TEXTURE_UNIT);
    setShadingUniforms(gl, this.program, view, shading);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, this.count);
    gl.bindVertexArray(null);
  }
}
