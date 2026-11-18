// Export: which photos the dialog is for, the remembered settings, and progress.
import { listen } from "@tauri-apps/api/event";
import { pictureDir, join } from "@tauri-apps/api/path";
import { api } from "$lib/api";
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
  progress = $state<{ done: number; total: number; name: string } | null>(null);
  result = $state<ExportSummary | null>(null);
  error = $state<string | null>(null);

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
    this.error = null;
    this.ids = ids;
  }

  close() {
    if (!this.running) this.ids = null;
  }

  async run() {
    if (!this.ids || this.running) return;
    try {
      localStorage.setItem(KEY, JSON.stringify(this.settings));
    } catch {
      /* per-viewer convenience only */
    }
    this.running = true;
    this.progress = null;
    this.result = null;
    this.error = null;
    try {
      this.result = await api.exportImages(this.ids, $state.snapshot(this.settings));
    } catch (e) {
      this.error = String(e);
    } finally {
      this.running = false;
    }
  }
}

export const exporter = new Exporter();
