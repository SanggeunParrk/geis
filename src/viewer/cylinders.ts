// Bonds, as instanced cylinders.
//
// Real geometry rather than impostors here, deliberately: a ray-cylinder impostor needs
// caps, end-on cases, and its own depth write, for a shape whose silhouette a 10-sided
// prism already matches at the width a bond is drawn. Spheres at the atom positions cover
// the joints, so the cylinders need no caps at all.

import { createBuffer, createProgram, type Program } from "./gl";
import {
  GBUFFER_OUTPUTS,
  PICK_GLSL,
  SELECTION_GLSL,
  SELECTION_OUTPUT_GLSL,
  SELECTION_TEXTURE_UNIT,
  PACK_NORMAL_GLSL,
  setShadingUniforms,
  SHADING_GLSL,
  type ShadingUniforms,
  ROW_GLSL,
  NO_ROW,
  applyRow,
  type Row,
} from "./shading";

const SEGMENTS = 16;
/**
 * The ring is drawn *around* the true circle, not inside it.
 *
 * A polygon inscribed in the circle has its flats at r*cos(pi/N) - for ten segments, 5%
 * inside the radius - and the sphere at each joint, which is exact, showed through every
 * flat as a thin sliver: the "trace of a ball" at every atom, plain once you zoomed in.
 * Scaling the ring by 1/cos(pi/N) puts the flats *on* the circle and the vertices just
 * outside it, so the stick covers the sphere's equator everywhere and the joint is one
 * surface.
 */
const CIRCUMSCRIBE = 1 / Math.cos(Math.PI / SEGMENTS);

const VERTEX = `#version 300 es
precision highp float;

in vec3 aLocal;       // per-vertex: unit circle x, y and z in [0, 1] along the bond
in vec2 aNormal;      // per-vertex: outward direction on the unit circle
in vec3 aStart;       // per-instance
in vec3 aEnd;         // per-instance
in float aRadius;     // per-instance: this bond's half-thickness
in vec3 aColorA;      // per-instance: colour of the start atom
in vec3 aColorB;      // per-instance: colour of the end atom
in uvec2 aAtoms;      // per-instance: the atoms at the two ends

uniform mat4 uView;
uniform mat4 uProj;

${SELECTION_GLSL}
${ROW_GLSL}

out vec3 vNormalView;
out vec3 vPositionView;
flat out vec3 vColorA;
flat out vec3 vColorB;
out float vAlong;      // 0 at the start atom, 1 at the end atom
flat out vec2 vSelected;   // the selection of the start atom and of the end atom
flat out uvec2 vAtoms;     // the two atoms, start and end

void main() {
  vec3 startWorld = rowPoint(aStart);
  vec3 endWorld = rowPoint(aEnd);
  float radius = aRadius * rowScale(aStart.x);
  vec3 axis = endWorld - startWorld;
  float len = length(axis);
  vec3 w = len > 1e-6 ? axis / len : vec3(0.0, 0.0, 1.0);

  // Any vector not parallel to the axis gives a usable frame. Picking the helper by which
  // component of w is smallest avoids the degenerate cross product that would collapse
  // the cylinder for bonds aligned with an axis.
  vec3 helper = abs(w.y) < 0.99 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
  vec3 u = normalize(cross(helper, w));
  vec3 v = cross(w, u);

  vec3 world = startWorld + (u * aLocal.x + v * aLocal.y) * radius + axis * aLocal.z;
  vec3 normalWorld = u * aNormal.x + v * aNormal.y;

  // The bond takes each atom's colour over its own half, which is how a viewer shows what
  // is bonded to what without drawing labels. Decided per *fragment*, not per vertex: the
  // strip has vertices only at the two ends, so a colour chosen here would be interpolated
  // across the whole bond and come out as a gradient - which is what it did.
  vColorA = aColorA;
  vColorB = aColorB;
  vAlong = aLocal.z;
  // A bond is selected when the atom whose half this is, is: half a stick highlighted is
  // exactly right for a bond that crosses out of the selection. Which half is decided per
  // fragment, like the colour and for the same reason - the strip has vertices only at
  // the two ends. Chosen here, the selection was interpolated along the bond, putting its
  // edge a quarter of the way along and a faint ring three quarters along, and the picked
  // atom was whichever end the triangle under the pointer happened to finish on.
  vSelected = vec2(selectedById(int(aAtoms.x)), selectedById(int(aAtoms.y)));
  vAtoms = aAtoms;

  vec4 positionView = uView * vec4(world, 1.0);
  vPositionView = positionView.xyz;
  // uView is a rigid transform, so it rotates normals correctly with no normal matrix.
  vNormalView = mat3(uView) * normalWorld;
  gl_Position = uProj * positionView;
}
`;

const FRAGMENT = `#version 300 es
precision highp float;

in vec3 vNormalView;
in vec3 vPositionView;
flat in vec3 vColorA;
flat in vec3 vColorB;
in float vAlong;
flat in vec2 vSelected;
flat in uvec2 vAtoms;

${SHADING_GLSL}
${PACK_NORMAL_GLSL}
${GBUFFER_OUTPUTS}
${SELECTION_OUTPUT_GLSL}
${PICK_GLSL}

void main() {
  vec3 normal = normalize(vNormalView);
  vec3 viewDir = normalize(-vPositionView);
  // Winding is correct now, but culling stays off: a stick seen through a gap in a cartoon
  // shows its inside, and lighting the facing side keeps that readable.
  if (dot(normal, viewDir) < 0.0) normal = -normal;

  if (uPicking == 1) {
    outColor = encodePick(vAlong < 0.5 ? vAtoms.x : vAtoms.y);
    outNormal = vec4(0.0);
    return;
  }

  vec3 color = vAlong < 0.5 ? vColorA : vColorB;
  outColor = vec4(shade(color, normal, viewDir), 1.0);
  outNormal = packNormal(normal);
  writeSelection(vAlong < 0.5 ? vSelected.x : vSelected.y);
}
`;

