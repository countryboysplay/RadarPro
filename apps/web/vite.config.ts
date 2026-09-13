import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { copyFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

/**
 * Copies MapLibre GL JS's prebuilt worker script(s) into the production
 * build's output directory, next to the hashed main chunk.
 *
 * # Why this is needed (found live, S10 desktop testing)
 *
 * MapLibre computes its worker's URL at runtime as
 * `new URL(\`./${workerName}\`, import.meta.url)` (see
 * `node_modules/maplibre-gl/src/util/web_worker.ts`) -- relative to
 * wherever ITS OWN bundled code ends up loading from. Vite inlines that
 * code into the main hashed chunk (`assets/index-*.js`), so at runtime the
 * relative path resolves to e.g. `assets/maplibre-gl-worker.mjs`. But
 * because the filename is built from a runtime template string, not a
 * static string literal, Vite's `new URL('literal.ext', import.meta.url)`
 * asset-detection never sees it -- the raw prebuilt file is never copied
 * into `dist/` at all. A plain `vite build` "succeeds" with zero errors
 * (this is a runtime-only failure), and it is invisible in `npm run dev`
 * too, for a different, already-documented reason (see this file's other
 * comment below on `optimizeDeps.exclude`) -- so the gap survived
 * undetected through every build/typecheck check this project ran, until
 * the very first time the actual production `dist/` output was loaded in
 * a real browser (RadarPro's desktop shell, S10): the worker request came
 * back as the SPA's `index.html` fallback (wrong MIME type), MapLibre's
 * worker never started, and no vector tiles rendered at all -- confirmed
 * via WebView2 remote debugging, not assumed.
 *
 * The worker module itself then imports a second runtime-relative file
 * (`maplibre-gl-shared(-dev).mjs`) the same way -- also confirmed live,
 * also not statically detectable by Vite for the same reason. Rather than
 * hand-list files one at a time as each missing import surfaces, this
 * copies every `maplibre-gl-{worker,shared}*.mjs` file in the installed
 * package's `dist/` -- the whole family of prebuilt, runtime-relative-URL
 * files that `maplibre-gl.mjs` (the part Vite DOES bundle normally) hands
 * off to, present and future.
 *
 * Resolves the real installed package's location each build (via Node's
 * CJS `require.resolve` against `maplibre-gl/package.json`, not a
 * hand-copied duplicate) so it can never go stale across a `maplibre-gl`
 * version bump.
 */
function copyMapLibreWorkerPlugin(): Plugin {
  return {
    name: "copy-maplibre-gl-worker",
    apply: "build",
    closeBundle() {
      const maplibreDistDir = join(dirname(require.resolve("maplibre-gl/package.json")), "dist");
      const outDir = join(dirname(fileURLToPath(import.meta.url)), "dist", "assets");
      for (const name of readdirSync(maplibreDistDir)) {
        if (!/^maplibre-gl-(worker|shared)[a-z-]*\.mjs$/.test(name)) continue;
        copyFileSync(join(maplibreDistDir, name), join(outDir, name));
      }
    },
  };
}

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
  plugins: [react(), copyMapLibreWorkerPlugin()],
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
  // it from pre-bundling fixes the dev-server case.
  //
  // The PRODUCTION build has a separate, distinct instance of this same
  // underlying problem -- see `copyMapLibreWorkerPlugin`'s doc comment
  // above (this was wrongly believed not to occur for `vite build`; it
  // does, just invisibly, since it only fails at runtime in a real
  // browser, never during the build itself).
  optimizeDeps: {
    exclude: ["maplibre-gl"],
  },
});
