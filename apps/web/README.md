# RadarPro web (`apps/web`)

Stage S05: a single-panel radar analysis workstation built on top of S04's
live-radar map view (`crates/radar-web`'s wasm/WebGPU renderer, MapLibre GL
JS, and the current public NOAA/Unidata NEXRAD Level II S3 bucket). See
`Agent Context/context/stages/S05-radar-workstation.md` for the stage
brief and `Agent Context/context/stages/S04-map-live-radar.md` for the
prior stage this builds on.

## S05: what changed on top of S04

`crates/radar-web`'s wasm API was generalized between S04 and S05 from a
fixed decode-and-render-one-sweep shape (`decodeSweep`/`renderFrame`/
`SweepInfo`) to decode-once/select-and-render-many
(`decodeVolume`/`selectAndRender`/`VolumeSummary`), plus color-table
load/probe/range-ring exports -- see `crates/radar-web/src/browser.rs` and
its own README. `src/radar/wasmModule.ts` and `src/radar/useRadarRenderer.ts`
were rewritten against that new surface.

New pieces added for S05 (all under `src/`):

- **`scan/useScanHistory.ts`**: a bounded (`MAX_HISTORY_SCANS = 15`),
  in-memory cache of recently-downloaded scans on top of S04's
  `useScanPoller`, with previous/next/loop-play/pause/jump-to-latest
  controls. Raw bytes live in a plain `Map` ref, never React state; only
  small per-scan metadata (key + timestamp) drives a `useReducer` state
  machine. Oldest scan is evicted (FIFO) once the cap is exceeded. Live
  polling always keeps running in the background regardless of playback
  mode; only `"live"` mode auto-jumps the displayed frame to a newly
  downloaded scan -- reviewing history or animating never gets yanked
  forward by a background download. See that file's doc comments for the
  full state machine.
- **`radar/useRadarRenderer.ts`**: decode-once (`decodeVolume`) /
  select-and-render-many (`selectAndRender`) against the new wasm API,
  plus `loadColorTable`/`resetColorTable`/`activeColorTableJson` (never
  throws -- returns a plain `{ ok, ... }` result) and `probeGate`/
  `rangeRings`.
