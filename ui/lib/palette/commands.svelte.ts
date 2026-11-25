// Every command in the app, and the palette's own state. Commands call the same state methods
// the buttons and shortcuts do; the shortcuts themselves stay in the components that own them,
// and `keys` here only names them. A few actions live in a component's local state (zoom, the
// filmstrip, naming a new album): that component `provide`s them while it's on screen.
import { untrack } from "svelte";
import { app } from "$lib/app.svelte";
import { session } from "$lib/edit/session.svelte";
import { exportCount, exportCurrent } from "$lib/export/targets";
import { BIN, library } from "$lib/library/state.svelte";
import { updates } from "$lib/settings/updates.svelte";
import { remember, shortcutText, type Command } from "./registry";

export const MAC = /Mac|iPhone|iPad/.test(navigator.userAgent);
/** A shortcut as it reads on this platform, for tooltips ("Ctrl+K", "⌘K"). */
export const shortcut = (combo: string) => shortcutText(combo, MAC);

type Provided = { run: () => unknown; title?: () => string };
const RECENT_KEY = "tonality.palette.recent";

function storedRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}

class Palette {
  open = $state(false);
  view = $state<"commands" | "shortcuts">("commands");
  recent = $state<string[]>(storedRecent());
  // Raw: the cleanup finds its own entries by identity, which a deep proxy would break.
  provided = $state.raw<Record<string, Provided>>({});
  query = $state("");

  show(view: "commands" | "shortcuts" = "commands") {
    this.view = view;
    this.query = "";
    this.open = true;
  }

  close() {
    this.open = false;
  }

  /** Run a command from the palette: close it first, so focus is back where it was. */
  run(c: Command) {
    this.close();
    this.recent = remember(this.recent, c.id);
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(this.recent));
    } catch {
      /* a convenience only */
    }
    Promise.resolve()
      .then(() => c.run?.())
      .catch((e) => console.error(`[palette] ${c.id}`, e));
  }

  /** Offer actions held in a component's state while it's mounted; returns the cleanup. */
  provide(actions: Record<string, Provided | (() => unknown)>): () => void {
    const entries = Object.entries(actions).map(
      ([id, a]) => [id, typeof a === "function" ? { run: a } : a] as const,
    );
    untrack(() => {
      this.provided = { ...this.provided, ...Object.fromEntries(entries) };
    });
    return () =>
      untrack(() => {
        const next = { ...this.provided };
        for (const [id, a] of entries) if (next[id] === a) delete next[id];
        this.provided = next;
      });
  }
}

export const palette = new Palette();

const inLibrary = () => app.mode === "library";
const inEdit = () => app.mode === "edit";
const photo = () => inEdit() && !!session.edit;
/** Edit with a photo, outside crop (which keeps undo and zoom to itself). */
const editing = () => photo() && !session.cropping;
const targets = () => inLibrary() && library.targets().length > 0;
const inAlbum = () => library.view.kind === "album";

/** A command run by a component's provided action (it's only offered while provided). */
function provided(c: Omit<Command, "run">): Command {
  return {
    ...c,
    get title() {
      return palette.provided[c.id]?.title?.() ?? c.title;
    },
    when: () => c.id in palette.provided && (c.when?.() ?? true),
    run: () => palette.provided[c.id]?.run(),
  };
}

const rate = (n: number): Command => ({
  id: `library.rate${n}`,
  title: n ? `Rate ${"★".repeat(n)}` : "Clear rating",
  group: "Library",
  keys: [String(n)],
  keywords: n ? `${n} stars rating` : "0 stars rating",
  when: targets,
  run: () => library.setRating(n),
});

