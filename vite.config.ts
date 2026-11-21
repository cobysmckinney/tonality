/// <reference types="vitest/config" />
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;
// `URL.pathname` is `/C:/...` on Windows and percent-encodes spaces; this gives a real path.
const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  root: "ui",
  plugins: [svelte({ configFile: here("./svelte.config.js") })],
  resolve: { alias: { $lib: here("./ui/lib") } },
  clearScreen: false,
  build: { outDir: "../dist", emptyOutDir: true, target: "es2022" },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
  test: { root: ".", include: ["ui/**/*.test.ts"] },
});
