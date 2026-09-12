# radar-web

Browser/WebAssembly glue proving the existing `nexrad-level2` / `radar-geo` /
`radar-render` crates can decode a real NEXRAD volume once and then render
any `(elevation, moment)` selection from it, load/apply a user-supplied
color table, probe a cursor position, and fetch range-ring geometry -- all
inside an actual browser via WebAssembly + WebGPU/WebGL2.

Originally an S04 proof-of-concept (decode + render exactly one fixed
sweep/moment); generalized in S05 -- see `src/lib.rs` and `src/browser.rs`
for the full design rationale, `crates/radar-render/COLOR_TABLE_FORMAT.md`
for the color-table format, and `docs/adr/0009-original-color-table-format.md`
for that format's design rationale.

This is **not** the production web integration. No MapLibre rendering
code, no live NOAA discovery/download, no `apps/web` UI (pickers, animation
controls, a color-table editor, the actual probe-on-click interaction) --
those consume this crate's API in a separate follow-up task. See
`Agent Context/context/stages/S05-radar-workstation.md`.

## Building the wasm module

Requires the `wasm32-unknown-unknown` target and `wasm-bindgen-cli` **pinned
to the exact same version** as the `wasm-bindgen` crate in `Cargo.toml`
(mismatched versions produce a runtime error, not a compile error):

```sh
rustup target add wasm32-unknown-unknown   # if not already installed
cargo install wasm-bindgen-cli --version 0.2.128 --locked
```

Then, from the repo root:

```sh
cargo build --target wasm32-unknown-unknown -p radar-web --lib --release
wasm-bindgen --target web \
  --out-dir crates/radar-web/www/pkg \
  --out-name radar_web \
  target/wasm32-unknown-unknown/release/radar_web.wasm
```

This regenerates `www/pkg/` (`radar_web.js`, `radar_web_bg.wasm`, and the
`.d.ts` files) -- a build artifact, not hand-written source; delete and
regenerate it freely. `--target web` was chosen (over `--target bundler` or
`--target no-modules`) so the test page can load it as a plain ES module
with zero bundler/npm step.

## Serving the test page

The page `fetch()`es the real KTLX fixture already used by `radar-render`'s
native harness directly from `fixtures/nexrad-level2/`, via a relative path
(`../../../fixtures/...`) from `www/index.js` -- so it must be served from
the **repo root**, not from `www/` itself, or that fetch 404s:

```sh
cd C:/RadarPro    # repo root
python -m http.server 8731
```

Then open `http://localhost:8731/crates/radar-web/www/index.html` in
Chrome. (Any static file server works; a bare `file://` open does not,
because `fetch()` of a local file and the wasm module's own `fetch()` of
its `.wasm` bytes are both blocked under the `file:` scheme by most
browsers.)

## What the page does

Loads the wasm module, requests a GPU device/surface for a 512x512
`<canvas>`, fetches and decodes the KTLX fixture **once**, then builds an
elevation `<select>` and a moment `<select>` from the decoded volume's own
metadata (`VolumeSummary`/`momentWireCodesForSweep`). Changing either
`<select>` calls `selectAndRender(sweepIndex, momentCode)` -- re-rendering
the newly-selected sweep/moment without ever re-fetching or re-decoding the
Archive II bytes. `window.__radarWebTest` additionally exposes the renderer
and a `select(sweepIndex, momentCode)` helper directly, for scripted
(CDP/puppeteer) verification without simulating real `<select>` pointer
events.

## The wasm-bindgen API surface

`RadarWebRenderer` (constructed via the async `initGpu(canvas, width,
height)`):

- `decodeVolume(bytes) -> VolumeSummary` -- decode once; `VolumeSummary`
  exposes `siteIcao`, `sweepCount`, `elevationDegs()`.
- `momentWireCodesForSweep(sweepIndex) -> string[]`
- `defaultSweepIndexForMoment(momentCode) -> number | undefined`
- `loadColorTable(json) -> ColorTableApplyResult` / `resetColorTable(momentCode)`
  / `activeColorTableJson(momentCode) -> string`
