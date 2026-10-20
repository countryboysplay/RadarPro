# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Storm chasers and spotters. They watch live radar during severe weather,
often for hours at a stretch and often overnight, making fast calls on storm
structure, motion and threat (rotation, hail, warnings) as conditions change.

## Product Purpose

RadarPro is a professional weather radar and forecasting workstation: live
NEXRAD Level II radar, the MRMS national mosaic, model forecasts, NWS alerts
and nowcasts in one tool. Success is a chaser reading what a storm is doing,
and what is coming, faster and with more confidence than they could by
switching between apps.

## Positioning

- **Free and open source:** no subscription (unlike RadarScope, GR2Analyst and
  RadarOmega); anyone can inspect and extend it.
- **Everything in one place:** live radar, MRMS, GEFS/HRRR forecasts, NWS
  alerts and Rainbow Weather nowcasts on one map, not spread across apps.
- **Strict about the data:** observations and forecasts are never presented as
  each other; units, timestamps, range folding and missing data are shown as
  they are, never smoothed for looks.
- **Runs anywhere:** a plain browser (WebGPU) as well as the Windows desktop
  app (Tauri).

## Operating Context

- Long sessions during active severe weather, frequently at night.
- Core loop: pick a radar site, step through tilts and products (REF, VEL, SRV,
  CC, ZDR, ...), probe values, scrub scan history, watch warnings update.
- Supporting work: MRMS for the multi-radar picture, model fields for
  environment and timing, archive loading to review past events, vertical
  cross-sections through storms.

## Capabilities and Constraints

- Live NEXRAD Level II for ~160 WSR-88D sites, every tilt; REF, VEL, SW, ZDR,
  CC, PHI and derived Storm-Relative Velocity; probe readout; range rings;
  custom color tables; scan-history playback; archive loading by UTC date;
  vertical cross-sections; click a site dot to switch radars.
- MRMS: composite reflectivity, precip rate, 1-h and 3-h rainfall,
  precipitation type, 60-min max hail (MESH), 18 dBZ echo tops, VIL, lightning
  density.
- GOES-East and GOES-West GLM lightning (last 15 min, separate toggles, both on
  by default) with a nearby-lightning notification.
- SPC outlooks, watches and MDs; WPC excessive rainfall and QPF; NWS storm
  tracks, storm reports and placefiles; notifications for warnings covering
  you and severe storms nearby.
- Forecasts (GEFS ensemble, HRRR): temperature, dewpoint, humidity, wind
  components, gusts, sea-level pressure, cloud cover.
- HRRR severe-weather fields on the map: SBCAPE, MLCAPE, 0-1/0-3 km SRH,
  0-6 km shear, LCL height, fixed-layer STP.
- HRRR point sounding: click the map for a skew-T, hodograph, SBCAPE and LCL
  at the forecast hour shown (39 levels from Open-Meteo; a coarse 5-level
  NOAA column when Open-Meteo is slow, see ADR-0016).
- RTMA hourly 2.5 km surface analysis (temperature, dewpoint, wind speed,
  gust) on the map as a third model, refreshed each hour. Surface fields
  read in °F and kt. Not yet: observed balloon soundings.
- NWS alert polygons with lifecycle tracking and hazard tags; archived scans
  show the warnings in effect then; Rainbow Weather tiles, nowcast
  and point forecast (needs an API key).
- Stack: React + MapLibre + Rust/wasm (wgpu) web app in `apps/web`, wrapped by
  a Tauri desktop shell.
- Live radar, Rainbow and MRMS are mutually exclusive full-frame rasters on the
  map (only one shows at a time).
- Not yet: multi-site radar compositing, model precipitation and heights, MRMS
  rotation tracks, desktop code signing/auto-update.
- Redesign constraints (confirmed by the owner): the map stays dominant and
  controls stay out of its way; dark theme; every existing feature stays
  reachable. Nothing else about the current look is binding.
- Forecast data (GEFS/HRRR model fields and Rainbow point forecasts) does not
  live in the sidebar at all (owner's call). It belongs with time, beyond the
  "now" mark, not among the radar/layer controls.
- Chase hot path (owner): product & tilt, loop/scan time, site switching. These
  must be one step away, never behind a menu. Primary screen: a 13-15" laptop
  in a moving vehicle (glare, trackpad). Red flags: hidden controls, chrome
  covering the radar.
- Lean sidebar (owner, 2026-10-20): set-once options (storms and lightning
  toggles, location alerts, placefiles, color tables, data) live in a Settings
  window opened from the gear in the rail head, floating over the map's
  top-left corner without dimming it; the sidebar keeps what a chase touches.

## Brand Commitments

- Name: RadarPro. Own branding, UI, assets, defaults and algorithms (no copied
  commercial assets or color tables).
- Desktop app icons exist in `apps/desktop/src-tauri/icons/`.

## Evidence on Hand

- Real data: live public NOAA/NWS feeds and sample fixtures in `fixtures/`.
- No users, testimonials, benchmarks or press yet; do not invent any.

## Product Principles

1. The radar is the product: every pixel of chrome must earn its place against
   map area.
2. Never blur observation and forecast; provenance and time are always visible.
3. Speed of reading under stress beats exploration: the common actions (site,
   product, tilt, time) are one step away.
4. Show the data honestly, including gaps, folding and staleness.
5. Depth stays reachable: power features live one level down, never removed.
