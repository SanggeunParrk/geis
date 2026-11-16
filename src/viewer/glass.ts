// The glass view's envelope: a molecule's smooth outer surface, drawn over its ribbons
// as a solid piece of glass.
//
// After the scene is composited, the picture under the envelope is copied aside, and the
// envelope is drawn over it as glass does it rather than as a tinted film:
//
// - Refraction. What is seen through the surface is the picture behind it, displaced
//   along the surface's slope - more where the glass is thicker - so the ribbons bend at
//   the shell's edge the way a thing in a glass bead does. This, more than any tint, is
//   what makes a surface read as glass.
// - Absorption. The glass is tinted by how much of it the light passed through: the
//   thickness, from the envelope's back faces drawn first into a depth texture of their
//   own. Thin at the middle of a face, so clear; thick at the rim, so coloured - in the
//   colour of the atoms under it, which is its chain's unless it was given its own.
// - Reflection. Fresnel by Schlick, of a soft studio: brighter above, darker below, two
//   sharp highlights. Nearly nothing face on, nearly a mirror at the rim.
//
// Only the nearest layer is drawn - depth first, then colour where it is nearest - so a
// fold of the surface does not add up layer on layer.

import { createProgram, type Program } from "./gl";
import type { Mat4 } from "./mat";
import { applyRow, NO_ROW, ROW_GLSL, type Row } from "./shading";

const VERTEX = `#version 300 es
precision highp float;
layout(location = 0) in vec3 aPosition;
layout(location = 1) in vec3 aNormal;
layout(location = 2) in vec3 aColour;
uniform mat4 uView;
uniform mat4 uProjection;
out vec3 vNormal;
out vec3 vPosition;
out vec3 vColour;
${ROW_GLSL}
void main() {
  // Posed with the shelf's row, as the rest of the geometry is.
  vec4 position = uView * vec4(rowPoint(aPosition), 1.0);
  vPosition = position.xyz;
  vNormal = mat3(uView) * rowDirection(aNormal, aPosition.x);
  vColour = aColour;
  gl_Position = uProjection * position;
}`;

/** The back faces' depth, for the thickness: nothing is written but depth. */
const DEPTH_FRAGMENT = `#version 300 es
precision highp float;
out vec4 colour;
void main() { colour = vec4(0.0); }`;

/**
 * The glass itself, shared by the viewport's glass and the home screen's (`cellRenderer`),
 * so the two are one material: what it lets through, bent and absorbed, and what it
 * reflects. Only the surroundings differ - a white studio in the viewport, the cell's blue
 * glow on the home screen - as the same glass would look in two rooms.
 *
 * - `glassBend`: how far, as a fraction of the screen's height, what is behind is displaced
 *   along the surface's slope: more through thicker glass and at a steeper face.
 * - `glassLight`: the colour, from what was seen behind (already displaced), the view-space
 *   normal and direction to the eye, the tint (the colour of the atoms under the glass),
 *   the thickness in angstroms, the surroundings' sky and floor, and how strong the
 *   highlights are.
 */
export const GLASS_GLSL = `
float glassBend(float facing, float thickness) {
  return 0.035 * (0.35 + 0.65 * smoothstep(0.0, 18.0, thickness)) * (1.0 - facing * 0.55);
}

vec3 glassLight(vec3 behind, vec3 n, vec3 eye, vec3 tint, float thickness, vec3 sky, vec3 floor_, float shineScale) {
  float facing = clamp(dot(n, eye), 0.0, 1.0);
  // Coloured by the path through it: a thin face is clear, a thick rim takes the tint.
  vec3 through = exp(-(vec3(1.0) - tint) * 0.012 * thickness);
  // The surroundings reflected, and two lamps.
  vec3 r = reflect(-eye, n);
  vec3 around = mix(floor_, sky, smoothstep(-0.35, 0.75, r.y));
  around = mix(around, tint * 1.2, 0.25);
  vec3 key = normalize(vec3(-0.45, 0.75, 0.55));
  vec3 fill = normalize(vec3(0.7, -0.15, 0.6));
  float shine = pow(max(dot(r, key), 0.0), 90.0) * 0.9 + pow(max(dot(r, fill), 0.0), 40.0) * 0.25;
  // Schlick's Fresnel, made a little stronger: nearly nothing face on, nearly a mirror at
  // the rim.
  float fresnel = clamp((0.03 + 0.97 * pow(1.0 - facing, 5.0)) * 1.6, 0.0, 0.92);
  vec3 lit = behind * through * (1.0 - fresnel) + around * fresnel + vec3(shine * shineScale);
  // A thin bright line where the surface turns away: the edge a glass object is drawn by.
  return lit + tint * 0.35 * smoothstep(0.82, 0.98, 1.0 - facing);
}
`;

