// The passes after geometry: occlusion, then composite.
//
// This is where the viewport stops looking like a diagram and starts looking like a
// render. Ambient occlusion darkens the crevices between atoms and inside a fold, which
// is most of what reads as depth on a flat screen; the composite lays the structure on the
// background, optionally outlines it, smooths the impostors' hard analytic edges, and
// encodes to sRGB.

import { createBuffer, createProgram, type Program } from "./gl";

const FULLSCREEN_VERTEX = `#version 300 es
precision highp float;
in vec2 aCorner;
out vec2 vUv;
void main() {
  vUv = aCorner * 0.5 + 0.5;
  gl_Position = vec4(aCorner, 0.0, 1.0);
}
`;

/**
 * Screen-space ambient occlusion.
 *
 * Sixteen samples in a hemisphere around each fragment, each tested against the depth
 * buffer. The count is a compromise: eight is visibly noisy on a curved surface, and
 * thirty-two costs more than the geometry pass on a large structure.
 */
const SSAO_FRAGMENT = `#version 300 es
precision highp float;

in vec2 vUv;

uniform sampler2D uDepth;
uniform sampler2D uNormal;
uniform mat4 uProjection;
uniform mat4 uInverseProjection;
uniform float uRadius;
uniform float uBias;

out vec4 outOcclusion;

#define SAMPLES 16

float hash(vec2 seed) {
  return fract(sin(dot(seed, vec2(12.9898, 78.233))) * 43758.5453);
}

vec3 viewPositionAt(vec2 uv) {
  float depth = texture(uDepth, uv).r;
  vec4 clip = vec4(uv * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
  vec4 view = uInverseProjection * clip;
  return view.xyz / view.w;
}

void main() {
  float depth = texture(uDepth, vUv).r;
  // Nothing was drawn here, so there is nothing to occlude. Skipping the background also
  // keeps occlusion from bleeding out past a silhouette.
  if (depth >= 1.0) {
    outOcclusion = vec4(1.0);
    return;
  }

  vec3 position = viewPositionAt(vUv);
  vec3 normal = normalize(texture(uNormal, vUv).xyz * 2.0 - 1.0);

  // A per-pixel rotation turns banding into noise, which the blur in the composite then
  // removes. Without it, sixteen fixed directions show up as visible rings.
  float rotation = hash(vUv * 1024.0) * 6.2831853;

  float occlusion = 0.0;
  for (int index = 0; index < SAMPLES; index++) {
    float t = (float(index) + 0.5) / float(SAMPLES);
    // A spiral through the hemisphere: even coverage without a lookup table.
    float angle = t * 6.2831853 * 3.0 + rotation;
    float radius = sqrt(t);
    float height = sqrt(max(1.0 - radius * radius, 0.0));
    vec3 hemisphere = vec3(cos(angle) * radius, sin(angle) * radius, height);

    // Orient the hemisphere to the surface.
    vec3 helper = abs(normal.z) < 0.99 ? vec3(0.0, 0.0, 1.0) : vec3(1.0, 0.0, 0.0);
    vec3 tangent = normalize(cross(helper, normal));
    vec3 bitangent = cross(normal, tangent);
    vec3 offset = tangent * hemisphere.x + bitangent * hemisphere.y + normal * hemisphere.z;

    vec3 samplePosition = position + offset * uRadius * mix(0.3, 1.0, t);
    vec4 projected = uProjection * vec4(samplePosition, 1.0);
    vec2 sampleUv = (projected.xy / projected.w) * 0.5 + 0.5;
    if (sampleUv.x < 0.0 || sampleUv.x > 1.0 || sampleUv.y < 0.0 || sampleUv.y > 1.0) continue;

    float sceneDepth = viewPositionAt(sampleUv).z;
    // View space looks down -Z, so a larger z is nearer the camera.
    if (sceneDepth >= samplePosition.z + uBias) {
      // Range check: a surface far behind this one is a different object, not a crevice,
      // and counting it produces dark halos around every silhouette.
      float range = smoothstep(0.0, 1.0, uRadius / abs(position.z - sceneDepth));
      occlusion += range;
    }
  }

  outOcclusion = vec4(vec3(clamp(1.0 - occlusion / float(SAMPLES), 0.0, 1.0)), 1.0);
}
`;

