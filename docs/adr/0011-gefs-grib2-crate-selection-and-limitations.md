# ADR-0011: `grib` crate selection for GEFS GRIB2 decoding, and its documented limitations

Status: Accepted

## Context
S07 (`context/stages/S07-gefs-ensemble-poc.md`) requires moving one real
GEFS field (2m temperature) end-to-end from NOAA's public `noaa-gefs-pds`
S3 bucket to a rendered GPU image, using "an established GRIB2 decoding
crate... rather than hand-rolling a partial GRIB2 parser," and to
"document the gap in an ADR" if that crate cannot handle a
section/encoding GEFS actually uses.

Before writing any decode code, this stage's own discipline ("verify the
real bucket layout, file naming, and GRIB2 message structure empirically")
was applied to both the data and the candidate crate:

**Bucket/`.idx` facts, verified live against `noaa-gefs-pds` on
2026-09-12/13:**
- Bucket layout, anonymous/unsigned access, and `gefs.<YYYYMMDD>/<HH>/atmos/pgrb2sp25/<member>.t<HH>z.pgrb2s.0p25.f<FFF>`
  naming all confirmed exactly as this stage's brief stated. Members
  confirmed: `gec00`, `gep01..gep30` (31 confirmed present via a real
  listing), `gep31` confirmed **absent** (HTTP 404), `geavg`.
- `.idx` format confirmed: `N:offset:d=YYYYMMDDHH:VAR:LEVEL:step:ENS=...`,
  one line per message, 1-indexed. **Correction to the brief's stated
  facts**: the `ENS=` suffix's exact text differs by member type --
  `ENS=low-res ctl` for the control member (`gec00`), `ENS=+1`/`ENS=+N`
  for perturbed members, and (no `ENS=` prefix at all) `ens mean` for the
  ensemble mean (`geavg`). This crate never parses that suffix for
  ensemble identity (see "Ensemble metadata gap" below) -- it is
  diagnostic text, not a field this crate relies on for correctness.
- Byte-range technique confirmed exactly as stated: `[this_offset,
  next_offset)` for all but the last message, `[last_offset,
  Content-Length)` for the last, one single Range GET returning a
  self-contained GRIB2 message starting with `GRIB` and ending in `7777`,
  with Section 0's declared total-message-length field matching the
  fetched byte count exactly in every case tested.
- `pgrb2sp25` for the run tested publishes forecast hours from `f000`
  through at least `f240` (10 days) at 3-hour steps.

**Fuzz/resilience testing performed against `grib` 0.18.5 before
committing to it** (see `crates/provider-gefs/src/decode.rs`'s and
`ensemble.rs`'s test suites for the smaller, permanent versions of these
checks): 159 single-byte-flip trials and 200 multi-byte random-corruption
trials against a real captured message, plus a truncation sweep at 3001-
byte increments across the whole message -- zero panics in any trial,
every corrupted input either still parsed (packed data with no CRC can
tolerate some bit corruption) or returned a structured `GribError`.
Garbage/empty input returns `Ok` with zero submessages/sections (not an
error) -- handled by this crate's own `GefsError::NoGrib2Submessage`,
never silently treated as "field present but empty."

## Decision
Use the `grib` crate (crates.io `grib`, repository `noritada/grib-rs`),
pinned to `=0.18.5`, with **`default-features = false`**.