/** The viewport's surroundings, reflected in its glass: a bright studio sky, a dim floor. */
const STUDIO_SKY = "vec3(1.05, 1.08, 1.15)";
const STUDIO_FLOOR = "vec3(0.20, 0.24, 0.32)";

const FRAGMENT = `#version 300 es
precision highp float;
in vec3 vNormal;
in vec3 vPosition;
in vec3 vColour;
uniform sampler2D uScene;
uniform sampler2D uBack;
uniform vec2 uSize;          // the drawing buffer, in pixels
uniform mat4 uProjection;
uniform float uOpacity;
out vec4 colour;
${GLASS_GLSL}

float linearDepth(float depth) {
  float ndc = depth * 2.0 - 1.0;
  return uProjection[3][2] / (ndc + uProjection[2][2]);
}

void main() {
  vec3 n = normalize(vNormal);
  vec3 eye = normalize(-vPosition);
  if (dot(n, eye) < 0.0) n = -n;
  float facing = clamp(dot(n, eye), 0.0, 1.0);
  vec2 uv = gl_FragCoord.xy / uSize;

  // How much glass the light behind has come through.
  float front = -vPosition.z;
  float back = linearDepth(texture(uBack, uv).r);
  float thickness = clamp(back - front, 0.0, 40.0);

  // A fraction of the screen, so the bend looks alike at any size.
  vec2 offset = n.xy * glassBend(facing, thickness) * vec2(uSize.y / uSize.x, 1.0);
  vec4 behind = texture(uScene, clamp(uv - offset, vec2(0.001), vec2(0.999)));
  // A little of the colours split apart at the rim, as a real lens does.
  float spread = 0.12 * (1.0 - facing);
  behind.r = texture(uScene, clamp(uv - offset * (1.0 + spread), vec2(0.001), vec2(0.999))).r;
  behind.b = texture(uScene, clamp(uv - offset * (1.0 - spread), vec2(0.001), vec2(0.999))).b;

  vec3 lit = glassLight(behind.rgb, n, eye, vColour, thickness, ${STUDIO_SKY}, ${STUDIO_FLOOR}, 1.0);
  float fresnel = clamp((0.03 + 0.97 * pow(1.0 - facing, 5.0)) * 1.6, 0.0, 0.92);
  float coverage = max(behind.a, fresnel);
  // Faded towards what is behind it, unbent, when the glass is turned down.
  colour = mix(texture(uScene, uv), vec4(lit, coverage), uOpacity);
}`;

export class GlassPass {
  private program: Program;
  private depthProgram: Program;
  private vao: WebGLVertexArrayObject;
  private buffers: WebGLBuffer[] = [];
  private count = 0;
  /** The picture under the glass, copied from the screen, and the back faces' depth. */
  private scene: WebGLTexture;
  private back: WebGLTexture;
  private backFramebuffer: WebGLFramebuffer;
  private size: [number, number] = [0, 0];
  opacity = 1;

  constructor(private gl: WebGL2RenderingContext) {
    this.program = createProgram(gl, "glass", VERTEX, FRAGMENT);
    this.depthProgram = createProgram(gl, "glass depth", VERTEX, DEPTH_FRAGMENT);
    this.vao = gl.createVertexArray()!;
    this.scene = gl.createTexture()!;
    this.back = gl.createTexture()!;
    this.backFramebuffer = gl.createFramebuffer()!;
  }

  get empty(): boolean {
    return this.count === 0;
  }

