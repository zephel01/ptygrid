import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// https://tauri.app/start/frontend/vite/
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [svelte()],

  resolve: {
    alias: {
      // svelte-splitpanes imports `browser` from SvelteKit's $app/environment;
      // this is a plain Vite SPA, so point it at a local shim.
      "$app/environment": fileURLToPath(
        new URL("./src/lib/app-environment-shim.ts", import.meta.url),
      ),
    },
  },

  // The dev-server dependency pre-bundler (esbuild) does not apply the alias
  // above, so `vite dev` would fail on "$app/environment". Excluding the
  // package routes it through Vite's normal pipeline where the alias works.
  optimizeDeps: {
    exclude: ["svelte-splitpanes"],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 5173,
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
  // 4. env variables starting with `TAURI_` are exposed to the frontend
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    // Tauri uses Chromium on Windows and WebKit on macOS and Linux
    target: "esnext",
    // don't minify for debug builds
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    // produce sourcemaps for debug builds
    sourcemap: !!process.env.TAURI_DEBUG,
  },
}));