export class CylinderPass {
  private program: Program;
  private vao: WebGLVertexArrayObject;
  private endpointBuffer: WebGLBuffer;
  private colorBuffer: WebGLBuffer;
  private atomBuffer: WebGLBuffer;
  private vertexCount: number;
  private count = 0;
  private capacity = 0;
  /** What the ids on these sticks are: 1 for atoms, 3 for the graph view's bonds. */
  pickKind: 1 | 3 = 1;

  constructor(private gl: WebGL2RenderingContext) {
    this.program = createProgram(gl, "cylinders", VERTEX, FRAGMENT);

    const vao = gl.createVertexArray();
    if (!vao) throw new Error("could not create cylinder VAO");
    this.vao = vao;
    gl.bindVertexArray(vao);

    // A triangle strip around the side: two vertices per segment boundary, and the seam
    // repeated to close the tube.
    const local: number[] = [];
    const normals: number[] = [];
    for (let segment = 0; segment <= SEGMENTS; segment += 1) {
      const angle = (segment / SEGMENTS) * Math.PI * 2;
      const x = Math.cos(angle);
      const y = Math.sin(angle);
      local.push(x * CIRCUMSCRIBE, y * CIRCUMSCRIBE, 0, x * CIRCUMSCRIBE, y * CIRCUMSCRIBE, 1);
      // Normals stay on the unit circle: the shading is of the round stick, the polygon is
      // only where its surface is sampled.
      normals.push(x, y, x, y);
    }
    this.vertexCount = (SEGMENTS + 1) * 2;

    createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array(local));
    const localLocation = this.program.attribute("aLocal");
    gl.enableVertexAttribArray(localLocation);
    gl.vertexAttribPointer(localLocation, 3, gl.FLOAT, false, 0, 0);

    createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array(normals));
    const normalLocation = this.program.attribute("aNormal");
    gl.enableVertexAttribArray(normalLocation);
    gl.vertexAttribPointer(normalLocation, 2, gl.FLOAT, false, 0, 0);

    // Interleaved start/end: 6 floats per bond.
    this.endpointBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array(0), gl.DYNAMIC_DRAW);
    const startLocation = this.program.attribute("aStart");
    gl.enableVertexAttribArray(startLocation);
    // Seven floats per bond: both ends and the radius. The radius is per bond because two
    // chains drawn differently must not share one - see the Rust `CylinderSet`.
    gl.vertexAttribPointer(startLocation, 3, gl.FLOAT, false, 28, 0);
    gl.vertexAttribDivisor(startLocation, 1);
    const endLocation = this.program.attribute("aEnd");
    gl.enableVertexAttribArray(endLocation);
    gl.vertexAttribPointer(endLocation, 3, gl.FLOAT, false, 28, 12);
    gl.vertexAttribDivisor(endLocation, 1);
    const radiusLocation = this.program.attribute("aRadius");
    gl.enableVertexAttribArray(radiusLocation);
    gl.vertexAttribPointer(radiusLocation, 1, gl.FLOAT, false, 28, 24);
    gl.vertexAttribDivisor(radiusLocation, 1);

    this.colorBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array(0), gl.DYNAMIC_DRAW);
    const colorALocation = this.program.attribute("aColorA");
    gl.enableVertexAttribArray(colorALocation);
    gl.vertexAttribPointer(colorALocation, 3, gl.FLOAT, false, 24, 0);
    gl.vertexAttribDivisor(colorALocation, 1);
    const colorBLocation = this.program.attribute("aColorB");
    gl.enableVertexAttribArray(colorBLocation);
    gl.vertexAttribPointer(colorBLocation, 3, gl.FLOAT, false, 24, 12);
    gl.vertexAttribDivisor(colorBLocation, 1);

    // The atoms at the two ends, for the selection mask.
    this.atomBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Uint32Array(0), gl.DYNAMIC_DRAW);
    const atomsLocation = this.program.attribute("aAtoms");
    gl.enableVertexAttribArray(atomsLocation);
    gl.vertexAttribIPointer(atomsLocation, 2, gl.UNSIGNED_INT, 8, 0);
    gl.vertexAttribDivisor(atomsLocation, 1);

    gl.bindVertexArray(null);
  }

  /** `endpoints` is [x1, y1, z1, x2, y2, z2, radius] per bond. */
  setInstances(endpoints: Float32Array): void {
    const gl = this.gl;
    this.count = endpoints.length / 7;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.endpointBuffer);
    if (this.count > this.capacity) {
      gl.bufferData(gl.ARRAY_BUFFER, endpoints, gl.DYNAMIC_DRAW);
      this.capacity = this.count;
    } else {
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, endpoints);
    }
  }

  /** `colors` is [r1, g1, b1, r2, g2, b2] per bond. */
  setColors(colors: Float32Array): void {
    const gl = this.gl;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.colorBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, colors, gl.DYNAMIC_DRAW);
  }

  /** `atoms` is the two atom ids per bond, for the selection mask. */
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
    setShadingUniforms(gl, this.program, view, shading);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, this.vertexCount, this.count);
    gl.bindVertexArray(null);
  }
}
