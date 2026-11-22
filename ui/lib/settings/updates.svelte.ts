// App settings and in-app updates: the release channel, and an update moving from found to
// downloaded to installed. Installing only happens when the user chooses to restart.
import { listen } from "@tauri-apps/api/event";
import { api } from "$lib/api";
import type { ReleaseChannel } from "$lib/bindings/ReleaseChannel";
import type { Settings } from "$lib/bindings/Settings";
import type { UpdateInfo } from "$lib/bindings/UpdateInfo";
import type { UpdateProgress } from "$lib/bindings/UpdateProgress";

export type UpdateStatus =
  "idle" | "checking" | "upToDate" | "available" | "downloading" | "ready" | "installing" | "error";

/** First background check after launch, then how often while the app stays open. */
const FIRST_CHECK_MS = 5_000;
const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;

class Updates {
  /** The settings dialog is open. */
  open = $state(false);
  settings = $state<Settings | null>(null);
  version = $state("");
  status = $state<UpdateStatus>("idle");
  update = $state<UpdateInfo | null>(null);
  progress = $state<UpdateProgress | null>(null);
  error = $state<string | null>(null);
  /** Bumped by each check, so a check that's overtaken (by a channel switch) is ignored. */
  #run = 0;
  #timer: ReturnType<typeof setInterval> | null = null;

  constructor() {
    listen<UpdateProgress>("update-progress", (e) => {
      if (this.status === "downloading") this.progress = e.payload;
    });
  }

  /** The version running is a nightly. */
  get nightlyBuild() {
    return this.version.includes("-");
  }

  async init() {
    try {
      [this.settings, this.version] = await Promise.all([
        api.settings.get(),
        api.updates.version(),
      ]);
    } catch (e) {
      this.fail(e);
      return;
    }
    // Development builds aren't installed, so there's nothing for an update to replace.
    if (import.meta.env.DEV) return;
    setTimeout(() => this.background(), FIRST_CHECK_MS);
    this.#timer ??= setInterval(() => this.background(), CHECK_EVERY_MS);
  }

  /** A check (and download) the user didn't ask for: only with auto-update on, and quietly. */
  private async background() {
    if (!this.settings?.autoUpdate) return;
    if (this.status !== "idle" && this.status !== "upToDate" && this.status !== "error") return;
    await this.check({ quiet: true });
  }

  async check({ quiet = false } = {}) {
    if (this.status === "downloading" || this.status === "installing") return;
    const run = ++this.#run;
    this.status = "checking";
    this.error = null;
    try {
      const update = await api.updates.check();
      if (run !== this.#run) return;
      this.update = update;
      this.status = update ? "available" : "upToDate";
      if (update && this.settings?.autoUpdate) await this.download(run);
    } catch (e) {
      if (run !== this.#run) return;
      // A failed background check (offline, say) isn't worth interrupting anyone for.
      if (quiet) this.status = "idle";
      else this.fail(e);
    }
  }

  async download(run = this.#run) {
    if (!this.update || this.status === "downloading") return;
    this.status = "downloading";
    this.progress = null;
    this.error = null;
    try {
      await api.updates.download();
      if (run === this.#run) this.status = "ready";
    } catch (e) {
      if (run === this.#run) this.fail(e);
    }
  }

  async install() {
    if (this.status !== "ready") return;
    this.status = "installing";
    try {
      await api.updates.install(); // restarts the app
    } catch (e) {
      this.fail(e);
    }
  }

  async setChannel(channel: ReleaseChannel) {
    if (!this.settings || this.settings.channel === channel) return;
    if (!(await this.save({ ...this.settings, channel }))) return;
    // What was found or downloaded was for the other channel.
    this.#run++;
    this.update = null;
    this.progress = null;
    this.status = "idle";
    await this.check();
  }

  async setAutoUpdate(autoUpdate: boolean) {
    if (!this.settings) return;
    if (!(await this.save({ ...this.settings, autoUpdate }))) return;
    if (autoUpdate && this.status === "available") await this.download();
  }

  private async save(next: Settings): Promise<boolean> {
    const prev = this.settings;
    this.settings = next;
    try {
      await api.settings.set(next);
      return true;
    } catch (e) {
      this.settings = prev;
      this.fail(e);
      return false;
    }
  }

  private fail(e: unknown) {
    this.status = "error";
    this.error = String(e);
  }
}

export const updates = new Updates();
