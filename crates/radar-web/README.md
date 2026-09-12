# radar-web

S04 proof-of-concept: browser/WebAssembly glue proving the existing
`nexrad-level2` / `radar-geo` / `radar-render` crates can decode and render a
real NEXRAD sweep inside an actual browser via WebAssembly + WebGPU/WebGL2.

This is **not** the production web integration. No MapLibre, no live
NOAA discovery/download, no `apps/web` changes -- see
`Agent Context/context/stages/S04-map-live-radar.md`. This crate and its
`www/` test page exist only to retire the "does the S01-S03 stack actually
work in a browser" risk before that integration is built.

See `src/lib.rs` and `src/browser.rs` for the full design rationale
(why GPU init needed no `pollster` workaround, what genuinely new `wgpu`
code this crate adds, and how WebGPU/WebGL2 backend negotiation works).

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
with zero bundler/npm step, per this task's "no build step beyond
`wasm-bindgen`/`wasm-pack`" constraint.

`wasm-pack build --target web --out-dir www/pkg` is equivalent (and would
also run `wasm-opt`), but was not used here since `wasm-pack` itself was not
already installed in this environment and pulling it in would mean
installing *two* tools instead of one (`wasm-pack` still shells out to
`wasm-bindgen-cli` internally) for no behavioral difference at this proof
stage.

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
`<canvas>` (fixed CSS size matching the backing buffer 1:1 -- no
`devicePixelRatio` scaling; real DPI-aware resizing is out of scope for
this proof), fetches and decodes the KTLX fixture, selects the
lowest-elevation sweep with a REF moment (same selection rule as
`radar-render`'s own harness), uploads it to the GPU, and renders one frame
-- writing a status line into the page (not just the console) after each
step, ending in `SUCCESS: ...` or `FAILED: ...`.

## Verification performed

- `cargo fmt --all -- --check`, `cargo build --workspace` (native),
  `cargo test -p radar-web` (native -- exercises `sweep_select`, the only
  part of this crate with no `wgpu`/`web-sys` dependency), `cargo clippy -p
  radar-web --all-targets -- -D warnings` (native), and `cargo clippy -p
  radar-web --target wasm32-unknown-unknown --lib -- -D warnings` (wasm32)
  all pass.
- The Claude-in-Chrome browser extension was not connected in the sandbox
  this crate was built in, so the actual browser check was done by driving
  the machine's real installed Chrome directly (via `puppeteer-core`'s CDP
  connection to `chrome.exe`, headless with GPU explicitly enabled --
  `--enable-gpu --enable-unsafe-webgpu --use-angle=d3d11`), against the real
  NVIDIA RTX 5070 GPU confirmed working in S03. This exercised the identical
  code path a manually-opened Chrome tab would (same binary, same GPU,
  `navigator.gpu` present), just launched programmatically instead of
  clicked through.
- Result: **WebGPU was negotiated** (`backend: BrowserWebGpu`), not the
  WebGL2 fallback -- `wgpu::util::new_instance_with_webgpu_detection`
  found a working `navigator.gpu` adapter. The page's own status text
  confirmed every step:

  ```
  GPU adapter acquired:  (backend: BrowserWebGpu)
  Fetched 4,706,439 bytes.
  Decoded 12 sweeps for site KTLX; rendering sweep at elevation 0.48 deg with 720 radials.
  Frame rendered and presented to the canvas.
  SUCCESS: end-to-end decode + render completed.
  ```

  The rendered canvas showed a plausible radar reflectivity sweep: green
  speckled/clustered returns (light echo, consistent with this fixture's
  actual scan) centered on the site (the orthographic camera is
  site-centered), plus a handful of magenta pixels -- `radar_sweep.wgsl`'s
  reserved, palette-independent `RANGE_FOLDED_COLOR` (full-alpha magenta),
  confirming the never-fabricate distinction between valid/range-folded
  data survives all the way through the browser render, not just the
  native one.

- Two things worth noting from the console, neither a defect in this
  crate: (1) `adapterName` came back as an empty string -- Chrome's
  `GPUAdapterInfo` vendor/architecture/device/description fields are
  deliberately blank in a default (non-isolated-context) origin for
  fingerprinting resistance; `backend` (which is not part of that
  privacy-restricted info) came through correctly. (2) Chrome logged
  `The powerPreference option is currently ignored when calling
  requestAdapter() on Windows` (a documented Chrome/Windows WebGPU
  limitation, https://crbug.com/369219127) -- harmless here since this
  proof has no competing low-power adapter to avoid.
- Not exercised: the WebGL2 fallback path (this machine's Chrome had
  working WebGPU, so `webgl` was compiled in but never negotiated), and
  Chrome's non-headless UI chrome itself (headless was used for the
  scripted check above) -- a manually-opened tab exercises the same wasm
  module and GPU path either way.