- **`colorTables/`**: `colorTableJson.ts` (a plain-TS mirror of
  `COLOR_TABLE_FORMAT.md`'s shape, used only to build the legend's CSS
  gradient from a table's own stops -- never a second copy of the color
  mapping) and `defaultColorTables.ts` (this project's six shipped
  defaults, vendored into `src/data/color_tables/` and imported as raw
  text for the color-table editor's preset dropdown).
- **`ui/`**: `Legend`, `ProbePanel`, `InfoPanel`, `PlaybackControls`,
  `ColorTableEditor` -- the workstation's small persistent panels.
- **`App.tsx`**: orchestrates all of the above, plus a handful of keyboard
  shortcuts (see below) and the mouse-driven data probe / geographic
  cursor readout wired through `MapView`'s `onCursorMove`/`onCursorLeave`.

### Keyboard shortcuts

Ignored while focus is in the color-table editor's textarea or any
`<select>`/`<input>`, so they never fight with editing:

| Key | Action |
|---|---|
| ← / → | Previous / next scan (from the held history, never re-downloads) |
| Space | Play / pause the loop |
| ↑ / ↓ | Step elevation (sweep index) up / down |
| L | Jump to the latest scan and resume live polling |

### Range rings: a canvas overlay, not a native MapLibre layer

`RadarWebRenderer`'s `rangeRingsGeoJson` free function returns plain `[lon,
lat]` ring geometry (no MapLibre knowledge). The literal reading of this
stage's brief -- add it as a MapLibre GeoJSON source + `line` layer -- was
tried first and does work as a map layer, but that layer paints into the
*map's own* canvas, which sits **underneath** the radar sweep `<canvas>`
(see `MapView.tsx`'s layout comment for why the two canvases must be DOM
siblings, not nested). The radar canvas is opaque (S04's black sweep
background, `radar-render`'s own clear color -- unrelated to this task and
not touched by it) and covers exactly the on-screen area a site-centered
ring would appear in, so a native map-layer ring rendered zero visible
pixels in practice -- confirmed during this task's own browser
verification. `MapView.tsx`'s `drawRangeRings` instead paints into a
dedicated transparent `<canvas>` one z-index *above* the radar sweep
canvas, still driven entirely by `map.project()` (this remains the only
MapLibre-aware component; `radar-web` still only ever hands back plain
coordinate geometry) -- the same way real radar workstations draw range
rings over the reflectivity image rather than under it. Fixed radii
50/100/150/200 km, 128 points/ring (`App.tsx`'s
`RANGE_RING_RADII_KM`/`RANGE_RING_NUM_POINTS`) -- both are documented,
reasonable defaults, not derived from any per-scan value.

### Known gap: VCP number is not displayed

`radar_types::Volume::volume_coverage_pattern` is decoded natively by
`nexrad-level2`, but `radar-web`'s wasm `VolumeSummary` (the only
JS-visible decoded-volume metadata) does not currently expose it --  only
`siteIcao`/`sweepCount`/`elevationDegs`. Per this task's scope (`radar-web`
is not to be modified here), `InfoPanel.tsx` documents this and simply
omits VCP rather than fabricating or guessing at a value. A narrowly-scoped
follow-up to `radar-web`'s `VolumeSummary` (adding a `volumeCoveragePattern`
getter, mirroring the existing `siteIcao`/`sweepCount` fields) would close
this gap. Volume **start time** does not have this problem -- it is read
directly from the S3 object key (`DiscoveredVolume.startTimeMillis`, the
same value NOAA encodes in the filename), not from `VolumeSummary`.

## Wasm build pipeline

`crates/radar-web` compiles to a `cdylib` wasm32 module via `cargo` +
`wasm-bindgen` (see `crates/radar-web/README.md` for the exact commands and
the required pinned `wasm-bindgen-cli` version). This app needs that
module's `wasm-bindgen --target web` output (`radar_web.js` +
`radar_web_bg.wasm` + `.d.ts` files) importable as a plain ES module.

**Chosen approach**: `scripts/build-wasm.mjs` runs those two commands and
writes the output directly into `apps/web/src/wasm/` (not `public/wasm/`).
Reasons:

- `src/wasm/` puts the generated `.js`/`.wasm` through Vite's normal module
  graph (dev-server transform pipeline, production hashing/optimization),
  rather than treating it as an opaque static file copied verbatim from
  `public/`. Vite has documented native support for the exact pattern
  `wasm-bindgen --target web` output uses internally
  (`new URL('radar_web_bg.wasm', import.meta.url)` + `fetch`), so no extra
  Vite plugin or config is needed either way -- see `vite.config.ts`.
  Verified: `radar_web.js`'s generated `initSync`/default-export path uses
  exactly this pattern (checked directly in the generated output).
- The generated `.d.ts` files (`radar_web.d.ts`, `radar_web_bg.wasm.d.ts`)
  let TypeScript type-check every call into the wasm module
  (`src/radar/wasmModule.ts`, `src/radar/useRadarRenderer.ts`) as strictly
  as any other import -- this would not work importing from `public/`,
  which is outside `tsconfig.json`'s `include`.

`apps/web/src/wasm/` is a **build artifact** (regenerated by
`npm run build:wasm`), not committed -- see the root `.gitignore` entry
added alongside the pre-existing `**/www/pkg/` one for
`crates/radar-web/www/`, same reasoning.

`npm run dev`, `npm run build`, and `npm run typecheck` all run
`npm run build:wasm` automatically first, via npm's automatic `pre<script>`
hook convention (`predev`/`prebuild`/`pretypecheck` in `package.json`) --
none of them can produce a correct result against a stale or missing wasm
module, so none of them skip this step silently.

### What CI will need (not wired here -- see task instructions)

Before running any of `apps/web`'s npm scripts in CI, the `web` job needs:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
```

(`0.2.128` must match `crates/radar-web/Cargo.toml`'s pinned
`wasm-bindgen` version exactly -- a mismatch is a *runtime* error, not a
compile error, so CI would appear to pass the build step and then fail
inexplicably at `npm run dev`/a smoke test.) After that, plain
`npm ci && npm run build` (run from `apps/web/`) is the single reproducible
command that builds everything, wasm included.

