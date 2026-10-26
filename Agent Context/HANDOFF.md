# RadarPro handoff — 2026-10-26 (latest session on top; older notes below)

## Session of 2026-10-26

- `main` at `08ac808` + a docs commit, pushed; CI green on `08ac808`.
  Installer rebuilt and installed on this PC (old version uninstalled).
- **Live mode holds complete scans:** a real-time volume still arriving is
  shown only once the watched tilt (`preferredSweepRef`, not the shown one,
  so VCP changes don't ping-pong) has fully arrived. `useScanHistory`
  `frameHasSweep`/`liveIndex`, `completeSweepCount` (radar-web wasm) on a
  separate decode. Loop and loop export skip frames lacking the tilt.
  Settings → Data & app "show a scan while it is still arriving" turns it off
  (`radarpro.showArrivingScans`). Verified live on KAMA.
- **Rotation and hail tracks (Phase 9 partly):** MRMS 0-2 km rotation track
  60 min and 4 h, MESH 4 h. Wide grids 2x2 max-pooled to fit 8192.
- Fixes: cross-section Cancel/Clear button and Escape; archive load jumps to
  the chosen time; moment lists from decode-time `sweepMomentCodes`
  instead of engine queries during render.
- Open: Rainbow untested in the desktop app; review items left: one-time
  check of pooled geometry, double decode for `completeSweepCount`.
  Field-feature ideas offered: MRMS following playback, 3-6 km rotation,
  contours.
- `npx tauri build` leaves a line-ending-only change in
  `apps/desktop/src-tauri/Cargo.toml`; `git restore` it.

# Eighth-session handoff — 2026-10-20

## Where things stand

- Branch `main` at `a6eb391`, pushed (github countryboysplay/RadarPro).
  **CI green** on all three eighth-session pushes (`7fd8ea9`, `c36161b`, `a6eb391`).
- **The plan:** `Agent Context/PLAN.md`. Phases 0-2 done; **Phase 3 in
  progress** (3.1, 3.2, 3.3, 3.7, 3.8 done; 3.6 and most of 3.4 were already
  done). See "Next step" below.
- **Ninth session (2026-10-20): 3.7 done.** In archive mode the Warnings
  list and map show the warnings, advisories and SPS in effect at the scan's
  minute (IEM `geojson/sbw.geojson?ts=` and `geojson/sps.geojson?valid=`,
  both CORS-open, cached via `spc/spc.ts` `getJson`), sorted by severity,
  with tornado/damage/hail/wind tags in the headline. Selecting one opens
  the NWS product text (IEM `nwstext`, reusing `MdText`). Live alerts still
  drive GPS notifications. Code: `apps/web/src/alerts/archived.ts`, wiring in
  `App.tsx` (`inArchive`, `shownAlerts`). Verified in Chrome on KTLX
  2024-05-06 23:00Z. Limits: IEM has no CAP certainty/urgency/areas, so
  those show "Unknown"/"—"; severity is mapped from VTEC (TO.W Extreme,
  other warnings Severe, advisories Minor, SPS Moderate). Desktop app not
  re-tested.
- **3.4 done (same session):** NWS hazard tags (tornadoDetection,
  waterspoutDetection, tornado/thunderstorm/flash-flood damage threat,
  maxHailSize, maxWindGust) parsed in `weather-alerts` (`parse.rs`
  `TAG_KEYS`, `Alert::tags`, emitted as `tags: [[key, value]]`), formatted by
  `formatTags` in `alerts/types.ts`, shown in the Warnings list rows and the
  detail panel; archived warnings map IEM's tag columns to the same keys.
  No live tagged warnings on test day; checked via the real wasm parser in
  the page and on the 2024-05-06 replay. Crates use rustfmt (CI checks it);
  only `apps/` must not be formatted.
- **3.5 done (same session):** WPC Excessive Rainfall Outlook Days 1-3 and
  QPF (Day 1, 2, 3, Days 1-3) added to the SPC "Outlook" picker (one outlook
  at a time; live only). Source: NOAA ArcGIS map services
  `mapservices.weather.noaa.gov/vector/rest/services/{hazards/wpc_precip_hazards,precip/wpc_qpf}`
  (CORS-open, `f=geojson`, simplified with `maxAllowableOffset=0.02`:
  QPF 0.5-1.2 MB instead of 7-16 MB). Code: `spc/wpc.ts`. QPF bands cut the
  next band out of each fill (even-odd) so stacked fills don't go opaque;
  hover is on the contour lines. WPC polls hourly. ERO colours approximate
  WPC's; QPF uses WPC's scale.
- **3.9 done (same session), Phase 3 complete:** MRMS layer gains `qpe_3h`
  (RadarOnly_QPE_03H) and `precip_type` (PrecipFlag, categorical palette
  with flat +-0.5 bands, colour key in the panel); `qpe_1h` switched from
  MultiSensor Pass 2 (~1 h late) to RadarOnly_QPE_01H (minutes). PrecipFlag
  codes checked on 5 archived events (snow, hurricane, hail, Beryl): only
  0/1/3/6/7/10/91/96 and -3 appear, all handled. `mrms-harness
  --decode-only` gives live stats without the GPU (the GPU render segfaults
  on this machine). Units stay mm, as the other MRMS products.
- **Phase 4 done (same session): GOES GLM lightning.** New crate
  `crates/glm` (decodes LCFA netCDF4 with pure-Rust `hdf5-pure =0.47.0`,
  MIT/Apache, only adds `byteorder`; shares flate2 1.1.5; builds for wasm;
  fixture `fixtures/glm/`). Web: `apps/web/src/lightning/glm.ts` polls
  `noaa-goes19` (CORS-open) every 20 s with `start-after` the last key seen
  (1 listing + 1 file per poll; ~45 files, ~11 MB, on first load; ~45 MB/h
  steady), keeps North America (lon -135..-55, lat 15..60), last 15 min.
  Drawn as the MRMS bolt (`BOLT` exported from `mrms/useMrmsOverlay.ts`, owner
  asked to match) in the MRMS colours by age: white <1 min, yellow <5,
  orange <10, red <15. Toggle in "Storm tracks", on by default. Hidden on
  archived scans but still polled there so the alert works: "Lightning N
  mi from you" when a flash in the last 5 min is within 10 mi, at most every
  15 min, using the notify/sound switches. Requests time out after 20 s;
  on failure ages keep advancing. Limits: GOES-East only (West Coast seen
  at a slant); not tested on a very active day (20k bolts) for draw speed;
  desktop app not re-tested.
- **Uncommitted, left alone:** seven RadarScope screenshots in the repo root
  (`Screenshot 2026-10-20 *.png`); ask before committing or deleting them.
  `apps/desktop/src-tauri/Cargo.toml`, `src/rainbow.rs` and
  `src/rainbow_weather.rs` show line-ending-only changes; ignore them.
- The owner approves each commit and push. Ask before each.
- Checks at handoff: web typecheck clean, `npm test` 43 pass, lint 0 errors
  (same 2 old warnings); desktop crate clippy `-D warnings` clean.
- **Formatting: never run prettier or `cargo fmt` under `apps/`.** Neither is
  the project's formatter; prettier rewraps whole files to 80 columns and
  `cargo fmt` rewrote untouched desktop files (both happened once this
  session and were undone). Root-workspace crates still use `cargo fmt`
  (CI checks it).
- **Shell gotcha:** a bash heredoc feeding `python -` collapses `\n`
  escapes, so TS string literals with `\n` came out as real newlines. Write
  edit scripts with the Write tool and run them with `python -I <file>`.
- A Vite dev server (from an earlier session) serves this repo on **port
  5173**. Desktop: from `apps/desktop`, run
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 npx tauri dev --config '{"build":{"beforeDevCommand":""}}'`
  and drive the window over CDP (`http://localhost:9222/json`; a small Node
  script using the global `WebSocket` + `Runtime.evaluate` worked, see
  "Eighth session").

## Working with the owner (new this session)

- **Ask one question at a time, then stop and wait for the answer.** Present
  decisions as multiple choice with a recommendation. (Saved to memory.)
- **Before every commit, run a Ponytail review (`/ponytail:ponytail-review`)
  automatically**, fix what it finds (or say what was left and why), then
  ask to commit. (Saved to memory; also in PLAN.md.)
- **Every paid RadarScope Pro feature must be in the app before it is
  finished.** The checklist with the phase for each is "RadarScope Pro
  parity" in PLAN.md.
