# Vendored data

`wsr88d-sites.json` is a byte-for-byte copy of
`fixtures/nexrad-sites/wsr88d-sites.json` (see that file's own
`README.md` for full provenance: fetched from NOAA/NWS's official
`api.weather.gov/radar/stations` API, 159 real WSR-88D sites, not
hand-transcribed).

It is vendored here (rather than `fetch()`ed at runtime from the repo's
`fixtures/` directory) because `fixtures/` is not part of the deployed
`apps/web` static build output and there is no reason to add a runtime
dependency on it just to populate a `<select>`.

To refresh after `fixtures/nexrad-sites/wsr88d-sites.json` changes, re-copy
it:

```sh
cp fixtures/nexrad-sites/wsr88d-sites.json apps/web/src/data/wsr88d-sites.json
```

There is no transformation step -- this file must stay byte-identical to
the fixture (or a strict superset produced by the same real NOAA fetch
process), never hand-edited.
