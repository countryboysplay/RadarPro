# RadarPro plan to finish (written 2026-10-20)

Goal: a professional chasing tool on par with RadarScope / GR2 / RadarOmega.
Owner rule: time is everything, so prefer automatic behaviour over toggles,
and keep a manual override only as a fallback.

**Owner rule (2026-10-20): every paid RadarScope Pro feature, Tier 1 and
Tier 2, is in the app before it is finished.** The checklist is
"RadarScope Pro parity" below; each row points at the phase that delivers it.

**Process rule (owner, 2026-10-20): before every commit, Claude runs a
Ponytail review (`/ponytail:ponytail-review`) of the change automatically,
fixes what it finds (or reports what it left and why), and only then asks
to commit.**

Order: loose ends first, then whatever makes chasing safer, then the
features paid apps have that we don't, and the heaviest work last. Sizes
are rough: S = under a session, M = about a session, L = several sessions.

---

## RadarScope Pro parity (owner requirement)

From the owner's screenshot of RadarScope Pro's subscription screen
(2026-10-20). Status: ✅ done, ◐ partial, ❌ missing.

### Tier 1 (the owner pays for this)

| RadarScope Pro feature | RadarPro today | Delivered by |
|---|---|---|
| Lightning data | ✅ GOES-East + GOES-West GLM flashes (separate toggles, both on, split at 106 W) (last 15 min, bolts coloured by age, live) + "lightning within 10 mi" alert; MRMS 5-min density too | done |
| 30-frame loops | ✅ 5-60 scans (default 15) | done |
| Tornado and severe thunderstorm watches | ✅ outlines on the map (live and archive), listed, notify when covering you | done (`c36161b`) |
| Special weather statements | ✅ listed, under "covers you", polygons drawn with warnings, live and archive | done |
| Mesoscale discussions | ✅ outlines, hover, full text (live and archive) | done (`c36161b`) |
| SPC and WPC outlooks | ✅ SPC Day 1 cat/torn/wind/hail; WPC excessive rainfall Days 1-3 and QPF Day 1/2/3 and 1-3 (live) | done |
| Local storm reports | ✅ native layer (live and archive) + Spotter Network reports placefile | done (`a6eb391`) |
| Custom color tables | ✅ editor and JSON tables | done |
| Multi-pane display | ❌ | Phase 10 |

### Tier 2

| RadarScope Pro feature | RadarPro today | Delivered by |
|---|---|---|
| 50-frame loops | ✅ up to 60 | done |
| NEXRAD Level II, all tilts | ✅ | done |
| Archived radar (30 days to 28 years) | ✅ full Level II archive; storm tracks from the Level 3 archive | done |
| MRMS products | ✅ several, including lightning | done (more in Phase 9) |
| RTMA products (surface analysis) | ✅ T/Td/wind/gust on the map, hourly refresh | done (5.3) |
| GOES satellite products | ❌ | Phase 8.1 |
| METAR surface observations | ❌ | Phase 8.2 |
| Archived NWS bulletins | ✅ warnings, advisories, SPS, watches, MDs and storm reports in effect at the archived scan, with full text (IEM) | done |
| Atmospheric soundings | ✅ HRRR forecast (5.2) and observed RAOB (5.4) skew-T + hodograph | done |
| Shear and hail contours | ❌ (MRMS hail is a grid only) | Phase 9 |
| Detailed road layer | ◐ basemap highways and main roads | Phase 8.4 |
| Satellite map layer | ❌ | Phase 8.3 |
| Cross-platform subscription sharing | n/a (free app); equivalent is settings sync | Phase 11.4 |

### Products (radar moments)

All match RadarScope's colors: REF, VEL, SRV, SW, ZDR, CC, KDP (2026-10-20).
RadarScope's "Precipitation Depiction" and "One-/Three-Hour Accumulation"
are covered by the MRMS layer (precip type, radar-only 1-h and 3-h rainfall;
Phase 3.9 done).

### RadarPro extras RadarScope Pro doesn't list

HRRR forecast radar on the map, GPS alerts for storms heading your way,
automatic SRV storm motion, velocity dealiasing, non-weather echo filter,
KDP computed on the device.

---

## Phase 0: loose ends from 2026-10-20 (S)

1. ~~**Loop playback after the sharpness change.**~~ Checked in Chrome on live KVWX 2026-10-20: aligned and sharp while panning/zooming mid-loop (cross-section not checked). Play a loop on a live
   site. Frames should stay aligned and sharp while panning mid-loop. Also
   check the cross-section and the gate readout against the pixel under
   the cursor.