- Asked "why can't you use the app yourself?": Claude in Chrome drives the
  web app (same code and renderer as the desktop app). The desktop WebView2
  window can be driven by launching it with
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` and
  scripting it over CDP; not done yet.

## Eighth session (2026-10-20)

### Commits
- `7fd8ea9` **Placefiles (Phase 2).** `apps/web/src/placefiles/`:
  `placefile.ts` parser (Title, Refresh/RefreshSeconds, Threshold, Color,
  Font, IconFile, Icon, Text, Place, Line, Polygon, Object; Triangles and
  Image skipped; polygons use the first vertex colour), `draw.ts` (draws on
  the alerts canvas between warnings and storm tracks; Threshold compared
  with half the view width in miles; hover hit test), `usePlacefiles.ts`
  (list in localStorage `radarpro.placefiles`, refresh as the file asks,
  30 s floor, keeps last good copy on error; icon sheets with no alpha get
  black made transparent). Fetch: desktop `fetch_url` command
  (`nws_products.rs`, http/https only, 8 MB cap, 20 s timeout), browser
  `/fetch-url?url=` middleware in `vite.config.ts` (dev and preview only).
  Hover text shows in the probe tip (`.map-probe-place`). Sidebar
  "Placefiles" (`ui/PlacefilesPanel.tsx`) with one-click Spotter Network
  positions (`feeds/gr.txt`) and reports (`feeds/reports.txt`).
  **IEM has no storm-report placefile** (their LSRs are CSV/KML/GeoJSON
  only), so SN reports replaced the planned IEM suggestion.
  Verified in Chrome and in the desktop app over CDP (fetch_url, drawing,
  hover, auto-refresh). The owner has a Spotter Network login; the public
  feeds need none, and Claude must not enter the password anywhere.
- `c36161b` **SPC layers (3.1-3.3).** `src/spc/spc.ts` + `useSpc.ts`, built
  as `PlacefileLayer`s so drawing and hover reuse `placefiles/draw.ts`
  (hover now also works inside polygons and lists every area under the
  cursor). Day 1 outlook from SPC `day1otlk_{cat,torn,wind,hail}.lyr.geojson`
  (its own `fill`/`stroke`; "SIGN" drawn as a white outline), off by
  default, live only. Watches (IEM `api/1/spc_watch_outline.geojson`) and
  MDs (IEM `api/1/nws/spc_mcd.geojson`), on by default, drawn under
  warnings (`drawAlerts` takes a `below` layer list); archive mode passes
  `valid=` (10-minute steps, session cache, archived requests not
  cancelled). MD full text from IEM `api/1/nwstext/{product_id}` in the
  right panel. Sidebar "SPC" (`ui/SpcPanel.tsx`). All these endpoints send
  `Access-Control-Allow-Origin: *`.
- `a6eb391` **Storm reports (3.8).** `src/spc/lsr.ts`: IEM
  `geojson/lsr.geojson` (`hours=3` live, `sts`/`ets` 3 h before an archived
  scan, cached). Only tornado, funnel, hail, tstm wind damage/gust and
  flash flood; lettered markers in SPC colours from a generated
  OffscreenCanvas icon sheet; hover has type, size, time, place, source,
  remark. Drawn over placefiles (spotters sit on their own reports). Toggle
  in "Storm tracks", on by default (`radarpro.showReports`).

### Verified
- Live: SPC general-thunder outlook draws and hovers.
- Archive 2024-05-06 23Z (KTLX, then KVWX): 4 tornado watches, 4-5 MDs
  (list follows the loop), MD 659 text, watch hover, ~115 storm reports
  with correct hover text.
- Not seen yet with live data: watches, MDs, storm reports in view (quiet
  day).

### Known limits
- Placefiles are live only, so today's spotters show over an archived replay
  (GR does the same).
- Replaying the last hour or two: storm reports filed later stay missing
  until restart (session cache; `shortcut:` comment in `spc.ts`).
- Outlook not shown in archive mode (IEM `spc_outlook` could supply it
  later, without SPC's colours).

## Seventh session (2026-10-20)

### Commits
- `a3fd627` `cargo fmt` (CI had failed on formatting since `b401bca`).
- `c12fb14` REF range-folded gates are invisible (`range_folded_color`
  alpha 0, still distinct from `missing_color`); the legend hides the
  range-folded swatch when it is transparent. VEL etc. still show it.
- `dab085f` The owner's `DESIGN.md`, `.impeccable/design.json` + config,
  shorter forecast label in `UnifiedTimeline.tsx`, two RadarScope shots.
- `95467f1` Product buttons in a fixed order (`PRODUCT_ORDER` in `App.tsx`:
  REF VEL SRV SW ZDR CC KDP PHI); a product the scan lacks is disabled, not
  removed. They used to reorder between tilts and while a live scan filled.
- `1fb9267` Storm labels clear of every track, meso ring, TVS, dot and
  earlier label: `storms/labelPlacement.ts` `placeLabel` tries 8 directions
  at 14 then 26 px, preferred side first, else fewest overlaps. Labels draw
  last. 3 tests in `core.test.ts`. Checked on the Owensboro archive.
- `751debf` VEL = RadarScope's SRV ramp (their bars are identical); CC
  sampled from RadarScope's bar over 0..1.05 (rain lands at 0.98-1.00).
- `b7720fd` ZDR -8..8 dB (black < -4, grey -4..0; verified: same KHDX storm,
  our median 0.59 dB vs RadarScope 0.62); SW 0..50 kt = 0..25.72 m/s
  (inferred: 75th/90th percentile matches both give 25.8 m/s).
- `f9fa5a1` **KDP**, a new derived `MomentKind::SpecificDifferentialPhase`
  ("KDP"), computed right after decode by `radar_geo::add_kdp`
  (`crates/radar-geo/src/kdp.rs`): PHI kept where CC >= 0.85, unwrapped,
  5-gate median, least-squares slope over 9 gates at 40+ dBZ else 25, only
  at 20+ dBZ; clear-air radials get no KDP moment. Fixture test
  `crates/radar-web/tests/kdp_fixture.rs`: KTLX rain cores median +0.83
  deg/km, KFTG (Denver hail) cores ~0. Costs ~5-8% of decode time.
  Colors from RadarScope's bar over **-2..10 deg/km, inferred** (a light
  storm, KDP ~0, sits where 0 falls). Checked 2026-10-20 on KVWX
  2025-03-15 10:01Z (0.68 deg): all echo median 0.07, REF>=55 median 2.56,
  p99 7.6, so the scale fits our values; not compared to RadarScope itself.

### How the colors were matched (reuse for any future table)
- Sample the color bar at y=1023 of the owner's 1920x1080 RadarScope
  screenshots with PIL (`python -I`); the small purple block left of each
  bar is that product's range-folded color.
- Bars have no ticks. Find the scale by loading the **same site and scan**
  in RadarPro (live loop, step back to the matching time), sampling our
  values over a storm by dispatching `mousemove` on `.maplibregl-canvas`
  and reading `.map-probe-tip` (scale screenshot coords by
  `innerWidth/1568`; keep under ~250 probes per call or CDP times out), and
  matching percentiles to the bar positions of RadarScope's pixels.
- Every table has two identical copies: `crates/radar-render/color_tables/`
  and `apps/web/src/data/color_tables/`.

### Checked, not changed
- Loop playback after the sharpness change (live KVWX, Chrome): frames stay
  aligned and sharp while panning and zooming mid-loop. Cross-section and
  probe-vs-pixel during playback not checked.
- REF range-folded gates at ~50 km on KVWX's 0.4 deg tilt: flagged by the
  radar itself; why at that short range was not investigated.

### RadarScope Pro comparison
Moved to PLAN.md, "RadarScope Pro parity". The owner made every paid
feature a requirement, so items once "not planned" (multi-pane, RTMA, GOES
satellite, METARs, archived bulletins, shear/hail contours, satellite
basemap, WPC outlooks, detailed roads, settings sync) now have phases 3, 5,
8, 9, 10 and 11. Release moved to Phase 11.

## Next step

Phase 3, remaining (PLAN.md). Recommended order:
1. ~~3.7 warnings and SPS on archived scans~~ done (ninth session).
2. ~~3.4 warning tags~~ done (ninth session). Was: the detail panel already shows full text; add
   tornado detection, max hail size, max wind gust, damage threat. The
   Rust `weather-alerts` model drops NWS `parameters`; carry them through
   `json.rs` and `alerts/types.ts`.
3. ~~3.5 WPC~~ done (ninth session). Was: Excessive Rainfall Outlook Day 1-3 and QPF as layers.
4. ~~3.9 precipitation products~~ done (ninth session).
Then Phase 4 (GOES GLM lightning).

## Sixth session (2026-10-20, earlier)

Driven by the owner comparing against RadarScope ("this should be an
extremely professional looking tool"; "I don't want to partially fix
things").

### Storm tracks look (`2335baf`)
- One casing pass under each whole track (casing per tick had cut the line
  into segments). Past track: bare 2.5px Paper Ink line. Forecast: 3.5px
  amber with a cross-bar every 15 min and an arrowhead.
- Cell position is a bullseye (7px disc, dark centre). A cell NWS has not
  tracked yet (no forecast) is a hollow ring plus "NEW" in Ink Muted.
- Labels sit on the upper side of the cell's forecast track. Known limit:
  a label can still land on a *neighbour's* track where cells bunch up.

### Sharp radar at any zoom (`3d2aa03`)
- Was: one fixed 900x900 canvas over the full 460 km range (~1 km/px),
  CSS-stretched, so zooming blurred gates and averaged away red cores.
- Now: the radar canvas covers the map at device resolution, and
  `RadarWebRenderer.setView(width, height, siteLat, siteLon, west, north,
  east, south)` (new wasm export) redraws for exactly the visible area on
  every pan/zoom/resize (`MapView` `onRadarView` -> `useRadarRenderer`
  `setView`). `renderedRangeKm` and `RADAR_CANVAS_SIZE` were removed.
- Shader (`radar_sweep.wgsl`): world space is site-relative Web Mercator
  scaled to km; per pixel it unprojects to lat/lon, takes great-circle
  range and bearing, then slant range = ground / cos(elev), matching
  `radar_geo::lookup::cursor_to_polar` (the gate probe). Helpers:
  `radar_render::camera::{mercator_view, full_range_view, mercator_y}`.
  With site lat 0 (native harness, GPU test) it reduces to the old flat
  plane.
- Map rotation and pitch are disabled (radar is drawn north-up).
- The shader now uses each color table's own `range_folded_color` (was
  hard-coded magenta; new `GpuUniforms.range_folded_color`).
- **Not verified:** loop playback after this change (the browser extension
  dropped before it could be checked). Check that first.

### RadarScope colors (`b401bca`)
- Sampled from the owner's RadarScope screenshots.
- REF: -32..95 dBZ, hard band breaks every 10 dBZ from 40 (orange, red 50,
  pink 60, purple 70, cyan 80, brown 90). The insect fade was kept:
  transparent below 5 dBZ, see-through to ~23. RadarScope shows everything
  down to -32; whether to match that is an open owner decision (PLAN.md).
  MRMS reflectivity shares this table. The REF parity test against
  `palette.rs`'s placeholder ramp was replaced by a band test; `palette.rs`
  is now only the harness/GPU-test placeholder.
- SRV: navy -> white -> dark red over +/-32 m/s, range folding dim purple
  (64,0,64). VEL still has the old grey-centred ramp (PLAN.md Phase 0).
- The web app's vendored copies in `src/data/color_tables/` were synced.

### Chase safety (`c8c63d9`), PLAN.md Phase 1
- Hazard alerts use the 3 radars nearest the GPS position (within 250 km),
  whichever site is on screen: `useNearbyStormTracks`, sharing
  `fetchStormTracks` with the on-screen hook.
- `stormAlerts` (`storms/nearbyHazards.ts`): one alert per storm (TVS, mesos
  within 10 km, 1"+ hail grouped), using the cell's NWS forecast track 30
  min ahead: "arrives in ~N min, passing X mi DIR of you". Re-alerts only
  when the storm's rank rises; a storm seen by two radars is reported once.
- Watches: `useAlertIdsAtPoint` asks `api.weather.gov/alerts/active?point=`
  (rounded to 0.01 deg) and `coveringAlerts` counts those ids. Of zone-only
  alerts, only watches/warnings notify; statements are listed, not notified.
- Archive playback: storm tracks and mesos for the scan on screen come from
  Unidata's Level 3 bucket (`unidata-nexrad-level3`, CORS-open, keys like
  `VWX_NST_2025_03_15_10_01_50`). NTV/NHI are not archived (no TVS/hail in
  archive). The live loop already crossed midnight; archive loads now do too.
- Verified in Chrome with a simulated GPS position and a recording stand-in
  for `Notification`: a live FL Flood Warning notified; a live zone-only GA
  Special Weather Statement was listed but did not notify; archive
  2025-03-15 05:13Z showed archived tracks and 8 mesos. Live NST/NTV/NHI/NMD
  from 5 sites parse. **Not yet seen:** a real tornado/severe warning or a
  live TVS/meso alert, in the desktop app.

## Fifth session (2026-10-19)

**All work from prior sessions tested and verified working in both browser
and desktop app. No new features; focus was testing, iteration on UX, and
preparing for handoff.**

- **Desktop app verification:** the owner ran `tauri dev` and tested:
  - Location permission prompt (Windows)
  - NWS storm track fetch and parsing
  - SRV with per-storm automatic motion from the NWS forecast track
  - Test notification (Windows toast) and sound
  - Cursor label with range/bearing
  - TVS, mesocyclone and hail markers
  - Hazard alerts (TVS, meso, hail near you)
  - All reported working.
- **Iteration items from fourth session:**
  - Fixed stalled storm-track detection: if NST is over 20 minutes old it
    says "NWS feed stalled, last 10/16 22:15Z" instead of appearing
    current. TVS/hail/meso from a stalled feed are not drawn.
  - Warning list now sorts by distance to the selected radar first, then
    severity. Near-warnings moved to the top.
  - Clicking a warning on the map now fits the polygon into view if
    off-screen.
  - Added cursor label (`.map-probe-tip`) showing value, range and bearing
    next to the mouse.
  - App now opens on the nearest radar from GPS on first startup, unless
    you've already picked one.
  - My location, notify and sound start **on** and remembered across reloads.
  - Warning details moved to the right panel (510 px wide) so they no
    longer push the sidebar around.
  - Cross-section moved to a larger panel at the top of the right side,
    above any open warning detail.
  - Site-switch mid-download bug fixed: clean transitions between radars,
    no old images appearing at new locations.
- **Three new fixtures and test data:** KVWX 10:01Z on 2025-03-15, the
  Owensboro KY hail outbreak with TVS, mesocyclone and hail data. Used
  for parser tests and browser visual checks.

## Built in the fourth session

### Storm tracking (roadmap item complete)

- **Source:** NWS tgftp, latest issue only:
  `https://tgftp.nws.noaa.gov/SL.us008001/DF.of/DC.radar/DS.<product>/SI.<icao lowercase>/sn.last`.
  Products: `58sti` (NST tracks), `61tvs` (NTV), `p59hi` (NHI hail;
  note the `p`), `141md` (NMD mesocyclone). The `unidata-nexrad-level3`
  S3 bucket stalled around 2026-10-04 and is unusable for live data.
