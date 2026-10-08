import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeft,
  Check,
  Crop,
  Download,
  Ellipsis,
  FolderOpen,
  Heart,
  History as HistoryIcon,
  Info as InfoIcon,
  Layers,
  LucideIcon,
  Redo2,
  SlidersHorizontal,
  SquareSplitHorizontal,
  SwatchBook,
  Undo2,
  X,
} from "lucide-react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Adjustments, DEFAULTS, isAsShot } from "../../adjustments";
import { withPreset } from "../../presets";
import { api, Photo, PhotoInfo, previewUrl, Region, thumbUrl } from "../../api";
import * as format from "../../format";
import { albumEntries } from "../../menus";
import { MenuEntry, neighbours, SidePanel as SidePanelName, useStore, visiblePhotos } from "../../store";
import { editorKeysBlocked } from "../../shortcuts";
import { menuBelow, useTitle } from "../Toolbar";
import { frameSize } from "../../crop";
import { AdjustPanel } from "./AdjustPanel";
import { CropOverlay } from "./CropOverlay";
import { CropPanel } from "./CropPanel";
import { Histogram } from "./Histogram";
import { MaskOverlay } from "./MaskOverlay";
import { MasksPanel } from "./MasksPanel";
import { HistoryPanel } from "./HistoryPanel";
import { PresetsPanel } from "./PresetsPanel";

const STRIP_ITEM = 54;
const STRIP_GAP = 4;
const MAX_ZOOM = 4;

/** The row of neighbouring photos under the open one. Mounts only what is on screen. */
function Filmstrip({ photos, current }: { photos: Photo[]; current: number }) {
  const scroller = useRef<HTMLDivElement>(null);
  const [range, setRange] = useState({ left: 0, width: 0 });
  const index = photos.findIndex((p) => p.id === current);
  const pitch = STRIP_ITEM + STRIP_GAP;

  const measure = () => {
    const element = scroller.current!;
    setRange({ left: element.scrollLeft, width: element.clientWidth });
  };

  useLayoutEffect(() => {
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(scroller.current!);
    return () => observer.disconnect();
  }, []);

  // Keep the open photo centred as you step through.
  useLayoutEffect(() => {
    const element = scroller.current!;
    element.scrollLeft = index * pitch + STRIP_ITEM / 2 - element.clientWidth / 2;
    measure();
  }, [index, pitch]);

  const first = Math.max(0, Math.floor(range.left / pitch) - 6);
  const last = Math.min(photos.length, Math.ceil((range.left + range.width) / pitch) + 6);

  return (
    <div className="filmstrip" ref={scroller} onScroll={measure}>
      <div className="filmstrip-track" style={{ width: photos.length * pitch - STRIP_GAP }}>
        {photos.slice(first, last).map((photo, offset) => (
          <button
            key={photo.id}
            className={`strip-item ${photo.id === current ? "current" : ""} ${photo.flag === -1 ? "rejected" : ""}`}
            style={{ left: (first + offset) * pitch, width: STRIP_ITEM }}
            aria-label={photo.fileName}
            // Arrow keys already step through photos; keeping these out of
            // the tab order keeps Tab for the tools.
            tabIndex={-1}
            onClick={() => useStore.getState().openPhoto(photo.id)}
          >
            <img src={thumbUrl(photo)} alt="" draggable={false} decoding="async" />
          </button>
        ))}
      </div>
    </div>
  );
}

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="fact">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** The folder a photo's file is in: in the library, the one named for the day it was taken. */
const folderOf = (path: string) => path.split(/[\\/]/).slice(-2, -1)[0] ?? path;