  /**
   * The scene's glass: positions, normals as signed bytes and colours as bytes, four to a
   * vertex (the fourth padding), as the ribbon's are sent. Each vertex is in the colour of
   * the atom under it - its chain's, by default - and that is what the glass absorbs the
   * rest of.
   */
  setMesh(positions: Float32Array, normals: Int8Array, colours: Uint8Array, indices: Uint32Array): void {
    const gl = this.gl;
    for (const buffer of this.buffers) gl.deleteBuffer(buffer);
    this.buffers = [];
    gl.bindVertexArray(this.vao);
    const attribute = (location: number, data: ArrayBufferView, size: number, type: number, normalised: boolean, stride: number) => {
      const buffer = gl.createBuffer();
      this.buffers.push(buffer);
      gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      gl.enableVertexAttribArray(location);
      gl.vertexAttribPointer(location, size, type, normalised, stride, 0);
    };
    attribute(0, positions, 3, gl.FLOAT, false, 0);
    attribute(1, normals, 3, gl.BYTE, true, 4);
    attribute(2, colours, 3, gl.UNSIGNED_BYTE, true, 4);
    const index = gl.createBuffer();
    this.buffers.push(index);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, index);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, indices, gl.STATIC_DRAW);
    gl.bindVertexArray(null);
    this.count = indices.length;
  }

  /** The two textures at the drawing buffer's size, made again only when it changes. */
  private ensureTargets(width: number, height: number): void {
    if (this.size[0] === width && this.size[1] === height) return;
    const gl = this.gl;
    this.size = [width, height];
    gl.bindTexture(gl.TEXTURE_2D, this.scene);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    for (const texture of [this.scene, this.back]) {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, texture === this.back ? gl.NEAREST : gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, texture === this.back ? gl.NEAREST : gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    }
    gl.bindTexture(gl.TEXTURE_2D, this.back);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.DEPTH_COMPONENT24, width, height, 0, gl.DEPTH_COMPONENT, gl.UNSIGNED_INT, null);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.backFramebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.TEXTURE_2D, this.back, 0);
    gl.drawBuffers([gl.NONE]);
    gl.readBuffer(gl.NONE);
    gl.bindTexture(gl.TEXTURE_2D, null);
    // Back to the screen, which the picture is copied from next.
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  }

  /**
   * Over the picture on the screen: the nearest layer of the envelope, as glass. The
   * screen's framebuffer must be bound, at `width` by `height`.
   */
  draw(view: Mat4, projection: Mat4, width: number, height: number, row: Row = NO_ROW): void {
    if (this.count === 0) return;
    const gl = this.gl;
    this.ensureTargets(width, height);

    // The picture as it stands, before any glass is on it.
    gl.bindTexture(gl.TEXTURE_2D, this.scene);
    gl.copyTexSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 0, 0, width, height);
    gl.bindTexture(gl.TEXTURE_2D, null);

    gl.bindVertexArray(this.vao);
    gl.enable(gl.DEPTH_TEST);
    gl.enable(gl.CULL_FACE);
    gl.clearDepth(1);

    // The back faces' depth: the far side of the glass, for its thickness.
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.backFramebuffer);
    gl.viewport(0, 0, width, height);
    gl.clear(gl.DEPTH_BUFFER_BIT);
    gl.cullFace(gl.FRONT);
    gl.depthFunc(gl.LESS);
    this.depthProgram.use();
    gl.uniformMatrix4fv(this.depthProgram.uniform("uView"), false, view);
    gl.uniformMatrix4fv(this.depthProgram.uniform("uProjection"), false, projection);
    applyRow(gl, (name) => this.depthProgram.uniform(name), row);
    gl.drawElements(gl.TRIANGLES, this.count, gl.UNSIGNED_INT, 0);

    // The front: depth alone, then the glass where it is nearest.
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, width, height);
    gl.clear(gl.DEPTH_BUFFER_BIT);
    gl.cullFace(gl.BACK);
    gl.colorMask(false, false, false, false);
    gl.drawElements(gl.TRIANGLES, this.count, gl.UNSIGNED_INT, 0);
    gl.colorMask(true, true, true, true);
    gl.depthFunc(gl.LEQUAL);
    gl.depthMask(false);

    this.program.use();
    gl.uniformMatrix4fv(this.program.uniform("uView"), false, view);
    gl.uniformMatrix4fv(this.program.uniform("uProjection"), false, projection);
    gl.uniform1f(this.program.uniform("uOpacity"), this.opacity);
    applyRow(gl, (name) => this.program.uniform(name), row);
    gl.uniform2f(this.program.uniform("uSize"), width, height);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.scene);
    gl.uniform1i(this.program.uniform("uScene"), 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.back);
    gl.uniform1i(this.program.uniform("uBack"), 1);
    gl.drawElements(gl.TRIANGLES, this.count, gl.UNSIGNED_INT, 0);

    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, null);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, null);
    gl.depthMask(true);
    gl.depthFunc(gl.LESS);
    gl.disable(gl.CULL_FACE);
    gl.bindVertexArray(null);
  }
}