**Why `default-features = false` is required, not optional:** `grib`
0.18.5's default features (`jpeg2000-unpack-with-openjpeg`,
`ccsds-unpack-with-libaec`, `gridpoints-proj`) each pull in a C/C++
dependency (`openjpeg-sys`, `libaec-sys`, `proj-sys` via CMake). Verified
empirically: `cargo build` with default features fails in `proj-sys`'s
build script (`CMake will not be able to correctly generate this
project` / MSBuild `FileTracker` errors) in this environment, which has no
C/C++ toolchain wired up for CMake's `TryCompile` step -- the same
constraint that already led `nexrad-level2` to `bzip2-rs` over
`bzip2-sys`/`bzip2` and `radar-cache` to `rustls`+`ring` over
`native-tls`/`aws-lc-rs`. None of those three features are needed for this
crate's scope: every real GEFS `pgrb2sp25` message fetched during this
stage's verification used Grid Definition Template 3.0 (regular lat/lon --
`Template3_0::latlons_unchecked` needs no PROJ; PROJ (`gridpoints-proj`)
is only required for other grid systems such as Lambert
conformal/Mercator/polar stereographic) and Data Representation Template
5.3 (complex packing with spatial differencing -- not JPEG2000/PNG/CCSDS,
which `grib` documents as built-in-supported alongside simple packing 5.0
and complex packing 5.2/5.3 regardless of those three optional features).
Building with `default-features = false` compiles and correctly decodes
every real message fetched for this stage -- confirmed by this crate's own
tests, all running against real, live-fetched fixture messages committed
under `fixtures/provider-gefs/`.

**Cross-checks performed against `grib`'s decode of a real message**
(before writing any of this crate's own decode code, via a throwaway probe
program, then formalized as this crate's actual tests):
- `Section3`/`GridDefinitionTemplateValues::Template0`: `ni=1440, nj=721,
  la1=90.0°, lo1=0.0°, la2=-90.0°, lo2=359.75°, di=dj=0.25°` -- a standard
  global 0.25° grid, confirmed against `grib`'s own `latlons()` iterator
  (`(90.0, 0.0)` first point, `(-90.0, -0.25)` last -- the `-0.25` vs
  `359.75` is just that iterator's own `[-180,180)` longitude
  normalization, not a discrepancy in the underlying `Grid` struct's raw
  fields, which this crate reads directly instead).
- `Section1`/`Identification::ref_time_unchecked()`: exactly matched the
  run being fetched (`2026-09-12 12:00:00`/`18:00:00`, etc.) for every
  message tested.
- `Section4`/`ProdDefinition::forecast_time()`: exactly matched the
  requested forecast hour (`f000` -> 0 hours) for every message tested.
- `Section5`/decoded values: physically plausible 2m-temperature range
  (≈200-320K globally; a real polar cell at `(row 0, col 0)` always in
  [213.15, 283.15]K across every run tested) -- see
  `decode::tests::spot_check_a_known_grid_cell_against_an_independent_source`
  and the harness's own printed min/max, cross-referenced against this
  session's own live captures (documented inline in test comments with the
  exact source message).

