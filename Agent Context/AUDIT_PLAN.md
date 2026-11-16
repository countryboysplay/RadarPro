# Audit fix plan (written 2026-11-16)

From the whole-repo audit of commit `0173d47`. Five reviewers each covered one
area. Every finding has an ID by area:

- **D** desktop app, CLI, CI and config
- **B** binary decoders (nexrad-level2, mrms, glm, radar-types)
- **R** radar and alerts crates (radar-cache, radar-geo, radar-render,
  radar-web, mrms-web, weather-alerts)
- **F** forecast crates (forecast-core, provider-hrrr, provider-gefs,
  forecast-web)
- **W** web app (`apps/web`)

The work is grouped into batches, most important first. Each batch is one or
a few commits. Before every commit, run `/ponytail:ponytail-review` and fix
what it finds. The coverage table at the end lists every finding and its
batch.

Size: S = under an hour, M = a few hours, L = a day or more.

## Status (updated 2026-11-16)

Batches 1-2 are on `main` and pushed (CI green on `8c26bef`). Continue on `main`.

| Batch | State | Commit |
|-------|-------|--------|
| 0 | Done: `apps/` restored with `git restore apps` | none needed |
| 1 | Done, all 8 items | `ef0d092` |
| 2 | Done, all 7 items | `1fc8862` |
| 3-8 | Not started; next is batch 3 | |

Notes from batches 1-2 that change later batches:

