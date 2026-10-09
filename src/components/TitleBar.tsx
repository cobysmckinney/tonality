import { useEffect, useState } from "react";
import { Copy, Keyboard, Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { count } from "../format";
import { useStore } from "../store";
import { AppMark } from "./About";

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
      <button
        className="app-mark-button"
        aria-label="About Tonality"
        title="About Tonality"
        onClick={() => useStore.getState().setAboutOpen(true)}
      >
        <AppMark className="app-mark" />
      </button>
      <Activity />
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
