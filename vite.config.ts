/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  root: "ui",
  plugins: [svelte({ configFile: new URL("./svelte.config.js", import.meta.url).pathname })],
  resolve: { alias: { $lib: new URL("./ui/lib", import.meta.url).pathname } },
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
