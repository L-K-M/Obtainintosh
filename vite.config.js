import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import bundledModules from "./scripts/bundled-modules.mjs";

// Minifying strips license comments; every chunk points to the notices file
// that `npm run build` writes next to it (scripts/third-party-notices.mjs).
const banner =
  "/*! Bundled third-party code keeps its own licenses: see Third-Party-Notices.txt at the site or app root. */";
// Emit fonts as files, never data: URIs, so the notices check can attribute them.
const keepFontFiles = (/** @type {string} */ file) =>
  /\.(?:woff2?|ttf|otf|eot)$/i.test(file) ? false : undefined;

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [sveltekit(), bundledModules()],
  // Keeps the /*! banner from Vite 7.2 on, whose minifier drops it by default.
  esbuild: { legalComments: /** @type {const} */ ("inline") },
  build: { assetsInlineLimit: keepFontFiles, rollupOptions: { output: { banner } } },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
