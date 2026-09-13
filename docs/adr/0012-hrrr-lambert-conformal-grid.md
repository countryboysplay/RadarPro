# ADR-0012: HRRR's real grid is Lambert Conformal Conic, handled by `grib` with no PROJ needed

Status: Accepted

## Context
S08 (`context/stages/S08-forecast-core-hrrr.md`) requires implementing NOAA
HRRR as a second `ForecastProvider`, specifically to test the abstraction
`forecast-core` generalizes from `provider-gefs` (S07). The stage brief's
own starting guess for HRRR's bucket/key/`.idx` shape needed empirical
verification before writing any decode code, and explicitly flagged the
real open question: "HRRR uses a Lambert conformal projection, NOT GEFS's
regular lat/lon... verify empirically whether HRRR's real grid definition
template... [is] handled by `grib` with `default-features = false`, or
whether HRRR genuinely needs `gridpoints-proj` (which needs a C/C++
toolchain this environment doesn't have, per ADR-0011)."

**Bucket/`.idx`/file-naming facts, verified live against
`noaa-hrrr-bdp-pds` on 2026-09-12:**
- Bucket `noaa-hrrr-bdp-pds`, confirmed anonymous/unsigned, exactly as the
  stage brief guessed.
- Real path shape: `hrrr.<YYYYMMDD>/conus/hrrr.t<HH>z.wrfsfcf<FF>.grib2`
  (plus `.idx` sidecar) for the `conus` surface ("wrfsfc") product --
  confirmed via a real `ListObjectsV2` listing. **Correction to the
  brief's exact guess**: the forecast-hour suffix is zero-padded to **2**
  digits (`f00`, `f01`, ...), not GEFS's 3-digit `f000`. HRRR also
  publishes `wrfnatf<FF>.grib2` (native levels) and other products under
  the same prefix, which this crate does not use.
- `.idx` format confirmed identical in shape to GEFS's
  (`N:offset:d=YYYYMMDDHH:VAR:LEVEL:step:`), and the 2m-temperature
  line's `VAR:LEVEL` naming (`TMP` / `2 m above ground`) is spelled
  **identically** to GEFS's -- confirmed live
  (`71:34722112:d=2026091212:TMP:2 m above ground:anl:`). **Correction to
  the brief's flagged possibility**: HRRR's real `.idx` lines carry no
  `ENS=...`-style annotation at all (not even an empty one) -- the trailing
  segment is always just `anl:` with nothing after the final colon, since
  HRRR is deterministic.
- Byte-range technique confirmed identical to GEFS's: `[this_offset,
  next_offset)`, `[last_offset, Content-Length)` for the last message; a
  captured real message (2026-09-12 12Z, `f00`, 2m temperature) is exactly
  1,196,627 bytes, starts with `GRIB`, ends with `7777`.

**Real GRIB2 message structure, verified via a throwaway probe program
decoding the captured message with `grib` 0.18.5, `default-features =
false`:**
- Product Definition Template **4.0** ("analysis or forecast at a
  horizontal level"), parameter category/number `(0, 0)` (`TMP`) --
  deterministic, no ensemble metadata at all (unlike GEFS's 4.1/4.2).
- Grid Definition Template **3.30** (Lambert Conformal Conic), not
  GEFS's 3.0: `ni=1799, nj=1059` (the well-known real HRRR CONUS ~3km grid
  size), `earth_shape=6` (WMO Code Table 3.2: spherical, radius
  6,371,229.0 m), `latin1=latin2=lad=38.5°N` (a tangent cone),
  `lov=262.5°E` (-97.5°), `Dx=Dy=3,000,000` (raw units of 10^-3 m = 3000 m,
  HRRR's real 3km spacing).
- `grib`'s own `LatLons::latlons()` (public API) decoded this template's
  per-point lat/lon correctly with **no PROJ dependency at all**: first
  point `(21.138123, -122.71953)`, last point `(47.842194, -60.917194)` --
  both physically correct corners of the real HRRR CONUS domain.
- Decoded 2m-temperature values for this message: min=265.5608K,
  max=308.1233K -- physically plausible for September CONUS.

## Decision
Use `grib` (already pinned `=0.18.5`, `default-features = false`, per
ADR-0011) for HRRR's GRIB2 decode exactly as `provider-gefs` does for
GEFS, with **no new dependency and no feature-flag change**.

**Why this works without PROJ, confirmed two ways:**
1. **Reading `grib` 0.18.5's own source**: `src/grid/lambert.rs` implements
   `LatLons for Template3_30` with two code paths --
   `#[cfg(feature = "gridpoints-proj")]` (delegates to the external `proj`
   crate) and `#[cfg(not(feature = "gridpoints-proj"))]` (a **pure-Rust**
   Lambert Conformal Conic projection, `crate::projection::Lcc`, used
   unconditionally when the feature is off). `grib`'s own committed unit
   test for this exact code path (`lambert_grid_latlon_computation`) uses
   Lambert parameters from a real NWS product (`latin1=latin2=lad=25°`,
   `lov=-95°`) and is not itself feature-gated, so it already runs and
   passes under `default-features = false`.
