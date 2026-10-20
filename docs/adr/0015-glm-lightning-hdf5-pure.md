# ADR-0015: GOES GLM lightning: `hdf5-pure` for netCDF4 in the browser, two satellites split at 106 W

Status: Accepted

## Context
Phase 4 adds live lightning from the GOES Geostationary Lightning Mapper
(GLM). NOAA publishes one Level 2 LCFA file every 20 s per satellite on
public, CORS-open S3 buckets: `noaa-goes19` (GOES-East, 75.2 W) and
`noaa-goes18` (GOES-West, 137.2 W). Files are netCDF4, which is HDF5. Like
radar, MRMS and alerts, decoding must live in Rust (ADR-0001) and run in the
browser as wasm.

The established crates (`hdf5`, `netcdf`) bind the HDF5/netCDF C libraries,
which do not build for `wasm32-unknown-unknown`.

## Decision
- Decode with `hdf5-pure` `=0.47.0` (pure Rust, MIT/Apache-2.0) in
  `crates/glm`. It adds only `byteorder`; its deflate support uses the
  workspace's existing `flate2`. Only the flash level is read: `flash_lat`,
  `flash_lon`, `flash_time_offset_of_first_event` (16-bit, flagged
  `_Unsigned`, with `scale_factor`/`add_offset`) and `product_time`
  (seconds since 2000-01-01 12:00 UTC). A real file is the test fixture
  (`fixtures/glm/`).
- The browser (`apps/web/src/lightning/glm.ts`) polls each enabled satellite
  every 20 s with one S3 listing (`start-after` the newest key seen) and
  downloads only new files, keeping the last 15 minutes. Flashes are kept
  only inside a North America box per satellite (most GOES-East flashes are
  over South America).
- Both satellites are on by default, each with its own toggle (owner's
  call). With both on, flashes west of 106 W come from GOES-West and east of
  it from GOES-East, so no flash is drawn twice. On a test night
  (2025-08-01 01Z) GOES-West caught 487 flashes over the western US to
  GOES-East's 330; east of the Plains GOES-East caught 323 to 80.

## Consequences
- Lightning works the same in the web app and the desktop shell, with no
  native dependency.
- Data use is about 11 MB for the first 15 minutes and about 45 MB an hour
  per satellite while lightning is on.
- `hdf5-pure` is young and pinned exactly; a format change NOAA makes would
  show as "N of M lightning files failed" under the lightning toggles, not a
  crash.