- **tgftp sends no CORS headers.** Desktop: Tauri command
  `nws_radar_product(product, site)` in
  `apps/desktop/src-tauri/src/nws_products.rs` (allow-lists the four
  products, validates a lowercase 4-letter site, returns raw bytes via
  `tauri::ipc::Response`). Browser: a `/tgftp` proxy in
  `apps/web/vite.config.ts` (dev and preview).
- **Parser:** `apps/web/src/storms/nst.ts`. A shared `readProduct` finds
  the message header (product code + -1 divider within the first 64
  bytes, so the WMO/AWIPS header is fine), reads radar lat/lon and volume
  time, and walks symbology packets. Packet I/J are 1/4 km, +I east,
  **+J north** (checked against the text tables).
  - NST: packet 15 = cell ID, 23 = past track, 24 = forecast track (each
    holds a linked vector, packet 6). Average motion from the text
    ("AVG SPEED ... AVG DIRECTION ..."); it is a *from* direction.
    Per-cell motion is computed from the forecast track's first and last
    points.
  - NTV: packet 12 = TVS, 26 = elevated TVS, each followed by packet 15
    with its storm ID.
  - NHI: binary packet 19 stores hail size in **whole inches**, so sizes
    come from the text table, matched to NST cells by ID. Table rows have
    no newlines: each starts with a 2-byte length 0x0050, which reads as
    "P". "<0.50" reads as 0; UNKNOWN rows are skipped.
  - NMD: packet 20 point features, type 9 (rank 5+, low base) and 10
    (rank 5+, base aloft) kept, 11 (weak) dropped; radius in 1/4 km; the
    circulation ID is in the following text packet 8.