## Alternatives
- **Hand-roll a from-scratch GRIB2 parser** (the approach `nexrad-level2`
  took for NEXRAD's own project-specific binary format): rejected per the
  stage brief's own explicit instruction -- GRIB2 is a large,
  WMO-standardized format (multiple grid definition templates, multiple
  data representation/packing templates, dozens of product definition
  templates) with far more general-purpose complexity than NEXRAD Level
  II's fixed, single-vendor layout; correctly reimplementing even just
  complex packing with spatial differencing (Template 5.3, the encoding
  every real GEFS message tested actually uses) from the raw WMO spec
  would be a substantial, error-prone undertaking for a PoC stage, with
  much higher real risk of a subtle scale/offset bug silently corrupting
  physical values than reusing a maintained, tested decoder.
- **A different Rust GRIB2 crate**: no viable alternative was found on
  crates.io at the time of this search -- `grib`/`grib-rs` is, per this
  stage's own brief, "the known one," and this search corroborated that
  (other results were unrelated crates with superficially similar names,
  a GRIB *builder*/codegen tool, or work-in-progress single-feature
  readers with far less template coverage).
- **`grib` with default features, accepting the C toolchain requirement**:
  rejected -- GLOBAL_CONTRACT-adjacent project precedent (`nexrad-level2`,
  `radar-cache`) already establishes that this project avoids
  C-toolchain-requiring dependencies wherever a pure-Rust or
  toolchain-free alternative exists for the actual feature set needed, and
  here one does (`default-features = false` loses nothing this crate
  needs).

## Consequences

**Benefits**: real GRIB2 decoding (grid definition, product definition,
data representation/packing, values) is fully delegated to a maintained,
independently-tested crate rather than a from-scratch parser, for a small,
toolchain-free dependency footprint (`default-features = false` adds no
new C/C++ build requirement to this workspace). Every fact this ADR
documents about the real bucket, `.idx` format, and message structure was
verified against live data, not assumed from documentation, matching this
project's `nexrad-level2`-precedent discipline.

**Costs/risks — the ensemble metadata gap**: `grib` 0.18.5's
`ProdDefinition` public API exposes `parameter_category`/
`parameter_number`/`generating_process`/`forecast_time`/`fixed_surfaces`,
but has **no accessor at all** for the ensemble-specific fields WMO GRIB2
Product Definition Templates 4.1 ("individual ensemble forecast") and 4.2
("derived forecast based on all ensemble members") carry: type of
ensemble forecast / perturbation number (4.1), derived forecast type
(4.2) -- verified by reading `grib-0.18.5`'s own source
(`src/datatypes/sections.rs`): `ProdDefinition`'s only way to reach
template-specific bytes beyond those five named accessors is `iter()`,
which yields the raw, already section-bounds-checked template payload
with no further interpretation.

This is exactly the kind of gap the stage brief anticipated ("if it can't
handle a section/encoding GEFS actually uses, document the gap... rather
than writing a partial from-scratch parser") -- but it is a narrow
*metadata accessor* gap, not a decoding gap: grid geometry, product
identity/forecast time, and the actual packed values are all fully and
correctly handled by `grib` itself. `crates/provider-gefs/src/ensemble.rs`
closes this specific gap by reading exactly two fixed-offset fields (WMO's
own spec-defined byte positions for templates 4.1/4.2) directly from
`ProdDefinition::iter()`'s already-validated raw payload, rather than
reimplementing general PDT parsing. The exact byte offsets were derived
empirically (a throwaway probe program decoding real fetched messages byte
by byte) and are cross-checked against three real, live-fetched messages
(`gec00`, `gep01`, `geavg`) in `ensemble.rs`'s own test suite, with the raw
bytes captured verbatim as test fixtures and documented inline.

**Portability**: `default-features = false` is a permanent constraint,
not a temporary workaround -- any future GEFS product this project adds
that genuinely needs JPEG2000/PNG/CCSDS-packed data (verified empirically
first, per this same stage's discipline) will need either a C/C++
toolchain to become available in every build environment this project
targets, or one of `grib`'s pure-Rust alternative feature flags
(`jpeg2000-unpack-with-hayro`, `ccsds-unpack-with-rust-aec`) evaluated on
its own merits at that time.

**Scientific implications**: none of this ADR's decisions affect decoded
physical values, units, or scale/offset -- `grib`'s own decode path is
used unmodified for all of that; this crate's only original code in the
decode path is (1) reading two documented WMO byte offsets for ensemble
identity, and (2) placing decoded values into a row-major grid via
`grib`'s own `ij()` iterator (which already correctly accounts for
GRIB2 scanning mode), not by hand-decoding scanning-mode bits.

## Validation
- `cargo test -p provider-gefs` (61 unit/integration tests in `src/`,
  including three tests decoding real, live-captured fixture GRIB2 bytes
  committed under `fixtures/provider-gefs/`) and
  `cargo test -p provider-gefs --test live_network` (2 tests against the
  real, live `noaa-gefs-pds` bucket: full discover -> `.idx` -> Range GET
  -> decode for `gec00`/`gep01`/`geavg`, and confirming `gep31` fails with
  a clean 404-mapped error, never a panic) all pass.
- `cargo run -p provider-gefs --bin provider-gefs-harness` performed a full live run
  (2026-09-12 18Z) end to end: fetched and decoded all three ensemble
  identities, printed per-field metadata and physically plausible
  min/max/mean values, computed real ensemble spread at a fixed point,
  cropped to a CONUS regional subset, and rendered a geographically
  recognizable image of North America (visually confirmed: warm
  interior/cooler coastal-and-poleward gradient matching real September
  temperature patterns) to `target/provider-gefs-harness/gefs_conus_temperature_2m.png`
  on a real GPU (NVIDIA GeForce RTX 5070, Vulkan backend).
- `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D
  warnings` both pass across the whole workspace with this crate added.