/**
 * Background, occlusion, outline, antialiasing and the sRGB encode, in one pass.
 *
 * Combined rather than chained because each is a couple of taps and a separate
 * framebuffer per step would cost more in bandwidth than the work itself.
 */
const COMPOSITE_FRAGMENT = `#version 300 es
precision highp float;

in vec2 vUv;

uniform sampler2D uColor;
uniform sampler2D uNormal;
uniform sampler2D uDepth;
uniform sampler2D uOcclusion;
uniform vec2 uTexel;
uniform vec3 uBackground;     // linear
uniform float uOcclusionStrength;
uniform float uOutlineStrength;
uniform float uAntialias;
uniform vec3 uSelectionColor;   // linear
uniform vec3 uHoverColor;       // linear
uniform float uSelectionWidth;  // outline half-width, in pixels; 0 disables it

out vec4 outColor;
uniform float uBackgroundAlpha;
uniform float uFade;            // 0 draws the scene as it is; towards 1 it recedes into the background

float linearDepth(vec2 uv, float near, float far) {
  float depth = texture(uDepth, uv).r * 2.0 - 1.0;
  return (2.0 * near * far) / (far + near - depth * (far - near));
}

/**
 * A small edge-directed blend, enough to soften the analytic edges of the impostors.
 *
 * Not full FXAA: the geometry here is spheres and ribbons whose edges are already
 * pixel-exact rather than staircased across a triangle, so what is needed is a nudge
 * along a detected edge, not a general-purpose filter.
 */
vec3 antialiased(vec2 uv) {
  vec3 center = texture(uColor, uv).rgb;
  if (uAntialias <= 0.0) return center;

  vec3 north = texture(uColor, uv + vec2(0.0, uTexel.y)).rgb;
  vec3 south = texture(uColor, uv - vec2(0.0, uTexel.y)).rgb;
  vec3 east = texture(uColor, uv + vec2(uTexel.x, 0.0)).rgb;
  vec3 west = texture(uColor, uv - vec2(uTexel.x, 0.0)).rgb;

  float luma = dot(center, vec3(0.299, 0.587, 0.114));
  float lumaMin = min(luma, min(min(dot(north, vec3(0.299, 0.587, 0.114)),
                                    dot(south, vec3(0.299, 0.587, 0.114))),
                                min(dot(east, vec3(0.299, 0.587, 0.114)),
                                    dot(west, vec3(0.299, 0.587, 0.114)))));
  float lumaMax = max(luma, max(max(dot(north, vec3(0.299, 0.587, 0.114)),
                                    dot(south, vec3(0.299, 0.587, 0.114))),
                                max(dot(east, vec3(0.299, 0.587, 0.114)),
                                    dot(west, vec3(0.299, 0.587, 0.114)))));
  float contrast = lumaMax - lumaMin;
  if (contrast < 0.05) return center;

  vec3 blurred = (north + south + east + west + center) * 0.2;
  return mix(center, blurred, clamp(contrast * 2.0, 0.0, 1.0) * uAntialias);
}

/** Occlusion, blurred over a 3x3 neighbourhood to remove the sampling noise. */
float occlusionAt(vec2 uv) {
  float total = 0.0;
  for (int y = -1; y <= 1; y++) {
    for (int x = -1; x <= 1; x++) {
      total += texture(uOcclusion, uv + vec2(float(x), float(y)) * uTexel).r;
    }
  }
  return total / 9.0;
}

/**
 * An outline where depth or normals break.
 *
 * Depth alone misses a fold pressed against itself; normals alone miss two surfaces at the
 * same angle with a gap between them. Both together find the edges a reader needs.
 */
float outlineAt(vec2 uv) {
  float here = texture(uDepth, uv).r;
  vec3 normalHere = texture(uNormal, uv).xyz * 2.0 - 1.0;

  float depthEdge = 0.0;
  float normalEdge = 0.0;
  vec2 offsets[4] = vec2[4](
    vec2(uTexel.x, 0.0), vec2(-uTexel.x, 0.0),
    vec2(0.0, uTexel.y), vec2(0.0, -uTexel.y)
  );
  for (int index = 0; index < 4; index++) {
    float there = texture(uDepth, uv + offsets[index]).r;
    depthEdge = max(depthEdge, abs(there - here));
    vec3 normalThere = texture(uNormal, uv + offsets[index]).xyz * 2.0 - 1.0;
    normalEdge = max(normalEdge, 1.0 - max(dot(normalHere, normalThere), 0.0));
  }
  // Depth is non-linear, so the threshold scales with the value itself.
  float depthScore = smoothstep(0.0004 * here + 0.00002, 0.004 * here + 0.0002, depthEdge);
  float normalScore = smoothstep(0.35, 0.8, normalEdge);
  return clamp(max(depthScore, normalScore), 0.0, 1.0);
}

vec3 toSrgb(vec3 linear) {
  vec3 low = linear * 12.92;
  vec3 high = 1.055 * pow(max(linear, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
  return mix(low, high, step(vec3(0.0031308), linear));
}

/**
 * How close this pixel is to the edge of the selection.
 *
 * The geometry passes write a 0 or 1 into the normal target's alpha - see
 * SELECTION_OUTPUT_GLSL in shading.ts - so the selected set is a mask in the G-buffer, and
 * its border
 * is where a ring of samples around a pixel disagrees with itself. Drawn outside the
 * selection as well as inside, so a chain against the background still gets a line.
 */
/**
 * The mask carries two levels - 1.0 for the selection, ~0.5 for what the pointer is over -
 * so each is binarised at its own threshold and its border found separately. The hover
 * threshold sits below both levels, so the hovered set includes the selected one and the
 * selection's line is drawn on top of it rather than beside it.
 */
vec2 maskEdges(vec2 uv) {
  if (uSelectionWidth <= 0.0) return vec2(0.0);
  float here = texture(uNormal, uv).a;
  float selectedHere = step(0.75, here);
  float hoveredHere = step(0.25, here);
  vec2 most = vec2(selectedHere, hoveredHere);
  vec2 least = most;
  // Eight samples on a ring: enough for a smooth border at one or two pixels, and cheap
  // enough to run on every pixel of the frame.
  for (int index = 0; index < 8; index++) {
    float angle = 6.2831853 * float(index) / 8.0;
    vec2 offset = vec2(cos(angle), sin(angle)) * uTexel * uSelectionWidth;
    float sampled = texture(uNormal, uv + offset).a;
    vec2 flags = vec2(step(0.75, sampled), step(0.25, sampled));
    most = max(most, flags);
    least = min(least, flags);
  }
  return clamp(most - least, 0.0, 1.0);
}

vec3 withOutlines(vec3 color, vec2 edges) {
  color = mix(color, uHoverColor, edges.y * (1.0 - edges.x));
  return mix(color, uSelectionColor, edges.x);
}

void main() {
  float depth = texture(uDepth, vUv).r;
  vec2 edges = maskEdges(vUv);

  if (depth >= 1.0) {
    // Background. Outlines still apply here, so a silhouette gets its edge from both
    // sides rather than only from inside.
    float edge = uOutlineStrength > 0.0 ? outlineAt(vUv) : 0.0;
    vec3 background = mix(uBackground, uBackground * 0.35, edge * uOutlineStrength);
    // The selection's own outline reaches into the background, which is what makes it
    // read as a line drawn around the thing rather than a stain on it.
    background = withOutlines(background, edges);
    // The background can be nothing at all - the shelf of worlds draws its structures onto
    // the page rather than onto a colour, and a preview with a white box around it would
    // look like a photograph of the app instead of the thing itself.
    outColor = vec4(toSrgb(background) * uBackgroundAlpha, uBackgroundAlpha);
    return;
  }

  vec3 color = antialiased(vUv);
  color *= mix(1.0, occlusionAt(vUv), uOcclusionStrength);

  if (uOutlineStrength > 0.0) {
    color = mix(color, color * 0.25, outlineAt(vUv) * uOutlineStrength);
  }
  color = withOutlines(color, edges);

  if (uFade > 0.0) {
    // Receding, for the graph view: most of the colour taken out, then most of what is
    // left taken towards the background, so the shape stays readable as a ghost and
    // nothing on it competes with what is drawn over it.
    float luma = dot(color, vec3(0.2126, 0.7152, 0.0722));
    color = mix(color, vec3(luma), 0.7 * uFade);
    color = mix(color, uBackground, uFade);
  }

  outColor = vec4(toSrgb(color), 1.0);
}
`;

