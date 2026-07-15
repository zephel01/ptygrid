// Shim for SvelteKit's `$app/environment`, aliased in vite.config.ts.
// svelte-splitpanes imports { browser } from it; this app is a plain
// Vite SPA (no SvelteKit), so provide the equivalent runtime check.
export const browser = typeof window !== "undefined";
export const dev = import.meta.env.DEV;
export const building = false;
export const version = "0";
