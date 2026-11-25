// Export: which photos the dialog is for, the remembered settings, and progress.
import { listen } from "@tauri-apps/api/event";
import { pictureDir, join } from "@tauri-apps/api/path";
import { api } from "$lib/api";
import { toasts } from "$lib/toasts/toasts.svelte";
import type { ExportSettings } from "$lib/bindings/ExportSettings";
import type { ExportSummary } from "$lib/bindings/ExportSummary";

const KEY = "tonality.export.settings";

function stored(): Partial<ExportSettings> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}");
  } catch {
    return {};
  }
}

class Exporter {
  /** Photos the open dialog will export; null when closed. */
  ids = $state<number[] | null>(null);
  settings = $state<ExportSettings>({
    folder: "",
    format: "jpeg",
    quality: 90,
    longEdge: null,
    versions: "current",
    colorSpace: "srgb",
    hdr: false,
    includeLocation: false,
    ...stored(),
  });
  running = $state(false);
  /** Stop was asked for: the file being written finishes first. */
  stopping = $state(false);
  progress = $state<{ done: number; total: number; name: string } | null>(null);
  result = $state<ExportSummary | null>(null);

  constructor() {
    listen<{ done: number; total: number; name: string }>("export-progress", (e) => {
      if (this.running) this.progress = e.payload;
    });
  }

  async open(ids: number[]) {
    if (!ids.length) return;
    if (!this.settings.folder) {
      try {
        this.settings.folder = await join(await pictureDir(), "tonality Exports");
      } catch {
        /* the user picks one */
      }
    }
    this.result = null;
    this.ids = ids;
  }

  close() {
    if (!this.running) this.ids = null;
  }

  async run() {
    if (!this.ids || this.running || !this.settings.folder) return;
    try {
      localStorage.setItem(KEY, JSON.stringify(this.settings));
    } catch {
      /* per-viewer convenience only */
    }
    this.running = true;
    this.stopping = false;
    this.progress = null;
    this.result = null;
    try {
      this.result = await api.exportImages(this.ids, $state.snapshot(this.settings));
    } catch (e) {
      const n = this.ids.length;
      toasts.error(`Couldn’t export ${n === 1 ? "the photo" : `${n} photos`}`, e);
    } finally {
      this.running = false;
      this.stopping = false;
    }
  }

  /** Stop the running export. Files already written stay. */
  async stop() {
    if (!this.running || this.stopping) return;
    this.stopping = true;
    await api.exportCancel().catch((e) => toasts.error("Couldn’t stop the export", e));
  }
}

export const exporter = new Exporter();
