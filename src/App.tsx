import { useEffect } from "react";
import { ExportSheet } from "./components/ExportSheet";
import { FilmSheet } from "./components/FilmSheet";
import { ImportSheet } from "./components/ImportSheet";
import { Overlays } from "./components/Overlays";
import { PhotoGrid } from "./components/PhotoGrid";
import { LibraryProblemScreen } from "./components/Problems";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { Toolbar } from "./components/Toolbar";
import { Editor } from "./components/editor/Editor";
import { searching } from "./search";
import { useStore, visiblePhotos } from "./store";

/** What a view says when it has nothing to show. */
function EmptyState() {
  const view = useStore((s) => s.view);
  const search = useStore((s) => s.search);
  const filtered = useStore((s) => s.photos.length > 0);
  const { importFiles, importFolder, showEverything } = useStore.getState();

  if (filtered || searching(search)) {
    return (
      <div className="empty">
        <h2>{searching(search) ? `Nothing here matches “${search.trim()}”` : "No photos match this filter"}</h2>
        <button className="button" onClick={showEverything}>
          Show all photos
        </button>
      </div>
    );
  }
  switch (view.kind) {
    case "library":
    case "imports":
      return (
        <div className="empty">
          <h2>Bring in some photos</h2>
          <p>
            Drop files or folders here, or plug in a camera card. Photos are copied into your library and sorted by the
            day they were taken.
          </p>
          <div className="empty-actions">
            <button className="button primary" onClick={() => void importFiles()}>
              Choose files…
            </button>
            <button className="button" onClick={() => void importFolder()}>
              Choose a folder…
            </button>
          </div>
        </div>
      );
    case "favorites":
      return (
        <div className="empty">
          <h2>No favorites yet</h2>
          <p>Click the heart on a photo, or press F, to keep it here.</p>
        </div>
      );
    case "deleted":
      return (
        <div className="empty">
          <h2>Nothing deleted</h2>
          <p>Photos you delete wait here for 30 days, in case you change your mind.</p>
        </div>
      );
    case "album":
      return (
        <div className="empty">
          <h2>This album is empty</h2>
          <p>Right-click any photo and choose “Add to album”.</p>
        </div>
      );
  }
}

export default function App() {
  const libraryProblem = useStore((s) => s.libraryProblem);
  const loaded = useStore((s) => s.loaded);
  const empty = useStore((s) => visiblePhotos(s).length === 0);
  const openPhoto = useStore((s) => (s.openId === null ? undefined : s.photos.find((p) => p.id === s.openId)));
  const modal = useStore(
    (s) =>
      s.importState !== null || s.exportState !== null || s.filmSheet !== null || s.confirmRequest !== null || s.shortcutsOpen,
  );

  useEffect(() => {
    const { subscribe, init } = useStore.getState();
    const unsubscribe = subscribe();
    void init();
    // The app draws its own menus.
    const block = (event: MouseEvent) => event.preventDefault();
    window.addEventListener("contextmenu", block);
    return () => {
      unsubscribe();
      window.removeEventListener("contextmenu", block);
    };
  }, []);

  if (libraryProblem) {
    return (
      <>
        <TitleBar />
        <LibraryProblemScreen problem={libraryProblem} />
        <Overlays />
      </>
    );
  }

  return (
    <>
      <TitleBar />
      <div className="workspace">
        {/* The grid stays mounted under the viewer so it keeps its scroll position. */}
        <div className="app" inert={openPhoto !== undefined || modal}>
          <Sidebar />
          <main className="panel main">
            <Toolbar />
            {loaded && empty ? <EmptyState /> : <PhotoGrid />}
          </main>
        </div>
        {openPhoto && <Editor photo={openPhoto} inert={modal} />}
      </div>
      <ImportSheet />
      <ExportSheet />
      <FilmSheet />
      <Overlays />
    </>
  );
}
