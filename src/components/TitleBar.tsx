import { useEffect, useState } from "react";
import { Aperture, Copy, Keyboard, Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { count } from "../format";
import { firesShutter } from "../keys";
import { useStore } from "../store";

/**
 * What the app is busy with, if anything. Lives in the title bar so it is
 * in the same place whatever else is on screen, dialogs included.
 */
function Activity() {
  const activities = useStore((s) => s.activities);
  const current = activities[0];
  if (!current) return null;
  const counted = current.total !== null && current.total > 1;
  return (
    <div className="activity" role="status" aria-live="polite">
      {counted ? (
        <span
          className="progress"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={current.total!}
          aria-valuenow={current.done}
        >
          <span style={{ width: `${(current.done / current.total!) * 100}%` }} />
        </span>
      ) : (
        <span className="spinner" />
      )}
      <span className="activity-label">{current.label}</span>
      {counted && (
        <span className="activity-count">
          {count(current.done)} of {count(current.total!)}
        </span>
      )}
      {activities.length > 1 && <span className="activity-count">and {count(activities.length - 1)} more</span>}
      {current.stop && (
        <button className="icon-button small" title="Stop" aria-label={`Stop ${current.label.toLowerCase()}`} onClick={current.stop}>
          <X size={12} />
        </button>
      )}
    </div>
  );
}

/**
 * The camera capturing, while one is: take a photo from here or with F12,
 * choose whether new frames follow the last, and stop. In the title bar
 * so it stays in reach in the grid and the editor alike.
 */
function CaptureBar() {
  const capture = useStore((s) => s.capture);
  const { takePhoto, setCaptureFollow, stopCapture } = useStore.getState();

  useEffect(() => {
    if (!capture) return;
    const onKey = (event: KeyboardEvent) => {
      if (!firesShutter(event.key, useStore.getState())) return;
      event.preventDefault();
      if (!event.repeat) void useStore.getState().takePhoto();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [capture]);

  if (!capture) return null;
  return (
    <div className="capture-bar" role="group" aria-label="Capture">
      <span className="capture-camera" title={`Each photo taken with ${capture.camera.name} comes into the library and opens`}>
        <Aperture size={14} />
        <span className="capture-name">{capture.camera.name}</span>
      </span>
      <button
        className={`button quiet ${capture.follow ? "engaged" : ""}`}
        aria-pressed={capture.follow}
        title="New frames take the film settings, crop and film details of the one before, with the frame number counted on"
        onClick={() => void setCaptureFollow(!capture.follow)}
      >
        Follow the last frame
      </button>
      <button className="button primary" title="Take a photo (F12)" onClick={() => void takePhoto()}>
        Take photo
      </button>
      <button className="button quiet" title="Stop capturing" onClick={() => void stopCapture()}>
        Stop
      </button>
    </div>
  );
}

/**
 * The window's own title bar: drag it to move the window, double-click to
 * maximize. Replaces the system decorations so the panels run edge to edge.
 */
export function TitleBar() {
  const appWindow = getCurrentWindow();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const sync = () => void appWindow.isMaximized().then(setMaximized);
    sync();
    const stop = appWindow.onResized(sync);
    return () => void stop.then((unlisten) => unlisten());
  }, [appWindow]);

  return (
    <header className="titlebar" data-tauri-drag-region>
      {/* The app's mark: its icon's tone curve, without the tile, drawn heavier to hold up at this size. */}
      <svg className="app-mark" viewBox="0 0 1024 1024" role="img" aria-label="Tonality">
        <path d="M156 846C356 814 438 692 512 512C586 332 668 210 868 178" />
        <circle cx="156" cy="846" r="92" />
        <circle cx="512" cy="512" r="92" />
        <circle cx="868" cy="178" r="92" />
      </svg>
      <Activity />
      <CaptureBar />
      <div className="window-controls">
        <button
          className="shortcuts-button"
          aria-label="Keyboard shortcuts"
          title="Keyboard shortcuts (?)"
          onClick={() => useStore.getState().setShortcutsOpen(true)}
        >
          <Keyboard size={16} />
        </button>
        <button aria-label="Minimize" title="Minimize" onClick={() => void appWindow.minimize()}>
          <Minus size={15} />
        </button>
        <button
          aria-label={maximized ? "Restore" : "Maximize"}
          title={maximized ? "Restore" : "Maximize"}
          onClick={() => void appWindow.toggleMaximize()}
        >
          {maximized ? <Copy size={12} /> : <Square size={12} />}
        </button>
        <button className="close" aria-label="Close" title="Close" onClick={() => void appWindow.close()}>
          <X size={15} />
        </button>
      </div>
    </header>
  );
}