export class PostPass {
  private ssao: Program;
  private composite: Program;
  private vao: WebGLVertexArrayObject;

  constructor(private gl: WebGL2RenderingContext) {
    this.ssao = createProgram(gl, "ssao", FULLSCREEN_VERTEX, SSAO_FRAGMENT);
    this.composite = createProgram(gl, "composite", FULLSCREEN_VERTEX, COMPOSITE_FRAGMENT);

    const vao = gl.createVertexArray();
    if (!vao) throw new Error("could not create post VAO");
    this.vao = vao;
    gl.bindVertexArray(vao);
    createBuffer(gl, gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]));
    // Both programs declare aCorner at the same place, so one VAO serves both.
    const corner = this.ssao.attribute("aCorner");
    gl.enableVertexAttribArray(corner);
    gl.vertexAttribPointer(corner, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
  }

  drawOcclusion(inputs: {
    depth: WebGLTexture;
    normal: WebGLTexture;
    projection: Float32Array;
    inverseProjection: Float32Array;
    radius: number;
  }): void {
    const gl = this.gl;
    this.ssao.use();
    gl.bindVertexArray(this.vao);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, inputs.depth);
    gl.uniform1i(this.ssao.uniform("uDepth"), 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, inputs.normal);
    gl.uniform1i(this.ssao.uniform("uNormal"), 1);
    gl.uniformMatrix4fv(this.ssao.uniform("uProjection"), false, inputs.projection);
    gl.uniformMatrix4fv(
      this.ssao.uniform("uInverseProjection"),
      false,
      inputs.inverseProjection,
    );
    gl.uniform1f(this.ssao.uniform("uRadius"), inputs.radius);
    // In angstroms: below this a sample is the same surface, not an occluder. Too small
    // and every flat face self-occludes into stripes.
    gl.uniform1f(this.ssao.uniform("uBias"), 0.08);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.bindVertexArray(null);
  }

  drawComposite(inputs: {
    color: WebGLTexture;
    normal: WebGLTexture;
    depth: WebGLTexture;
    occlusion: WebGLTexture;
    width: number;
    height: number;
    background: [number, number, number];
    occlusionStrength: number;
    outlineStrength: number;
    antialias: number;
    /** Linear RGB for the selection outline. */
    selectionColor: [number, number, number];
    /** Linear RGB for the outline of whatever the pointer is over. */
    hoverColor: [number, number, number];
    /** Half-width in pixels. 0 when nothing is selected. */
    selectionWidth: number;
    /** 0 leaves the background out entirely, so the page shows through. */
    backgroundAlpha?: number;
    fade?: number;
    /**
     * How many of the frame's pixels make one pixel of the picture it is for: 1 on screen,
     * more when a picture is drawn large to be shrunk. Edges, outlines and the occlusion
     * blur are measured in pixels of the picture, so they keep their weight once shrunk.
     */
    pixelScale?: number;
  }): void {
    const gl = this.gl;
    this.composite.use();
    gl.bindVertexArray(this.vao);

    const textures: [string, WebGLTexture][] = [
      ["uColor", inputs.color],
      ["uNormal", inputs.normal],
      ["uDepth", inputs.depth],
      ["uOcclusion", inputs.occlusion],
    ];
    textures.forEach(([name, texture], unit) => {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.uniform1i(this.composite.uniform(name), unit);
    });

    const scale = inputs.pixelScale ?? 1;
    gl.uniform2f(
      this.composite.uniform("uTexel"),
      scale / Math.max(inputs.width, 1),
      scale / Math.max(inputs.height, 1),
    );
    gl.uniform3fv(this.composite.uniform("uBackground"), inputs.background);
    gl.uniform1f(this.composite.uniform("uOcclusionStrength"), inputs.occlusionStrength);
    gl.uniform1f(this.composite.uniform("uOutlineStrength"), inputs.outlineStrength);
    gl.uniform1f(this.composite.uniform("uBackgroundAlpha"), inputs.backgroundAlpha ?? 1);
    gl.uniform1f(this.composite.uniform("uFade"), inputs.fade ?? 0);
    gl.uniform1f(this.composite.uniform("uAntialias"), inputs.antialias);
    gl.uniform3fv(this.composite.uniform("uSelectionColor"), inputs.selectionColor);
    gl.uniform3fv(this.composite.uniform("uHoverColor"), inputs.hoverColor);
    gl.uniform1f(this.composite.uniform("uSelectionWidth"), inputs.selectionWidth);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.bindVertexArray(null);
  }
}