## Live NOAA Level II pipeline

`src/nexrad/` is a from-scratch TypeScript re-implementation (browser
`fetch`/`DOMParser`, no SDK) of the same `ListObjectsV2` discovery +
`GetObject` download shape `crates/radar-cache/src/{xml,keys,discovery}.rs`
already implements natively -- read for reference (exact XML shape, key
filtering rule: `_V0[2-7]` only, `_MDM` and anything else excluded), not
reused directly, since that crate is `tokio`/`reqwest`-coupled and cannot
run in a browser bundle.

Bucket: `https://unidata-nexrad-level2.s3.amazonaws.com` (the current public
Unidata bucket, successor to the retired `noaa-nexrad-level2` bucket).

### CORS -- empirically verified from a real browser, not assumed

Per the task brief, this was checked with a real Chrome instance (via
`puppeteer-core`'s CDP connection to the machine's installed Chrome, **not**
`curl`/Node) fetching the bucket directly from a page served on
`http://localhost`, before writing `src/nexrad/bucket.ts`:

- `GET /?list-type=2&prefix=...` (discovery): **succeeded**, HTTP 200, full
  XML body readable from page JS.
- `GET /<key>` with a `Range` header (download; `Range` forces a CORS
  preflight `OPTIONS`, unlike a plain `GET`, so this is a stricter check
  than discovery alone): **succeeded**, HTTP 206, body bytes readable from
  page JS.

**Conclusion: the bucket's CORS policy allows both request shapes this app
needs, from an arbitrary origin.** No same-origin dev-server proxy and no
backend relay are needed for this stage -- see `src/nexrad/bucket.ts`'s
module doc comment for the same summary next to the code it justifies.

### Background update loop

`src/scan/useScanPoller.ts` polls the selected site's latest scan every 45s
(`POLL_INTERVAL_MS`) -- same reasoning `crates/radar-cache/src/update_loop.rs`
documents (WSR-88D volumes complete every ~4-10 minutes; 45s sits in a
reasonable 30-60s range without spending bandwidth re-listing a directory
that usually hasn't changed). Switching the selected site cancels the old
site's in-flight request/interval via a `useEffect` cleanup (`AbortController`
+ `clearInterval`) before starting a new one -- never two loops running at
once, matching `update_loop.rs`'s cancellation discipline.

## Map + radar overlay

`src/map/MapView.tsx` renders a full-viewport MapLibre GL JS map (MapLibre's
free public demo style, **flagged in that file as an unresolved production
concern** per `Agent Context/reference/DATA_SOURCES.md` -- not silently
accepted) with the radar `<canvas>` (owned/rendered by
`src/radar/useRadarRenderer.ts`) overlaid on top. The canvas's on-screen
CSS position/size is recomputed on every MapLibre `move`/`zoom`/`resize`
event to keep it centered on the selected site and scaled to a fixed,
documented approximate radius (`APPROX_SWEEP_RADIUS_KM`, see that file's
comments) -- **not** a real geographic (Mercator) reprojection of the
sweep's actual polar geometry, which is explicitly out of scope for this
stage. The radar canvas's backing-buffer resolution never changes (only its
CSS box does), so pan/zoom/resize never touch the GPU surface at all --
`radar-web`'s exported API did not need a resize/re-render addition for
this stage.

## Site directory

`src/data/wsr88d-sites.json` is a vendored, byte-identical copy of
`fixtures/nexrad-sites/wsr88d-sites.json` (159 real WSR-88D sites, sourced
from NOAA/NWS's own `api.weather.gov` API -- see that fixture's own
`README.md` for full provenance). See `src/data/README.md` for how to
refresh it. Default selected site: `KTLX`.

## Verification performed

See the task's final report for the full command output, browser
console/network log, and screenshot description. Summary:

- `npm run typecheck` and `npm run build` (both including the wasm build)
  pass.
- `npm run dev` was driven with a real installed Chrome via
  `puppeteer-core` (same approach `crates/radar-web/README.md` used): the
  map renders, the site picker works, a real live NOAA scan is discovered
  and downloaded from the browser, decoded, and rendered onto the canvas
  overlaid on the map.