- **Hook:** `storms/useStormTracks.ts` polls every 2 minutes. NST failing
  is an error; TVS/hail/meso failing just omits them. Anything older than
  20 minutes is treated as stalled (KTLX's NST was stuck at 2026-10-16).
- **Map** (`drawStormTracks` in `MapView.tsx`, on the alerts canvas):
  past track Paper Ink, NWS forecast track Forecast Amber, cell dot + ID.
  Hazards in Danger Red (#ef6a5a): hail size after the ID when 1"+,
  mesocyclone circles (solid low-level, dashed aloft), TVS point-down
  triangle (hollow = elevated). The SRV storm gets a steel-blue ring.
- **Storm tracks heading** leads with hazards: "1 TVS · 3 meso · hail to
  3.00″ · 28 cells · 10:01Z". On by default.
- **SRV storm motion is automatic:** it follows the on-screen storm
  nearest the map center (per-cell motion), else the NWS average. Typing
  a value switches to manual; "Back to automatic" undoes it. Clicking a
  storm dot centers the map on it, which selects it. Under SRV the cells
  are fetched and drawn even if the Storm tracks toggle is off.
- **Hazard alerts** (`storms/nearbyHazards.ts`): a TVS, mesocyclone or
  1"+ hail within 25 km (~15 mi) of the GPS position notifies through the
  same "Notify me" flag and sound as a covering warning, once per hazard
  key (`tvs:L8`, `meso:677`, `hail:L8`) while it stays near. TVS and
  low-level meso alerts are sticky.
- **Test data:** `fixtures/nexrad-level3/`: `KBYX_NST_20261019_2005`
  (live, Key West) and the KVWX 2025-03-15 10:01Z set (NST, NTV, NHI,
  NMD): the Owensboro, KY severe hail event the owner pointed to (L8
  2.75" with a TVS, F1 3.00", A0 2.25"). Historical NTV/NHI after 2021
  exist only in NOAA's Google Cloud archive
  (`gcp-public-data-nexrad-l3/YYYY/MM/DD/KXXX/*.tar.gz`, one tar per site
  per day, hundreds of MB; stream it and extract with
  `tar -xz --wildcards '*NHI*' ...`).
- **Visual check without live storms:** in a Chrome tab, override
  `window.fetch` to answer `/tgftp/...DS.<product>/...` with the fixture
  files (served by a small CORS http.server; **port 8765 is taken by the
  owner's own "CYD layout" server, so use another**), set `Date.now` to
  the fixture time, stub `Geolocation.prototype.watchPosition` and
  `window.Notification`, then toggle Storm tracks and My location off and
  on.

### Products and tilts

- Product buttons list every product in the volume. Picking one the
  current tilt lacks (VEL/SRV on a split-cut reflectivity tilt) jumps to
  the nearest tilt in elevation that has it (`changeMoment` in App.tsx).

### Chase-speed usability pass (owner: "time is everything")

- Warnings near the selected radar are listed first (each group still
  severity-sorted).
- Selecting a warning fits the map to its polygon when it is off screen.
- Probe value, range and bearing follow the cursor (`.map-probe-tip`);
  the sidebar readout stays too (`probeText` in `ProbePanel.tsx`).
- Last radar site is remembered in the browser too (`localStorage`
  `radarpro.defaultSiteIcao`; desktop still uses its store). The first GPS
  fix of a session switches to the nearest radar unless the user already
  picked one.
- My location, Notify and Sound are **on by default** and remembered
  (`useRememberedFlag`, `localStorage` keys `radarpro.showMyLocation`,
  `radarpro.notifyCovering`, `radarpro.notifySound`). In a browser the
  notification permission is requested on the first click anywhere.
- Warning detail and the cross-section moved to the right panel (now
  510 px wide) so the rail no longer shifts; `CrossSectionPanel` was split
  into controls (rail) and `CrossSectionSlice` (panel, shown first).

### Bug fixed: site switch mid-download (`0e08b5d`)

- A backfill or poller download that finished just after a site switch
  was added to the new site's loop: the old radar's image showed at the
  new site and Product/Tilt stuck on "Waiting for a scan". Both
  `useScanHistory` loops and the poller's `emit` now drop it. It was
  pre-existing; the GPS auto-switch made it common.

## Built in the third session

### My location and "a warning covers you" (`8cdf68e`, `8a5797a`)

- **Code:** `apps/web/src/location/useMyLocation.ts` (new), `MapView.tsx`,
  `App.tsx`, `platform/desktop.ts` (`notifyDesktop`), and a
  `coveringAlerts` test in `src/__tests__/core.test.ts`.
- **`useMyLocation(enabled)`:** `navigator.geolocation.watchPosition`
  (high accuracy, maximumAge 5 s). Heading comes from GPS when it reports
  one with speed. Otherwise it is the bearing of travel from the last fix
  at least 25 m away, since Windows location rarely reports a heading.
- **Map** (`drawMyLocation` in `MapView.tsx`): drawn on the alerts canvas,
  so it sits above warning polygons. A Paper Ink (#ecebe4) dot on the
  course casing, an arrow ahead of it along the heading (rotation-aware),
  and a faint accuracy circle once it is wider than 12 px. Neutral ink on
  purpose: purple means "the user placed this", steel blue means
  "selected".
- **Center on me:** a button bumps `centerOnMeRequest`; MapView `easeTo`s
  to the current fix, keeping the zoom.
- **`coveringAlerts(point, active, prevKeys)`:** alerts whose polygon
  contains the point, plus the "fresh" ones not covering it last time.
  App keeps the notified keys in a ref, so each alert notifies once while
  it keeps covering you. Turning the toggle on reports alerts already
  covering you.
- **Notifications** (`showNotification` in `App.tsx`):
  - In the desktop app: `notifyDesktop()` →
    `tauri-plugin-notification` native toast.
  - In a browser: the web `Notification` API, with the alert key as the
    tag, `requireInteraction` for Severe and Extreme, and click selects
    the alert.
  - Ticking "Notify me" in a browser awaits
    `Notification.requestPermission()` before turning on. That fixed a bug
    the owner caught: the first covering alert used to play the sound and
    never notify.
  - **"Send a test notification"** button (shown once notify is on) uses
    the same path as a real alert.
- **Sound:** `playAlertTone()`, two Web Audio tones (880 Hz then 660 Hz).
  No asset.
- **UI:** a "My location" sidebar category after Warnings: show location,
  Center on me, notify, sound, test button, a blocked-notifications note
  (browser only), and the list of alerts covering you. The summary reads
  "off", "locating…", the error, "no alerts here", or "inside N alerts".
  All toggles are off by default and reset on reload.
- **Verified:**
  - Chrome, with a stubbed position inside a live NWS polygon: "inside 1
    alert", one notification and no repeats, the dot and arrow drawn.
  - Chrome, with the owner's real position: "no alerts here", Center on me
    panned about 950 km, sound heard.
  - Installed desktop app: location dot shows (WebView2 needed no extra
    permission setup), and the test notification appeared as a Windows
    toast.
- **Chrome toasts don't show on this PC:** Windows has notifications off
  for Chrome (`HKCU\...\Notifications\Settings\Chrome` `Enabled=0`). The
  owner chose to rely on the desktop app instead.
- **Desktop limits:** clicking a desktop toast doesn't select the alert
  (the plugin's click events are mobile-only; a `shortcut:` comment in
  `desktop.ts`). There is no `requireInteraction` on desktop.

### Map snap-back fix (`2ecc1e7`)

- MapView's overlay-sync effect also called `easeTo(site)` and re-ran
  whenever `sweepRangeKm` changed. That range differs by product, tilt and
  live scan, so switching REF to VEL, changing tilt, or a new scan undid
  the user's pan and Center on me.
- Recentering now has its own effect keyed on `[site.lon, site.lat]`.
- Verified in Chrome: panned away, then REF → 1.3° → VEL → 3.9° kept the
  view and the radar image resized correctly; switching to KINX still
  recenters. Live scan rollover and loop playback while panned were not
  watched, but they go through the same change.

### Desktop notification plugin: version gotcha

- `tauri build` refuses mismatched Tauri JS and Rust versions (same
  major.minor). `npm install @tauri-apps/plugin-notification` pulled 2.7
  and bumped `@tauri-apps/api` to 2.12, against crates `tauri` 2.11 and
  `tauri-plugin-notification` 2.4. Fixed by restoring the lockfiles and
  pinning `@tauri-apps/plugin-notification@~2.4.0` in both `apps/web` and
  `apps/desktop`.
- **When adding any Tauri plugin:** pin its npm package to the crate's
  minor, and check `npm ls @tauri-apps/api` stays 2.11.x.
- The capability grants only `notification:allow-notify`,
  `allow-is-permission-granted` and `allow-request-permission`.

### Claude Code setup cleanup (`/doctor`, user scope, not in the repo)

- Skills `graphify` and `ui-ux-pro-max` were set `"off"` in
  `~/.claude/settings.json` `skillOverrides` (backup at
  `~/.claude/settings.json.bak-doctor`). Two abandoned
  `~/.claude/plugins/cache/temp_git_*` folders were deleted.
- The user-scope MCP server `nuclear` is unused; the owner may run
  `/mcp disable nuclear` themselves.
- The 74 agents removed in `3e7cee7` were never called in the 6 surviving
  RadarPro logs. The impeccable agents actually used live in
  `~/.claude/agents`. Restore any with `git checkout 8a5797a --
  .claude/agents/<file>`.

## Open items (fourth session), updated in the sixth

- ~~Hazard alerts only check the selected radar~~: fixed (`c8c63d9`).
- ~~Hazard alerts use distance only~~ and ~~one complex raises several
  alerts~~: fixed (`c8c63d9`).
- ~~Storm tracks show *now* in archive playback~~: fixed (`c8c63d9`).
- Live TVS/meso detections have still never been seen through the desktop
  app.
- ~~Where cells bunch up, labels overlap neighbours' tracks/circles~~: fixed
  (`1fb9267`).
- The right panel (510 px) can overlap the forecast drawer on small
  screens; not tested.

## Open items (third session)

- ~~Zone-only alerts (most watches) never trigger "covers you"~~: fixed
  in the sixth session via the NWS point query.
- A real warning covering the owner was never tested end to end; only the
  test button and the stubbed-position run were.

## Second session, part 2 (in `7eb805e`)

### 1. Softer REF low end (fixes "why is that green so strong")

(Superseded in the sixth session: the REF table is now RadarScope's, with
the same transparent-below-5 fade; the JSON no longer equals `palette.rs`,
which is only the harness placeholder now.)

- **Problem:** clear-air echo (insects, about 5-20 dBZ) painted a solid
  bright-green disc around the radar (seen on KVWX).
- **New stops:**
  - 5.01 = `[60,80,60,40]` (dim grey, see-through).
  - 15 = `[50,120,50,120]`.
  - Full green only at 25 dBZ.
  - Stops from 25 up are unchanged.
- **Three copies, kept identical:** the two JSON files and
  `palette.rs` `default_ref_palette_stops`. A test asserts the JSON equals
  that list.
- MRMS reflectivity reuses this table.

### 2. "Hide non-weather echo" filter (owner asked for it)

- **Why it exists:** the owner asked "if it isn't rain why show it at
  all?". The answer given:
  - Weak echo carries boundaries (gust fronts, outflow, drylines).
  - Light snow and drizzle are just as weak, so a dBZ cutoff can't
    separate them.
  - PRODUCT.md says don't remove data.
  - So it's an optional dual-pol filter, **off by default**.
- **Code:** `radar_geo::hide_non_weather_reflectivity`
  (`crates/radar-geo/src/non_weather.rs`, 3 unit tests).
- **Rule:** hide a REF gate below 35 dBZ when either:
  - CC < 0.80, or
  - REF < 30 dBZ and ZDR > 4 dB (insects).
- **Always kept:**
  - Gates at 35 dBZ and up (hail, tornado debris).
  - Gates with no CC reading.
  - Big drops at storm edges, since the ZDR test stops at 30 dBZ.
- **Split cuts:** a sweep without CC borrows CC and ZDR from the
  CC-carrying sweep within 0.25° elevation.
- **Why ZDR too:** live KVWX clear air (2026-10-19 18:50Z) was 64%
  CC >= 0.9, so CC alone only thinned it; its median ZDR was 6.9 dB, all
  below 2 km, with no MRMS precipitation there.
- **Measured** (tilts at or below 1°):
  - KVWX: 98% of 5-25 dBZ hidden.
  - The 3 storm fixtures: 3-7% of 25-35 dBZ hidden.
  - 0% hidden at 35 dBZ and up.
  - `crates/radar-web/tests/non_weather_fixture.rs` asserts the 35 dBZ
    rule on all 3 fixtures.
- **Renderer** (`browser.rs`):
  - `setHideNonWeather(bool)`.
  - The dealias stash was generalized to
    `raw_moments: HashMap<(sweep, MomentKind), Vec<Moment>>`, with
    `transform_sweeps_if_enabled(moment, sweeps)` and
    `restore_raw(moment)`. VEL dealias and the REF filter share this lazy,
    in-place, restorable path.
  - REF render, probe and cross-section all honor it.
- **UI:** a "Reflectivity" sidebar category (shown on REF) with the
  checkbox, off by default. The cross-section `renderKey` includes it.
- **Known limits:**
  - Filtered gates become `Missing`, so the probe says "missing / below
    threshold" rather than "filtered".
  - Pristine plate-like snow crystals can have high ZDR at low dBZ, so the
    ZDR test could hide some real light snow.
  - Measured on only 4 volumes.

## Owner preferences (standing)

- Users are storm chasers and spotters, on a laptop in a vehicle. Phones
  don't matter.
- Every displayed time is **12-hour local** (`hour12: true`). No UTC/Z in
  the UI.
- Forecast data never goes in the left rail. It lives with time (the strip
  and the Forecast panel).
- #739BD0 is a favourite colour. It is the accent, used for selection and
  the primary action.
- Red flags: hidden controls, and chrome covering the radar.
- Ponytail mode was on: smallest complete change. End replies with what
  was skipped and any risk.
- Ask one question at a time, multiple choice with a recommendation, then
  stop and wait for the answer.
- Wants all RadarScope products matched (colors and KDP); "heavy rainfall
  and hail cores matter most".
- Wants features tested in the real app in Chrome before they count as
  working.
- Storm chasing: "time is everything." Prefer automatic behaviour over
  toggles, buttons and selection steps; keep manual override only as a
  fallback.
- RadarScope Pro is the visual reference; the owner drops screenshots in
  the repo root to match. "Extremely professional looking"; no partial
  fixes.
- Windows has notifications **off for Chrome**, so in the browser only the
  sound plays; the desktop app uses native toasts. To test notifications
  in Chrome, stub `window.Notification` and record the calls.
- To fake location in Chrome: override `navigator.geolocation.watchPosition`
  but **keep the real `clearWatch`**, then toggle "Show my location" off
  and on; otherwise the page-load real watch keeps overwriting the fake.

## Environment notes

- **No GPU on this PC.** Native wgpu GPU tests crash
  (`STATUS_ACCESS_VIOLATION`). Run:
  `cargo test --all --no-fail-fast -- --skip gpu`.
  `provider-hrrr --test cross_provider_render` is the known crash.
  Chrome's WebGPU works.
- **Headless Chrome has no WebGPU here.** It renders "Starting the GPU…".
  For real checks use the owner's Chrome (Claude-in-Chrome tab), not
  headless.
- **GPU tests:** these GPU suites crash here (`STATUS_ACCESS_VIOLATION`)
  on untouched code too: `forecast-core --lib`
  (`gpu_tests::lambert_conformal_grid_renders...`), `radar-render --lib`,
  and `provider-hrrr --test cross_provider_render`. The `mrms-harness` bin
  also dies at its first GPU render; use `--decode-only`.
- **Testing in Chrome:** use the real dev server
  (`npm run dev -- --port 5180 --strictPort`). After a Rust change, run
  `npm run build:wasm` (or `typecheck`), then reload the page.
  A dev server may already be running on 5180 from an earlier session;
  it serves the same folder, so just use it. The Claude-in-Chrome
  extension sometimes refuses localhost ("Could not verify this site's
  safety category"); it worked again a few minutes later in a new tab
  group.
- **Location permission:** Chrome has location and notification
  permission granted for `localhost:5180`. Stubbing
  `navigator.geolocation.watchPosition` and `Notification` from the page
  console tests "covers you" without a real warning overhead.
- **Live radar while testing:** a new live scan can arrive mid-test. Its
  first tilt has no VEL, so the product drops to REF. Pause, or step back
  to a complete scan.
- **Archive:** the picker reads its input as UTC; the timeline shows local
  time. For known-aliased data, load KFTG 2024-06-01 00:01 UTC; frame 1 is
  the `KFTG20240601_000116_V06` fixture.
- **Hot reload resets app state:** editing `App.tsx` while the dev
  server runs triggers a full page reload. Site, toggles and archive mode
  reset (live data; the site is now remembered, and a GPS fix may switch
  to the nearest radar). Re-check toggles before testing.
- **Don't run Prettier.** The repo isn't formatted with it, and it rewrites
  hundreds of lines.
- **Desktop build:** `cd apps/desktop && npm run build` (about 7 minutes
  from cold; it builds `apps/web` first). Output:
  `apps/desktop/src-tauri/target/release/bundle/msi/RadarPro_0.1.0_x64_en-US.msi`
  and the NSIS `-setup.exe`. Capture the real exit code (`> log 2>&1;
  echo $?`). Piping into `grep`/`tail` hid a failed build once.
- **Installing the MSI:** run it through PowerShell
  (`Start-Process msiexec.exe -ArgumentList '/i','"<msi>"','/l*v','"<log>"' -Wait`).
  From Git Bash, `msiexec //i ... //l*v` fails with 1639 (invalid command
  line). The owner must close the running app first and click through
  the installer.
- **Tauri rewrites `apps/desktop/src-tauri/Cargo.toml`** during a build
  with identical content, so git may show it modified with an empty diff.
  Ignore it.
- Web app: `cd apps/web && npm run dev`, which rebuilds the wasm first.
  `npm run build` also rebuilds the wasm.
- Use `npm run typecheck` (= `tsc --noEmit`). Plain `npx tsc -b` reports
  bogus `vite.config.ts` errors and leaves `*.tsbuildinfo` files behind.
  Delete those.
- Review screenshots: a headless-Chrome CDP script (`capture.mjs`, in the
  session scratchpad, not the repo). It shoots exact 1440×900 and 390×844
  into `.impeccable/review/`.

## What was built in the second session (in `d14b168`)

### MRMS lightning (`lightning_5min`)

- **Data:** `MrmsProduct::LightningCg5min` = `CONUS/NLDN_CG_005min_AvgDensity_00.00`,
  cloud-to-ground flash density over 5 minutes.
- **Sentinels** (checked live): `-1` means no flashes, which maps to
  `Missing`. It is in the hail/echo-top/VIL group in `decode.rs`; `-3`
  never appears.
- **Unit:** `flashes/km^2` over the 5-minute window. NOAA's table wasn't
  reachable; the unit is inferred, and the inference checked out live. One
  flash reads 1/cell-area (0.869 at 21.5N, 0.957 at 32.5N), and 2 flashes
  read 1.734.
- **Drawn as markers, not raster:**
  - The per-pixel GPU raster drops ~1 km cells at regional zoom.
  - `MrmsGrid::value_points()` (crates/mrms, unit-tested) is exposed to
    wasm as `MrmsHandle.valuePoints()`. It returns flat
    `[lon, lat, value]` for valued cells only (a few dozen).
  - `useMrmsOverlay` has `POINT_PRODUCTS`. For those it skips
    `renderCurrentGrid` and `drawPoints` paints **lightning-bolt shapes**:
    about 20 css px tall for 1 flash, up to about 32 px.
  - Colour by density: yellow, orange at 2+, red at 5+, white at 10+.
  - Each bolt has a dark outline. y uses Web Mercator, so placement is
    exact.
  - The bolts redraw after the 300 ms viewport debounce, like the other
    MRMS layers.
  - The Rust lightning palette in `palette.rs` is now unused by the UI. It
    is kept for the enum and the harness.
- **Verified in Chrome:** strikes over Cuba, the Bahamas and the Gulf,
  placed correctly (the cluster sits on Camagüey).

### Velocity dealiasing (toggle "Dealias velocity", off by default)

- **Decoder:** `Radial.nyquist_velocity_mps: Option<f32>` comes from the
  RAD block (u16 x 0.01). Every `Radial {}` literal got the field.
- **Algorithm:** `radar_geo::dealias_velocity_sweep` (`crates/radar-geo/src/dealias.rs`)
  is region-based, after Py-ART:
  - Regions are union-find over neighbor gates differing by less than
    0.5·Vn (azimuth wraps).
  - The largest region gets k=0. Neighbors get k from the mean boundary
    difference, strongest boundary first (heap).
  - Guards:
    - |k| <= 1.
    - A region is only unfolded if its mean |raw| > 0.5·Vn.
    - A gate is only changed if the result stays within 1.5·Vn.
    - These guards fixed a real bug: noisy near-zero KFTG gates were going
      5.5 -> 62 m/s.
  - A radial whose Vn differs from the sweep median by more than 0.5 m/s
    is left raw.
- **Renderer** (`radar-web/src/browser.rs`):
  - `setDealias(bool)`.
  - Sweeps are dealiased **lazily, in place**, only when viewed:
    VEL/SRV select and VEL probe do one sweep; a VEL cross-section does
    all sweeps.
  - Raw VEL moments are kept in `raw_velocity` (keyed by sweep) and
    restored exactly when the toggle goes off. The map is cleared on
    decode.
  - Why lazy: the whole volume takes 160-310 ms natively, and decode runs
    on every loop frame and live chunk update.
  - `probeGate` and `crossSection` are now `&mut self`.
- **UI:** a "Velocity" sidebar category, shown when VEL or SRV is
  selected. The cross-section `renderKey` includes `dealias`.
- **Tests:**
  - 4 unit tests in `dealias.rs`.
  - `crates/radar-web/tests/dealias_fixture.rs` runs over all 3 Level II
    fixtures. It asserts that only whole-Nyquist shifts happen, that
    non-value gates pass through, that Vn is plausible, and that more than
    half of the >1.5·Vn seams are removed. Results: 65->9, 26->2, 327->26.
- **Verified in Chrome** on KFTG 2024-06-01 00:01Z, 4.9° tilt:
  - Aliased blue specks in the red core disappear when the toggle is on.
  - The probe at the same gate reads -27.00 off and +29.78 on
    (= +2·28.39).
  - SRV renders with dealiasing on.

### Not verified / risks

- Dealiasing in a cross-section, or during loop playback, was not checked
  in the browser.
- True radial speeds above 1.5·Vn are not recovered (a documented
  `shortcut:` in `dealias.rs`).
- Console errors `decodeVolume failed ... no decodable message 31` appeared
  during live scan rollover. They happened before this session's wasm was
  loaded, so they're probably a chunk-feed edge case: a scan's header
  arriving before any radials. Not investigated.

## What was built in the first session (in `8e1459d`)

### 1. UI redesign (Impeccable, "Orienteering Course" direction)

- **Direction contract:** `.impeccable/surfaces/apps-web-src-app-tsx.md`.
  `PRODUCT.md` (repo root) holds the product brief. Decision artifacts are in
  `.impeccable/mocks/decision/`.
- **Layout:** `App.tsx` and `App.css` were rewritten. A `.workstation` grid
  holds:
  - A 320px left rail: site (search input with a datalist, plus 8 nearby-site
    buttons), product and tilt segments, a Course block (probe row,
    cross-section leg, legend), then collapsible Warnings, Layers, Storm
    motion, Archive, Color table and Settings.
  - The map.
  - A 64px time strip: playback, timeline, Forecast button.
- **Forecast panel:** a 380px right-side panel opened from the strip. It
  stays mounted with `hidden`, because its canvas is painted imperatively.
- **Icons and font:**
  - New `ui/Icon.tsx`, one SVG icon set.
  - Roboto Condensed via `@fontsource-variable/roboto-condensed`, added to
    package.json.
  - `ui/Sidebar.tsx` deleted.
- **Course vocabulary** (purple `#b56cff` is reserved for marks the user
  places):
  - The cross-section leg draws on the map as start triangle → leg → finish
    double circle, with a 200ms draw-on.
  - The Course table rows show km and bearing.
  - The probe is a course row (control circle, value, range, bearing).
- **Warnings:** all US alerts by default. A checkbox narrows the list to
  polygon warnings within 460 km. Zone-only watches have no polygon and
  would be hidden, and the label says so. Logic: `alerts/geometryHitTest.ts`
  `alertNearSite`.
- **Basemap:** OpenFreeMap dark style.
- **Review:** the Impeccable finish reviewer ran 2 rounds and ended at
  "fix". It deferred the items the owner later chose; see Open items.
  `DESIGN.md` has **not** been written yet (that's the impeccable-documenter
  step).

### 2. Scan-loop backfill (`scan/useScanHistory.ts`)

- On site switch, the app lists the archive (today, plus yesterday if
  needed) and downloads the newest N scans, newest first.
- N = the history limit: 15 by default, 5–60 in Settings.
- History entries stay sorted by time and are de-duplicated by key.

### 3. Forecast radar: HRRR simulated reflectivity

- **Rust:**
  - New `ForecastVariable::CompositeReflectivity`
    (`composite_reflectivity`).
  - In HRRR's `idx_names` it maps to `REFC:entire atmosphere`, parameter
    (16, 196). It was verified live, and a fixture test decodes it.
  - GEFS returns None for it.
  - Display scale: NWS-style dBZ ramp, transparent below 5 dBZ.
- **Web defaults:** HRRR and simulated reflectivity. HRRR lead hours run
  0–18, hourly (`leadHourOptions`). The render size is 1440×960 (about 4 km
  per pixel).
- **On the map** (`MapView` `drawForecastField`):
  - Picking a forecast hour past NOW (`timeRegion === "forecast"`) paints the
    field on the map and hides the radar.
  - The field is drawn row by row, so plate-carrée latitudes land correctly
    on Mercator.
  - Only reflectivity goes on the map. Other variables stay in the panel.
  - A badge says "Forecast radar … model, not observed", with a Back to
    radar button.
- **Loop playback:**
  - Rendered frames are cached per variable, ensemble and hour (about
    105 MB for 19 hours).
  - Model-handle calls are serialised through `queueRef`.
  - `loadLoop` preloads the hours.
  - On the forecast side, Play and ← → Space loop and step the forecast
    hours. Only one loop runs at a time: the radar loop pauses.
- **Unpublished hours:**
  - The wasm `publishedLeadHours()` lists the run's `.idx` files (HRRR
    only; GEFS returns null, meaning all hours are available). It is
    re-checked every 2 minutes.
  - Unpublished hours are greyed out and disabled.
  - HRRR opens on +0h, the one hour discovery guarantees.

### 4. Real-time radar feed (`nexrad/chunks.ts`, `scan/useScanPoller.ts`)

- **Source:** the live poller follows
  `unidata-nexrad-level2-chunks` (`ICAO/<vol 1-999>/<YYYYMMDD-HHMMSS>-<NNN>-<S|I|E>`).
  It polls every 8s, downloads new chunks in order, and re-emits the growing
  volume.
- **Newest volume:** found from the folder numbers by picking the number
  just before the widest circular gap. Numbers wrap at 999, and old folders
  linger.
- **Keys:** a volume uses the archive's key
  (`YYYY/MM/DD/ICAO/ICAOyyyymmdd_hhmmss_V06`), so a scan from both feeds is
  held once.
- **In-place updates:** history entries carry `revision` (the byte length).
  `App` re-decodes when the revision changes.
- **Tilt preference:** `preferredSweepRef` keeps the user's tilt while a new
  volume only has its lowest tilt.
- **Fallbacks:**
  - If the newest real-time volume is more than 30 minutes old, or the feed
    stalls for more than 3 minutes, the poller falls back to, or
    re-discovers via, the archive.
  - On startup it also emits the just-finished previous volume, which isn't
    archived yet.
- **Verified live:** a new scan appeared 16s after its start time. Before,
  it took about 7 minutes.
- **Decoder:** needed no change. A fixture test decodes a real partial
  volume.

### 5. Smaller changes

- Times are 12-hour local everywhere: strip, timeline, Forecast panel,
  alerts, MRMS and Rainbow.
- The time strip is one 64px row, and the observed/forecast label is in the
  date row.
- Frame tick marks: buttons opt out of the generic chrome with
  `:where(:not(.unified-timeline-tick))`. That rule caused the 30px tick
  overflow bug.
- Tilt chips are in elevation order. Duplicate angles are labelled
  `0.9·3`, with the sweep number.
- Legend copy and the cross-section empty state are fixed. Product and Tilt
  empty text now reflects GPU state.

## Open items (older)

- Ask whether dealiasing should default to on. It is off today, in line
  with the "never alter data" positioning.
- Look into the `no decodable message 31` console errors on live scan
  rollover.
- DESIGN.md's documenter found places where the build breaks its own
  design. The owner said **leave these for now; don't fix unless asked**:
  - UTC still shown in three places: the Archive picker ("Archive (UTC)",
    which reads its input as UTC), the model run in the Forecast panel
    ("12Z 20261019"), and the raw scan file name in the feed status.
  - The Forecast panel (380px, nearly full height) still covers part of the
    radar.
  - Amber used outside model time: range-folded probe values, the
    stale-feed warning, and the Rainbow panel titles.
  - Alert severity colours sit close to the reserved colours (Moderate and
    Severe look like amber; Minor looks like the accent).
- Item 6, course-map basemap restyle: explained to the owner, who was unsure.
  It is cosmetic. Don't do it unless asked.
- Forecast radar is a fixed CONUS image, so it's blocky when zoomed in.
  Viewport-framed rendering would sharpen it.
- Untested:
  - The archive fallback for a site that isn't on the real-time feed.
  - Real-time chunks arriving out of order (the code waits for the gap).
  - Switching to GEFS or another variable while the loop is loading.
- Data use on cellular: about 150 KB per 12s on the real-time feed.
  Backfill is up to N full volumes on every site switch.
- Rainbow precipitation nowcast tiles past NOW: an alternative to HRRR,
  offered to the owner but not chosen. Needs a Rainbow API key.

## Known limits left as-is

- Zone-only NWS alerts (most watches) have no polygon, so they can't be
  placed near a site without NWS zone shapes.
- HRRR runs other than 00/06/12/18Z only reach +18h. The app caps at 18.
- The archive bucket is `unidata-nexrad-level2` (the successor to
  `noaa-nexrad-level2`).

## 2026-10-20 — 5ef66d2 GOES-West lightning + Settings window
- Lightning: GOES-East (G19) and GOES-West (G18) via `useGlmFlashes(satellite, enabled)`; toggles `radarpro.showLightning` / `radarpro.showLightningWest`, both default on; `mergeFlashes` splits at 106 W.
- Settings window (`ui/SettingsWindow.tsx`), opened by the gear in the rail head: Storms & lightning, My location (toggles), Placefiles, Color tables, Data & app. Rail keeps Site/Product/Tilt/Course, Warnings, SPC, My location (Center on me + covering list), Layers, Reflectivity/Velocity/Storm motion, Archive.
- Correction: the Phase 3.5 review finding that WPC QPF bands overlap was wrong (bands are rings with holes); QPF now uses `area()` per band with "X to Y in" hovers.
- SPS tags "Up to .75" / "Up to 50 MPH" now shown.
- Open: lightning hidden past ~2000-mi-wide views (placefile threshold 999); 20k bolts draw at ~159 ms/frame (declutter idea); desktop app and <900px layout not re-tested; 3.7 archive-load footer date quirk.

## 2026-10-20 — f9ade59 Phase 5.1 HRRR severe fields
- New ForecastVariable: SbCape (CAPE surface), MlCape (CAPE 90-0 mb), Srh01/Srh03 (HLCY), LclHeight (HGT level of adiabatic condensation from sfc — AGL, checked at Leadville), BulkShear06 + Stp derived in `provider-hrrr::decode::{input_messages, derive, stp}` (SPC fixed-layer STP). Shear displayed in kt.
- On the map like forecast radar (`onMap` in FORECAST_VARIABLES), badge colour key `ForecastKey` from `displayStops`.
- STP = 5 sequential range GETs (~10 MB/hour); loop of 19 h ~190 MB.
- forecast-core GPU tests crash locally (DisplayLink, 0xc0000005); use `-- --skip gpu`; CI runs them.
- Next: 5.2 point soundings (skew-T + hodograph), 5.3 RTMA, 5.4 RAOB.

## 2026-10-20 — open items cleared
- Built-in placefile layers (GLM, LSR, SPC/WPC areas) use `threshold: Infinity`, so they show at any zoom (were hidden past ~2000-mi views).
- `placefiles/draw.ts` declutters icons per layer: only the topmost icon in each 4 px cell is drawn; unrotated icons skip save/rotate/restore. 20k clustered synthetic bolts over CONUS: ~2,200 drawn, ~5-15 ms (was ~159 ms). glmLayer build ~37 ms, once per 20 s poll.
- Timeline left label uses `history.archiveStartMillis` while archive scans load (no more today's date).
- Re-tested OK: <900 px layout + Settings, desktop app (tauri dev + CDP), loop playback with panning after the setView change, KDP scale vs decoded values.
- STP loop ~190 MB left as is: 5 whole-CONUS HRRR messages per hour, only on play. Shrinking it needs server-side cropping (NOMADS filter), a design change.

## 2026-10-20 — Phase 5.2 HRRR point sounding
- Course block "Sounding at a point" arms one map click; the right panel shows skew-T (T red, Td green, surface parcel dashed), hodograph (0-3/3-6/6-9/9-12 km AGL colours, kt rings), surface T/Td, SBCAPE, LCL AGL. Valid time = forecast hour shown, else nearest hour to now.
- Data (ADR-0016): AWS wrfprsf is ~128 MB per sounding (template 5.3, no point ranges). Open-Meteo `models=ncep_hrrr_conus` first (8 s timeout), then NOMADS `filter_hrrr_2d.pl` crop of the app's HRRR run (5 levels + HRRR surface CAPE) via `fetchUrlBytes`, sampled by `provider_hrrr::decode::sample_column` / wasm `sampleHrrrColumn`.
- Files: `apps/web/src/forecast/sounding.ts` (fetch + parcel math), `apps/web/src/ui/SoundingPanel.tsx`, fixture `fixtures/provider-hrrr/nomads_crop_okc_t16z_f01.grib2`.
- Verified in Chrome: fallback path end to end, pick-mode cancel (Escape, leg start).
- Verified in the desktop app (`b0a292a`, tauri dev + CDP): full Open-Meteo sounding (39 levels, ~200 mb tropopause, hodograph colours), NOAA fallback through `fetch_url` (1.7 s, 925 mb height interpolated), sounding follows the forecast hour (+6 h -> 00Z).
- Open-Meteo free tier was overloaded/429 for much of the session; the 8 s timeout + fallback covers it.

## 2026-10-20 — Phase 5.3 RTMA surface analysis
- `noaa-rtma-pds` (CORS open): `rtma2p5.YYYYMMDD/rtma2p5.tHHz.2dvaranl_ndfd.grb2_wexp` (+ `.idx`), one per hour, ~50 min after; template 3.30 2345x1597, earth shape 1 (r=6,371,200), simple packing 16-bit, ~5-7.5 MB per field.
- `provider_hrrr::rtma::RtmaProvider` reuses HrrrClient, idx, decode; shared `provider::fetch_messages`. Decoder takes the earth radius from the file (spherical only, 6.3-6.4e6 m guard). New `ForecastVariable::WindSpeed10m` (RTMA WIND; HRRR derives from 10 m U/V).
- Display: Temperature2m/Dewpoint2m in °F (own dewpoint moisture ramp), gust/wind speed in kt (clear < 10 kt). GEFS panel shows these units too.
- Web: `ForecastProviderId` += "rtma"; `FORECAST_VARIABLES[].models` replaces `hrrrOnly`; `variableAvailable`/`variableOnMap` (never GEFS on map). RTMA re-discovers every 120 s and swaps in a new hour in place. Opaque fields at 70% (`forecastOpacity`).
- Sounding fallback with a non-HRRR model loaded assumes the HRRR run 2 h before the valid hour.
- Verified in Chrome (T, Td, wind on map, geography lines up with the Great Lakes/coast) and the desktop app (RTMA loads; recheck runs clean). A real hour rollover wasn't watched.

## 2026-10-20 — Phase 5.4 observed balloon soundings (new PC)
- Repo freshly cloned to `D:\RadarPro` on a new PC with an **RTX 5070**; `npm ci` in `apps/web` and `apps/desktop` needed after a clone. The old GPU crashes are gone: `radar-render --lib` 51, `forecast-core --lib` 41, `provider-hrrr --test cross_provider_render` 1, all pass (PLAN Phase 0.6 closed). Headless Chrome (`--headless=new --enable-unsafe-webgpu`) has WebGPU here, so `apps/desktop/scripts/cdp.mjs` can drive a headless page on port 9222 with no window: the owner games while Claude works, so never pop or focus windows (tauri dev opens one).
- Owner chose IEM (ADR-0017) and "same click, 2 tabs". `fetchRaobSounding` / `raobLevels` in `forecast/sounding.ts`: one all-sites `raob.py?ts=` per 00Z/12Z (cached unless under 6 h old), sites from `RAOB.geojson`, nearest usable profile, steps back up to 2 days. `SoundingView` has an HRRR | Observed switch; Observed takes `observedAtMs` (archive frame, else now). The panel is titled "Sounding" now, and its times are 12-hour local (they were UTC "Z").
- Verified in headless Chrome: live KTLX click -> "KOUN Norman OK · 47 mi W · launched Oct 20, 7:00 AM", surface 51/48 °F matching the raw IEM row, skew-T + hodograph drawn; archive 2024-05-06 23Z -> KOUN 12Z (105 levels); HRRR tab still loads via Open-Meteo. Desktop app (tauri dev + CDP) verified after the commit: HRRR tab via Open-Meteo, Observed on KVWX -> "KBNA Nashville Metro TN · 126 mi S", plots drawn. Not handled: specials (18Z), which aren't looked for. The owner says it's fine to open the desktop app when they aren't gaming; ask if unsure.

## 2026-10-20 — Phase 6 export, overlay scaling fix (worked on the devbox)
- "Save image" / "Save loop" under the legend (`ui/ExportPanel.tsx`). `MapView`'s `exportRef` re-renders the radar (WebGPU) and calls `map.redraw()` (WebGL) in the same task, then flattens every visible canvas in DOM order: both GPU canvases read back blank once presented. Stamp (site · product tilt, 12-hour local time, "RadarPro") and colour bar are drawn on the copy. Loop: `jumpToNearestByTimestamp` each held scan, then waits for `frameReady` (App: the scan decoded and drawn, `drawnScan`, plus `settled` from archive warnings `usePolled`, `useStormTracks` and `useSpc`; 15 s cap per frame). Capture also calls MapView's `updateOverlay` (`redrawOverlaysRef`) because the overlay canvases otherwise redraw in effects that ran after the capture: loops showed the previous scan's storm tracks until this. MediaRecorder on a `captureStream(0)` canvas at the playback speed, last scan held 3x (needs a redraw + `requestFrame` at the end or the hold is lost), capped at 1920 px wide; MP4 (H.264) in Chrome and WebView2, WebM fallback. Playback goes back to live/where it was afterwards.
- Verified at 150% in headless Chrome (devbox Vite over tailnet) and the desktop app (tauri dev on the PC, devbox Vite tunnelled with `ssh -N -L 5173:127.0.0.1:5173 zima`): PNG and MP4 land in Downloads in WebView2. Forecast-hour loops are deliberately not exported (owner: observed is enough; button disabled past NOW). Archive loop verified on KTLX 2024-05-06 23Z: each frame's storm cells match the archived NST for that scan, and a frame matches a settled still of the same scan by PSNR. Exports include whatever placefiles are on, e.g. Spotter Network names.
- **Bug fixed:** at Windows 150% scaling the overlay canvases (range rings, course leg, warnings, placefiles, storm tracks, my location) drew in CSS pixels on a device-pixel buffer, so they shrank toward the top-left and slid off the map when panning. Now `setTransform(scale)` after each resize in `updateOverlay`. Every earlier test ran at 100%, which hid it.

## Owner preferences (read before starting)
Collected from Claude's memory on the old computer (2026-10-20); these override defaults.

**Approvals and questions**
- Ask before every commit and every push; one approval covers one commit. Exception: when the owner says "commit and push", do both.
- Before asking to commit, run `/ponytail:ponytail-review` on the change, fix what it finds (or say what was left and why), then ask.
- Ask one question at a time, then stop and wait. Give decisions as multiple choice with a recommendation.
- Owner decides fast and prefers working code over perfect design; once a direction is set, just build it.

**How to work**
- Ponytail mode: the smallest complete change, no features nobody asked for. End replies with what was skipped and any risk.
- New data source: measure it first (size, speed, CORS, format), then put the real options to the owner before designing. 5.2 and 5.3 both went this way.
- Test every feature in Chrome and in the desktop app (CDP) before calling it done, and say plainly what was not verified.
- Test map features at 150% display scaling, not only 100%: start headless Chrome with `--force-device-scale-factor=1.5` (the owner's PC runs 150%; a 100%-only test hid the overlay bug above).
- Update the docs with each feature: README, PRODUCT.md, DESIGN.md, an ADR for data-source or format decisions, plus PLAN.md and this file.
- The owner lets Claude do the end-of-session handoff and push docs to main when asked; check CI after pushing.

**Product rules**
- "Time is everything" (the owner chases storms): automatic behaviour over toggles and selection steps; manual override only as a fallback.
- Every RadarScope Pro Tier 1 and Tier 2 feature must ship before the app is finished (parity table in PLAN.md).
- Primary use: a storm chaser on a laptop in a vehicle; phones get the fallback layout only. The map stays dominant; nothing hides the radar.
- US chaser units: °F, kt, miles. Times are 12-hour local.
- Design follows DESIGN.md: dark theme; #739BD0 (a favourite) only for selection/primary action; #b56cff only for marks the user places; amber only for model time. Set-once options go in the Settings window, keeping the sidebar lean.

**Code rules**
- Never run prettier or `cargo fmt` under `apps/` (wide lines on purpose); crates are rustfmt-checked in CI.
- Adding a Tauri plugin: the JS and Rust versions must match or the build fails.

## START HERE (next session, after 2026-10-20)
- main is at the docs commit after `7262c3a` (Phase 5.3 RTMA), pushed. Session 10 shipped `b0a292a` (5.2 soundings), `e0ecc5d` (sounding plot colours + docs), `7262c3a` (5.3 RTMA). CI green for `b0a292a` and `e0ecc5d`; `7262c3a` was still running at handoff (check `gh run list`).
- Next task: Phase 7.1 multi-radar composite per PLAN.md (Phase 6 export done; forecast-hour loop export not wanted). Done since: 5.4 observed soundings and Phase 6 export (sections above). 7.1 needs no new data source: render each nearby site into the same Mercator view and combine (closest radar or highest value per pixel).
- Still to see live: an RTMA hour rollover swapping in place (recheck every 120 s in `useForecastProvider`); HRRR temperature/dewpoint on the map.
- Known limits: Open-Meteo free tier (soundings) was overloaded/429 much of 2026-10-20 and is non-commercial only (ADR-0016); the NOAA fallback needs the desktop app or dev-server relay. STP loop downloads ~190 MB (only on play). The installed desktop app predates 5.2/5.3 (rebuild with `npm run tauri:build`).
- Browser testing: two Chrome browsers are connected to the extension now; the owner must pick one before browser tools work. The desktop app over CDP needs no pick: start it with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 npx tauri dev --config '{"build":{"beforeDevCommand":""}}'` from `apps/desktop`, then `node apps/desktop/scripts/cdp.mjs eval "<js>"` or `node apps/desktop/scripts/cdp.mjs shot out.png`.
- `tauri dev` rewrites `apps/desktop/src-tauri/Cargo.toml` line endings; `git restore` it before committing.
- Moving computers: Claude's memory folder (`~/.claudecodemax/memory`) stays on the old machine; this file and PLAN.md are the portable record. On a new machine: Rust from `rust-toolchain.toml` plus the `wasm32-unknown-unknown` target and `wasm-bindgen-cli` 0.2.128 (see `crates/radar-web/README.md`); Node 20+; `npm install` in `apps/web` and `apps/desktop`; then `npm run dev` in `apps/web` (builds the wasm first, serves port 5173) before starting the desktop app.
- **Build locations (owner, 2026-10-20):** all builds and caches on D:. Repo `D:\RadarPro` (targets in-repo); `CARGO_HOME=D:\DevCache\cargo`, `RUSTUP_HOME=D:\DevCache\rustup` (user env vars, PATH has `D:\DevCache\cargo\bin`), npm cache `D:\DevCache\npm-cache`. A shell started before that change has the old env: pass both vars, or rustup re-downloads toolchains to `C:\Users\PC\.rustup` (happened once; deleted).
- **Server dev box:** the owner wants heavy builds on the ZimaOS server. Container `devbox` there (any repo; defined in `D:\ZimaOS Server\ZimaOS Server\devbox\`, README inside): `ssh devbox` from the PC, repos in `/work` on the Seagate drive. RadarPro is cloned at `/work/RadarPro`; `cargo test --all` (GPU tests on lavapipe), wasm, `npm test` and `npm run build` all pass there. Vite: run with `__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS=zimaos.tail6ecc51.ts.net`, open `https://zimaos.tail6ecc51.ts.net:5173`. Windows desktop/MSI builds stay on the PC.
- Leave alone: 7 `Screenshot 2026-10-20 *.png` in repo root (owner's, uncommitted).
- Local quirks: forecast-core GPU tests and mrms-harness render crash here (DisplayLink) -> `cargo test -p forecast-core --lib -- --skip gpu`; never prettier/cargo fmt under apps/.
