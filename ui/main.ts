import { mount } from "svelte";
import "./lib/design/tokens.css";
import App from "./App.svelte";

mount(App, { target: document.getElementById("app")! });

// Dev: mirror warnings, errors and `console.info` to the terminal running `tauri dev`.
if (import.meta.env.DEV) {
  const { invoke } = await import("@tauri-apps/api/core");
  const send = (level: string, args: unknown[]) =>
    invoke("dev_log", { level, message: args.map((a) => (a instanceof Error ? a.stack : typeof a === "string" ? a : JSON.stringify(a))).join(" ") }).catch(() => {});
  for (const level of ["info", "warn", "error"] as const) {
    const orig = console[level].bind(console);
    console[level] = (...args: unknown[]) => {
      orig(...args);
      send(level, args);
    };
  }
  window.addEventListener("error", (e) => send("error", [e.message, `${e.filename}:${e.lineno}`]));
  window.addEventListener("unhandledrejection", (e) => send("error", ["unhandled rejection:", String(e.reason)]));
}
