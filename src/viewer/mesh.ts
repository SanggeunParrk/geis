// An indexed triangle mesh with per-vertex normals and colours.
//
// The cartoon is one mesh, so this is one draw call for a whole protein. Uses the same
// lighting as the cylinder pass, deliberately: a ribbon and a stick sitting side by side
// must look lit by the same lamp.

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

const VERTEX = `#version 300 es
precision highp float;

in vec3 aPosition;
in vec3 aNormal;
in vec3 aColor;
in uint aResidue;

uniform mat4 uView;
uniform mat4 uProj;

${SELECTION_GLSL}
${ROW_GLSL}

out vec3 vNormalView;
out vec3 vPositionView;
out vec3 vColor;
out float vSelected;
flat out uint vId;

void main() {
  vec4 positionView = uView * vec4(rowPoint(aPosition), 1.0);
  vPositionView = positionView.xyz;
  vNormalView = mat3(uView) * rowDirection(aNormal, aPosition.x);
  vColor = aColor;
  // A ribbon's vertices know which residue they belong to, so the mask is read by residue
  // here and by atom in the sphere and stick passes.
  vSelected = selectedById(int(aResidue));
  vId = aResidue;
  gl_Position = uProj * positionView;
}
`;

const FRAGMENT = `#version 300 es
precision highp float;

in vec3 vNormalView;
in vec3 vPositionView;
in vec3 vColor;
in float vSelected;
flat in uint vId;

${SHADING_GLSL}
${PACK_NORMAL_GLSL}
${GBUFFER_OUTPUTS}
${SELECTION_OUTPUT_GLSL}
${PICK_GLSL}

void main() {
  vec3 normal = normalize(vNormalView);
  vec3 viewDir = normalize(-vPositionView);
  // A ribbon is a thin shell, so its far side is visible through the near side of a helix;
  // lighting whichever side faces the camera keeps both readable.
  if (dot(normal, viewDir) < 0.0) normal = -normal;

  if (uPicking == 1) {
    outColor = encodePick(vId);
    outNormal = vec4(0.0);
    return;
  }

  outColor = vec4(shade(vColor, normal, viewDir), 1.0);
  outNormal = packNormal(normal);
  writeSelection(vSelected);
}
`;

export interface MeshData {
  positions: Float32Array;
  /** Three floats a vertex, or four signed bytes as the core packs them. */
  normals: Float32Array | Int8Array;
  /** Three floats a vertex, or four bytes as the core packs them. */
  colors: Float32Array | Uint8Array;
  indices: Uint32Array;
  /** Residue per vertex, for the selection mask. Zeros when a caller has none. */
  residues?: Uint32Array;
}

export class MeshPass {
  private program: Program;
  private vao: WebGLVertexArrayObject;
  private positionBuffer: WebGLBuffer;
  private normalBuffer: WebGLBuffer;
  private colorBuffer: WebGLBuffer;
  private residueBuffer: WebGLBuffer;
  private indexBuffer: WebGLBuffer;
  private indexCount = 0;

  constructor(private gl: WebGL2RenderingContext) {
    this.program = createProgram(gl, "mesh", VERTEX, FRAGMENT);

    const vao = gl.createVertexArray();
    if (!vao) throw new Error("could not create mesh VAO");
    this.vao = vao;
    gl.bindVertexArray(vao);

    this.positionBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array(0), gl.DYNAMIC_DRAW);
    const position = this.program.attribute("aPosition");
    gl.enableVertexAttribArray(position);
    gl.vertexAttribPointer(position, 3, gl.FLOAT, false, 0, 0);

    this.normalBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array(0), gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(this.program.attribute("aNormal"));

    this.colorBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array(0), gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(this.program.attribute("aColor"));
    this.pointAttributes(false, false);

