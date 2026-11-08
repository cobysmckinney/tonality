// Global UI state shared by both modes.
export type Mode = "library" | "edit";

class AppState {
  mode = $state<Mode>("library");
}

export const app = new AppState();
