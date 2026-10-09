import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Check, ChevronRight, X } from "lucide-react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { MenuEntry, useStore } from "../store";
import { canStartImport } from "../sheets";
import { opensShortcuts, SHORTCUTS } from "../keys";
import { focusToRestore } from "../focus";
import { About } from "./About";

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
              {entry.hint && <span className="menu-hint">{entry.hint}</span>}
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
  // What had the focus when the menu opened, to give it back when it closes.
  // This runs before the effect below moves the focus onto the first item.
  const opener = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (menu) {
      opener.current ??= document.activeElement as HTMLElement | null;
      return;
    }
    const back = focusToRestore(opener.current, document.activeElement, document.body);
    opener.current = null;
    back?.focus({ preventScroll: true });
  }, [menu]);

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

/** One shortcut's keys, each key in its own box: "Ctrl+Shift+Z" as Ctrl, Shift, Z. */
function Keys({ keys }: { keys: string[] }) {
  return (
    <span className="keys">
      {keys.map((combo, i) => (
        <span key={combo} className="combo">
          {i > 0 && <span className="or">or</span>}
          {(combo === "+" ? ["+"] : combo.split("+")).map((key) => (
            <kbd key={key}>{key}</kbd>
          ))}
        </span>
      ))}
    </span>
  );
}

/** Every keyboard shortcut, opened with ? or the keyboard button in the title bar. */
function Shortcuts() {
  const open = useStore((s) => s.shortcutsOpen);
  const busy = useStore(
    (s) => s.importState !== null || s.exportState !== null || s.filmSheet !== null || s.confirmRequest !== null,
  );
  const opener = useRef<HTMLElement | null>(null);
  const closeButton = useRef<HTMLButtonElement>(null);

  // Taken before anything else hears the key, so Esc here doesn't also close the photo behind.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const s = useStore.getState();
      if (s.shortcutsOpen && (event.key === "Escape" || event.key === "?")) {
        event.stopPropagation();
        event.preventDefault();
        s.setShortcutsOpen(false);
      } else if (!s.shortcutsOpen) {
        const typing = (event.target as HTMLElement).closest?.("input:not([type=range]), textarea") != null;
        if (!opensShortcuts(event.key, typing, s)) return;
        event.stopPropagation();
        event.preventDefault();
        // Noted now: the panels behind go inert as the list opens, which can drop their focus.
        opener.current = document.activeElement as HTMLElement | null;
        s.setShortcutsOpen(true);
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, []);

  // An import, export or question that opens meanwhile (a dropped folder, say) takes its place.
  useEffect(() => {
    if (open && busy) useStore.getState().setShortcutsOpen(false);
  }, [open, busy]);

  // The list takes the focus onto its Close button, and gives it back where it was
  // when it closes, so the place in a panel isn't lost.
  useEffect(() => {
    if (open) {
      opener.current ??= document.activeElement as HTMLElement | null;
      closeButton.current?.focus();
      return;
    }
    const back = opener.current;
    opener.current = null;
    if (back?.isConnected && back !== document.body && (document.activeElement === document.body || !document.activeElement)) {
      back.focus({ preventScroll: true });
    }
  }, [open]);

  if (!open) return null;
  const close = () => useStore.getState().setShortcutsOpen(false);
  return (
    <div className="scrim" onMouseDown={(event) => event.target === event.currentTarget && close()}>
      <div className="panel dialog shortcuts" role="dialog" aria-modal="true" aria-labelledby="shortcuts-title">
        <header className="shortcuts-header">
          <h2 id="shortcuts-title">Keyboard shortcuts</h2>
          <button className="icon-button small" aria-label="Close" title="Close (Esc)" ref={closeButton} onClick={close}>
            <X size={14} />
          </button>
        </header>
        <div className="shortcut-groups">
          {SHORTCUTS.map((group) => (
            <section key={group.title}>
              <h3>{group.title}</h3>
              <dl>
                {group.shortcuts.map((shortcut) => (
                  <div key={shortcut.does} className="shortcut">
                    <dt>{shortcut.does}</dt>
                    <dd>
                      <Keys keys={shortcut.keys} />
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
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
  // Not offered while another sheet or a question is open; a drop then says why it did nothing.
  const offered = useStore(canStartImport);
  useEffect(() => {
    const stop = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "enter" || payload.type === "over") setOver(true);
      else setOver(false);
      if (payload.type === "drop" && payload.paths.length > 0) void useStore.getState().startImport(payload.paths);
    });
    return () => void stop.then((unlisten) => unlisten());
  }, []);

  if (!over || !offered) return null;
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
      <Shortcuts />
      <About />
      <Menu />
      <Toasts />
    </>
  );
}
