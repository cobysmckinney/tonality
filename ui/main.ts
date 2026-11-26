import { mount } from "svelte";
import "./lib/design/tokens.css";
import App from "./App.svelte";
import StartupError from "./lib/StartupError.svelte";

const { invoke } = await import("@tauri-apps/api/core");

// A library that couldn't be opened (it's open in another window, or was last used by a newer
// version) leaves the app with nothing behind it: say why, and mount nothing else.
const target = document.getElementById("app")!;
const failed = await invoke<string | null>("startup_error").catch(() => null);
if (failed) mount(StartupError, { target, props: { message: failed } });
else mount(App, { target });

// Errors nothing caught go to the log file (Settings shows where it is), so a bug report can
// include them. In development the console's warnings, errors and `console.info` go there too,
// which also puts them in the terminal running `tauri dev`.
const send = (level: string, args: unknown[]) =>
  invoke("ui_log", {
    level,
    message: args
      .map((a) => (a instanceof Error ? a.stack : typeof a === "string" ? a : JSON.stringify(a)))
      .join(" "),
  }).catch(() => {});
window.addEventListener("error", (e) => send("error", [e.message, `${e.filename}:${e.lineno}`]));
window.addEventListener("unhandledrejection", (e) =>
  send("error", ["unhandled rejection:", String(e.reason)]),
);
if (import.meta.env.DEV) {
  for (const level of ["info", "warn", "error"] as const) {
    const orig = console[level].bind(console);
    console[level] = (...args: unknown[]) => {
      orig(...args);
      send(level, args);
    };
  }
}