function Info({ photo }: { photo: Photo }) {
  const [info, setInfo] = useState<PhotoInfo | null>(null);
  const size = useStore((s) => s.editor.size);
  const adjustments = useStore((s) => s.editor.adjustments);
  useEffect(() => {
    let current = true;
    setInfo(null);
    void api.getPhotoInfo(photo.id).then((loaded) => current && setInfo(loaded));
    return () => {
      current = false;
    };
  }, [photo.id]);

  const camera = [info?.make, info?.model].filter(Boolean).join(" ");
  const readings = info
    ? [
        { label: "Shutter", value: info.shutter !== null ? format.shutter(info.shutter) : null },
        { label: "Aperture", value: info.aperture !== null ? format.aperture(info.aperture) : null },
        { label: "Focal length", value: info.focalLength !== null ? format.focalLength(info.focalLength) : null },
        { label: "ISO", value: info.iso !== null ? String(info.iso) : null },
      ].filter((reading) => reading.value !== null)
    : [];
  // What an export at full size would measure, once the crop is taken into account.
  const cropped = size && frameSize(size, adjustments, false);
  const croppedSize = cropped && { width: Math.round(cropped.width), height: Math.round(cropped.height) };
  // Against the size the photo is held at, which is smaller than its own if the graphics card can't hold it whole.
  const isCropped =
    croppedSize && size
      ? !(croppedSize.width === size.width && croppedSize.height === size.height) &&
        !(croppedSize.width === size.height && croppedSize.height === size.width)
      : false;

  return (
    <div className="info">
      <header className="info-header">
        <h2>{photo.fileName}</h2>
        <p>{format.longDateTime(format.parseLocal(photo.takenAt))}</p>
      </header>

      {readings.length > 0 && (
        <dl className="readings">
          {readings.map(({ label, value }) => (
            <div key={label}>
              <dd>{value}</dd>
              <dt>{label}</dt>
            </div>
          ))}
        </dl>
      )}

      {info && (camera || info.lens) && (
        <dl className="facts">
          {camera && <Fact label="Camera">{camera}</Fact>}
          {info.lens && <Fact label="Lens">{info.lens}</Fact>}
        </dl>
      )}

      {info && (
        <dl className="facts">
          <Fact label="Format">
            {photo.kind === "raw" ? "RAW" : "Image"}
            {photo.hasJpeg && " + JPEG"}
          </Fact>
          {photo.width !== null && photo.height !== null && (
            <Fact label="Dimensions">
              {photo.width} × {photo.height}
              <span className="fact-note">{format.megapixels(photo.width, photo.height)}</span>
            </Fact>
          )}
          {isCropped && croppedSize && (
            <Fact label="Cropped to">
              {croppedSize.width} × {croppedSize.height}
              <span className="fact-note">{format.megapixels(croppedSize.width, croppedSize.height)}</span>
            </Fact>
          )}
          <Fact label="File size">{format.fileSize(info.fileSize)}</Fact>
          <Fact label="Imported">{format.longDateTime(new Date(info.importedAt * 1000))}</Fact>
          {info.albums.length > 0 && <Fact label="Albums">{info.albums.join(", ")}</Fact>}
          <Fact label="Folder">
            <button className="link" title={info.path} onClick={() => void revealItemInDir(info.path)}>
              {folderOf(info.path)}
              <FolderOpen size={13} />
            </button>
          </Fact>
        </dl>
      )}
    </div>
  );
}

/** How far in the picture is zoomed (screen pixels per photo pixel; null fits it to the stage) and where it's centred. */
interface View {
  zoom: number | null;
  x: number;
  y: number;
}

const FIT: View = { zoom: null, x: 0.5, y: 0.5 };
const clamp = (v: number, low: number, high: number) => Math.min(high, Math.max(low, v));

interface FrameRequest {
  id: number;
  adjustments: Adjustments;
  region: Region;
  width: number;
  height: number;
  showClipping: boolean;
  uncropped: boolean;
  maskOverlay: number | null;
}

/**
 * The photo itself: a canvas the backend paints edited frames into. It asks
 * for exactly the pixels on screen, so zooming in asks for a smaller part of
 * the picture at the same size. In the crop tool it shows the whole photo,
 * tilted by the straighten angle, with the crop frame over it.
 */
