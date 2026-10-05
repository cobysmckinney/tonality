import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Check, ChevronRight, X } from "lucide-react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { MenuEntry, useStore } from "../store";

function MenuList({ entries, x, y }: { entries: MenuEntry[]; x: number; y: number }) {
  const element = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState({ left: x, top: y });
  const [submenu, setSubmenu] = useState<{ entries: MenuEntry[]; x: number; y: number } | null>(null);

  // Nudge the menu back inside the window when it would spill over an edge.
  useLayoutEffect(() => {
    const rect = element.current!.getBoundingClientRect();
    setPlace({
      left: Math.max(8, Math.min(x, window.innerWidth - rect.width - 8)),
      top: Math.max(8, Math.min(y, window.innerHeight - rect.height - 8)),
    });
  }, [x, y]);

  // Leave room for tick marks only in menus that have some.
  const ticks = entries.some((entry) => entry !== "separator" && entry.checked !== undefined);
  return (
    <>
      <div ref={element} className={`menu ${ticks ? "" : "plain"}`} role="menu" style={place}>
        {entries.map((entry, index) =>
          entry === "separator" ? (
            <hr key={index} />
          ) : (
            <button
              key={index}
              role="menuitem"
              className={entry.danger ? "danger" : ""}
              disabled={entry.disabled}
              onMouseEnter={(event) => {
                const rect = event.currentTarget.getBoundingClientRect();
                setSubmenu(entry.submenu ? { entries: entry.submenu, x: rect.right + 2, y: rect.top - 5 } : null);
              }}
              onClick={() => {
                if (entry.submenu) return;
                useStore.getState().closeMenu();
                entry.run?.();
              }}
            >
              <span className="menu-check">{entry.checked && <Check size={13} strokeWidth={2.5} />}</span>
              {entry.label}
              {entry.submenu && <ChevronRight size={14} className="menu-arrow" />}
            </button>
          ),
        )}
      </div>
      {submenu && <MenuList key={`${submenu.x}:${submenu.y}`} {...submenu} />}
    </>
  );
}

function Menu() {
  const menu = useStore((s) => s.menu);
  useEffect(() => {
    if (!menu) return;
    const close = () => useStore.getState().closeMenu();
    // Arrow keys walk the items; the first one takes the focus as the menu opens.
    const items = () => [...document.querySelectorAll<HTMLButtonElement>(".menu-layer .menu button:not(:disabled)")];
    items()[0]?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      event.preventDefault();
      const all = items();
      const at = all.indexOf(document.activeElement as HTMLButtonElement);
      all[(at + (event.key === "ArrowDown" ? 1 : -1) + all.length) % all.length]?.focus();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", close);
    };
  }, [menu]);

  if (!menu) return null;
  return (
    <div
      className="menu-layer"
      onMouseDown={(event) => event.target === event.currentTarget && useStore.getState().closeMenu()}
      onContextMenu={(event) => {
        event.preventDefault();
        useStore.getState().closeMenu();
      }}
    >
      <MenuList key={`${menu.x}:${menu.y}`} {...menu} />
    </div>
  );
}

function Confirm() {
  const request = useStore((s) => s.confirmRequest);
  useEffect(() => {
    if (!request) return;
    // Only Escape is a shortcut: Enter lands on Cancel, which holds the focus,
    // so a stray keypress can never confirm something irreversible.
    const onKey = (event: KeyboardEvent) => event.key === "Escape" && request.resolve(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [request]);

  if (!request) return null;
  return (
    <div className="scrim" onMouseDown={(event) => event.target === event.currentTarget && request.resolve(false)}>
      <div className="panel dialog" role="alertdialog" aria-labelledby="confirm-title">
        <h2 id="confirm-title">{request.title}</h2>
        <p>{request.body}</p>
        <div className="dialog-actions">
          <button className="button" autoFocus onClick={() => request.resolve(false)}>
            Cancel
          </button>
          <button className="button danger solid" onClick={() => request.resolve(true)}>
            {request.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  return (
    <div className="toasts" role="status">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast ${toast.tone ?? ""}`}>
          <span>{toast.text}</span>
          {toast.action && (
            <button
              className="toast-action"
              onClick={() => {
                dismiss(toast.id);
                toast.action!.run();
              }}
            >
              {toast.action.label}
            </button>
          )}
          <button className="icon-button small" aria-label="Dismiss" onClick={() => dismiss(toast.id)}>
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}

/** Dropping files or folders anywhere on the window starts an import. */
function DropTarget() {
  const [over, setOver] = useState(false);
  useEffect(() => {
    const stop = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "enter" || payload.type === "over") setOver(true);
      else setOver(false);
      if (payload.type === "drop" && payload.paths.length > 0) void useStore.getState().startImport(payload.paths);
    });
    return () => void stop.then((unlisten) => unlisten());
  }, []);

  if (!over) return null;
  return (
    <div className="drop-target">
      <div>
        <h2>Drop to import</h2>
        <p>You’ll choose which photos to copy before anything is added.</p>
      </div>
    </div>
  );
}

export function Overlays() {
  return (
    <>
      <DropTarget />
      <Confirm />
      <Menu />
      <Toasts />
    </>
  );
}
