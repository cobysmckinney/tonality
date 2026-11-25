// The white balance eyedropper (W): while it's on, a click on the photo sets temperature and tint
// so that spot becomes neutral grey, as one undo step. Esc, W again or the button turns it off.
import { api } from "$lib/api";
import type { EditState } from "$lib/bindings/EditState";
import { session } from "./session.svelte";

class WbPick {
  active = $state(false);
  busy = $state(false);
  /** Why the last click didn't set anything (cleared by the next one). */
  miss = $state<string | null>(null);

  start() {
    if (!session.image || !session.edit || session.cropping || session.masking) return;
    this.active = true;
    this.miss = null;
  }

  stop() {
    this.active = false;
    this.miss = null;
  }

  toggle() {
    if (this.active) this.stop();
    else this.start();
  }

  /** A click at frame position `at` (0..1 across the photo as framed). */
  async pick(at: [number, number]) {
    const image = session.image;
    const edit = session.edit;
    if (!this.active || this.busy || !image || !edit) return;
    this.busy = true;
    this.miss = null;
    try {
      const r = await api.pickWhiteBalance(image.id, $state.snapshot(edit) as EditState, at);
      if (session.image?.id !== image.id || !session.edit || !this.active) return;
      if (!r) {
        this.miss = "That spot is too bright or too dark to balance on. Try another.";
        return;
      }
      session.edit.whiteBalance = { temperature: r[0], tint: r[1] };
      session.commit(null);
      this.stop();
    } catch (e) {
      console.warn("[wb] pick:", e);
      this.miss = String(e);
    } finally {
      this.busy = false;
    }
  }
}

export const wbPick = new WbPick();