export const COMMANDS: Command[] = [
  // ---- Library ----
  {
    id: "library.import",
    title: "Import photos…",
    group: "Library",
    keys: ["Mod+O"],
    keywords: "open files raw add",
    when: () => !library.importing && !session.cropping,
    run: () => library.importFiles({ open: inEdit() }),
  },
  {
    id: "library.importFolder",
    title: "Import folder…",
    group: "Library",
    keys: ["Mod+Shift+I"],
    keywords: "add",
    when: () => inLibrary() && !library.importing,
    run: () => library.importFolder(),
  },
  {
    id: "library.stopImport",
    title: "Stop importing",
    group: "Library",
    keywords: "cancel",
    when: () => !!library.importing && !library.stoppingImport,
    run: () => library.stopImport(),
  },
  {
    id: "library.export",
    title: "Export…",
    group: "Library",
    keys: ["Mod+Shift+E"],
    keywords: "save jpeg tiff",
    when: () => exportCount() > 0,
    run: exportCurrent,
  },
  {
    id: "library.open",
    title: "Open in Edit",
    group: "Library",
    keys: ["Enter"],
    when: () => inLibrary() && library.focus !== null,
    run: () => library.open(),
  },
  {
    id: "library.selectAll",
    title: "Select all",
    group: "Library",
    keys: ["Mod+A"],
    when: () => inLibrary() && library.images.length > 0,
    run: () => library.selectAll(),
  },
  {
    id: "library.clearSelection",
    title: "Clear selection",
    group: "Library",
    keys: ["Esc"],
    keywords: "deselect none",
    when: () => inLibrary() && library.selected.size > 0,
    run: () => library.clearSelection(),
  },
  { id: "library.move", title: "Move focus", group: "Library", keys: ["Arrows"] },
  { id: "library.extend", title: "Extend selection", group: "Library", keys: ["Shift+Arrows"] },
  rate(1),
  rate(2),
  rate(3),
  rate(4),
  rate(5),
  rate(0),
  {
    id: "library.pick",
    title: "Flag as pick",
    group: "Library",
    keys: ["P"],
    when: targets,
    run: () => library.setFlag(1),
  },
  {
    id: "library.reject",
    title: "Flag as rejected",
    group: "Library",
    keys: ["X"],
    when: targets,
    run: () => library.setFlag(-1),
  },
  {
    id: "library.unflag",
    title: "Remove flag",
    group: "Library",
    keys: ["U"],
    keywords: "unflag clear",
    when: targets,
    run: () => library.setFlag(0),
  },
  provided({ id: "library.newAlbum", title: "New album…", group: "Library" }),
  provided({
    id: "library.newAlbumWith",
    title: "New album with the selected photos…",
    group: "Library",
    when: () => library.selected.size > 0,
  }),
  {
    id: "library.removeFromAlbum",
    title: "Remove from this album",
    group: "Library",
    keys: ["Delete"],
    when: () => targets() && inAlbum(),
    run: () => library.removeFromAlbum(),
  },
  {
    id: "library.remove",
    title: `Move to ${BIN}…`,
    group: "Library",
    get keys() {
      return [inAlbum() ? "Mod+Delete" : "Delete"];
    },
    keywords: "delete remove trash",
    when: targets,
    run: () => library.remove(),
  },
  {
    id: "library.resetHistory",
    title: "Reset history…",
    group: "Library",
    keywords: "as-shot revert versions",
    when: targets,
    run: () => library.resetHistory(),
  },

  // ---- Edit ----
  {
    id: "edit.previous",
    title: "Previous photo",
    group: "Edit",
    keys: ["Left"],
    when: () => inEdit() && !session.cropping && library.images.length > 1,
    run: () => library.step(-1),
  },
  {
    id: "edit.next",
    title: "Next photo",
    group: "Edit",
    keys: ["Right"],
    when: () => inEdit() && !session.cropping && library.images.length > 1,
    run: () => library.step(1),
  },
  {
    id: "edit.crop",
    title: "Crop and straighten",
    group: "Edit",
    keys: ["R"],
    keywords: "rotate level",
    when: () => photo() && !session.cropping,
    run: () => session.startCrop(),
  },
  {
    id: "edit.masks",
    get title() {
      return session.masking ? "Close masks" : "Masks";
    },
    group: "Edit",
    get keys() {
      return session.masking ? ["M", "Esc"] : ["M"];
    },
    keywords: "local adjustments brush",
    when: editing,
    run: () => (session.masking ? session.stopMasks() : session.startMasks()),
  },
  {
    id: "edit.stopCompare",
    title: "Stop comparing",
    group: "Edit",
    keys: ["Esc"],
    when: () => !!session.compare,
    run: () => session.stopCompare(),
  },

  // ---- Crop (while cropping) ----
  provided({
    id: "crop.apply",
    title: "Apply crop",
    group: "Crop",
    keys: ["Enter", "R"],
    when: () => session.cropping,
  }),
  provided({
    id: "crop.cancel",
    title: "Cancel crop",
    group: "Crop",
    keys: ["Esc"],
    when: () => session.cropping,
  }),
  { id: "crop.rotate", title: "Rotate 90° left / right", group: "Crop", keys: ["[", "]"] },

  // ---- Masks (while masking) ----
  {
    id: "masks.overlay",
    get title() {
      return session.maskOverlay ? "Hide mask overlay" : "Show mask overlay";
    },
    group: "Masks",
    keys: ["O"],
    when: () => editing() && session.masking,
    run: () => (session.maskOverlay = !session.maskOverlay),
  },
  {
    id: "masks.delete",
    title: "Delete mask",
    group: "Masks",
    keys: ["Delete"],
    when: () => editing() && session.masking && session.selected !== null,
    run: () => session.selected !== null && session.deleteMask(session.selected),
  },
  { id: "masks.brush", title: "Smaller / larger brush", group: "Masks", keys: ["[", "]"] },

  // ---- History ----
  {
    id: "history.undo",
    title: "Undo",
    group: "History",
    keys: ["Mod+Z"],
    when: () => editing() && !!session.history,
    run: () => session.undo(),
  },
  {
    id: "history.redo",
    title: "Redo",
    group: "History",
    keys: ["Mod+Shift+Z", "Mod+Y"],
    when: () => editing() && !!session.history,
    run: () => session.redo(),
  },

  // ---- View ----
  {
    id: "view.library",
    title: "Go to Library",
    group: "View",
    keys: ["G"],
    keywords: "grid mode",
    when: inEdit,
    run: () => (app.mode = "library"),
  },
  {
    id: "view.edit",
    title: "Go to Edit",
    group: "View",
    keys: ["E"],
    keywords: "develop mode",
    when: inLibrary,
    run: () => (library.focus !== null ? library.open() : (app.mode = "edit")),
  },
  provided({
    id: "view.zoomFit",
    title: "Fit to window",
    group: "View",
    keys: ["Mod+0"],
    keywords: "zoom",
    when: editing,
  }),
  provided({
    id: "view.zoom100",
    title: "Zoom to 100%",
    group: "View",
    keys: ["Mod+1"],
    keywords: "actual pixels",
    when: editing,
  }),
  provided({
    id: "view.zoomIn",
    title: "Zoom in",
    group: "View",
    keys: ["Mod+="],
    when: editing,
  }),
  provided({
    id: "view.zoomOut",
    title: "Zoom out",
    group: "View",
    keys: ["Mod+-"],
    when: editing,
  }),
  provided({
    id: "view.zoomToggle",
    title: "Toggle fit / 100%",
    group: "View",
    keys: ["Z"],
    keywords: "zoom",
    when: editing,
  }),
  { id: "view.before", title: "Show the original (hold)", group: "View", keys: ["\\"] },
  provided({
    id: "view.filmstrip",
    title: "Show or hide the filmstrip",
    group: "View",
    keywords: "thumbnails",
  }),
  provided({
    id: "view.versions",
    title: "Show or hide the versions panel",
    group: "View",
    keywords: "history sidebar",
  }),

  // ---- App ----
  {
    id: "app.palette",
    title: "Command palette",
    group: "App",
    keys: ["Mod+K"],
  },
  {
    id: "app.shortcuts",
    title: "Keyboard shortcuts",
    group: "App",
    keys: ["?"],
    keywords: "help keys",
    run: () => palette.show("shortcuts"),
  },
  {
    id: "app.settings",
    title: "Settings…",
    group: "App",
    keys: ["Mod+,"],
    keywords: "preferences options",
    run: () => (updates.open = true),
  },
  {
    id: "app.checkUpdates",
    title: "Check for updates",
    group: "App",
    keywords: "version upgrade",
    when: () => !["checking", "downloading", "installing"].includes(updates.status),
    run: () => {
      updates.open = true;
      return updates.check();
    },
  },
  {
    id: "app.restart",
    title: "Restart to update",
    group: "App",
    keywords: "install",
    when: () => updates.status === "ready",
    run: () => updates.install(),
  },
];