- `selectAndRender(sweepIndex, momentCode)` -- rebuilds/re-uploads GPU
  resources only if the selection (or that moment's active color table)
  actually changed since the last call; otherwise just re-renders.
- `probeGate(siteLat, siteLon, sweepIndex, momentCode, cursorLat, cursorLon) -> GateProbeResult | undefined`
- `adapterName` / `backend` getters (unchanged from S04)

Free function: `rangeRingsGeoJson(siteLat, siteLon, radiiKm, numPoints) ->`
a nested `[[ [lon, lat], ... ], ...]` array (one ring per radius, GeoJSON
coordinate order), directly usable as a MapLibre `MultiLineString`'s
`coordinates`.

See `src/browser.rs`'s doc comments for the full contract of each method
(error behavior, what "selection changed" means, missing/range-folded
state handling in `probeGate`, etc.).

## Verification performed

- `cargo fmt --all -- --check`, `cargo build --workspace` (native),
  `cargo test --workspace` (native -- exercises `sweep_select`/
  `render_select`, including a structural proof that decoding a real
  fixture once and building GPU-buffer input for two different
  `(elevation, moment)` selections never calls the decoder twice),
  `cargo clippy --all-targets -- -D warnings` (native), and `cargo clippy
  -p radar-web --target wasm32-unknown-unknown --lib -- -D warnings`
  (wasm32) all pass.
- Real-browser check via `puppeteer-core`'s CDP connection to the
  machine's installed Chrome (`chrome.exe`), headless with GPU explicitly
  enabled (`--enable-gpu --enable-unsafe-webgpu --use-angle=d3d11
  --no-sandbox`), serving this repo over a plain local HTTP server. This
  exercised the identical code path a manually-opened Chrome tab would.
  - **WebGPU was negotiated** (`backend: BrowserWebGpu`).
  - Decoded the real KTLX fixture once (12 sweeps), then rendered three
    distinct selections **without re-fetching or re-decoding**: REF at
    elevation 0.48 deg (sweep 1), VEL at elevation 0.88 deg (sweep 3, a
    *different* sweep), and SW at that same 0.88 deg sweep (a pure
    moment-only switch, same elevation as the VEL render). Screenshots of
    each were inspected directly:
    - **REF** (sweep 1): green speckled/clustered near-site returns plus
      two larger green blobs further out -- the same shape as S04's own
      original REF screenshot (this fixture/sweep is unchanged), with a
      handful of magenta range-folded pixels.
    - **VEL** (sweep 3, different elevation): the same two far-field blobs
      now render green under VEL's diverging green(inbound)/gray(near
      zero)/red(outbound) palette -- plausible (a green result under this
      palette just means "strongly inbound," not "the REF palette leaked
      through") -- while the near-site cluster shows a mixed
      red/white/green speckle, consistent with turbulent, sign-varying
      near-radar velocities. Visibly different from the REF render.
    - **SW** (sweep 3, same elevation as the VEL render): every echo --
      both the far-field blobs and the near-site cluster -- renders in a
      single cyan/teal hue (SW's default single-hue palette), visibly
      different from both the REF and VEL renders of the very same
      underlying sweep shapes, confirming a moment-only switch (same
      elevation, no re-decode) changes the render correctly.
  - Loaded a custom, distinctive "all blue" REF color table via
    `loadColorTable` and re-rendered the same REF sweep: the identical
    shapes from the default-REF screenshot rendered entirely in blue
    instead of green, confirming a user-supplied table is parsed,
    validated, applied, and actually changes what is drawn.
  - Fed `loadColorTable` a deliberately malformed JSON string
    (`"{ not json"`): it threw a structured error
    (`"invalid color-table JSON: key must be a string at line 1 column 3"`)
    rather than crashing the page or corrupting renderer state -- the page
    kept working normally afterward.
  - Called `probeGate` for a cursor near the KTLX site on the active REF
    sweep: resolved to azimuth ~53.8 deg, slant range ~3.13 km, state
    `"valid"`, value `0` dBZ, units `"dBZ"` -- a real, distinctly-typed
    gate resolution, not a collapsed/fabricated result.
  - Called `rangeRingsGeoJson` for 50/100/150 km rings around KTLX: three
    8-point rings of `[lon, lat]` pairs at geographically plausible
    coordinates around the site.
  - The only console error was a `404` for `/favicon.ico` (confirmed via
    the HTTP server's own access log) -- unrelated to the wasm module,
    the fixture fetch, or any rendering step.