2. ~~**Storm label collisions.**~~ Done (`1fb9267`, `storms/labelPlacement.ts`). Where cells bunch up, a label can land on
   a neighbour's track or a meso circle. Place each label clear of every
   track and circle, not just its own.
3. ~~**VEL colours.**~~ VEL, CC (`751debf`), SW and ZDR (`b7720fd`) matched to RadarScope. KDP added (`f9fa5a1`, RadarScope colors on an inferred -2..10 deg/km scale; scale checked against our decoded values 2026-10-20, fits).
4. ~~**Weak-echo choice for REF.**~~ Owner chose: keep the fade (2026-10-20). RadarScope shows everything down to −32
   dBZ, while we hide below 5 and fade to 23. Owner decides: keep the fade
   (cleaner), or match RadarScope fully (shows boundaries, but also the
   insect disc).
5. ~~**Update HANDOFF.md** with the 2026-10-20 work~~: done (end of the
   sixth session).
6. ~~**GPU pixel test crash**~~ Gone on the new PC (RTX 5070, 2026-10-20): radar-render --lib 51, forecast-core --lib 41, provider-hrrr cross_provider_render 1, all pass.

## Phase 1: chase safety and reliability (M), done 2026-10-20

These are about not missing something that can hurt you. Status and how each
was built:

1. **Hazard alerts from the radars near you, not the one on screen.** Done.
   `useNearbyStormTracks` (`storms/useStormTracks.ts`) polls storm tracks,
   TVS, hail and meso for the 3 radars nearest your GPS position (within 250
   km) every 2 minutes, whichever site is on screen. The fetch-and-merge body
   was pulled out as `fetchStormTracks`, shared with the on-screen hook.
2. **Heading-aware alerts, one per storm.** Done. `stormAlerts`
   (`storms/nearbyHazards.ts`):
   - groups each storm's TVS, mesocyclones (a meso within 10 km belongs to
     its nearest cell) and 1"+ hail into one alert;
   - walks the cell's NWS forecast track, shifted to the hazard's own
     position, for the closest approach in the next 30 min;
   - alerts when that is within 25 km (15 mi): "now 6 mi NE of you" or
     "arrives in ~15 min, passing 3 mi N of you";
   - ranks TVS > low-level meso > meso > hail, and re-alerts only when a
     storm's rank rises;
   - reports a storm two radars both see once, from the nearer radar.
   Unit-tested: approaching, receding, 30-minute cutoff, two-radar dedupe,
   and the Owensboro fixture.
3. **Watches that cover you.** Done. `useAlertIdsAtPoint` asks
   `api.weather.gov/alerts/active?point=lat,lon` (rounded to 0.01 deg, about
   1 km) which alerts cover you; `coveringAlerts` counts those as well as
   polygon hits. Zone-only alerts include statements and advisories, so of
   those only watches and warnings notify; all are listed under My location.
4. **End-to-end warning test.** Done in the browser with a simulated GPS
   position and a recording stand-in for the notification API:
   - inside a live Flood Warning polygon (Lake/Volusia FL), it notified
     "Flood Warning covers your location" and listed it;
   - inside a live zone-only Special Weather Statement (north GA), it listed
     it as covering you and, correctly, did not notify.
   Still to do: a real tornado or severe thunderstorm warning in the desktop
   app, and the storm-hazard notification with a live TVS/meso. None
   existed during the session.
5. **Live TVS, hail and meso.** Parsers verified on live products from
   KHDX, KABX, KFDX, KEPZ and KAMA (all four products parse; hail estimates
   present, no TVS or meso active). The desktop relay passes the same bytes
   through unchanged. Not yet run inside the desktop app itself.
6. **Storm tracks in archive playback.** Done. In archive mode
   `useStormTracks` fetches the NST and NMD products archived for the scan
   on screen from Unidata's public Level 3 bucket on AWS
   (`unidata-nexrad-level3`, CORS-open, keys like
   `VWX_NST_2025_03_15_10_01_50`); listings and products are cached for loop
   playback. NTV and NHI are not archived, so archive views have no TVS
   markers or hail, and the Storm tracks summary says so. SRV in archive mode
   now uses the archived storm motion.
7. **Loops across midnight UTC.** The live loop already read the previous
   day. Archive loads starting late in the UTC day now carry on into the
   next day instead of stopping at 23:59Z.

## Phase 2: placefiles (M-L), ✅ done in `7fd8ea9`

GRLevelX placefiles: spotter positions, chaser feeds, storm reports.