- Batch 1.8 (F2): GEFS discovery now checks the mean at **+6h** (the
  panel's default view), not +0h.
- Batch 2.6 (F9) was done in Rust with a fetch sequence number in
  forecast-web (`fetch_seq`), with no JS change.
- Batch 2.3 (R4): KDP is offered wherever PHI is
  (`render_select::moments_present_in`) and derived per sweep on first view
  (`browser.rs` `kdp_sweeps`). `defaultSweepIndexForMoment("KDP")` maps to PHI.
- Batch 2.2 (R3): `nexrad_level2::decode_volume_structure` decodes without
  gates; it shares `decode()` with `decode_volume`.
- Batch 4.1 (B2) is now cheaper: R2 and R3 are done.
- Not yet verified: loop export (2.7) was not run in the browser, because
  it downloads a file.
- The native GPU tests (`forecast-core` gpu tests, `provider-hrrr
  cross_provider_render`) crash on this PC's GPU driver with or without the
  changes. GPU paths were checked in the browser instead.

## Batch 0: restore `apps/` (owner, before anything else)

The working tree has the whole `apps/` folder (140 files) deleted but not
committed. Every fix below in `apps/` needs it.

- If the deletion was a mistake: `git restore apps`.
- If it was on purpose (a move to another repo?), say so. Then the W and D
  items move with the code, and this plan drops them.

## Batch 1: wrong or alarming behaviour users see today (S-M)

1. **W1: the covering-alert alarm repeats.** (`App.tsx:662-684`, `:772-785`,
   `location/useMyLocation.ts:65-68`, `storms/useStormTracks.ts:128-130`)
   - Fix: stop replacing `notifiedKeysRef` with `covering`. Keep a key until
     its alert is gone from `alerts.activeAlerts`:
     `new Set([...prev].filter(k => activeKeys.has(k)).concat(covering.map(c => c.key)))`.
   - In `useAlertIdsAtPoint`, keep the last ids until the new fetch answers.
     Don't `setIds(new Set())` each time the point moves.
   - Storms: only drop a storm from `notifiedStormsRef` when it really
     leaves, not when the nearby-radar list resets `bySite`.
   - Test: pull the bookkeeping into a pure function (`nextNotified(prev,
     covering, activeKeys)`) and unit-test this sequence: a watch covers you,
     the point moves, the ids are briefly empty, the ids come back. It must
     notify once.
   - Verify in the browser with a simulated GPS track crossing a polygon
     edge, the same way Phase 1.4 of PLAN.md was tested.
2. **W2: the forecast map and "Valid" time drift from the chosen hour.**
   (`forecast/useForecastProvider.ts:332-362`, `:449-461`,
   `ForecastPanel.tsx:200`)
   - Add `requestedLeadRef`, set in `setLeadHours`. In `fetchAndRender`,
     paint and `setState` only when the frame's lead equals it.
   - Store `{ grid, imageData }` in `framesRef`, and set `grid` in the cached
     branch too. Make `loadLoop` keep the grid it already gets.
   - Test: step to an uncached hour, then back to a cached one before the
     first one finishes. The map and label must stay on the cached hour.
3. **R1: the alert store keeps every change forever.** (`weather-alerts/src/store.rs`)
   - Delete `pending_changes`, `drain_changes`, the wasm `drainChanges`, and
     both `extend(changes.clone())` lines. Fix the doc comments that mention
     them (`json.rs:208`, `store.rs:71,127,229,251,319-323`, `wasm_api.rs:144,183`).
   - Change the 4 uses in `tests/lifecycle.rs` to use the return values of
     `ingest_poll` and `expire_stale`.
4. **D1: no HTTP timeout on desktop.** (`apps/desktop/src-tauri/src/rainbow.rs:172`)
   - Add `.connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30))`
     to the shared client. `fetch_url` keeps its own 20 s timeout per request.
5. **D4: tgftp errors are called "Rainbow Weather" errors.** (`nws_products.rs:21,25`)
   - Use the `unreachable` closure that `fetch_url` already has. This touches
     the same lines as D1, so do it in the same commit.
6. **W3: clearing the Rainbow key leaves the overlay stuck on.**
   (`rainbow/useRainbowOverlay.ts:140-146`)
   - In the `!configured` branch: `setEnabled(false)`,
     `setTileUrlTemplate(null)`, `setStatus("unconfigured")`.
7. **F1 + B5: discovery hides network errors.** (`provider-hrrr/src/client.rs:195`,
   `provider-gefs/src/client.rs:212`, `mrms/src/client.rs:177`)
   - Change `if let Ok(..)` to a `match`: on `Ok(found)` return, on
     `Ok(absent)` walk back, on `Err` return the error. RTMA already does this.
   - Test: a fake lister that fails for day 0 must return the error, not
     yesterday's snapshot or "no run found".
8. **F2: GEFS picks a run before its files are readable.** (`provider-gefs/src/client.rs:122-150`)
   - Filter on `.idx` keys and require the mean member to be present.
     HRRR's `find_recent_run`: check `idx_key` instead of `object_key`.
   - Test: a listing with `gec00…f000.grib2` but no `geavg…f006.idx` must not
     pick that run.

## Batch 2: hangs and slow paths (M)

1. **F3: a new GPU device and shader for every forecast frame.** (`forecast-core/src/gpu.rs:367-405`)
   - Keep `Option<(GpuContext, GridPipeline)>` in forecast-web's
     `ProviderState`. Create it on first render, reuse it after that. Upload
     only the texture and uniforms per frame.
   - Verify: a 49-hour HRRR loop loads with no hang. Time it before and after.
   - Then check whether the hang workarounds in `useForecastProvider.ts` are
     still needed. Delete them if not.
2. **R3: live chunks are decoded twice.** (`radar-web/src/browser.rs:1183-1188`)
   - Add a header-only sweep counter to nexrad-level2 that reads the
     elevation numbers without decoding gates. Use it in `completeSweepCount`.
   - Test: on the `KTLX_20261019_172040_chunks_1-4.bin` fixture, the counter
     matches today's full-decode count.
3. **R4: KDP is computed for every volume.** (`radar-web/src/browser.rs:511`)
   - Remove `add_kdp` from `decodeVolume`. Compute it per sweep, only when the
     moment is KDP, the way `transform_sweeps_if_enabled` handles dealiasing.
   - Check that the cross-section and the probe still get KDP.
4. **R2: each sweep upload clones every radial.** (`radar-render/src/sweep_buffers.rs:150-176`,
   `radar-web/src/browser.rs:1038-1047`)
   - `build_radial_lookup` takes azimuths (`radial_meta[i].azimuth_deg`).
     Store the azimuth resolution as a field. Delete `source_radials`.
5. **F8: STP inputs are fetched one at a time.** (`provider-hrrr/src/provider.rs:122-139`)
   - Fetch all specs at once with `try_join_all`. Check whether `futures` is
     already in the tree first; if it isn't, `join!` on a fixed list will do.
6. **F9: an old fetch can overwrite the grid the next render uses.**
   (`forecast-web/src/wasm_api.rs:294-333`)
   - Give `renderCurrentGrid` the variable and lead it expects, and reject a
     mismatch. W2's `requestedLeadRef` covers the JS side.
7. **W5: loop export runs out of memory, and an error locks the buttons.**
   (`ui/ExportPanel.tsx:40-61`, `map/MapView.tsx:819-845`)
   - Downscale each frame to at most 1920 px wide when it is captured.
   - Wrap the encode in `try { … } finally { setBusy(null) }`.

## Batch 3: input from the network (S-M)

The data comes from NOAA and NWS, and the project treats it as untrusted.
Placefiles come from anyone.

1. **D2: a placefile can reach your LAN.** (`nws_products.rs:33-51`)
   - After `Url::parse`, reject `localhost` and IPs that are loopback,
     private, link-local or unspecified. Resolve hostnames and check those
     IPs too. Use a custom `redirect::Policy` that runs the same check.
   - Test: `http://127.0.0.1/`, `http://192.168.1.1/` and `http://[::1]/` are
     rejected; a public URL passes.
   - The web build's dev proxy (`vite.config.ts` `fetchUrlProxyPlugin`) is dev
     only. Leave it.
2. **B1: the MRMS grid is sized from untrusted `ni × nj`.** (`mrms/src/decode.rs:270-272, 318-319`)
   - `checked_mul` plus a cap (for example 100M cells) before allocating.
     Return `GridSizeMismatch`. Test with a hand-made header.
3. **B3: decompression has no size limit.** (`nexrad-level2/src/ldm.rs:126-135`,
   `mrms/src/client.rs:226-236`)
   - `reader.take(LIMIT).read_to_end(..)`, and fail if it hits the limit:
     64 MB per LDM record, 256 MB for MRMS.
4. **B4: a non-ASCII S3 key panics MRMS discovery.** (`mrms/src/keys.rs` `parse_snapshot_key`)
   - Use `.get(0..4)?` (and so on) instead of `[0..4]`. Test with `202é913`.
5. **B6: duplicate data-block pointers multiply decode work.**
   (`nexrad-level2/src/message31.rs:123-183`)
   - Reject `data_block_count > 16`. A test with 17 pointers must error.
6. **B7: no cap on radials per volume.** (`nexrad-level2/src/lib.rs:178-191`)
   - Error past 20,000 radials total. Real volumes have about 7,000.
7. **CSP (D note, no finding).** `tauri.conf.json` has `csp: null`. There is
   no XSS sink today, so this is defence in depth. Optional: a CSP allowing
   `'wasm-unsafe-eval'` and `blob:` workers. Test the desktop app fully
   afterwards, because MapLibre and WASM break easily under a CSP.

## Batch 4: radar memory (L)

1. **B2: 8 bytes per gate.** (`radar-types/src/lib.rs:406-433`,
   `nexrad-level2/src/message31.rs:404-412`)
   - Step 1 (M): store gates as `Vec<f32>`, with Missing and RangeFolded as
     two fixed NaN payloads plus a `get(i) -> GateValue` accessor. Half the
     memory, few callers change.
   - Step 2 (only if memory is still a problem): keep the raw `u8`/`u16`
     codes and decode on access.
   - Do R2 and R3 first; they are cheaper and remove the biggest copies.
   - Measure: the decoded-volume size for the KTLX fixture before and after.
2. **R8: dealiasing allocates per-gate structures.** (`radar-geo/src/dealias.rs:124-211`)
   - Union neighbours while scanning, without storing `pairs`, on a flat
     `Vec<f32>` grid. The existing dealias tests must still pass unchanged.

## Batch 5: data that goes stale (S-M)

1. **W4: Rainbow and MRMS never refresh on their own.**
   (`rainbow/useRainbowOverlay.ts:145-205`, `mrms/useMrmsOverlay.ts:366-466`)
   - Rainbow: add a 5-minute tick to the resolve effect's dependencies, and
     show the snapshot time like MRMS does.
   - MRMS: call `runPipeline` every 5 minutes while enabled. Keep the old
     image until the new one is ready.
2. **R5 side note: live radar at UTC midnight.** (`apps/web/src/nexrad/bucket.ts:143-146`)
   - `discoverLatest` lists only today's UTC prefix. Between 00:00Z and the
     first scan of the new day, it may find nothing.
   - Check first whether the live chunk feed already covers this. If not,
     fall back to yesterday's prefix when today's is empty.

## Batch 6: CI and tests (S)

1. **D3: CI never runs the desktop crate's checks.** (`.github/workflows/ci.yml`)
   - Add a `windows-latest` job: `cargo fmt --check`, `cargo clippy -- -D warnings`
     and `cargo test`, all with `--manifest-path apps/desktop/src-tauri/Cargo.toml`.
     Fix whatever it turns up.
2. **reqwest `json` feature (D note).** Add `"json"` to the desktop
   `reqwest` features. It works today only because another crate enables it.
3. **serde_json (D note).** Move it to `[dev-dependencies]` in the desktop
   crate if it is still used only in tests.
4. **B9: no test for MRMS 2×2 pooling.** (`mrms/src/decode.rs:288-300, 377`)
   - A synthetic 4×2 grid with `pool = 2`. Assert the origin shift and the
     index mapping.
5. **W1's test** is in batch 1.

## Batch 7: lean (M, one commit per item)

Do these after batches 1-3, so fixes don't land in code about to move.

1. **R5: delete the `radar-cache` crate.** (about 2,175 lines)
   - Remove `crates/radar-cache` and its line in the root `Cargo.toml`.
   - Fix the comments that point to it: desktop `Cargo.toml:60`,
     `rainbow.rs:130`, `forecast-web/Cargo.toml:40`, `provider-gefs/Cargo.toml:33,58,73`,
     `radar-render/Cargo.toml:56`, `weather-alerts/Cargo.toml:25,29`.
   - Update the README layout and any ADR that lists it.
2. **B8 + R6 + F5: one copy of the S3 and idx code.** (about 600 lines)
   - Keep the HRRR versions of `idx.rs`, `xml.rs`, and the list, get,
     byte-range, retry and crypto-setup client code. Move them into
     `forecast-core` behind one small error type that each crate maps to its
     own.
   - Point GEFS and MRMS at them. GEFS keeps only `idx_names`, its lat/lon
     geometry, `ensemble.rs` and `time_range_extra_hours`.
   - Optional, after that: the ~120-line shared decode skeleton in the two
     `decode.rs` files.
   - Every existing provider test must pass unchanged.
3. **R7: calendar math copied.** Expose `days_from_civil` and
   `civil_from_days` from `radar-types`. Delete the copies in
   `weather-alerts/src/time.rs` and `forecast-core/src/time.rs` (the
   `radar-cache` copy goes with item 1). Add one leap-year test there.
4. **D5: Rainbow tiles go over IPC as base64.** Return
   `tauri::ipc::Response::new(bytes)`; in `desktopTiles.ts`, use
   `invoke<ArrayBuffer>`. Delete the `base64` dependency, its comment block,
   and the unused `content_type`.
5. **D6: Rainbow request code is copied 4 times.** One
   `async fn get_json<T: DeserializeOwned>(url) -> Result<T, String>`.
6. **W6: small JS helpers are copied.** Keep `withTimeout` from
   `rainbow/rainbowAsync.ts`, one `errMessage`, and one `time` formatter.
   Delete the rest.
7. **W7: MapView repeats the canvas buffer sync 4 times.** One
   `fit(canvas, dpr)` helper in `MapView.tsx`.
8. **W8: notification logic lives in `App.tsx`.** Move it into
   `useLocationNotifications`. This holds W1's pure function.
9. **Dead code: R9 + F6 + B10.**
   - Delete `parseAlertsJson` (`weather-alerts/src/wasm_api.rs:59-113`, plus
     its re-export in `alerts/wasmModule.ts`).
   - Delete `ForecastVariable::grib2_parameter`,
     `ForecastGrid::kelvin_to_fahrenheit`, and `idx::find_message` (test-only)
     in both providers.
   - Have `GenericMessageHeader` read only `message_type`.
   - Grep once more right before deleting each one.
10. **F7: `image` is a normal dependency of provider-hrrr.** Move it behind a
    `harness` feature (`required-features` on the bin) and into
    `[dev-dependencies]` for the test.
11. **D7: `apps/radar-cli`.** Owner decides. If it is unused, delete it and
    its workspace line, and update the README, ADR 0001 and
    `fixtures/README.md`. That also drops `clap`.
12. **D8: stale release-workflow comment.** (`desktop-release.yml:108-112`)
    Correct it to say updater artifacts are off, to match `apps/desktop/README.md`.

## Batch 8: check first, fix only if real (S each)

1. **F4: no longitude wraparound on lat/lon grids.** (`forecast-core/src/grid.rs:63-77`,
   `forecast_grid.wgsl`)
   - Latent: today's callers shift to 0..360 first.
   - Fix it if GEFS ever draws on the map outside the fixed CONUS camera:
     `rem_euclid(360.0)` in Rust and `fract` in the shader. Add a test with
     lon -97.5 that today returns `None`.
2. **GLM fill values (B note).** Check the fixture's `_FillValue` on
   `flash_time_offset_of_first_event`. If it is -1, `raw as u16` gives a
   flash time about 25 s late. Skip fill values if so.
3. **HRRR shear units (F note).** Check whether VUCSH/VVCSH are in m/s or
   1/s. STP and the shear display depend on it. Compare one point against a
   published HRRR shear map.
4. **NWS Update before original (R note).** Check whether
   `/alerts/active` ever lists an alert and its update in one response, with
   the update first. If so, a new store makes two polygons. Order the poll by
   `sent` before ingesting.
5. **`grib` crate lat/lon cost (F note).** Check whether
   `Template30::latlons()` is lazy. If it is eager, every HRRR decode
   computes 1.9M points just to get the spacing. Read the spacing from
   Section 3 instead.

## Coverage table

| ID | Finding | Batch |
|----|---------|-------|
| D1 | No HTTP timeout on desktop | 1.4 |
| D2 | `fetch_url` reaches LAN | 3.1 |
| D3 | Desktop crate not in CI | 6.1 |
| D4 | tgftp errors called Rainbow errors | 1.5 |
| D5 | Base64 tiles over IPC | 7.4 |
| D6 | Rainbow request code ×4 | 7.5 |
| D7 | radar-cli unused | 7.11 (owner) |
| D8 | Stale release comment | 7.12 |
| D notes | CSP, reqwest json, serde_json | 3.7, 6.2, 6.3 |
| B1 | MRMS grid alloc from `ni × nj` | 3.2 |
| B2 | 8 bytes per gate | 4.1 |
| B3 | No decompression limit | 3.3 |
| B4 | Non-ASCII key panic | 3.4 |
| B5 | MRMS discovery hides errors | 1.7 |
| B6 | Duplicate block pointers | 3.5 |
| B7 | No radial cap | 3.6 |
| B8 | Copied S3 helpers | 7.2 |
| B9 | No pooling test | 6.4 |
| B10 | Unread header fields | 7.9 |
| B note | GLM fill value | 8.2 |
| R1 | Alert change log grows | 1.3 |
| R2 | Radial clone per upload | 2.4 |
| R3 | Double decode per chunk | 2.2 |
| R4 | KDP always computed | 2.3 |
| R5 | radar-cache dead (+ midnight note) | 7.1, 5.2 |
| R6 | XML parser ×4 | 7.2 |
| R7 | Calendar math copied | 7.3 |
| R8 | Dealias memory | 4.2 |
| R9 | `parseAlertsJson` unused | 7.9 |
| R note | Update before original | 8.4 |
| F1 | Discovery hides errors | 1.7 |
| F2 | GEFS run not ready | 1.8 |
| F3 | GPU device per frame | 2.1 |
| F4 | No longitude wrap | 8.1 |
| F5 | HRRR/GEFS plumbing copied | 7.2 |
| F6 | Dead forecast code | 7.9 |
| F7 | `image` dependency | 7.10 |
| F8 | STP fetches in series | 2.5 |
| F9 | Stale `last_grid` | 2.6 |
| F notes | Shear units, latlons cost | 8.3, 8.5 |
| W1 | Alarm repeats | 1.1 |
| W2 | Forecast hour drift | 1.2 |
| W3 | Rainbow stuck on | 1.6 |
| W4 | Overlays never refresh | 5.1 |
| W5 | Loop export memory / stuck buttons | 2.7 |
| W6 | JS helpers copied | 7.6 |
| W7 | Canvas sync ×4 | 7.7 |
| W8 | Notifications in App.tsx | 7.8 |

## Owner decisions

1. Batch 0: restore `apps/`, or confirm it was deleted on purpose.
2. D7: keep or delete `apps/radar-cli`.
3. Batch 3.7: add a CSP now, or later.
4. Batch 4.1: stop at step 1 (`f32`), or go on to raw codes.
