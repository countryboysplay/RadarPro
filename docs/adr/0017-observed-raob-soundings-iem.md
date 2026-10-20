# ADR-0017: Observed balloon soundings from IEM, nearest site to the click

Status: Accepted

## Context
Phase 5.4 adds observed (RAOB, 00Z/12Z) soundings in the same skew-T and
hodograph view as the HRRR sounding (ADR-0016). Measured 2026-10-20 on
KOUN 12Z:
- IEM `json/raob.py?ts=YYYYMMDDHH00&station=KOUN`: 10 KB in 0.4 s; without
  `station`, every US launch at that time in one request (43 profiles,
  ~650 KB, 0.65 s). Mandatory and significant levels: pres, hght, tmpc,
  dwpc, drct, sknt; winds and dew points are null on some levels. CORS open.
  Site locations come from `geojson/network/RAOB.geojson` (347 sites,
  185 KB, CORS open), whose `sid` matches the profile `station` one-to-one.
  A synoptic time with no launches in yet returns `{"profiles": []}`.
- University of Wyoming `wsgi/sounding` (CSV): full 1-second data, 400 KB,
  13 s, no CORS headers, so it would need the desktop `fetch_url` relay.

## Decision
- IEM only (owner's choice). One all-sites request per synoptic time,
  cached, so any click reuses it; the newest time (under 6 h old) is not
  cached because launches keep arriving.
- Same "Sounding at a point" click as the HRRR sounding; the panel has an
  HRRR | Observed switch. Observed takes the site nearest the click with a
  usable profile, at the latest 00Z/12Z at or before the archived scan (or
  now), stepping back up to 2 days past empty times.
- Missing winds on significant levels are interpolated in height; missing
  dew points leave a gap in the Td line. The parcel math is the HRRR one.

## Consequences
- Works in a plain static web build; no relay needed.
- Right after 00Z/12Z, a far site that already reported beats a near one
  still in flight (`ponytail:` comment in `fetchRaobSounding`).
- Off-hour special launches (18Z/06Z on outbreak days) are not looked for.
- IEM's levels are coarser than UWyo's 1-second data, which is fine for a
  skew-T at this size.
