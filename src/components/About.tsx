import { useEffect, useRef, useState } from "react";
import { ArrowLeft, ExternalLink, X } from "lucide-react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Notices } from "../notices";
import { useStore } from "../store";

const SOURCE = "https://github.com/cobysmckinney/tonality";

type Page = "about" | "licence" | "notices";

/** The app's own mark, as in the title bar. */
export function AppMark({ className }: { className: string }) {
  return (
    <svg className={className} viewBox="0 0 1024 1024" aria-hidden="true">
      <path d="M156 846C356 814 438 692 512 512C586 332 668 210 868 178" />
      <circle cx="156" cy="846" r="92" />
      <circle cx="512" cy="512" r="92" />
      <circle cx="868" cy="178" r="92" />
    </svg>
  );
}

/** Tonality's own licence, the GPL, word for word. Loaded only when asked for. */
function Licence() {
  const [text, setText] = useState<string | null>(null);
  useEffect(() => void import("../../LICENSE?raw").then((module) => setText(module.default)), []);
  return <pre className="licence-text">{text ?? "Loading…"}</pre>;
}

/** Everything Tonality is built on, with the licence each is used under. Loaded only when asked for. */
function ThirdParty() {
  const [notices, setNotices] = useState<Notices | null>(null);
  useEffect(() => void import("../notices.json").then((module) => setNotices(module.default as Notices)), []);
  if (!notices) return <p className="about-loading">Loading…</p>;
  return (
    <>
      <p className="about-intro">
        Tonality is built on the work of these projects, each used under the licence shown. Choose one to read its
        licence in full.
      </p>
      {notices.groups.map((group) => (
        <section key={group.title} className="notice-group">
          <h3>{group.title}</h3>
          {group.note && <p className="notice-note">{group.note}</p>}
          {group.items.map((item) => (
            <details key={`${item.name} ${item.version ?? ""}`} className="notice">
              <summary>
                <span className="notice-name">{item.name}</span>
                {item.version && <span className="notice-version">{item.version}</span>}
                <span className="notice-licence">{item.licence}</span>
              </summary>
              {item.note && <p className="notice-note">{item.note}</p>}
              {item.texts.map((index) => (
                <pre key={index} className="licence-text">
                  {notices.texts[index]}
                </pre>
              ))}
            </details>
          ))}
        </section>
      ))}
    </>
  );
}

/** What Tonality is, its version and licence, and the licences of what it is built on. Opened from the mark in the title bar. */
export function About() {
  const open = useStore((s) => s.aboutOpen);
  const busy = useStore(
    (s) => s.importState !== null || s.exportState !== null || s.filmSheet !== null || s.confirmRequest !== null,
  );
  const [page, setPage] = useState<Page>("about");
  const [version, setVersion] = useState("");
  const opener = useRef<HTMLElement | null>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const backButton = useRef<HTMLButtonElement>(null);

  useEffect(() => void getVersion().then(setVersion), []);

  // Taken before anything else hears the key, so Esc here doesn't also close the photo behind.
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      event.preventDefault();
      useStore.getState().setAboutOpen(false);
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [open]);

  // An import, export or question that opens meanwhile (a dropped folder, say) takes its place.
  useEffect(() => {
    if (open && busy) useStore.getState().setAboutOpen(false);
  }, [open, busy]);

  // Opens on its first page with the focus on Close, and gives the focus back where it was.
  useEffect(() => {
    if (open) {
      opener.current ??= document.activeElement as HTMLElement | null;
      setPage("about");
      closeButton.current?.focus();
      return;
    }
    const back = opener.current;
    opener.current = null;
    if (back?.isConnected && back !== document.body && (document.activeElement === document.body || !document.activeElement)) {
      back.focus({ preventScroll: true });
    }
  }, [open]);

  // The link that was chosen goes as the page changes; the focus moves to Back, or back to Close.
  useEffect(() => {
    if (open) (page === "about" ? closeButton : backButton).current?.focus();
  }, [open, page]);

  if (!open) return null;
  const close = () => useStore.getState().setAboutOpen(false);
  return (
    <div className="scrim" onMouseDown={(event) => event.target === event.currentTarget && close()}>
      <div className={`panel dialog about ${page}`} role="dialog" aria-modal="true" aria-labelledby="about-title">
        <header className="about-header">
          {page !== "about" && (
            <button className="icon-button small" aria-label="Back" title="Back" ref={backButton} onClick={() => setPage("about")}>
              <ArrowLeft size={14} />
            </button>
          )}
          {page !== "about" && <h2 id="about-title">{page === "licence" ? "Licence" : "Third-party notices"}</h2>}
          <button className="icon-button small" aria-label="Close" title="Close (Esc)" ref={closeButton} onClick={close}>
            <X size={14} />
          </button>
        </header>
        {page === "about" ? (
          <div className="about-main">
            <AppMark className="about-mark" />
            <h2 id="about-title">Tonality</h2>
            {version && <p className="about-version">Version {version}</p>}
            <p className="about-licence">
              Free software: you can share it and change it under the GNU General Public License, version 3 or later. It
              comes with no warranty.
            </p>
            <div className="about-links">
              <button className="link" onClick={() => void openUrl(SOURCE)}>
                Source code
                <ExternalLink size={12} />
              </button>
              <button className="link" onClick={() => setPage("licence")}>
                Licence
              </button>
              <button className="link" onClick={() => setPage("notices")}>
                Third-party notices
              </button>
            </div>
          </div>
        ) : (
          <div className="about-scroll">{page === "licence" ? <Licence /> : <ThirdParty />}</div>
        )}
      </div>
    </div>
  );
}