    // The residue each vertex belongs to, so a chain's ribbon can be masked without
    // rebuilding the spline.
    this.residueBuffer = createBuffer(gl, gl.ARRAY_BUFFER, new Uint32Array(0), gl.DYNAMIC_DRAW);
    const residue = this.program.attribute("aResidue");
    gl.enableVertexAttribArray(residue);
    gl.vertexAttribIPointer(residue, 1, gl.UNSIGNED_INT, 0, 0);

    this.indexBuffer = createBuffer(gl, gl.ELEMENT_ARRAY_BUFFER, new Uint32Array(0), gl.DYNAMIC_DRAW);
    gl.bindVertexArray(null);
  }

  /**
   * Says how the normal and colour buffers are laid out: three floats a vertex, or - as
   * the core sends a scene - four bytes, read back as values from -1 to 1 and 0 to 1.
   * Must be called with the vertex array bound.
   */
  private pointAttributes(packedNormals: boolean, packedColors: boolean): void {
    const gl = this.gl;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.normalBuffer);
    const normal = this.program.attribute("aNormal");
    if (packedNormals) gl.vertexAttribPointer(normal, 3, gl.BYTE, true, 4, 0);
    else gl.vertexAttribPointer(normal, 3, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.colorBuffer);
    const color = this.program.attribute("aColor");
    if (packedColors) gl.vertexAttribPointer(color, 3, gl.UNSIGNED_BYTE, true, 4, 0);
    else gl.vertexAttribPointer(color, 3, gl.FLOAT, false, 0, 0);
  }

  setMesh(mesh: MeshData): void {
    const gl = this.gl;
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.positionBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, mesh.positions, gl.DYNAMIC_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.normalBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, mesh.normals, gl.DYNAMIC_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.colorBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, mesh.colors, gl.DYNAMIC_DRAW);
    this.pointAttributes(mesh.normals instanceof Int8Array, mesh.colors instanceof Uint8Array);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.residueBuffer);
    gl.bufferData(
      gl.ARRAY_BUFFER,
      mesh.residues ?? new Uint32Array(mesh.positions.length / 3),
      gl.DYNAMIC_DRAW,
    );
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.indexBuffer);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, mesh.indices, gl.DYNAMIC_DRAW);
    gl.bindVertexArray(null);
    this.indexCount = mesh.indices.length;
  }

  setColors(colors: Float32Array | Uint8Array): void {
    const gl = this.gl;
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.colorBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, colors, gl.DYNAMIC_DRAW);
    const color = this.program.attribute("aColor");
    if (colors instanceof Uint8Array) gl.vertexAttribPointer(color, 3, gl.UNSIGNED_BYTE, true, 4, 0);
    else gl.vertexAttribPointer(color, 3, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
  }

  clear(): void {
    this.indexCount = 0;
  }

  draw(
    view: Float32Array,
    projection: Float32Array,
    shading: ShadingUniforms,
    selectionWidth = 0,
    picking = false,
    row: Row = NO_ROW,
  ): void {
    if (this.indexCount === 0) return;
    const gl = this.gl;
    this.program.use();
    gl.bindVertexArray(this.vao);
    gl.uniform1i(this.program.uniform("uSelection"), SELECTION_TEXTURE_UNIT);
    gl.uniform1i(this.program.uniform("uSelectionWidth"), selectionWidth);
    gl.uniform1i(this.program.uniform("uPicking"), picking ? 1 : 0);
    // A sphere and a stick end know their atom; a ribbon vertex knows its residue.
    gl.uniform1i(this.program.uniform("uPickKind"), 2);
    gl.uniformMatrix4fv(this.program.uniform("uView"), false, view);
    gl.uniformMatrix4fv(this.program.uniform("uProj"), false, projection);
    applyRow(gl, (name) => this.program.uniform(name), row);
    setShadingUniforms(gl, this.program, view, shading);
    // Uint32 indices need no extension in WebGL2 - a large protein's cartoon exceeds
    // 65535 vertices comfortably.
    gl.drawElements(gl.TRIANGLES, this.indexCount, gl.UNSIGNED_INT, 0);
    gl.bindVertexArray(null);
  }
}
