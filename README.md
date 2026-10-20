# RadarPro

RadarPro is an open-source professional weather radar and forecasting
workstation. It aims to provide capabilities comparable to commercial radar
software — Level II radar decoding and rendering, multi-product analysis, and
model/forecast overlays — with its own branding, UI, assets, defaults, and
algorithms.

RadarPro treats the distinction between **observations** and **forecasts** as
a hard scientific rule: model output is never presented as observed radar,
and source metadata (units, timestamps, range folding, missing values) is
preserved end to end rather than smoothed away for looks.

## Status

Working desktop (Windows, Tauri) and web workstation, through stage S11, roadmap Phases 0-4 and Phases 5.1-5.2:

- **Live NEXRAD Level II** for ~160 WSR-88D sites, from the real-time
  chunk feed (the newest scan fills in tilt by tilt within seconds; falls
  back to the archive): REF, VEL, SW, ZDR, CC, PHI, plus derived
  Storm-Relative Velocity; optional velocity dealiasing; every tilt; GPU
  rendering; probe readout; range
  rings; custom color tables; a scan loop backfilled from the archive on
  site switch; historical archive loading by date; vertical cross-sections;
  site search, nearby-site buttons, and clickable site dots.
- **MRMS national mosaic**: composite reflectivity, precip rate, 1- and
  3-hour radar-only rainfall, precipitation type, 60-minute max hail size
  (MESH), 18 dBZ echo tops, VIL, and NLDN cloud-to-ground lightning density
  (5 min).
- **GOES lightning (GLM)**: flashes from the last 15 minutes from GOES-East
  and GOES-West (each toggled on its own, both on by default), live every
  20 s, drawn as bolts coloured by age; a "lightning N mi from you"
  notification within 10 mi.
- **SPC and WPC**: convective outlooks, watches and mesoscale discussions;
  WPC excessive rainfall outlook (Days 1-3) and rain forecast (QPF).
- **Storms**: NWS storm cell tracks with TVS/mesocyclone/hail markers, local
  storm reports, GR-style placefiles (e.g. spotter networks), and
  notifications when a warning covers you or a severe storm nears you.
- **Forecast radar**: HRRR simulated composite reflectivity, hourly to
  +18 h, shown on the map past NOW on the timeline and playable as a loop;
  hours not yet published are greyed out.
- **Model forecasts** (GEFS ensemble, HRRR): temperature, dewpoint,
  humidity, wind components, gusts, sea-level pressure, cloud cover (in the
  Forecast panel).
- **Severe-weather model fields** (HRRR, hourly to +18 h, on the map past
  NOW with a colour key): surface-based and mixed-layer CAPE, 0-1 and 0-3 km
  storm-relative helicity, 0-6 km bulk shear (kt), LCL height, and the
  fixed-layer significant tornado parameter (STP).
- **HRRR point soundings**: click the map for a skew-T, hodograph, SBCAPE
  and LCL at the forecast hour shown (39 levels via Open-Meteo, or a coarse
  5-level NOAA column when Open-Meteo is slow; see ADR-0016).
- **NWS alerts**, live polygons with lifecycle tracking and hazard tags
  (tornado detection, damage threat, hail size, wind gust); archived scans
  show the warnings and special weather statements in effect at that time.
- **Rainbow Weather** tiles, nowcast, and point forecast (needs an API key).

Not yet: multi-site radar compositing (use MRMS for a multi-radar view),
model precipitation and geopotential height, MRMS rotation tracks (grid too
large for the browser as-is), and code signing / auto-update activation for
the desktop build.

Stage planning lives in `Agent Context/` (kept out of git).

## Repository layout

```
apps/
  desktop/           Tauri desktop shell (Windows)
  radar-cli/         Command-line decode/inspection tool
  web/               Workstation UI (React + MapLibre + wasm)
crates/
  radar-types/       Shared polar radar domain types
  nexrad-level2/     NEXRAD Archive II decoder
  radar-geo/         Polar/geographic geometry, SRV
  radar-render/      wgpu sweep renderer and color tables
  radar-cache/       Live Level II discovery/download/cache
  radar-web/         Browser glue for decode/render/probe/cross-section
  weather-alerts/    NWS alert parsing and lifecycle store
  forecast-core/     Gridded forecast abstraction and renderer
  provider-gefs/     GEFS provider
  provider-hrrr/     HRRR provider
  forecast-web/      Browser glue for forecasts
  mrms/              MRMS mosaic products
  mrms-web/          Browser glue for MRMS
  glm/               GOES GLM lightning flash decoder (+ wasm API)
fixtures/            Small sample data for tests
docs/adr/            Architecture Decision Records
```

## Architecture at a glance

- **Rust** owns radar decoding, core meteorological models, geospatial math,
  and any performance-sensitive processing.
- **wgpu** is the preferred GPU abstraction for radar rendering.
- Mapping lives behind an adapter; radar rendering does not depend directly
  on any specific mapping or UI framework.
- Provider-specific formats and naming stop at provider boundaries — internal
  types are provider-neutral.
- Internal time is always UTC; forecasts track initialization time, lead
  time, and valid time separately from each other.

See `Agent Context/context/GLOBAL_CONTRACT.md` for the full set of rules that
apply across every development stage.

## Building

### Rust workspace

Requires the toolchain pinned in `rust-toolchain.toml` (stable, with
`rustfmt` and `clippy`).

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

### Web app (`apps/web`)

Requires a current Node.js LTS release (Node 20+).

```sh
cd apps/web
npm install
npm run typecheck
npm test
npm run build
```

Both are checked in CI on every push and pull request to `main` — see
`.github/workflows/ci.yml`.

## License

RadarPro is licensed under the **Apache License, Version 2.0** (see
[`LICENSE`](./LICENSE)). This license is **provisional** for the current
development stage: dependencies and third-party data-source terms will be
audited before a public release. See
`Agent Context/reference/RELEASE_AND_GOVERNANCE.md` for details.

## Documentation

Full project background, staged development context, architecture notes,
and reference material for contributors (human or AI) live under
[`Agent Context/`](./Agent%20Context/). Start with
`Agent Context/CLAUDE.md` and `Agent Context/context/CURRENT_STAGE.md`.
