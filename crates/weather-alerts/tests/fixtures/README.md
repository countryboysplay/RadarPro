# NWS Alerts Fixture Data

Real, unmodified NWS alerts data, fetched directly from the official public
API during development -- not invented, not hand-transcribed. Matches this
project's established "ground-truth real data where practical" discipline
(see `fixtures/nexrad-level2/README.md` and `fixtures/nexrad-sites/README.md`).

## Files

- `alerts-active-ok-20260912.json` -- the full, unmodified response from
  `https://api.weather.gov/alerts/active?area=OK`, fetched 2026-09-12
  (13 features at fetch time: a mix of `messageType: "Alert"` and
  `messageType: "Update"`, all `status: "Actual"`, `Polygon` geometry on
  every feature that had one). Exercised by `tests/parse.rs`'s
  `parses_real_nws_fixture_without_errors` and
  `real_fixture_update_message_carries_references` to prove this crate's
  parser against real production data shapes, not only hand-constructed
  JSON.

## Fetch requirements

Per NWS's API usage policy, every request to `api.weather.gov` must send a
descriptive `User-Agent` header identifying the calling application and a
contact method -- unlike the anonymous NOAA S3 bucket this project also
uses (`crates/radar-cache`), which has no such requirement. This fixture
was fetched with:

```
User-Agent: RadarPro (github.com/countryboysplay/RadarPro, open-source project)
```

Any code that fetches this endpoint live (this crate's own future
`apps/web` polling loop included) must set an equivalent header, or NWS may
reject or rate-limit the request.

## What real data did *not* cover

At fetch time, neither a real `messageType: "Cancel"` message nor a real
`MultiPolygon` geometry appeared in either the `?area=OK` query or a full
national `/alerts/active` snapshot fetched the same session (275 features
nationwide: 211 `null` geometry, 64 `Polygon`, 0 `MultiPolygon`; messageType
split 178 `Alert` / 97 `Update` / 0 `Cancel`). This is expected, not a
sampling error: NWS's `/alerts/active` feed only ever shows currently-active
alerts, a `Cancel` message's whole purpose is to make the alert it
references stop being active (so a `Cancel` that has already been acted on
elsewhere is not expected to still be sitting in this endpoint's response),
and `MultiPolygon` geometry is real but comparatively rare in NWS's CAP
output. `tests/parse.rs`'s `cancel_message_scenario` and
`multipolygon_geometry_parses_with_distinct_rings` therefore use
hand-constructed JSON instead -- built to the exact real CAP/GeoJSON shape
confirmed by the fetched fixtures above (same field set, same
`references[]` shape, same coordinate nesting), not guessed independently.
A real `status: "Test"` (`"KEEPALIVE"`) message *was* observed in the
national snapshot and is what grounds this crate's `status != "Actual"`
filtering decision (see `src/parse.rs`'s module docs).
