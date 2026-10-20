# ADR-0016: HRRR point soundings: Open-Meteo first, a 5-level NOMADS crop as fallback

Status: Accepted

## Context
Phase 5.2 adds an HRRR forecast sounding (skew-T plus hodograph) at a map
click. A proper skew-T needs the pressure-level column: TMP, DPT, UGRD,
VGRD and HGT on HRRR's 39 levels (1000-50 mb).

Measured 2026-10-20 (`hrrr.t16z.wrfprsf01`):
- AWS (`noaa-hrrr-bdp-pds`, what `provider-hrrr` reads): those 195 messages
  are whole-CONUS and total ~128 MB. They use GRIB2 template 5.3 (complex
  packing with spatial differencing), so one grid point can't be byte-ranged;
  the whole message must be decoded.
- NOMADS grib filter: only `filter_hrrr_2d.pl` (the `wrfsfcf` surface file)
  can crop to a subregion; there is no pressure-level filter. The surface
  file has T/Td only at 1000/925/850/700/500 mb. A 0.1° crop is ~6 KB in
  ~1.4 s, but NOMADS sends no CORS headers. NOMADS OpenDAP is retired
  (SCN 25-81).
- Open-Meteo (`api.open-meteo.com/v1/gfs?models=ncep_hrrr_conus`): all 39
  levels for one point, ~14 KB, CORS open. Its free tier was overloaded on
  the test day: "service overloaded" 503s, 429 "too many concurrent
  requests", and fresh points taking 110 s or more; a cached repeat took
  0.35 s.

## Decision
- Ask Open-Meteo first with an 8 s timeout (`apps/web/src/forecast/sounding.ts`).
- On failure, fetch the NOMADS crop of the app's own HRRR run through
  `fetchUrlBytes` (the desktop `fetch_url` command, or the dev-server relay)
  and sample the nearest grid point in Rust
  (`provider_hrrr::decode::sample_column`, exposed as `sampleHrrrColumn`).
  This column is labelled coarse, and its CAPE is HRRR's own surface CAPE,
  because a parcel lifted only to 500 mb would undercount it.
- For the full column, the parcel (LCL, CAPE) is computed in the browser.
  The surface height is the model's ground, from the hypsometric thickness
  below the first level above it, not Open-Meteo's DEM `elevation`.

## Consequences
- Open-Meteo serves its own latest HRRR run, which can differ from the run
  on the map; the fallback uses the map's run.
- Open-Meteo's free tier is for non-commercial use; RadarPro needs its paid
  plan if it is ever sold or used commercially.
- A plain static web build has no `/fetch-url` relay, so it gets no fallback
  (same limit as placefiles). The desktop app and dev server have it.
- The parcel uses plain, not virtual, temperature: CAPE reads slightly low in
  moist air (`shortcut:` comment in `surfaceParcel`).
