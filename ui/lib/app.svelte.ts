// Global UI state shared by both modes.
export type Mode = "library" | "edit";

class AppState {
  mode = $state<Mode>("library");
  /** The startup animation is playing (the top bar's wordmark waits for it to land). */
  splash = $state(true);
}

export const app = new AppState();