2. **Compiling and running `provider-hrrr`'s actual decode path** against
   the real, live-fetched HRRR message above, with `default-features =
   false`: it decoded Grid Definition Template 3.30's full geometry and
   2m-temperature values correctly (see the physically-plausible min/max
   above), with no `proj`/`proj-sys`/CMake dependency anywhere in the
   resulting dependency tree.

This directly resolves the stage brief's open question: HRRR's Lambert
conformal grid did **not** need anything beyond `grib`'s
`default-features = false` capabilities for decode-time grid geometry and
values.

**What still needed hand-rolling, and why:** `grib`'s Lambert
implementation only ever computes grid-index -> lat/lon (used at decode
time to place already-known values). The render path needs the opposite
direction -- world (longitude, latitude) -> nearest grid cell, for the
fragment shader's per-pixel inverse lookup (`forecast-core`'s single grid
renderer, `shaders/forecast_grid.wgsl`) -- and `grib` has no public API for
that at all (its own forward projection is a private implementation detail
of its decode path). Per the stage brief's own guidance ("this IS a
well-defined, standard, closed-form map projection... hand-rolling just
the projection math... may be reasonable if justified in an ADR"), this
crate implements a minimal (~150 line, including tests and docs) spherical
Lambert Conformal Conic forward/inverse projection
(`forecast_core::projection`, Snyder's standard formulas) for exactly this
one missing direction. This is emphatically **not** a GRIB2 parser and not
PROJ -- it is one well-defined, closed-form trigonometric formula,
independently validated (see Validation below) rather than assumed
correct from the textbook derivation alone.

## Alternatives
- **Require `grib`'s `gridpoints-proj` feature (PROJ/C toolchain)**:
  rejected -- unnecessary, since decode-time geometry works without it
  (see above), and this environment has no C/C++ toolchain wired up for
  PROJ's CMake build (ADR-0011's same constraint).
- **Silently treat HRRR's grid as regular lat/lon** (reuse GEFS's simple
  origin+step formula): rejected outright -- this is exactly the "real
  geometry bug" the stage brief warns against. A Lambert Conformal grid's
  rows/columns are lines of constant *projected* x/y, not constant
  latitude/longitude; treating them as the latter would silently
  mis-place every rendered pixel by an amount that grows with distance
  from the grid's center, in a way that would not be visually obvious at
  a glance (HRRR's grid is very slightly "less wrong" near its center).
- **Store a precomputed per-cell lat/lon array instead of a closed-form
  projection** (using `grib`'s own `latlons()` for every point, kept
  around): rejected for the render path -- it would cost ~15 MB per field
  (1,905,141 points x 2 x f64) for data fully recoverable from ~10 scalar
  parameters via a well-defined formula, and still would not answer "which
  cell is at this world position" (the direction the renderer actually
  needs) without either a spatial index or hand-rolling the same forward
  projection anyway. It *is* used, cheaply and locally, at decode time only
  to derive this grid's own real, scanning-mode-correct spacing (see
  `provider-hrrr::decode`'s module doc) -- a one-time, 3-point lookup
  cost, not a per-pixel or per-render one.

## Consequences

