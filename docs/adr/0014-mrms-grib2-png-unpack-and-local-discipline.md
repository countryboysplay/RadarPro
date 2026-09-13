# ADR-0014: MRMS GRIB2 PNG-unpack feature selection, local-use discipline, and missing-value sentinels

Status: Accepted

## Context

S09 Phase 1 (`context/stages/S09-timeline-mrms.md`) requires a new crate
(`crates/mrms`, already reserved in `ARCHITECTURE.md`) that fetches,
decodes, and renders two real, live MRMS national gridded radar-mosaic
products end to end natively, mirroring `provider-gefs`'s S07/ADR-0011
process exactly. `DATA_SOURCES.md`'s MRMS section (already updated,
verified live 2026-09-13) flagged three things this ADR needed to resolve
before writing any decode code:

1. MRMS's GRIB2 Section 5 (data representation) is Template 5.41 (PNG
   compression) -- different from GEFS/HRRR's Template 5.3/5.0 packing --
   and `grib` 0.18.5 (already pinned workspace-wide per ADR-0011, with
   `default-features = false`) only supports 5.41 via its
   `png-unpack-with-png-crate` feature, which is normally a *default*
   feature. Whether re-enabling just that one feature reopens the
   C/C++-toolchain problem ADR-0011 solved, or is a pure-Rust addition
   consistent with that ADR's actual reasoning, needed to be checked before
   assuming either way.
2. MRMS's GRIB2 discipline byte is `209` (WMO Table 0.0's "local use"
   range), not a standard meteorological discipline -- whether this affects
   `grib`'s actual Section 5 decode path (as opposed to just metadata
   labels) needed empirical confirmation, not assumption.
3. MRMS's missing/no-coverage sentinel values were not to be assumed from
   secondary documentation -- this project's own discipline (`ADR-0011`,
   `nexrad-level2`, `radar-cache`) requires determining them empirically
   from a real decoded array.

