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

/** Wait this long after the view settles before fetching full-resolution pixels. */
const DETAIL_DELAY_MS = 120;
/** Fetch this much extra around the visible area (fraction of its size, per side). */
const DETAIL_MARGIN = 0.25;
export const MIN_ZOOM = 0.05;
export const MAX_ZOOM = 8;

/** Offset of the framing block (geo0, geo1) in the params uniform (see `DevelopParams`). */
const GEO_OFFSET = 68;

/** The framed output: size in source pixels and the frame → source-uv mapping. */
export interface Framing {
  w: number;
  h: number;
  /** uv = [m[0], m[1], m[2]] · (f, 1) and [m[3], m[4], m[5]] · (f, 1). */
  m: number[];
}

/** Framing carried in a params block (computed in Rust, so it can't disagree with export). */
export function framingOf(params: Float32Array): Framing {
  const g = params.subarray(GEO_OFFSET, GEO_OFFSET + 8);
  return { w: g[3], h: g[7], m: [g[0], g[1], g[2], g[4], g[5], g[6]] };
}

function toSource(fr: Framing, fx: number, fy: number): [number, number] {
  const m = fr.m;
  return [m[0] * fx + m[1] * fy + m[2], m[3] * fx + m[4] * fy + m[5]];
}

/** A rectangle in uv units (0..1): of the frame, or of the source, depending on use. */
interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A request for full-resolution pixels: a region in full-res pixels, downsampled by 2^level. */
export interface DetailRequest {
  x: number;
  y: number;
  w: number;
  h: number;
  level: number;
}

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
  /** Full-resolution source size. */
  private fullW = 1;
  private fullH = 1;
  /** Framed output of the main view (and the compare pane); zoom is relative to its pixels. */
  private out: Framing = { w: 1, h: 1, m: [1, 0, 0, 0, 1, 0] };
  private compareOut: Framing = this.out;
  /** Called after each draw with the main frame's on-screen rect (CSS px, canvas-relative). */
  onLayout: ((r: { x: number; y: number; w: number; h: number }) => void) | null = null;
  private lastLayout = "";
  /** Device pixels per full-res pixel, or null to fit the canvas. */
  private zoom: number | null = null;
  /** Point of the image (uv) at the canvas centre while zoomed. */
  private center = { x: 0.5, y: 0.5 };
  private detail: { texture: WebGLTexture; rect: Rect; level: number } | null = null;
  private detailWanted: string | null = null;
  private detailTimer: ReturnType<typeof setTimeout> | undefined;
  /** Side-by-side compare: the right pane renders these params (and curve). */
  private compareUbo: WebGLBuffer;
  private compareCurve: WebGLTexture;
  private comparing = false;
  /** Asked to fetch full-res pixels for the visible area; reply with `setDetail`. */
  onNeedDetail: ((req: DetailRequest) => void) | null = null;
  /** Called whenever the zoom level changes (scale in device px per full-res px, fit?). */
  onView: ((scale: number, fit: boolean) => void) | null = null;
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
    this.compareUbo = gl.createBuffer()!;
    this.viewUbo = gl.createBuffer()!;
    bindBlock(gl, this.program, src.paramsBlock, PARAMS_BINDING);
    bindBlock(gl, this.program, src.viewBlock, VIEW_BINDING);
    gl.useProgram(this.program);
    gl.uniform1i(gl.getUniformLocation(this.program, src.imageSampler), 0);
    gl.uniform1i(gl.getUniformLocation(this.program, src.curveSampler), 1);

    // Identity curves until the first real ones arrive.
    this.curve = identityCurve(gl);
    this.compareCurve = identityCurve(gl);
    this.stillCurve = identityCurve(gl);
    this.stillUbo = gl.createBuffer()!;
  }

  /** Upload a tone-curve LUT (RGBA16F, CURVE_SIZE × 1). */
  setCurve(rgba16f: Uint16Array) {
    uploadCurve(this.gl, this.curve, rgba16f);
    this.requestDraw(true);
  }

  /** Compare with another version side by side (null to stop). Zoom and pan are shared. */
  setCompare(params: Float32Array | null, curve?: Uint16Array | null) {
    const gl = this.gl;
    const was = this.comparing;
    this.comparing = params !== null;
    if (params) {
      this.compareOut = framingOf(params);
      gl.bindBuffer(gl.UNIFORM_BUFFER, this.compareUbo);
      gl.bufferData(gl.UNIFORM_BUFFER, params, gl.DYNAMIC_DRAW);
    }
    if (curve) uploadCurve(gl, this.compareCurve, curve);
    if (was !== this.comparing) this.setZoom(this.isFit ? null : this.scale);
    else this.requestDraw();
  }

  get isComparing() {
    return this.comparing;
  }

  // ---- stills (thumbnails) ----
  private stillUbo: WebGLBuffer;
  private stillCurve: WebGLTexture;

  /**
   * Render the whole image with `params` (and `curve`) at `edge` px on the long side, into a
   * new 2D canvas. Used for branch and library thumbnails; doesn't disturb the main view.
   */
  renderStill(edge: number, params: Float32Array, curve: Uint16Array | null): HTMLCanvasElement | null {
    const gl = this.gl;
    if (!this.texture) return null;
    const fr = framingOf(params);
    const s = Math.min(1, edge / Math.max(fr.w, fr.h));
    const w = Math.max(1, Math.round(fr.w * s));
    const h = Math.max(1, Math.round(fr.h * s));
    const tex = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, w, h);
    const fbo = gl.createFramebuffer()!;
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    gl.viewport(0, 0, w, h);

    gl.bindBuffer(gl.UNIFORM_BUFFER, this.stillUbo);
    gl.bufferData(gl.UNIFORM_BUFFER, params, gl.DYNAMIC_DRAW);
    if (curve) uploadCurve(gl, this.stillCurve, curve);
    const whole = { x: 0, y: 0, w: 1, h: 1 };
    gl.bindBuffer(gl.UNIFORM_BUFFER, this.viewUbo);
    gl.bufferData(gl.UNIFORM_BUFFER, this.viewBlock(1, 1, whole, whole, whole), gl.DYNAMIC_DRAW);
    gl.useProgram(this.program);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, PARAMS_BINDING, this.stillUbo);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, VIEW_BINDING, this.viewUbo);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, curve ? this.stillCurve : this.curve);
    gl.bindVertexArray(this.vao);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    const px = new Uint8Array(w * h * 4);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.deleteFramebuffer(fbo);
    gl.deleteTexture(tex);

    // GL rows run bottom-up.
    const out = document.createElement("canvas");
    out.width = w;
    out.height = h;
    const ctx = out.getContext("2d")!;
    const data = ctx.createImageData(w, h);
    const row = w * 4;
    for (let y = 0; y < h; y++) data.data.set(px.subarray((h - 1 - y) * row, (h - y) * row), y * row);
    ctx.putImageData(data, 0, 0);
    return out;
  }

  setImage(width: number, height: number, rgb16f: Uint16Array, fullW = width, fullH = height) {
    const gl = this.gl;
    if (this.texture) gl.deleteTexture(this.texture);
    this.dropDetail();
    this.fullW = fullW;
    this.fullH = fullH;
    this.out = this.compareOut = { w: fullW, h: fullH, m: [1, 0, 0, 0, 1, 0] };
    this.zoom = null;
    this.center = { x: 0.5, y: 0.5 };
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
    const prev = this.out;
    this.out = framingOf(params);
    if (Math.abs(prev.w - this.out.w) > 0.5 || Math.abs(prev.h - this.out.h) > 0.5) {
      this.resizeHistogramTarget();
      this.setZoom(this.isFit ? null : this.scale); // re-fit / re-clamp to the new frame
    }
    this.requestDraw(true);
  }

  /** The current framing of the main view. */
  get framing(): Framing {
    return this.out;
  }

  /** Full-resolution pixels for `region` (full-res px), `width × height` RGB16F samples. */
  setDetail(region: { x: number; y: number; w: number; h: number }, level: number, width: number, height: number, rgb16f: Uint16Array) {
    const gl = this.gl;
    this.dropDetail();
    const texture = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 2);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB16F, width, height, 0, gl.RGB, gl.HALF_FLOAT, rgb16f);
    setLinearClamp(gl);
    const rect = { x: region.x / this.fullW, y: region.y / this.fullH, w: region.w / this.fullW, h: region.h / this.fullH };
    this.detail = { texture, rect, level };
    this.requestDraw();
  }

  private dropDetail() {
    if (this.detail) this.gl.deleteTexture(this.detail.texture);
    this.detail = null;
    this.detailWanted = null;
    clearTimeout(this.detailTimer);
  }

  // ---- zoom & pan ----

  get hasDetail() {
    return this.detail !== null;
  }

  get isFit() {
    return this.zoom === null;
  }

  /** Current scale: device pixels per full-res pixel. */
  get scale() {
    return this.zoom ?? this.fitScale();
  }

  /** CSS width of one pane (half the canvas while comparing). */
  private get paneW() {
    return this.comparing ? this.canvas.clientWidth / 2 : this.canvas.clientWidth;
  }

  /** Map a canvas x coordinate into its pane. */
  private paneX(cssX: number) {
    const p = this.paneW;
    return this.comparing && cssX >= p ? cssX - p : cssX;
  }

  private fitScale() {
    const dpr = window.devicePixelRatio || 1;
    const availW = Math.max(1, this.paneW - 2 * MARGIN_CSS_PX) * dpr;
    const availH = Math.max(1, this.canvas.clientHeight - 2 * MARGIN_CSS_PX) * dpr;
    return this.fitFor(this.out, availW, availH);
  }

  private fitFor(o: Framing, availW: number, availH: number) {
    return Math.min(availW / o.w, availH / o.h, 1);
  }

  /** Zoom to `scale` (null = fit), keeping the image point under (cssX, cssY) in place. */
  setZoom(scale: number | null, cssX?: number, cssY?: number) {
    const dpr = window.devicePixelRatio || 1;
    const cw = this.paneW * dpr;
    const ch = this.canvas.clientHeight * dpr;
    if (cssX !== undefined) cssX = this.paneX(cssX);
    if (scale === null || scale <= this.fitScale() + 1e-6) {
      this.zoom = null;
      this.center = { x: 0.5, y: 0.5 };
    } else {
      scale = Math.min(MAX_ZOOM, scale);
      const px = (cssX ?? this.paneW / 2) * dpr;
      const py = (cssY ?? this.canvas.clientHeight / 2) * dpr;
      const before = this.layout(cw, ch);
      // Image point under the cursor now…
      const u = (px - before.x) / before.w;
      const v = (py - before.y) / before.h;
      // …stays under the cursor at the new scale.
      const w = this.out.w * scale;
      const h = this.out.h * scale;
      this.zoom = scale;
      this.center = { x: (cw / 2 - (px - u * w)) / w, y: (ch / 2 - (py - v * h)) / h };
    }
    this.clampCenter(cw, ch);
    this.onView?.(this.scale, this.isFit);
    this.requestDraw();
  }

  /** Pan by a CSS-pixel drag delta. */
  pan(dxCss: number, dyCss: number) {
    if (this.zoom === null) return;
    const dpr = window.devicePixelRatio || 1;
    this.center.x -= (dxCss * dpr) / (this.out.w * this.zoom);
    this.center.y -= (dyCss * dpr) / (this.out.h * this.zoom);
    this.clampCenter(this.paneW * dpr, this.canvas.clientHeight * dpr);
    this.requestDraw();
  }

  /** Is (cssX, cssY) over the image? */
  hit(cssX: number, cssY: number) {
    const dpr = window.devicePixelRatio || 1;
    const r = this.layout(this.paneW * dpr, this.canvas.clientHeight * dpr);
    const x = this.paneX(cssX) * dpr, y = cssY * dpr;
    return x >= r.x && y >= r.y && x <= r.x + r.w && y <= r.y + r.h;
  }

  private clampCenter(cw: number, ch: number) {
    if (this.zoom === null) return;
    const w = this.out.w * this.zoom;
    const h = this.out.h * this.zoom;
    const clamp = (c: number, view: number, size: number) =>
      size <= view ? 0.5 : Math.min(1 - view / 2 / size, Math.max(view / 2 / size, c));
    this.center.x = clamp(this.center.x, cw, w);
    this.center.y = clamp(this.center.y, ch, h);
  }

  /** Where the whole frame sits on a `cw × ch` device-pixel pane. */
  private layout(cw: number, ch: number, o: Framing = this.out) {
    const dpr = window.devicePixelRatio || 1;
    const m = 2 * MARGIN_CSS_PX * dpr;
    const s = this.zoom ?? this.fitFor(o, Math.max(1, cw - m), Math.max(1, ch - m));
    const w = o.w * s;
    const h = o.h * s;
    if (this.zoom === null) return { x: (cw - w) / 2, y: (ch - h) / 2, w, h };
    return { x: cw / 2 - this.center.x * w, y: ch / 2 - this.center.y * h, w, h };
  }

  /** View uniforms (xform, quad, tex, frame) to draw `quad` of the frame from a texture holding `tex` of the source. */
  private viewBlock(cw: number, ch: number, img: { x: number; y: number; w: number; h: number }, quad: Rect, tex: Rect) {
    const x = img.x + quad.x * img.w;
    const y = img.y + quad.y * img.h;
    const w = quad.w * img.w;
    const h = quad.h * img.h;
    return new Float32Array([
      w / cw, h / ch, ((x + w / 2) / cw) * 2 - 1, 1 - ((y + h / 2) / ch) * 2,
      quad.x, quad.y, quad.w, quad.h,
      tex.x, tex.y, tex.w, tex.h,
      this.fullW, this.fullH, 0, 0,
    ]);
  }

  /** While zoomed past the preview's resolution, make sure full-res pixels for the view are on their way. */
  private wantDetail(cw: number, ch: number, img: { x: number; y: number; w: number; h: number }) {
    const s = this.scale;
    if (!this.onNeedDetail || s <= (this.imageW / this.fullW) * 1.05) return;
    // Visible part of the frame, then the source area it reads.
    const vx0 = Math.max(0, -img.x / img.w), vy0 = Math.max(0, -img.y / img.h);
    const vx1 = Math.min(1, (cw - img.x) / img.w), vy1 = Math.min(1, (ch - img.y) / img.h);
    if (vx1 <= vx0 || vy1 <= vy0) return;
    const pts = [toSource(this.out, vx0, vy0), toSource(this.out, vx1, vy0), toSource(this.out, vx0, vy1), toSource(this.out, vx1, vy1)];
    const u0 = Math.max(0, Math.min(...pts.map((p) => p[0]))), u1 = Math.min(1, Math.max(...pts.map((p) => p[0])));
    const w0 = Math.max(0, Math.min(...pts.map((p) => p[1]))), w1 = Math.min(1, Math.max(...pts.map((p) => p[1])));
    if (u1 <= u0 || w1 <= w0) return;
    const level = s >= 1 ? 0 : Math.floor(Math.log2(1 / s));
    const d = this.detail;
    if (d && d.level === level && d.rect.x <= u0 + 1e-6 && d.rect.y <= w0 + 1e-6 && d.rect.x + d.rect.w >= u1 - 1e-6 && d.rect.y + d.rect.h >= w1 - 1e-6) {
      return;
    }
    const mx = (u1 - u0) * DETAIL_MARGIN, my = (w1 - w0) * DETAIL_MARGIN;
    const x0 = Math.floor(Math.max(0, u0 - mx) * this.fullW);
    const y0 = Math.floor(Math.max(0, w0 - my) * this.fullH);
    const x1 = Math.ceil(Math.min(1, u1 + mx) * this.fullW);
    const y1 = Math.ceil(Math.min(1, w1 + my) * this.fullH);
    const req = { x: x0, y: y0, w: x1 - x0, h: y1 - y0, level };
    const key = JSON.stringify(req);
    if (key === this.detailWanted) return;
    this.detailWanted = key;
    clearTimeout(this.detailTimer);
    this.detailTimer = setTimeout(() => this.onNeedDetail?.(req), DETAIL_DELAY_MS);
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

    // One pane, or two side by side (current | compare) with a hairline between them.
    const panes = this.comparing ? 2 : 1;
    const gap = this.comparing ? Math.round(dpr) : 0;
    const pw = Math.floor((w - gap) / panes);
    const img = this.layout(pw, h);
    const whole = { x: 0, y: 0, w: 1, h: 1 };
    const key = `${img.x / dpr},${img.y / dpr},${img.w / dpr},${img.h / dpr}`;
    if (key !== this.lastLayout) {
      this.lastLayout = key;
      this.onLayout?.({ x: img.x / dpr, y: img.y / dpr, w: img.w / dpr, h: img.h / dpr });
    }
    gl.useProgram(this.program);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, VIEW_BINDING, this.viewUbo);
    gl.bindVertexArray(this.vao);
    for (let i = 0; i < panes; i++) {
      gl.viewport(i ? w - pw : 0, 0, pw, h);
      gl.bindBufferBase(gl.UNIFORM_BUFFER, PARAMS_BINDING, i ? this.compareUbo : this.paramsUbo);
      gl.activeTexture(gl.TEXTURE1);
      gl.bindTexture(gl.TEXTURE_2D, i ? this.compareCurve : this.curve);
      const pane = i ? this.layout(pw, h, this.compareOut) : img;

      // The preview covers the whole frame; full-res pixels (when zoomed in) draw on top,
      // wherever their texture has data.
      gl.bindBuffer(gl.UNIFORM_BUFFER, this.viewUbo);
      gl.bufferData(gl.UNIFORM_BUFFER, this.viewBlock(pw, h, pane, whole, whole), gl.DYNAMIC_DRAW);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, this.texture);
      gl.drawArrays(gl.TRIANGLES, 0, 6);

      if (this.detail && this.scale > this.imageW / this.fullW) {
        gl.bufferData(gl.UNIFORM_BUFFER, this.viewBlock(pw, h, pane, whole, this.detail.rect), gl.DYNAMIC_DRAW);
        gl.bindTexture(gl.TEXTURE_2D, this.detail.texture);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
      }
    }
    gl.viewport(0, 0, w, h);
    this.wantDetail(pw, h, img);
    // Must run in the same task as the draw: the drawing buffer is not preserved.
    this.onDrawn?.(this.canvas);
    if (this.histDirty) this.scheduleHistogram();
  }

  // ---- histogram ----

  private resizeHistogramTarget() {
    const gl = this.gl;
    const s = Math.min(1, HISTOGRAM_EDGE / Math.max(this.out.w, this.out.h));
    this.histW = Math.max(1, Math.round(this.out.w * s));
    this.histH = Math.max(1, Math.round(this.out.h * s));
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
    gl.clearColor(0, 0, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.bindBuffer(gl.UNIFORM_BUFFER, this.viewUbo);
    const whole = { x: 0, y: 0, w: 1, h: 1 };
    gl.bufferData(gl.UNIFORM_BUFFER, this.viewBlock(1, 1, { x: 0, y: 0, w: 1, h: 1 }, whole, whole), gl.DYNAMIC_DRAW);
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
    this.dropDetail();
    const gl = this.gl;
    if (this.texture) gl.deleteTexture(this.texture);
    gl.deleteTexture(this.curve);
    gl.deleteTexture(this.compareCurve);
    gl.deleteTexture(this.stillCurve);
    gl.deleteBuffer(this.compareUbo);
    gl.deleteBuffer(this.stillUbo);
    if (this.histTex) gl.deleteTexture(this.histTex);
    if (this.histFbo) gl.deleteFramebuffer(this.histFbo);
    gl.deleteBuffer(this.paramsUbo);
    gl.deleteBuffer(this.viewUbo);
    gl.deleteVertexArray(this.vao);
    gl.deleteProgram(this.program);
  }
}

function identityCurve(gl: WebGL2RenderingContext): WebGLTexture {
  const t = gl.createTexture()!;
  const ramp = new Float32Array(CURVE_SIZE * 4);
  for (let i = 0; i < CURVE_SIZE; i++) ramp.fill(i / (CURVE_SIZE - 1), i * 4, i * 4 + 3), (ramp[i * 4 + 3] = 1);
  gl.bindTexture(gl.TEXTURE_2D, t);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, CURVE_SIZE, 1, 0, gl.RGBA, gl.FLOAT, ramp);
  setLinearClamp(gl);
  return t;
}

function uploadCurve(gl: WebGL2RenderingContext, t: WebGLTexture, rgba16f: Uint16Array) {
  gl.bindTexture(gl.TEXTURE_2D, t);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 2);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, CURVE_SIZE, 1, 0, gl.RGBA, gl.HALF_FLOAT, rgba16f);
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