1. **Parser** (`apps/web/src/placefiles/placefile.ts`, unit-tested):
   `Title`, `Refresh`/`RefreshSeconds`, `Threshold`, `Color`, `Font`,
   `IconFile`, `Icon`, `Text`, `Place`, `Line ... End:`,
   `Polygon ... End:`, and `Object ... End:` with pixel offsets inside.
   Skip `Triangles:` and `Image:` blocks cleanly for now. Test against
   real files (Spotter Network, IEM storm reports).
2. **Fetching:** most placefile servers send no CORS headers.
   - Desktop: a Rust command `fetch_url(url)` (http/https only, size
     cap), next to `nws_radar_product`.
   - Browser: a small dev-server proxy route.
   - Icon sheets come through the same path as blobs, which also avoids
     mixed-content blocking of `http://` icons.
3. **Drawing:** lines, polygons, icons and text on the existing overlay
   canvas, under storm tracks. Honour `Threshold` (hide when zoomed out)
   and `Refresh`.
4. **Hover text:** show an icon's hover text in the existing probe label.
   That's how chasers read spotter names and report details.
5. **UI:** a "Placefiles" sidebar section with a URL list, an on/off
   switch per file, a status line (last refresh, item count, errors), all
   remembered across launches. Ship 2-3 well-known free feeds as one-click
   suggestions.

Done when: Spotter Network and an LSR storm-report placefile show with
their icons, refresh on their own, and show names on hover.

## Phase 3: NWS products chasers read (M), done

Phase 3 complete (2026-10-20): 3.1-3.9 all done; 3.9 is MRMS radar-only
1-h and 3-h rainfall plus precipitation type (PrecipFlag) in the MRMS layer.

1. **SPC outlooks:** Day 1 categorical and the tornado/wind/hail
   probabilities as a map layer, from SPC's GeoJSON, with the standard SPC
   colours.
