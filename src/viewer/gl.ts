// The WebGL2 layer, and the seam a WebGPU backend would slot into.
//
// WebGL2 rather than WebGPU because WebGPU is simply not present in this webview - see
// `capabilities()`. It costs nothing in quality here: impostor ray-casting needs
// gl_FragDepth and instanced draws, and GLSL ES 3.00 has both. Everything above this file
// works in scene terms (atoms, bonds, colours), so a second backend is an addition rather
// than a rewrite.

export interface GpuCapabilities {
  webgpu: boolean;
  webgl2: boolean;
  /** GL_RENDERER, which is the only hint available about what is doing the work. */
  renderer: string;
  vendor: string;
  /** True when the driver is a CPU rasteriser. Quality settings drop rather than fail. */
  software: boolean;
  maxTextureSize: number;
}

const SOFTWARE_MARKERS = ["llvmpipe", "swiftshader", "software", "softpipe", "basic render"];

let probed: GpuCapabilities | null = null;

/**
 * What this machine's renderer can do. Asked once: the answer cannot change while the
 * app runs, and each asking made a WebGL context of its own that nothing released - a
 * webview allows only so many, and takes the oldest away when the count is passed, which
 * may be the viewer's.
 */
export function capabilities(): GpuCapabilities {
  probed ??= probe();
  return probed;
}

function probe(): GpuCapabilities {
  const webgpu = typeof (navigator as unknown as { gpu?: unknown }).gpu !== "undefined";
  const probe = document.createElement("canvas");
  const gl = probe.getContext("webgl2");
  if (!gl) {
    return {
      webgpu,
      webgl2: false,
      renderer: "none",
      vendor: "none",
      software: false,
      maxTextureSize: 0,
    };
  }
  const debugInfo = gl.getExtension("WEBGL_debug_renderer_info");
  const renderer = String(
    debugInfo ? gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
  );
  const vendor = String(
    debugInfo ? gl.getParameter(debugInfo.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR),
  );
  const lower = renderer.toLowerCase();
  const found: GpuCapabilities = {
    webgpu,
    webgl2: true,
    renderer,
    vendor,
    software: SOFTWARE_MARKERS.some((marker) => lower.includes(marker)),
    maxTextureSize: gl.getParameter(gl.MAX_TEXTURE_SIZE) as number,
  };
  // The probe's context has told everything it knows.
  gl.getExtension("WEBGL_lose_context")?.loseContext();
  return found;
}

export class GlError extends Error {}

export function createContext(canvas: HTMLCanvasElement): WebGL2RenderingContext {
  const gl = canvas.getContext("webgl2", {
    // Geometry is antialiased in the offscreen composite pass. Multisampling the
    // default framebuffer only samples the final fullscreen quad, adding a resolve
    // before presentation without smoothing any molecular geometry.
    antialias: false,
    depth: true,
    // Premultiplied, which is what a compositor expects: a canvas that says its pixels are
    // *not* premultiplied is composited by WebKit as if they were, and every pixel the
    // shelf leaves transparent came out black instead of showing the page. The shader
    // multiplies the colour by the alpha itself, so an opaque frame is unchanged.
    premultipliedAlpha: true,
    powerPreference: "high-performance",
  });
  if (!gl) {
    throw new GlError(
      "This system has no WebGL2. The interactive viewer needs it; offline Blender renders do not.",
    );
  }
  return gl;
}

function compile(gl: WebGL2RenderingContext, type: number, source: string, label: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) throw new GlError(`could not create ${label} shader`);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader) ?? "";
    gl.deleteShader(shader);
    // Shader compile errors are otherwise invisible - the pass silently draws nothing -
    // so the log and the numbered source both go into the message.
    const numbered = source
      .split("\n")
      .map((line, index) => `${String(index + 1).padStart(3)} | ${line}`)
      .join("\n");
    throw new GlError(`${label} shader failed to compile:\n${log}\n${numbered}`);
  }
  return shader;
}

export interface Program {
  handle: WebGLProgram;
  uniform(name: string): WebGLUniformLocation | null;
  attribute(name: string): number;
  use(): void;
}

export function createProgram(
  gl: WebGL2RenderingContext,
  label: string,
  vertexSource: string,
  fragmentSource: string,
): Program {
  const vertex = compile(gl, gl.VERTEX_SHADER, vertexSource, `${label} vertex`);
  const fragment = compile(gl, gl.FRAGMENT_SHADER, fragmentSource, `${label} fragment`);
  const handle = gl.createProgram();
  if (!handle) throw new GlError(`could not create ${label} program`);
  gl.attachShader(handle, vertex);
  gl.attachShader(handle, fragment);
  gl.linkProgram(handle);
  gl.deleteShader(vertex);
  gl.deleteShader(fragment);
  if (!gl.getProgramParameter(handle, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(handle) ?? "";
    gl.deleteProgram(handle);
    throw new GlError(`${label} program failed to link: ${log}`);
  }

  // Locations are looked up once and memoised: getUniformLocation is a string lookup, and
  // calling it per frame per uniform is a measurable cost at 60fps.
  const uniforms = new Map<string, WebGLUniformLocation | null>();
  const attributes = new Map<string, number>();
  return {
    handle,
    uniform(name) {
      if (!uniforms.has(name)) uniforms.set(name, gl.getUniformLocation(handle, name));
      return uniforms.get(name) ?? null;
    },
    attribute(name) {
      if (!attributes.has(name)) attributes.set(name, gl.getAttribLocation(handle, name));
      return attributes.get(name) ?? -1;
    },
    use() {
      gl.useProgram(handle);
    },
  };
}

export function createBuffer(
  gl: WebGL2RenderingContext,
  target: number,
  data: BufferSource,
  usage: number = gl.STATIC_DRAW,
): WebGLBuffer {
  const buffer = gl.createBuffer();
  if (!buffer) throw new GlError("could not create buffer");
  gl.bindBuffer(target, buffer);
  gl.bufferData(target, data, usage);
  return buffer;
}
