// WebGL2 preview renderer. Runs the GLSL that naga transpiled from develop.wgsl.
// The image is uploaded once; edits only change a small uniform block (and the curve LUT).
import type { GlslProgram } from "$lib/bindings/GlslProgram";

const PARAMS_BINDING = 0;
const VIEW_BINDING = 1;
const MARGIN_CSS_PX = 24;
const CURVE_SIZE = 1024;
/** Long edge of the offscreen render the histogram is computed from. */
const HISTOGRAM_EDGE = 384;
const HISTOGRAM_INTERVAL_MS = 80;

/** 256 bins each for red, green, blue and luma, plus the pixel count. */
export interface Histogram {
  r: Uint32Array;
  g: Uint32Array;
  b: Uint32Array;
  y: Uint32Array;
  total: number;
}

export class PreviewRenderer {
  private gl: WebGL2RenderingContext;
  private program: WebGLProgram;
  private vao: WebGLVertexArrayObject;
  private texture: WebGLTexture | null = null;
  private curve: WebGLTexture;
  private histFbo: WebGLFramebuffer | null = null;
  private histTex: WebGLTexture | null = null;
  private histW = 0;
  private histH = 0;
  private histPixels: Uint8Array | null = null;
  private lastHist = 0;
  private histTimer: ReturnType<typeof setTimeout> | undefined;
  private paramsUbo: WebGLBuffer;
  private viewUbo: WebGLBuffer;
  private imageW = 0;
  private imageH = 0;
  private hasParams = false;
  private frame = 0;
  /** Called after each completed draw of an image (used by the dev snapshot hook). */
  onDrawn: ((canvas: HTMLCanvasElement) => void) | null = null;
  /** Called with a fresh histogram after the image or edit changes (throttled). */
  onHistogram: ((h: Histogram) => void) | null = null;

