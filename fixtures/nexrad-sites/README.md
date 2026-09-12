# WSR-88D Site Directory Data

Real radar site metadata (ICAO identifier, name, latitude, longitude,
antenna elevation) for the 159 WSR-88D radars, sourced directly from
NOAA/NWS's own public API — not invented, not transcribed by hand.

## Files

- `nws-radar-stations-raw.json` — the full, unmodified response from the
  official NWS API endpoint `https://api.weather.gov/radar/stations`
  (GeoJSON `FeatureCollection`, includes WSR-88D plus TDWR and a few other
  station types; retained for provenance/auditability).
- `wsr88d-sites.json` — derived: filtered to `stationType == "WSR-88D"`
  only (159 sites), reshaped to `{icao, name, lat, lon, height_m}`, sorted
  by ICAO. This is the file `crates/radar-cache`'s site directory should
  load from.

Cross-checked against NOAA/NWS's official "WSR-88D Radar List" (PDF,
`https://www.weather.gov/media/tg/wsr88d-radar-list.pdf`, 158 sites,
alphabetical by site name, lists site name/ID/owner/antenna elevation but
not lat/lon) — site IDs and antenna elevations match; the one-site count
difference is not a data-quality concern for this project (both sources
list the operational CONUS/territories/overseas WSR-88D network; the
`api.weather.gov` feed is live and authoritative for coordinates, which
the PDF does not provide at all).

Fetched 2026-09-12.