All three were investigated with a throwaway probe program (`src/bin/probe.rs`,
deleted before this stage shipped -- its findings are captured here and in
this crate's own committed tests) decoding two real, live-fetched MRMS
objects, exactly mirroring ADR-0011's own process.

**Live bucket facts confirmed directly against `noaa-mrms-pds`
(2026-09-13):**
- Anonymous, unsigned S3 access, same trust model as `noaa-gefs-pds`/
  `noaa-hrrr-bdp-pds`. Key format exactly as `DATA_SOURCES.md` states:
  `CONUS/<Product>_<height>/<YYYYMMDD>/MRMS_<Product>_<height>_<YYYYMMDD>-<HHMMSS>.grib2.gz`.
- Cadence is genuinely ~2 minutes but **not** on a fixed, guessable
  `HHMMSS` grid -- three consecutive real reflectivity objects on
  2026-09-13 were timestamped `...-000042`, `...-000242`, `...-000441`
  (seconds drift by a couple of seconds run to run). A full UTC calendar
  day's listing for either product was at most a few hundred keys and
  never truncated (`IsTruncated=false` in every real response observed),
  confirming a per-day `ListObjectsV2` listing is both sufficient and cheap
  for discovery -- see `crates/mrms/src/client.rs`'s module doc for why
  this crate's `discover_latest_snapshot` lists a day rather than
  constructing/probing a guessed key the way `provider-hrrr`'s
  `find_recent_run` does.

**Feature investigation: `png-unpack-with-png-crate` is pure Rust.**
Reading `grib` 0.18.5's own `Cargo.toml` (fetched into the local registry
cache for this investigation) shows its `default` feature list is actually
**four** features, not the three ADR-0011 named (ADR-0011 only needed to
reject the three GEFS's `pgrb2sp25` product never touched):
```
default = [
    "jpeg2000-unpack-with-openjpeg",   # dep:openjpeg-sys (C, CMake)
    "png-unpack-with-png-crate",       # dep:png -- pure Rust
    "ccsds-unpack-with-libaec",        # dep:libaec-sys (C, CMake)
    "gridpoints-proj",                 # dep:proj (C, CMake)
]
png-unpack-with-png-crate = ["dep:png"]
```
`png-unpack-with-png-crate` depends on nothing but the `png` crate (and
transitively `miniz_oxide`/`crc32fast`, both pure Rust) -- confirmed by
building `crates/mrms` with `default-features = false, features =
["png-unpack-with-png-crate"]` and inspecting the full dependency tree
(`cargo tree -p mrms`): the only new crates pulled in versus GEFS/HRRR's
existing dependency set were `png`, `flate2` (this crate's own gzip
dependency, also pure-Rust `rust_backend`), and their ordinary pure-Rust
transitive dependencies -- **zero** occurrences of `openjpeg-sys`,
`libaec-sys`, `proj-sys`, or `cmake` anywhere in the tree. The build itself
succeeds with no C/C++ toolchain configured in this environment (the same
environment ADR-0011 documented as lacking one), which is the actual test
that matters, not just the absence of `-sys` crate names.

Reading `grib`'s own decoder source
(`src/decoder/png.rs`/`src/decoder.rs`) confirms the implementation: PNG
decoding is delegated entirely to the `png` crate
(`png::Decoder::new(...).read_info()...next_frame(...)`), then the decoded
raster bytes are fed through the same bit-depth-aware
`NBitwiseIterator`/`NonZeroSimplePackingDecoder` machinery every other
Section 5 template already uses for its final scale/offset step. This is
architecturally the same shape as GEFS/HRRR's already-accepted Template
5.0/5.2/5.3 decoders, just with PNG (instead of raw bit-packing or complex
packing) as the up-front unpacking step.

**Discipline-209 confirmed not to affect Section 5 decoding.** The probe
decoded both real messages fully (correct grid geometry, correct value
count, physically plausible values -- see below) despite discipline byte
`209`; reading `grib`'s `dispatch()` (`src/decoder.rs`) shows the Section 5
decode dispatch switches purely on `DataRepresentationTemplate`'s own
enum variant (i.e. Section 5's own template number), never on Section 0's
discipline byte -- discipline only ever affects how a caller interprets
Section 4's `parameter_category`/`parameter_number` as physical field
identity, which this crate's design deliberately never relies on (see
`crates/mrms/src/keys.rs`'s and `src/decode.rs`'s module docs: field
identity comes from which URL/product was fetched). Confirmed empirically:
`MergedReflectivityQCComposite` decoded as category 10 / number 0;
`PrecipRate` decoded as category 6 / number 1 -- different center-local
codes for different real products, exactly as expected for discipline
209's "local use" meaning, with no effect on either message's successful,
correct decode.

**Grid geometry confirmed against real Section 3 bytes** (Grid Definition
Template 0, same family GEFS uses -- no PROJ needed): `ni=7000, nj=3500`,
`La1=54.995°N, Lo1=230.005°E (-129.995°W)`, `La2=20.005°N,
Lo2=299.995°E (-60.005°W)`, `Di=Dj=0.01°` -- matching `DATA_SOURCES.md`
exactly, for both products.

**Missing-value sentinels, determined empirically, not assumed.** The
probe placed decoded values into their real `(row, col)` grid positions
(via `grib`'s own scanning-mode-aware `ij()`, exactly like
`provider-gefs`/`provider-hrrr`'s decoders) and rendered an ASCII map of
which cells held which of the two most common raw values. The result is an
immediately recognizable silhouette of the continental United States: one
sentinel value fills exactly the ocean/border areas outside the CONUS
radar network's reach, and the other fills the interior landmass wherever
there is currently no significant radar return -- confirmed again,
independently, by a second throwaway diagnostic (`src/bin/debug_coverage.rs`,
also deleted before shipping) that rendered the two sentinels plus real
values as three flat, distinct colors: the output is an unmistakable CONUS
coastline (Gulf Coast curve, Pacific Northwest bump, a Florida-peninsula-
shaped notch on the lower right), with real echo cells scattered inside it
in physically plausible storm-cell/squall-line shapes, never in the ocean
region.

- `MergedReflectivityQCComposite`: `-999.0` = outside the radar network's
  domain entirely (confirmed: traces the ocean/border shape); `-99.0` =
  inside the domain, no significant echo currently detected (confirmed:
  fills the CONUS interior). Real decoded values that stage's live
  snapshot: min `-999.0` (before classification) / max `63.5` dBZ, with
  quantization steps of `0.5` dBZ near the real data range -- nowhere near
  either sentinel, so no ambiguity is possible.
- `PrecipRate`: `-3.0` = outside the domain (same ocean/border shape,
  confirmed via a second live snapshot). `PrecipRate` has **no** separate
  "covered but dry" sentinel: a genuine `0.0` mm/hr reading is already the
  correct physical representation of "no precipitation" there, so it is
  never reclassified as missing. Live snapshot: max `175.0` mm/hr (an
  extreme convective rate, physically plausible), quantization steps of
  `0.1` mm/hr near the real data range.

These are implemented as an explicit three-state
`crates/mrms/src/grid.rs::MrmsCellValue` enum (`NoCoverage` / `Missing` /
`Value(f32)`, mirroring `radar_types::GateValue`'s existing
`Missing`/`RangeFolded`/`Value(f32)` shape for NEXRAD) rather than a raw
float array with sentinels left for every downstream consumer to
rediscover.

## Decision

1. **`grib` feature set**: keep `default-features = false` (ADR-0011's
   decision stands unmodified for the three C-toolchain-requiring
   features), and additionally enable `png-unpack-with-png-crate` in
   `crates/mrms/Cargo.toml` only (not workspace-wide -- GEFS/HRRR still
   need no PNG support). This is **not a reversal** of ADR-0011: ADR-0011's
   actual reasoning was "avoid C/C++-toolchain dependencies wherever a
   toolchain-free alternative exists for the feature set actually needed,"
   not "avoid every optional `grib` feature unconditionally." Confirmed
   pure-Rust (see above), this extends ADR-0011's discipline to a second
   crate with a genuinely different Section 5 requirement, exactly as
   ADR-0011's own "Portability" section anticipated ("any future ...
   product ... that genuinely needs JPEG2000/PNG/CCSDS-packed data
   (verified empirically first) will need either a C/C++ toolchain ... or
   one of `grib`'s pure-Rust alternative feature flags evaluated on its own
   merits at that time" -- `png-unpack-with-png-crate` turned out to
   already be that pure-Rust alternative for PNG specifically, no new
   crate evaluation needed).
2. **Discipline 209**: never interpreted semantically. `crates/mrms`
   identifies a decoded field's physical identity purely by which
   `MrmsProduct`/URL was fetched, records category/number only as
   diagnostic metadata, and never builds a Table-4.2-style lookup for
   discipline-209 codes.
3. **Missing-value sentinels**: classified explicitly into
   `MrmsCellValue::{NoCoverage, Missing, Value}` at decode time
   (`crates/mrms/src/decode.rs::classify_raw_value`), with the exact
   sentinel constants and tolerance documented inline. `MrmsGrid::stats()`
   computes min/max/mean only over `Value` cells; `MrmsGrid::display_values`
   requires a caller-chosen fill value for the GPU path rather than ever
   silently passing a sentinel through as if it were real data.
4. **Shared GPU renderer decoupled from forecast semantics**: this crate
   needs `forecast_core::gpu::render_forecast_grid`'s exact rendering
   pipeline (upload/pipeline/palette machinery, unmodified) but has no
   `ForecastGrid` to hand it -- MRMS has no run/lead-time/ensemble concept
   (see `crates/mrms/src/lib.rs`'s module doc), so constructing a fake
   `ForecastGrid` just to call that function would misrepresent an
   observation as forecast-shaped data. Reading the function's actual body
   confirmed it only ever touches `grid.geometry` from its `ForecastGrid`
   parameter (`display_values`/`palette_lut`/etc. are already separate
   arguments) -- so `forecast_core::gpu::render_forecast_grid`'s signature
   was changed to take `&forecast_core::grid::GridGeometry` directly
   instead of `&ForecastGrid`, with every existing call site
   (`forecast-core`'s own GPU tests, `forecast-web`'s wasm API,
   `provider-gefs`/`provider-hrrr`'s harnesses, and
   `provider-hrrr`'s cross-provider test) updated to pass `&grid.geometry`.
   This is a small, low-risk refactor consistent with this project's "avoid
   premature abstraction, but this is genuinely the second concrete need"
   rule: GEFS/HRRR forecast rendering and MRMS observation rendering now
   call the literal same function identically, and the GPU pipeline itself
   (shader, pipeline setup, palette upload) is not duplicated anywhere.

## Alternatives

- **Hand-roll PNG/Section-5.41 unpacking**: rejected for the same reason
  ADR-0011 rejected hand-rolling GRIB2 entirely -- an established, tested
  decoder (`grib`'s own `png-unpack-with-png-crate` path, itself built on
  the mature `png` crate) is available and toolchain-free; reimplementing
  PNG decoding from scratch would add real risk for no benefit.
- **Enable `grib`'s default features wholesale** (accepting the C toolchain
  requirement for the other three): rejected for the identical reason
  ADR-0011 rejected it -- this workspace still has no C/C++ toolchain wired
  up, and none of the other three features are needed for either MRMS
  product actually implemented.
- **Force MRMS through `forecast_core::provider::ForecastProvider`/
  `ForecastGrid`** (e.g. by setting `forecast_lead_hours: 0` and
  `ensemble: None` on a synthetic `ForecastGrid`): rejected per the stage
  brief's own explicit instruction and GLOBAL_CONTRACT's observation/
  forecast distinction -- a `ForecastGrid` with meaningless forecast fields
  set to placeholder values is exactly the kind of "dress an observation up
  in forecast-shaped metadata" this project's own contract forbids. A
  plain client + fetch/decode API (`crates/mrms`'s actual shape) is
  correct here instead.
- **Give `mrms` its own duplicate GPU render pipeline** (rather than
  refactoring `render_forecast_grid`'s signature): rejected once reading
  the function's body confirmed it never needed anything from
  `ForecastGrid` beyond `geometry` -- duplicating the shader/pipeline/
  palette-upload code to avoid a one-parameter-type signature change would
  have been the actual premature-abstraction-avoidance failure mode (two
  near-identical GPU pipelines instead of one shared one).
- **Treat `-99`/`-999`/`-3` as literal physical values** (e.g. clamp and
  plot them through the palette unmodified, relying on them falling below
  the palette's visible floor): rejected as insufficiently explicit --
  while it happens to render transparently for reflectivity today (both
  sentinels are far below the REF ramp's domain minimum), it would
  conflate "no coverage" with "missing" in any numeric context (stats,
  point lookups, future analysis) and provides no structural guarantee
  against a future palette/domain change silently making a sentinel appear
  as a real color. The explicit `MrmsCellValue` enum costs one small type
  and pays for itself in every consumer that isn't purely visual.

## Consequences

**Benefits**: real GRIB2 PNG-Section-5.41 decoding is fully delegated to
`grib`'s own maintained, pure-Rust path; this crate's own code never
touches pixel/PNG decoding directly. Missing/no-coverage handling is a
type-level guarantee (an `MrmsGrid::stats()` caller cannot accidentally
average in a sentinel; a GPU caller must explicitly choose a fill value).
The `render_forecast_grid` refactor removes an unnecessary coupling between
the shared GPU renderer and forecast-specific metadata with zero pipeline
duplication.

**Costs/risks**: `png-unpack-with-png-crate` is now a second, product-
specific `grib` feature combination in this workspace (GEFS/HRRR:
`default-features = false` only; MRMS: that plus
`png-unpack-with-png-crate`) -- a future contributor adding a new MRMS-like
product must re-verify (not assume) which Section 5 template it uses,
exactly as this ADR did. The `MrmsCellValue` sentinel values
(`-999.0`/`-99.0`/`-3.0`) and their tolerance (`1e-3`) are this crate's own
empirical findings for the two products it decodes as of this stage --
verified against two real live snapshots each is a reasonable but not
exhaustive sample; a future MRMS product this crate adds must re-verify its
own sentinel(s) rather than assuming these three values generalize.

**Scientific implications**: none of this ADR's decisions affect decoded
physical values, units, or scale/offset -- `grib`'s own PNG decode path is
used unmodified; this crate's only original numeric logic in the decode
path is classifying already-decoded floats against three documented
sentinel constants.

## Validation

- `cargo test -p mrms` (32 unit/integration tests, including real
  fixture-based decode tests against two real, live-captured gzip-wrapped
  GRIB2 messages committed under `fixtures/mrms/` -- one per product,
  mirroring `fixtures/provider-gefs/`/`fixtures/provider-hrrr/`) passes.
  Tests cover: real-message decode + grid geometry, physical min/max
  matching this stage's own live probe, sentinel classification never
  misfiring on a plausible near-sentinel real value, `NoCoverage`
  geography (a deep-ocean point) versus a CONUS-interior point, and
  malformed/truncated/empty input handled cleanly (no panics).
- `cargo run -p mrms --bin mrms-harness` performed a full live run
  (2026-09-13 ~05:18Z) end to end for both products: discovered the latest
  published snapshot via a real `ListObjectsV2` day listing, fetched and
  gunzipped a real object, decoded it, printed real min/max/mean plus
  missing/no-coverage percentages, and rendered a geographically
  recognizable image to `target/mrms-harness/mrms_conus_{reflectivity,precip_rate}.png`
  on a real GPU (NVIDIA GeForce RTX 5070, Vulkan backend) via the shared
  `forecast_core::gpu::render_forecast_grid` path.
  - Reflectivity: min=-23.0 dBZ, max=62.5 dBZ, mean=17.2 dBZ over the
    989,450 real-valued cells (4.0% of the grid); 62.2% of cells were
    "covered, no echo," 33.8% "no coverage" (outside the network). The
    rendered PNG shows several scattered green/yellow/orange storm-cell and
    squall-line-shaped regions against a transparent background, in
    plausible relative CONUS positions (a comma-shaped cluster over the
    north-central US, a cluster over the southeast, a north-south line over
    the northeast) -- physically plausible for a real overnight snapshot.
  - Precipitation rate: min=0.0 mm/hr, max=130.0 mm/hr (this run's snapshot
    differed slightly by a couple of minutes from the reflectivity
    snapshot fetched in the same harness run, since each product is
    discovered/fetched independently -- expected, not a bug), mean=0.083
    mm/hr; 34.1% "no coverage," 0% "missing" (`PrecipRate`'s `0.0` is real
    data, never reclassified), matching this ADR's documented sentinel
    design. The rendered PNG shows the same storm systems in blue/green/
    yellow/red, positioned consistently with the reflectivity render.
  - A separate, deliberate three-color coverage-mask diagnostic (not part
    of the shipped renderer -- see this ADR's Context section) confirmed
    the sentinel classification's geography is an unmistakable CONUS
    coastline shape, independent of the actual weather that night.
- `cargo build --workspace`, `cargo test --workspace`,
  `cargo fmt --all -- --check`, and `cargo clippy --all-targets -- -D
  warnings` all pass with `crates/mrms` added and
  `forecast_core::gpu::render_forecast_grid`'s signature change applied
  across the whole workspace.
