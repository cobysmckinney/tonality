// WebGL2 preview renderer. Runs the GLSL that naga transpiled from develop.wgsl.
// The image is uploaded once; edits only change a 96-byte uniform block.
import type { GlslProgram } from "$lib/bindings/GlslProgram";

const PARAMS_BINDING = 0;
const VIEW_BINDING = 1;
const MARGIN_CSS_PX = 24;

export class PreviewRenderer {
  private gl: WebGL2RenderingContext;
  private program: WebGLProgram;
  private vao: WebGLVertexArrayObject;
  private texture: WebGLTexture | null = null;
  private paramsUbo: WebGLBuffer;
  private viewUbo: WebGLBuffer;
  private imageW = 0;
  private imageH = 0;
  private hasParams = false;
  private frame = 0;
  /** Called after each completed draw of an image (used by the dev snapshot hook). */
  onDrawn: ((canvas: HTMLCanvasElement) => void) | null = null;

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
  }

  setImage(width: number, height: number, rgb16f: Uint16Array) {
    const gl = this.gl;
    if (this.texture) gl.deleteTexture(this.texture);
    this.texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 2);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB16F, width, height, 0, gl.RGB, gl.HALF_FLOAT, rgb16f);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    this.imageW = width;
    this.imageH = height;
    this.requestDraw();
  }

  setParams(params: Float32Array) {
    const gl = this.gl;
    gl.bindBuffer(gl.UNIFORM_BUFFER, this.paramsUbo);
    gl.bufferData(gl.UNIFORM_BUFFER, params, gl.DYNAMIC_DRAW);
    this.hasParams = true;
    this.requestDraw();
  }

  /** Schedule a redraw on the next animation frame (coalesces multiple requests). */
  requestDraw() {
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
    gl.bindVertexArray(this.vao);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    // Must run in the same task as the draw: the drawing buffer is not preserved.
    this.onDrawn?.(this.canvas);
  }

  dispose() {
    cancelAnimationFrame(this.frame);
    const gl = this.gl;
    if (this.texture) gl.deleteTexture(this.texture);
    gl.deleteBuffer(this.paramsUbo);
    gl.deleteBuffer(this.viewUbo);
    gl.deleteVertexArray(this.vao);
    gl.deleteProgram(this.program);
  }
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
