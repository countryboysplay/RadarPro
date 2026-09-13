# ADR-0013: Extracting `forecast-core` -- `ForecastProvider`/`ForecastGrid`/`EnsembleStatistic` shape

Status: Accepted

## Context
S07 built `provider-gefs` deliberately as a single-provider proof of
concept, explicitly *not* a general forecast framework (its own "Critical
rule"). S08 (`context/stages/S08-forecast-core-hrrr.md`) requires
extracting the genuinely generic pieces into a new `forecast-core` crate
and validating the extraction with a second, real provider (HRRR) --
matching CLAUDE.md's "avoid premature abstractions; prefer a second
concrete implementation before generalizing." This ADR records the shape
that extraction took and why, now that a second real provider exists to
generalize *from* (not just *to*).

## Decision

**Canonical types**, one crate (`forecast-core`), no provider-specific
knowledge in any of them:
- `ForecastVariable` (`variable.rs`): generalizes S07's single-variant
  `CanonicalField` to the full S08-named set (temperature_2m, dewpoint_2m,
  wind_u_10m, wind_v_10m, wind_gust, MSLP, precipitation_1h, cloud_cover,
  geopotential_height, relative_humidity) even though only
  `temperature_2m` is actually decoded by any provider as of this stage --
  the stage file explicitly asks for the named set to exist now, decode
  support to follow later, matching the "avoid premature abstraction" rule
  at the *decode* layer while still giving the *vocabulary* layer its full
  shape up front (a vocabulary is cheap and low-risk to pre-declare; decode
  wiring for a variable no provider proves out yet is the expensive,
  risky part this ADR does *not* do prematurely).
- `EnsembleStatistic` (`ensemble.rs`): generalizes S07's `EnsembleIdentity`
  (`Control`/`Member(u8)`/`Mean`) with an added `Percentile(u8)` reserved
  for a future named-percentile provider (`FORECASTING.md`'s "mean,
  median/percentiles, probabilities, spread, and members"), carried as
  `Option<EnsembleStatistic>` on a `ForecastGrid` rather than adding a
  `None`-like variant inside the enum itself -- see that module's own doc
  comment for why `Option` was chosen over an in-enum "not applicable"
  case. This is the literal mechanism that lets HRRR (deterministic, no
  ensemble at all -- confirmed empirically, every real HRRR message uses
  GRIB2 Product Definition Template 4.0, which carries no ensemble
  metadata whatsoever) avoid faking an ensemble identity it does not have,
  per the stage file's explicit instruction.
- `GridGeometry` (`grid.rs`): generalizes S07's single-shape
  `GridGeometry` (implicitly always regular lat/lon) into an enum of two
  concrete, empirically-real shapes (`RegularLatLon`, GEFS's;
  `LambertConformal`, HRRR's real Grid Definition Template 3.30 -- see
  ADR-0012). Chosen as a closed enum of concrete shapes, not a single
  generalized "any projection" abstraction (e.g. a trait object or a
  generic transform matrix) -- a Lambert grid's rows/columns are not
  constant-latitude/longitude lines the way a regular grid's are, so a
  single linear formula cannot honestly represent both, and an open-ended
  trait-object design would be exactly the kind of premature, speculative
  generality CLAUDE.md warns against with only two real concrete cases in
  hand. `ForecastGrid::subset`/`point_forecast`/`value_at` all work
  identically for either variant via the enum's own shared methods.
- `ForecastGrid` (`grid.rs`): generalizes S07's `GriddedField`, adding
  `NativeVariableMetadata` (provider-native variable/level name and unit,
  preserved per `FORECASTING.md`'s "preserve native names as metadata")
  and `provider_id` (attribution, never used to branch rendering
  behavior -- see `forecast_core::gpu`'s module doc).
- `ModelMetadata`/`ModelRun` (`model.rs`), `FieldRequest` (`request.rs`),
  `PointForecast` (`point.rs`): thin, stage-file-named canonical types;
  see their own module docs.
- `ForecastProvider` (`provider.rs`): a trait with **associated** `Run` and
  `Error` types, not a fixed `ModelRun`/shared error enum -- a provider's
  real run identity (GEFS's UTC-date-plus-one-of-four-run-hours
  `RunReference`; HRRR's UTC-date-plus-any-of-24-hourly-run-hours
  `RunReference`, a different concrete shape despite the same name) and
  real failure modes are exactly the kind of "provider-specific names and
  formats" GLOBAL_CONTRACT says "stop at provider boundaries." `Self::Run`
  converts to the canonical `ModelRun` via `run_metadata()` for anything
  that needs to *display* a run without knowing its concrete type.
  Implemented with ordinary `async fn` in the trait (stable Rust since
  1.75, this workspace's toolchain; no `async-trait` dependency needed),
  explicitly **not** required to be `dyn`-safe -- used generically
  (`fn f<P: ForecastProvider>(p: &P)`), since nothing in this stage's exit
  criteria requires runtime provider switching behind one trait object (a
  real UI provider switcher is explicitly out of scope for this stage;
  `apps/web` wiring is a separate follow-up task per this task's own
  constraints).

**The shared GPU grid renderer** (`gpu.rs`, `render.rs`,
`shaders/forecast_grid.wgsl`): moved here from `provider-gefs::gpu`/
`render` verbatim in spirit, generalized to take a `ForecastGrid`/
`GridGeometry` with zero provider-specific knowledge. The shader branches
on `projection_kind` (0 = regular lat/lon, 1 = Lambert conformal conic) --
a property of the grid's own geometry, not of which provider produced it
-- so a hypothetical third provider using either existing projection needs
no shader change at all, and one using a third projection would extend
this same branch rather than fork a new shader. This is the literal
mechanism satisfying the stage file's exit criteria ("the same... grid
renderer") and its own rule ("if HRRR requires provider-specific
conditionals throughout the UI, improve the abstraction rather than
special-casing broadly") -- see `shaders/forecast_grid.wgsl`'s own doc
comment.

**What stays in each provider crate, not generalized**: bucket/key layout,
`.idx` parsing, and GRIB2 decode wiring (parameter-category/number
cross-checks, product/grid template dispatch, ensemble-identity
extraction for GEFS, Lambert-spacing derivation for HRRR) -- explicitly
named as GEFS-specific in this task's own instructions, and confirmed
different enough in practice (2-digit vs. 3-digit forecast-hour padding,
presence/absence of an `ENS=` idx annotation, hourly vs. four-times-daily
run cadence, PDT 4.0 vs. 4.1/4.2) that generalizing them now would be
exactly the premature abstraction CLAUDE.md warns against.

## Alternatives
- **Generalize `GridGeometry` as a single origin+step formula with an
  extra "projection type" enum applied as a post-hoc transform**: rejected
  -- this would still require the same branching this ADR's actual design
  has, but hidden behind a less honest, harder-to-reason-about single
  struct shape (most fields meaningless for one variant or the other),
  rather than an enum whose variants are each fully self-describing.
- **A `dyn ForecastProvider` trait object from the start**: rejected as
  premature -- would require either boxing every `async fn` return (a real
  runtime cost and complexity increase) or an `async-trait`-style macro
  dependency, for a UI-switching capability this stage does not build or
  test (that is explicitly `apps/web`'s later, separate task).
  `ForecastProvider` can be made `dyn`-friendly later (e.g. via a
  hand-written vtable-free wrapper) once a real caller needs it, guided by
  that caller's actual requirements rather than speculation now.
- **Keep `EnsembleIdentity`-style types per-provider, sharing only the
  renderer**: rejected -- would leave `HrrrProvider::fetch_field` unable
  to share `FieldRequest`'s `ensemble` field's *type* with GEFS at all,
  defeating the point of one shared request type a caller could use
  identically for either provider.
- **Store per-cell lat/lon arrays on `ForecastGrid` instead of a closed-form
  `GridGeometry`**: rejected -- wasteful (tens of MB per HRRR field for
  data recoverable from ~10 scalar parameters) and still would not answer
  "which cell is at this world position" without a spatial index or the
  same closed-form projection this ADR already has (see ADR-0012).

## Consequences

**Benefits**: `provider-gefs` and `provider-hrrr` share one canonical
vocabulary and one GPU render pipeline, proven (not just asserted) to
genuinely generalize by S08 Part D's cross-provider test
(`provider-hrrr/tests/cross_provider_render.rs`), which fetches a real
field from each and renders both through the identical
`forecast_core::gpu::render_forecast_grid` call with no provider-specific
branching in the test's own render call. Every existing S07 correctness
check (GEFS's decode/ensemble/idx tests) still passes unchanged in
substance, only relocated to use the new shared types.

**Costs/risks**: `ForecastProvider`'s associated-type design means generic
code written against it (`fn f<P: ForecastProvider>`) must be
monomorphized per concrete provider -- there is no single function today
that can hold "a GEFS provider or an HRRR provider, chosen at runtime"
without either an enum wrapper or a `dyn`-safety redesign; this is a known,
accepted limitation given this stage's explicit scope boundary (no real UI
switcher yet).

**Portability**: adding a third provider (e.g. MRMS, mentioned as a
possible future gridded model product in `ARCHITECTURE.md`) should mostly
mean writing a new provider crate against the existing `ForecastProvider`
trait and `ForecastGrid`/`GridGeometry`/`EnsembleStatistic` types; a third
grid projection would extend `GridGeometry`'s enum and the shader's
`projection_kind` branch, following the exact pattern this stage
established for Lambert Conformal.

**Scientific implications**: none of this ADR's decisions touch decoded
physical values -- it only reorganizes *where* metadata/geometry types and
the renderer live, preserving every unit/timestamp/ensemble-identity
distinction GLOBAL_CONTRACT requires (see each type's own module doc for
its specific preservation guarantee).

## Validation
- `cargo test -p forecast-core` (37 tests), `cargo test -p provider-gefs`
  (39 unit tests + 3 live-network tests), `cargo test -p provider-hrrr`
  (29 unit tests + 2 live-network tests + 1 cross-provider render test)
  all pass -- see ADR-0012's Validation section for the cross-provider
  render test's specific assertions.
- `cargo run -p provider-gefs --bin provider-gefs-harness --release` and
  `cargo run -p provider-hrrr --bin provider-hrrr-harness --release` both
  performed full live runs end to end and rendered geographically
  recognizable CONUS images, visually inspected: GEFS's regular-lat-lon
  grid renders as a rectangle; HRRR's real Lambert Conformal grid renders
  as a visibly curved trapezoid within the same flat-plane camera --
  exactly the expected, physically-meaningful visual signature of the two
  real, different grid geometries both being handled correctly by the one
  shared renderer, not a rendering bug.
- `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D
  warnings` both pass across the whole workspace with these crates added.
