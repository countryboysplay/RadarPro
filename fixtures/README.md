# Fixtures

Real NEXRAD Level II (Archive II) test fixtures used by `nexrad-level2` and
`radar-cli` integration tests.

## nexrad-level2/

Full, unmodified Archive II Level II volume files downloaded from the
Unidata NEXRAD Level II public archive on AWS S3
(`https://unidata-nexrad-level2.s3.amazonaws.com/`, successor to the
retired `noaa-nexrad-level2` bucket as of September 2025). Public domain
NOAA/NWS data, no redistribution restrictions.

| File | Site | ICAO | Volume start (UTC) | Format version | Size |
|---|---|---|---|---|---|
| `KTLX20240601_000353_V06` | Twin Lakes, OK (KTLX) | KTLX | 2024-06-01 00:03:53 | V06 (Super Resolution, Build 12.0+, dual-pol) | ~4.7 MB |
| `KFTG20240601_000116_V06` | Denver/Boulder, CO (KFTG) | KFTG | 2024-06-01 00:01:16 | V06 (Super Resolution, Build 12.0+, dual-pol) | ~7.0 MB |
| `KTLX20260912_203032_V06` | Twin Lakes, OK (KTLX) | KTLX | 2026-09-12 20:30:32 | V06 (Super Resolution, dual-pol) | ~11.2 MB |

Chosen only for being ordinary, unremarkable full volumes from two
different sites — not a severe-weather case study. Ground-truthed by hand
against the ICD before use: Volume Header Record text, extension number,
NEXRAD-modified Julian date, milliseconds-of-day, and ICAO all decode as
expected, and the first LDM Compressed Record's bzip2 payload decompresses
to exactly 325,888 bytes (134 × 2432), matching the Archive II/User ICD
(2620010E) metadata-record size exactly.

`KTLX20260912_203032_V06` is kept for a different reason: unlike the two
2024 fixtures above, it is a real, live volume containing a Message Type
32 (RDA PRF Data) record — the gap that caused `decode_volume` to fail
with "unsupported message type 32" against every live WSR-88D scan before
that message type (and 33, RDA Log Data) were added to the recognized
fixed-slot legacy metadata set (see
`crates/nexrad-level2/src/message.rs`'s `is_legacy_metadata_message_type`
doc comment for the full empirical writeup). Downloaded fresh from the
public Unidata NEXRAD Level II S3 bucket
(`https://unidata-nexrad-level2.s3.amazonaws.com/2026/09/12/KTLX/KTLX20260912_203032_V06`)
on 2026-09-12 specifically to exercise this framing fix end-to-end against
real bytes, not a synthetic test double.

More/varied fixtures (different VCPs, legacy pre-dual-pol volumes, and
deliberately truncated/corrupted files for error-handling tests) should be
added as S01 test coverage grows.
