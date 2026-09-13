import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// RadarPro web — Stage S04 (Map Integration and Live Radar).
//
// No special config needed for `radar-web`'s wasm module: Vite has native
// support for `wasm-bindgen --target web`'s output pattern
// (`new URL('*_bg.wasm', import.meta.url)` + `fetch`) with zero plugin
// configuration -- see `README.md` "Wasm build pipeline" for the full
// explanation and where that generated output lives (`src/wasm/`, gitignored,
// produced by `npm run build:wasm`).
// `apps/desktop` (Tauri) proxies this dev server for `tauri dev` -- see
// `apps/desktop/src-tauri/tauri.conf.json`'s `devUrl`. `server.port` +
// `strictPort` pin the port Tauri expects instead of Vite silently picking
// the next free one if 5173 is busy; `clearScreen: false` keeps Vite from
// wiping Tauri's own CLI/Rust build output from the shared terminal. Both
// are no-ops for the standalone `npm run dev` browser workflow.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  // MapLibre GL JS loads its tile-parsing code in a module Web Worker,
  // constructed internally via `new Worker(new URL(..., import.meta.url))`.
  // Vite's dev-server dependency pre-bundling (`optimizeDeps`) rewrites
  // `maplibre-gl`'s own module graph into a single `node_modules/.vite/deps`
  // chunk, which breaks that worker's relative URL and makes it 404 in dev
  // (`net::ERR_FAILED` fetching `maplibre-gl-worker.mjs`) -- confirmed by
  // reproducing it during this stage's own verification: with pre-bundling
  // left on, the map rendered only its flat background color, with no
  // basemap tiles and (since the radar canvas overlay never received a
  // `load`/`move` event either) no radar canvas visible at all. Excluding
  // it from pre-bundling (it is still bundled normally for `vite build`,
  // where this issue does not occur) fixes both.
  optimizeDeps: {
    exclude: ["maplibre-gl"],
  },
});