  constructor(private canvas: HTMLCanvasElement, src: GlslProgram) {
    const gl = canvas.getContext("webgl2", { antialias: false, alpha: false, premultipliedAlpha: false });
    if (!gl) throw new Error("WebGL2 is not available");
    this.gl = gl;
    this.program = link(gl, src.vertex, src.fragment);
    this.vao = gl.createVertexArray()!;

    this.paramsUbo = gl.createBuffer()!;
    this.viewUbo = gl.createBuffer()!;
    bindBlock(gl, this.program, src.paramsBlock, PARAMS_BINDING);
    bindBlock(gl, this.program, src.viewBlock, VIEW_BINDING);
    gl.useProgram(this.program);
    gl.uniform1i(gl.getUniformLocation(this.program, src.imageSampler), 0);
    gl.uniform1i(gl.getUniformLocation(this.program, src.curveSampler), 1);

    // Identity curve until the first real one arrives.
    this.curve = gl.createTexture()!;
    const ramp = new Float32Array(CURVE_SIZE * 4);
    for (let i = 0; i < CURVE_SIZE; i++) ramp.fill(i / (CURVE_SIZE - 1), i * 4, i * 4 + 3), (ramp[i * 4 + 3] = 1);
    gl.bindTexture(gl.TEXTURE_2D, this.curve);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, CURVE_SIZE, 1, 0, gl.RGBA, gl.FLOAT, ramp);
    setLinearClamp(gl);
  }

  /** Upload a tone-curve LUT (RGBA16F, CURVE_SIZE × 1). */
  setCurve(rgba16f: Uint16Array) {
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, this.curve);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 2);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, CURVE_SIZE, 1, 0, gl.RGBA, gl.HALF_FLOAT, rgba16f);
    this.requestDraw(true);
  }

  setImage(width: number, height: number, rgb16f: Uint16Array) {
    const gl = this.gl;
    if (this.texture) gl.deleteTexture(this.texture);
    this.texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 2);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB16F, width, height, 0, gl.RGB, gl.HALF_FLOAT, rgb16f);
    setLinearClamp(gl);
    this.imageW = width;
    this.imageH = height;
    this.resizeHistogramTarget();
    this.requestDraw(true);
  }

  setParams(params: Float32Array) {
    const gl = this.gl;
    gl.bindBuffer(gl.UNIFORM_BUFFER, this.paramsUbo);
    gl.bufferData(gl.UNIFORM_BUFFER, params, gl.DYNAMIC_DRAW);
    this.hasParams = true;
    this.requestDraw(true);
  }

  private histDirty = false;

  /** Schedule a redraw on the next animation frame (coalesces multiple requests). */
  requestDraw(contentChanged = false) {
    if (contentChanged) this.histDirty = true;
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.draw();
    });
  }

  private draw() {
    const gl = this.gl;
    const dpr = window.devicePixelRatio || 1;
    const cssW = this.canvas.clientWidth;
    const cssH = this.canvas.clientHeight;
    const w = Math.max(1, Math.round(cssW * dpr));
    const h = Math.max(1, Math.round(cssH * dpr));
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
    }
    gl.viewport(0, 0, w, h);
    gl.clearColor(0.0706, 0.0706, 0.0706, 1); // --bg-0
    gl.clear(gl.COLOR_BUFFER_BIT);
    if (!this.texture || !this.hasParams) return;

    // Fit the image inside the canvas with a margin; never upscale past 1:1 device pixels.
    const availW = Math.max(1, cssW - 2 * MARGIN_CSS_PX) * dpr;
    const availH = Math.max(1, cssH - 2 * MARGIN_CSS_PX) * dpr;
    const scale = Math.min(availW / this.imageW, availH / this.imageH, 1);
    const view = new Float32Array([(this.imageW * scale) / w, (this.imageH * scale) / h, 0, 0]);
    gl.bindBuffer(gl.UNIFORM_BUFFER, this.viewUbo);
    gl.bufferData(gl.UNIFORM_BUFFER, view, gl.DYNAMIC_DRAW);

    gl.useProgram(this.program);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, PARAMS_BINDING, this.paramsUbo);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, VIEW_BINDING, this.viewUbo);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.curve);
    gl.bindVertexArray(this.vao);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    // Must run in the same task as the draw: the drawing buffer is not preserved.
    this.onDrawn?.(this.canvas);
    if (this.histDirty) this.scheduleHistogram();
  }

  // ---- histogram ----

  private resizeHistogramTarget() {
    const gl = this.gl;
    const s = Math.min(1, HISTOGRAM_EDGE / Math.max(this.imageW, this.imageH));
    this.histW = Math.max(1, Math.round(this.imageW * s));
    this.histH = Math.max(1, Math.round(this.imageH * s));
    if (this.histTex) gl.deleteTexture(this.histTex);
    if (this.histFbo) gl.deleteFramebuffer(this.histFbo);
    this.histTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.histTex);
    gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, this.histW, this.histH);
    this.histFbo = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.histFbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.histTex, 0);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    this.histPixels = new Uint8Array(this.histW * this.histH * 4);
  }

  /** Throttled: at most one readback per HISTOGRAM_INTERVAL_MS, always ending with a fresh one. */
  private scheduleHistogram() {
    if (!this.onHistogram || this.histTimer !== undefined) return;
    const wait = Math.max(0, HISTOGRAM_INTERVAL_MS - (performance.now() - this.lastHist));
    this.histTimer = setTimeout(() => {
      this.histTimer = undefined;
      this.histDirty = false;
      this.lastHist = performance.now();
      this.computeHistogram();
    }, wait);
  }

  private computeHistogram() {
    const gl = this.gl;
    if (!this.histFbo || !this.histPixels || !this.texture || !this.hasParams || !this.onHistogram) return;
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.histFbo);
    gl.viewport(0, 0, this.histW, this.histH);
    gl.bindBuffer(gl.UNIFORM_BUFFER, this.viewUbo);
    gl.bufferData(gl.UNIFORM_BUFFER, new Float32Array([1, 1, 0, 0]), gl.DYNAMIC_DRAW);
    gl.useProgram(this.program);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, PARAMS_BINDING, this.paramsUbo);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, VIEW_BINDING, this.viewUbo);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.curve);
    gl.bindVertexArray(this.vao);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    gl.readPixels(0, 0, this.histW, this.histH, gl.RGBA, gl.UNSIGNED_BYTE, this.histPixels);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);

    const px = this.histPixels;
    const h: Histogram = {
      r: new Uint32Array(256),
      g: new Uint32Array(256),
      b: new Uint32Array(256),
      y: new Uint32Array(256),
      total: this.histW * this.histH,
    };
    for (let i = 0; i < px.length; i += 4) {
      const r = px[i], g = px[i + 1], b = px[i + 2];
      h.r[r]++;
      h.g[g]++;
      h.b[b]++;
      h.y[(r * 54 + g * 183 + b * 19) >> 8]++;
    }
    this.onHistogram(h);
    // The main canvas was not touched; no redraw needed.
  }

  dispose() {
    cancelAnimationFrame(this.frame);
    clearTimeout(this.histTimer);
    const gl = this.gl;
    if (this.texture) gl.deleteTexture(this.texture);
    gl.deleteTexture(this.curve);
    if (this.histTex) gl.deleteTexture(this.histTex);
    if (this.histFbo) gl.deleteFramebuffer(this.histFbo);
    gl.deleteBuffer(this.paramsUbo);
    gl.deleteBuffer(this.viewUbo);
    gl.deleteVertexArray(this.vao);
    gl.deleteProgram(this.program);
  }
}

function setLinearClamp(gl: WebGL2RenderingContext) {
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
}

function compile(gl: WebGL2RenderingContext, type: number, src: string) {
  const s = gl.createShader(type)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    throw new Error(`shader compile failed: ${gl.getShaderInfoLog(s)}`);
  }
  return s;
}

function link(gl: WebGL2RenderingContext, vs: string, fs: string) {
  const p = gl.createProgram()!;
  gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, vs));
  gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    throw new Error(`program link failed: ${gl.getProgramInfoLog(p)}`);
  }
  return p;
}

function bindBlock(gl: WebGL2RenderingContext, p: WebGLProgram, name: string, binding: number) {
  const idx = gl.getUniformBlockIndex(p, name);
  if (idx === gl.INVALID_INDEX) throw new Error(`uniform block ${name} not found`);
  gl.uniformBlockBinding(p, idx, binding);
}