**Benefits**: `forecast-core::grid::GridGeometry` now has two real,
empirically-verified concrete shapes (`RegularLatLon`, `LambertConformal`),
proving the S08 abstraction actually generalizes rather than merely
compiling for a second provider. No new dependency, no new C/C++ toolchain
requirement anywhere in the workspace. `forecast-core`'s GPU renderer
handles both through one shader (`shaders/forecast_grid.wgsl`), branching
on `projection_kind` -- a property of the grid geometry, not of provider
identity -- exactly matching the S08 stage file's own rule about improving
the abstraction rather than special-casing broadly.

**Costs/risks**: `forecast_core::projection::LccProjection` is
this project's own, independently-derived implementation of a standard
formula, not a call into a vetted third-party library for this one
direction -- a subtle sign or normalization error is a real (if narrow)
risk class, mitigated by the validation below (external ground truth from
`grib`'s own decode, not just internal self-consistency). Its scope is
deliberately narrow (spherical only, matching HRRR's real earth shape;
`provider-hrrr::decode` returns a structured error for any other earth
shape rather than silently assuming spherical).

**Portability**: if a future provider needs a Lambert grid over a
non-spherical (ellipsoidal) earth shape, `LccProjection` would need
extending (Snyder's formulas have a documented ellipsoidal case) or a
`gridpoints-proj`/PROJ-based path evaluated on its own merits at that time
-- not assumed to work today.

**Scientific implications**: none of this ADR's decisions affect decoded
physical values, units, or scale/offset -- `grib`'s own decode path
(grid geometry via its built-in Lambert implementation, values via its
existing packing/dispatch code) is used unmodified for all of that.
`forecast_core::projection` is used only (1) at decode time, to derive
this grid's own real spacing from three of `grib`'s own already-decoded
reference points (never to compute or alter a physical value), and (2) at
render time, to resolve a screen pixel to the nearest already-decoded
grid cell (the same nearest-only, no-interpolation convention this
project already uses for radar and for GEFS).

## Validation
- `cargo test -p forecast-core` (37 tests): `projection::tests` validates
  `LccProjection` via (a) round-trip (`project` then `unproject` recovers
  the original point), (b) invariance under a 360-degree longitude shift
  (the exact bug class `normalize_delta` exists to prevent), (c) forward-
  projecting two real, live-fetched HRRR grid points and recovering the
  message's own declared 3000 m `Dx`/`Dy` to well within `f32` rounding,
  and (d) agreement with `grib` 0.18.5's own committed Lambert test
  fixture (different real parameters than HRRR's). `gpu_tests` renders a
  synthetic Lambert-conformal grid through the exact same
  `render_forecast_grid` call as a regular-lat-lon one and confirms
  correct per-cell color placement.
- `cargo test -p provider-hrrr` (29 unit tests + 2 live-network tests):
  decodes a real, committed HRRR fixture message
  (`fixtures/provider-hrrr/hrrr_t12z_f00_tmp2m.grib2`) end to end,
  confirms the grid is `LambertConformal` with the real `ni=1799, nj=1059`
  shape, confirms the empirically-derived `dx_m`/`dy_m` match HRRR's own
  declared 3000 m, and confirms the first decoded grid cell's lat/lon
  matches `grib`'s own independently-computed value. The live-network
  test performs a full discover -> `.idx` -> Range GET -> decode against
  the real bucket.
- `cargo run -p provider-hrrr --bin provider-hrrr-harness --release`
  performed a full live run (2026-09-13 00Z) end to end and rendered a
  geographically recognizable, visually distinctive **curved** CONUS
  shape (a direct visual signature of a Lambert Conformal grid rendered
  through a flat-plane camera, versus GEFS's rectangular render) to
  `target/provider-hrrr-harness/hrrr_conus_temperature_2m.png` --
  visually inspected: recognizable US coastline/terrain texture, physically
  plausible September temperature gradient.
- `cargo test -p provider-hrrr --test cross_provider_render` (S08 Part D):
  fetches one real field from each of GEFS and HRRR and renders both
  through the *exact same* `forecast_core::gpu::render_forecast_grid`
  call, with no provider-specific branching in the test's own render call;
  asserts GEFS renders as a near-complete rectangle (>85% opaque) while
  HRRR's real curved shape leaves a measurably larger transparent border
  (>10 percentage points less opaque) -- a numeric, not just visual,
  confirmation that the shared renderer treats the two providers'
  genuinely different grid geometries correctly rather than identically
  by coincidence.
- `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D
  warnings` both pass across the whole workspace with these crates added.