2. **Watches:** active tornado and severe thunderstorm watch outlines
   (from SPC's watch GeoJSON, or the zone outlines from Phase 1.3), drawn
   under warnings.
3. **Mesoscale discussions:** outline plus full text, since an MD often
   comes 30-60 min before a watch.
4. **Full warning text:** show the complete NWS text in the warning detail
   panel (hazard, source, impacts, and the tornado/hail tags), not just
   the headline.
5. **WPC outlooks:** Excessive Rainfall Outlook (Day 1-3) and QPF as map
   layers (WPC publishes GeoJSON/KML/shapefiles).
6. **Special weather statements on the map:** draw SWS polygons (most
   significant-weather SWS have one), styled under warnings.
7. **Archived NWS bulletins:** in archive mode, the warnings, watches, SWS
   and MDs that were active at the scan on screen (IEM's archive of NWS
   text and polygons), so a past event replays with its products.
8. **Native storm-report layer:** Local Storm Reports (tornado, hail, wind,
   flood) with type icons, time and remarks on hover, live and in archive
   (IEM LSR GeoJSON); also the "storm reports" placefile from Phase 2.
9. **Precipitation products:** RadarScope's Precipitation Depiction and
   One-/Three-Hour Accumulation (Level 3 DPR/DAA/DTA, or MRMS QPE).

## Phase 4: lightning (M), GOES GLM, done 2026-10-20

What we have: MRMS NLDN flash density, drawn as bolts. Paid apps show
individual strikes with age fading. Options:

- **GOES GLM** (free, NOAA on AWS): satellite flash detections every 20 s,
  covering the US. Coarser (about 8 km) and with some delay, but free and
  real-time enough. Recommended.
- **A paid ground network** (Vaisala, Earth Networks): exact strikes and
  cloud-to-ground typing, but costs money and needs a licence.

Plan with GLM: decode the Level 2 flash files (netCDF), draw flashes
that fade over 15 min, and add an optional "strike within N mi"
notification that ties into Phase 1's alerting.

Done: `e5d1a40` (GOES-East, bolts matching the MRMS ones, 10-mi alert),
`5ef66d2` (GOES-West added, both default on; set-once options moved to a
Settings window at the owner's request). See ADR-0015. Bolts now draw at
any zoom (built-in layers use threshold Infinity) and icons are decluttered
per layer (topmost per 4 px cell): 20k clustered bolts draw ~5-15 ms.

## Phase 5: severe-weather model fields and soundings (M-L)

5.1 done 2026-10-20 (`f9ade59`): SBCAPE, MLCAPE, SRH 0-1/0-3, 0-6 km shear (kt), LCL, fixed-layer STP on the map with a colour key.

5.3 done 2026-10-20 (`7262c3a`): RTMA as a third model in the Forecast drawer; T/Td (°F), wind speed and gust (kt) on the map, hourly in-place refresh.

5.4 done 2026-10-20: observed balloon soundings, "Observed" tab in the sounding panel, nearest site, IEM (ADR-0017).

5.2 done 2026-10-20 (`b0a292a`): point sounding (Course block, "Sounding at a point") in the right panel: skew-T, hodograph, SBCAPE, LCL. Data sources and fallback in ADR-0016.

`provider-hrrr` already exists for simulated reflectivity.

1. **HRRR fields:** surface-based and mixed-layer CAPE, 0-1 km and 0-3 km
   storm-relative helicity, 0-6 km bulk shear, significant tornado
   parameter (STP), and lifting condensation level (LCL) height. Use the
   same forecast drawer and on-map display as forecast radar.
2. **Point sounding:** tap the map for an HRRR forecast sounding
   (skew-T plus hodograph) at that point and time. This is how chasers
   pick targets.
3. **RTMA:** the Real-Time Mesoscale Analysis (hourly 2.5 km surface
   temperature, dewpoint, wind, pressure) as map fields, through the same
   GRIB2 path as HRRR (NOAA's RTMA on AWS).
4. **Observed soundings:** the 00Z/12Z balloon soundings (RAOB) near the
   view, same skew-T/hodograph view as 5.2.

## Phase 6: export (S-M)

6.1 and 6.2 done 2026-10-20: "Save image" (PNG) and "Save loop" (MP4, WebM fallback) under the legend; `ui/ExportPanel.tsx`, `MapView`'s `exportRef`. Forecast-hour loops not exported yet.

1. **Image export:** the current map with radar, overlays, legend, and a
   site/time stamp, as a PNG.
2. **Loop export:** the current loop as an animated GIF or MP4, for
   sharing with spotter networks and social media.

## Phase 7: heavy rendering (L each)

1. **Multi-radar composite:** a seamless mosaic of nearby radars (closest
   radar or highest value per pixel). The per-view renderer from
   2026-10-20 is the foundation: render each site in the same Mercator
   frame and combine them. This is the bigger chasing win of the two.
2. **3D volume view:** all tilts as a 3D volume (hail cores, BWER, hook
   structure). Large, separate WebGPU work; do it last.

## Phase 8: observations and map layers (M-L)

1. **GOES satellite:** GOES-East/West imagery (visible, clean IR, water
   vapor) as a map layer under the radar, looped with time (NOAA's GOES
   buckets on AWS, or a pre-rendered tile service).
2. **METAR station plots:** temperature, dewpoint, wind barb and sky cover
   at airports in view (aviationweather.gov data API, through the same
   desktop command / dev proxy as placefiles if it lacks CORS).
3. **Satellite basemap:** a satellite-imagery basemap option alongside the
   dark map (check the imagery licence for an app).
4. **Detailed road layer:** county and local roads with names at chase
   zoom, on top of the radar, as a toggle.

## Phase 9: MRMS shear and hail contours (M)

1. **Rotation tracks:** MRMS azimuthal shear (0-2 km and 3-6 km AGL,
   30/60/120 min) as contour lines on the map.
2. **Hail contours:** MRMS MESH (and its 30/60/120 min tracks) as
   contoured hail size, instead of only a grid.

## Phase 10: multi-pane display (M-L)

Two or four linked panes (same map view, cursor and time) each showing a
different product or tilt, like RadarScope's split display. The per-view
renderer draws one view today; each pane needs its own renderer view.

## Phase 11: release (S-M)

1. **Code signing:** decided against earlier because of cost. Revisit
   before sharing the app widely, since unsigned installers trigger
   Windows SmartScreen warnings.
2. **Auto-update:** the updater plugin is wired in. Do one real release
   through it end to end.
3. **Settings polish:** remember the remaining settings (storm tracks
   toggle, opacity in the browser), and do a first-launch walkthrough of
   location and notification permissions.
4. **Settings sync** (RadarScope's cross-platform sharing equivalent):
   export/import settings, favorites, color tables and placefiles as one
   file, so desktop and browser can share them.

---

## Decisions needed from the owner

1. ~~Weak-echo REF~~: keep the fade (decided 2026-10-20).
2. ~~Lightning source~~: free GOES GLM (decided 2026-10-20).
3. ~~Placefile suggestions~~: Spotter Network positions + IEM storm
   reports (decided 2026-10-20).
4. ~~Code signing~~: wait until the app is shared beyond the owner
   (decided 2026-10-20).
5. ~~Phase order~~: Phase 0, then placefiles (Phase 2) (decided
   2026-10-20).