function Stage({ photo, onZoomChange }: { photo: Photo; onZoomChange: (label: string) => void }) {
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [area, setArea] = useState({ width: 0, height: 0 });
  const [view, setView] = useState<View>(FIT);
  /** What is on the canvas: which photo, which part of it, and whether it is the crop tool's uncropped view. */
  const [painted, setPainted] = useState<{ id: number; region: Region; uncropped: boolean } | null>(null);

  const ready = useStore((s) => s.editor.ready && s.editor.photoId === photo.id);
  const photoSize = useStore((s) => s.editor.size);
  const own = useStore((s) => s.editor.adjustments);
  const trying = useStore((s) => s.editor.preview);
  // A preset being tried is laid over the edits as they are now, so undo or a paste shows through it.
  const current = useMemo(() => (trying ? withPreset(own, trying.settings) : own), [own, trying]);
  const showOriginal = useStore((s) => s.editor.showOriginal);
  const showClipping = useStore((s) => s.editor.showClipping && !s.editor.showOriginal);
  const uncropped = useStore((s) => s.sidePanel === "crop" && !s.editor.showOriginal);
  const masking = useStore((s) => s.sidePanel === "masks" && !s.editor.showOriginal);
  const circling = useStore((s) => s.sidePanel === "masks" && s.editor.circling !== null && s.editor.finding === null);
  const maskOverlay = useStore((s) =>
    s.sidePanel === "masks" && s.editor.showMask && !s.editor.showOriginal ? s.editor.maskId : null,
  );

  // A vignette follows the crop, so it has no meaning on the uncropped view.
  const adjustments = useMemo(
    () => (showOriginal ? DEFAULTS : uncropped ? { ...current, vignette: 0 } : current),
    [current, showOriginal, uncropped],
  );
  /** The size, in photo pixels, of the picture being shown. */
  const frame = useMemo(
    () => (photoSize ? frameSize(photoSize, adjustments, uncropped) : null),
    [photoSize, adjustments, uncropped],
  );

  useLayoutEffect(() => {
    const element = box.current!;
    const measure = () => setArea({ width: element.clientWidth, height: element.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => setView(FIT), [photo.id, uncropped]);

  // Everything below is in device pixels, so frames are sharp on scaled displays.
  const geometry = useMemo(() => {
    if (!ready || !frame || area.width === 0 || area.height === 0) return null;
    const ratio = window.devicePixelRatio || 1;
    const available = { width: Math.floor(area.width * ratio), height: Math.floor(area.height * ratio) };
    const fit = Math.min(available.width / frame.width, available.height / frame.height, 1);
    // The crop tool always shows everything.
    const zoom = view.zoom === null || uncropped ? fit : clamp(view.zoom, fit, MAX_ZOOM);
    const width = Math.max(1, Math.min(available.width, Math.round(frame.width * zoom)));
    const height = Math.max(1, Math.min(available.height, Math.round(frame.height * zoom)));
    const span = { width: width / (frame.width * zoom), height: height / (frame.height * zoom) };
    const region: Region = {
      x: clamp(view.x - span.width / 2, 0, 1 - span.width),
      y: clamp(view.y - span.height / 2, 0, 1 - span.height),
      ...span,
    };
    return { ratio, fit, zoom, width, height, region, fitted: zoom <= fit };
  }, [ready, frame, area, view, uncropped]);

  useEffect(() => {
    onZoomChange(!geometry || geometry.fitted ? "Fit" : `${Math.round(geometry.zoom * 100)}%`);
  }, [geometry, onZoomChange]);

  // One frame is in flight at a time; while it is, only the newest request is kept.
  const wanted = useRef<FrameRequest | null>(null);
  const busy = useRef(false);
  const pump = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    while (wanted.current) {
      const request = wanted.current;
      wanted.current = null;
      try {
        const rendered = await api.renderFrame(
          request.id,
          request.adjustments,
          request.region,
          request.width,
          request.height,
          request.showClipping,
          request.uncropped,
          request.maskOverlay,
        );
        const target = canvas.current;
        if (target && useStore.getState().editor.photoId === request.id) {
          if (target.width !== rendered.pixels.width || target.height !== rendered.pixels.height) {
            target.width = rendered.pixels.width;
            target.height = rendered.pixels.height;
          }
          target.getContext("2d")!.putImageData(rendered.pixels, 0, 0);
          useStore.getState().noteFrame(rendered);
          setPainted({ id: request.id, region: request.region, uncropped: request.uncropped });
        }
      } catch {
        // The photo was closed or swapped while this frame was on its way.
      }
    }
    busy.current = false;
  }, []);

  useEffect(() => {
    if (!geometry) return;
    const { region, width, height } = geometry;
    wanted.current = { id: photo.id, adjustments, region, width, height, showClipping, uncropped, maskOverlay };
    void pump();
  }, [geometry, adjustments, showClipping, uncropped, maskOverlay, photo.id, pump]);

  /** Zooms to `zoom`, keeping the point under the pointer where it is. */
  const zoomAt = useCallback(
    (zoom: number | null, clientX: number, clientY: number) => {
      if (!geometry || !frame || uncropped) return;
      // The box, not the canvas: a waiting frame may sit off it.
      const shown = canvas.current!.closest(".canvas-box")!.getBoundingClientRect();
      const stage = box.current!.getBoundingClientRect();
      // The point of the picture under the pointer…
      const u = geometry.region.x + clamp((clientX - shown.left) / shown.width, 0, 1) * geometry.region.width;
      const v = geometry.region.y + clamp((clientY - shown.top) / shown.height, 0, 1) * geometry.region.height;
      if (zoom === null || zoom <= geometry.fit * 1.001) {
        setView(FIT);
        return;
      }
      // …should sit at the same place on the stage after zooming.
      const next = clamp(zoom, geometry.fit, MAX_ZOOM);
      const span = {
        width: Math.min(1, (stage.width * geometry.ratio) / (frame.width * next)),
        height: Math.min(1, (stage.height * geometry.ratio) / (frame.height * next)),
      };
      const fx = clamp((clientX - stage.left) / stage.width, 0, 1);
      const fy = clamp((clientY - stage.top) / stage.height, 0, 1);
      setView({ zoom: next, x: u + (0.5 - fx) * span.width, y: v + (0.5 - fy) * span.height });
    },
    [geometry, frame, uncropped],
  );

  const toggleZoom = useCallback(
    (clientX: number, clientY: number) => zoomAt(geometry?.fitted ? 1 : null, clientX, clientY),
    [geometry, zoomAt],
  );

  // Wheel to zoom, Z to jump between fit and 100%.
  useEffect(() => {
    const element = box.current!;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      if (geometry) zoomAt(geometry.zoom * Math.exp(-event.deltaY * 0.0015), event.clientX, event.clientY);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "z" || event.ctrlKey || event.metaKey || event.altKey || event.repeat) return;
      const typing = (event.target as HTMLElement).closest("input:not([type=range]), textarea");
      if (typing || editorKeysBlocked(useStore.getState())) return;
      const stage = element.getBoundingClientRect();
      toggleZoom(stage.left + stage.width / 2, stage.top + stage.height / 2);
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("keydown", onKey);
    return () => {
      element.removeEventListener("wheel", onWheel);
      window.removeEventListener("keydown", onKey);
    };
  }, [geometry, zoomAt, toggleZoom]);

  // Drag to pan while zoomed in.
  const drag = useRef<{ x: number; y: number } | null>(null);
  const onPointerMove = (event: React.PointerEvent) => {
    if (!drag.current || !geometry || !frame) return;
    const dx = ((event.clientX - drag.current.x) * geometry.ratio) / (frame.width * geometry.zoom);
    const dy = ((event.clientY - drag.current.y) * geometry.ratio) / (frame.height * geometry.zoom);
    drag.current = { x: event.clientX, y: event.clientY };
    const { region } = geometry;
    setView({
      zoom: geometry.zoom,
      x: clamp(region.x + region.width / 2 - dx, region.width / 2, 1 - region.width / 2),
      y: clamp(region.y + region.height / 2 - dy, region.height / 2, 1 - region.height / 2),
    });
  };

  const live = painted?.id === photo.id && geometry !== null;
  const shown = geometry && { width: geometry.width / geometry.ratio, height: geometry.height / geometry.ratio };
  // While a zoom or pan waits for its frame, the last one is scaled and moved
  // to where its part of the picture now sits, rather than stretched to fit.
  const placed =
    live && shown && painted.uncropped === uncropped
      ? {
          left: ((painted.region.x - geometry.region.x) / geometry.region.width) * shown.width,
          top: ((painted.region.y - geometry.region.y) / geometry.region.height) * shown.height,
          width: (painted.region.width / geometry.region.width) * shown.width,
          height: (painted.region.height / geometry.region.height) * shown.height,
        }
      : undefined;
  return (
    <div
      ref={box}
      className={`stage-image ${geometry && !geometry.fitted ? "zoomed" : ""}`}
      onDoubleClick={(event) => toggleZoom(event.clientX, event.clientY)}
      onPointerDown={(event) => {
        if (event.button !== 0 || !geometry || geometry.fitted) return;
        drag.current = { x: event.clientX, y: event.clientY };
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={onPointerMove}
      onPointerUp={() => (drag.current = null)}
      onPointerCancel={() => (drag.current = null)}
    >
      {/* Until the editor has drawn its first frame, show the saved preview. */}
      {!live && (
        <>
          <img src={thumbUrl(photo)} alt="" draggable={false} />
          <img key={photo.id} src={previewUrl(photo)} alt={photo.fileName} draggable={false} />
        </>
      )}
      <div className={`canvas-box ${live ? "" : "pending"}`} style={shown ?? undefined}>
        <div className="canvas-clip">
          <canvas ref={canvas} style={placed} />
        </div>
        {/* The frame goes on only once the canvas really shows the uncropped photo underneath it. */}
        {live && uncropped && painted?.uncropped && shown && frame && photoSize && (
          <CropOverlay photo={photoSize} scale={shown.width / frame.width} width={shown.width} height={shown.height} />
        )}
        {live && masking && shown && photoSize && (
          <MaskOverlay photo={photoSize} region={geometry.region} width={shown.width} height={shown.height} />
        )}
      </div>
      {!ready && <span className="stage-note">Preparing photo…</span>}
      {ready && circling && <span className="stage-note centered">Draw a loop around what you want · Esc cancels</span>}
    </div>
  );
}

/** The side panel's tools, in the order of the rail, with the key that opens each. */
const TOOLS: { panel: SidePanelName; label: string; key: string; icon: LucideIcon }[] = [
  { panel: "adjust", label: "Adjust", key: "A", icon: SlidersHorizontal },
  { panel: "crop", label: "Crop", key: "C", icon: Crop },
  { panel: "masks", label: "Masks", key: "M", icon: Layers },
  { panel: "presets", label: "Presets", key: "Shift+P", icon: SwatchBook },
  { panel: "history", label: "History", key: "H", icon: HistoryIcon },
  { panel: "info", label: "Info", key: "I", icon: InfoIcon },
];

/** The tools, as a column of icons along the window's right edge. */
function Rail() {
  const panel = useStore((s) => s.sidePanel);
  const setPanel = useStore((s) => s.setSidePanel);
  return (
    <nav className="panel rail" role="tablist" aria-orientation="vertical" aria-label="Tools">
      {TOOLS.map(({ panel: tool, label, key, icon: Icon }) => (
        <button
          key={tool}
          role="tab"
          aria-selected={panel === tool}
          aria-label={label}
          title={`${label} (${key})`}
          className={panel === tool ? "active" : ""}
          onClick={() => setPanel(tool)}
        >
          <Icon size={17} strokeWidth={1.75} />
        </button>
      ))}
    </nav>
  );
}

function SidePanel({ photo }: { photo: Photo }) {
  const panel = useStore((s) => s.sidePanel);
  const title = TOOLS.find((tool) => tool.panel === panel)?.label;
  return (
    <aside className="side" aria-label={title}>
      <h2 className="side-title">{title}</h2>
      {panel === "adjust" && <AdjustPanel />}
      {panel === "crop" && <CropPanel />}
      {panel === "masks" && <MasksPanel />}
      {panel === "presets" && <PresetsPanel />}
      {panel === "history" && <HistoryPanel />}
      {panel === "info" && <Info photo={photo} />}
    </aside>
  );
}

/** One photo, large, with the tools to edit it and the rest of the view in a filmstrip underneath. */
export function Editor({ photo, inert }: { photo: Photo; inert: boolean }) {
  const photos = useStore(visiblePhotos);
  const viewTitle = useTitle();
  const canUndo = useStore(
    (s) => neighbours(s.editor.history).undo !== null || s.editor.adjustments !== s.editor.committed,
  );
  const canRedo = useStore((s) => neighbours(s.editor.history).redo !== null);
  const edited = useStore((s) => s.editor.ready && !isAsShot(s.editor.adjustments));
  const showOriginal = useStore((s) => s.editor.showOriginal);
  const hasClipboard = useStore((s) => s.clipboard !== null);
  const [zoomLabel, setZoomLabel] = useState("Fit");
  const s = useStore.getState();
  const ids = [photo.id];

  // Load whichever photo is open; let go of the GPU when the editor closes.
  useEffect(() => void useStore.getState().loadEditor(photo.id), [photo.id]);
  useEffect(() => () => void useStore.getState().leaveEditor(), []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const state = useStore.getState();
      const target = event.target as HTMLElement;
      if (editorKeysBlocked(state) || state.openId === null) return;
      if (target.closest("input:not([type=range]), textarea")) return;
      const onSlider = target.matches("input[type=range]");
      const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
      const open = state.openId;

      if (event.ctrlKey || event.metaKey) {
        if (key === "z") void (event.shiftKey ? state.redo() : state.undo());
        else if (key === "y") void state.redo();
        else if (key === "c") void state.copyEdits(open);
        else if (key === "v") void state.pasteEdits([open]);
        else if (key === "e") void state.startExport([open]);
        else return;
        event.preventDefault();
        return;
      }
      if (event.altKey) return;

      if (key === "ArrowLeft" || key === "ArrowRight") {
        // On a slider the arrows nudge it; anywhere else they change photo.
        if (onSlider) return;
        event.preventDefault();
        const list = visiblePhotos(state);
        const next = list[list.findIndex((p) => p.id === open) + (key === "ArrowLeft" ? -1 : 1)];
        if (next) state.openPhoto(next.id);
      } else if (key === "Escape" || (key === "Enter" && state.sidePanel === "crop" && !target.closest("button"))) {
        // Escape backs out one level at a time: off a slider, out of a circle being drawn, off the chosen mask, out of the crop or mask tools, out of the photo.
        if (onSlider) target.blur();
        else if (state.editor.circling && !state.editor.finding) state.cancelCircle();
        else if (state.sidePanel === "masks" && state.editor.maskId !== null) state.selectMask(null);
        else if (state.sidePanel === "crop" || state.sidePanel === "masks") state.setSidePanel("adjust");
        else state.closePhoto();
      } else if (key === "\\") state.setShowOriginal(true);
      else if (key === "j") state.toggleClipping();
      else if (key === "a") state.setSidePanel("adjust");
      else if (key === "c") state.setSidePanel(state.sidePanel === "crop" ? "adjust" : "crop");
      else if (key === "m") state.setSidePanel(state.sidePanel === "masks" ? "adjust" : "masks");
      else if (key === "o" && state.sidePanel === "masks") state.toggleMaskOverlay();
      else if ((key === "[" || key === "]") && state.sidePanel === "masks") {
        state.setBrush({ size: Math.min(100, Math.max(1, state.brush.size + (key === "]" ? 4 : -4))) });
      }
      else if (key === "i") state.setSidePanel(state.sidePanel === "info" ? "adjust" : "info");
      else if (key === "h") state.setSidePanel(state.sidePanel === "history" ? "adjust" : "history");
      else if (key === "p" && event.shiftKey) state.setSidePanel(state.sidePanel === "presets" ? "adjust" : "presets");
      else if (key === "f") void state.toggleFavorite([open]);
      else if (key === "p") void state.toggleFlag([open], 1);
      else if (key === "x") void state.toggleFlag([open], -1);
      else if (key === "u") void state.toggleFlag([open], 0);
      else if (key === "Delete" || key === "Backspace") {
        // Only from the photo itself: with a tool's button or slider focused, a stray Backspace
        // shouldn't throw the photo away. The filmstrip counts as the photo.
        if (!target.closest("button:not(.strip-item), select, input")) void state.trash([open]);
      }
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (event.key === "\\") useStore.getState().setShowOriginal(false);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, []);

  // Everything the toolbar doesn't need to show all the time. The keys still work without it.
  const more = (): MenuEntry[] => [
    { label: "Favorite", hint: "F", checked: photo.favorite, run: () => void s.toggleFavorite(ids) },
    { label: "Pick", hint: "P", checked: photo.flag === 1, run: () => void s.toggleFlag(ids, 1) },
    { label: "Reject", hint: "X", checked: photo.flag === -1, run: () => void s.toggleFlag(ids, -1) },
    "separator",
    { label: "Copy edits", hint: "Ctrl+C", run: () => void s.copyEdits(photo.id) },
    { label: "Paste edits", hint: "Ctrl+V", disabled: !hasClipboard, run: () => void s.pasteEdits(ids) },
    { label: "Remove all edits", disabled: !edited, run: s.resetEdits },
    "separator",
    { label: "Add to album", submenu: albumEntries(ids) },
    { label: "Delete", hint: "Del", danger: true, run: () => void s.trash(ids) },
  ];

  return (
    <div className="viewer" inert={inert}>
      <section className="stage">
        <header className="toolbar">
          <button className="button quiet back" title={`Back to ${viewTitle} (Esc)`} onClick={s.closePhoto}>
            <ArrowLeft size={15} /> <span className="collapsible">{viewTitle}</span>
          </button>
          <div className="photo-title">
            <span className="file-name">{photo.fileName}</span>
            {photo.favorite && <Heart size={13} fill="currentColor" aria-label="Favorite" />}
            {photo.flag === 1 && <Check size={14} strokeWidth={2.5} className="picked" aria-label="Picked" />}
            {photo.flag === -1 && <X size={14} strokeWidth={2.5} className="rejected" aria-label="Rejected" />}
          </div>

          <div className="toolbar-controls">
            <button className="icon-button" title="Undo (Ctrl+Z)" aria-label="Undo" disabled={!canUndo} onClick={() => void s.undo()}>
              <Undo2 size={16} />
            </button>
            <button className="icon-button" title="Redo (Ctrl+Shift+Z)" aria-label="Redo" disabled={!canRedo} onClick={() => void s.redo()}>
              <Redo2 size={16} />
            </button>
            <button
              className={`icon-button ${showOriginal ? "engaged" : ""}`}
              title="Hold to see the original (\)"
              aria-label="Show original"
              disabled={!edited}
              onPointerDown={() => s.setShowOriginal(true)}
              onPointerUp={() => s.setShowOriginal(false)}
              onPointerLeave={() => s.setShowOriginal(false)}
            >
              <SquareSplitHorizontal size={16} />
            </button>
            <span className="zoom-label" title="Scroll to zoom, double-click or Z for 100%">
              {showOriginal ? "Original" : zoomLabel}
            </span>
            <button className="icon-button" title="More" aria-label="More" onClick={(event) => menuBelow(event, more())}>
              <Ellipsis size={16} />
            </button>
            <button className="button primary" title="Export this photo (Ctrl+E)" onClick={() => void s.startExport(ids)}>
              <Download size={15} /> <span className="collapsible">Export</span>
            </button>
          </div>
        </header>
        <Stage photo={photo} onZoomChange={setZoomLabel} />
      </section>
      <div className="panel side-column">
        <Histogram />
        <SidePanel photo={photo} />
      </div>
      <Rail />
      <Filmstrip photos={photos} current={photo.id} />
    </div>
  );
}
