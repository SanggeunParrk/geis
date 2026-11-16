// The offscreen render target.
//
// Geometry draws into a G-buffer - shaded colour plus view-space normals, with a real
// depth texture - so the passes after it can compute occlusion and find edges. Drawing
// straight to the screen, as this used to, leaves nothing to read back afterwards.

export interface RenderTarget {
  framebuffer: WebGLFramebuffer;
  color: WebGLTexture;
  normal: WebGLTexture;
  depth: WebGLTexture;
  width: number;
  height: number;
}

function createTexture(
  gl: WebGL2RenderingContext,
  width: number,
  height: number,
  internalFormat: number,
  format: number,
  type: number,
): WebGLTexture {
  const texture = gl.createTexture();
  if (!texture) throw new Error("could not create texture");
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texImage2D(gl.TEXTURE_2D, 0, internalFormat, width, height, 0, format, type, null);
  // Nearest and clamped: every sampler here reads exact pixels, and filtering a depth or
  // normal buffer produces values that were never written.
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return texture;
}

export function createRenderTarget(
  gl: WebGL2RenderingContext,
  width: number,
  height: number,
): RenderTarget {
  const framebuffer = gl.createFramebuffer();
  if (!framebuffer) throw new Error("could not create framebuffer");

  const color = createTexture(gl, width, height, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE);
  const normal = createTexture(gl, width, height, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE);
  // A depth *texture*, not a renderbuffer: occlusion needs to sample it. DEPTH_COMPONENT24
  // is core in WebGL2, so this needs no extension.
  const depth = createTexture(
    gl,
    width,
    height,
    gl.DEPTH_COMPONENT24,
    gl.DEPTH_COMPONENT,
    gl.UNSIGNED_INT,
  );

  gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, color, 0);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT1, gl.TEXTURE_2D, normal, 0);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.TEXTURE_2D, depth, 0);
  gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1]);

  const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
  if (status !== gl.FRAMEBUFFER_COMPLETE) {
    // Worth failing loudly: an incomplete framebuffer silently discards every draw, which
    // looks exactly like a shader that is not running.
    throw new Error(`render target is incomplete (0x${status.toString(16)})`);
  }
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);

  return { framebuffer, color, normal, depth, width, height };
}

/** A single-channel target, for occlusion. */
export interface SingleTarget {
  framebuffer: WebGLFramebuffer;
  texture: WebGLTexture;
  width: number;
  height: number;
}

export function createSingleTarget(
  gl: WebGL2RenderingContext,
  width: number,
  height: number,
): SingleTarget {
  const framebuffer = gl.createFramebuffer();
  if (!framebuffer) throw new Error("could not create framebuffer");
  const texture = createTexture(gl, width, height, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE);
  gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
  gl.drawBuffers([gl.COLOR_ATTACHMENT0]);
  const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
  if (status !== gl.FRAMEBUFFER_COMPLETE) {
    throw new Error(`occlusion target is incomplete (0x${status.toString(16)})`);
  }
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  return { framebuffer, texture, width, height };
}

export function disposeRenderTarget(gl: WebGL2RenderingContext, target: RenderTarget): void {
  gl.deleteFramebuffer(target.framebuffer);
  gl.deleteTexture(target.color);
  gl.deleteTexture(target.normal);
  gl.deleteTexture(target.depth);
}

export function disposeSingleTarget(gl: WebGL2RenderingContext, target: SingleTarget): void {
  gl.deleteFramebuffer(target.framebuffer);
  gl.deleteTexture(target.texture);
}
