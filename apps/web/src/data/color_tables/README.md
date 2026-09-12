# Vendored default color tables

Byte-for-byte copies of `crates/radar-render/color_tables/*.json` (this
project's six shipped default color tables -- one per moment: REF, VEL, SW,
ZDR, CC, PHI). See `crates/radar-render/COLOR_TABLE_FORMAT.md` for the
format itself.

Vendored here (rather than `fetch()`ed at runtime from the repo's
`crates/` directory) for the same reason `src/data/wsr88d-sites.json` is
vendored -- see `src/data/README.md`: `crates/` is not part of the deployed
`apps/web` static build output. `src/colorTables/defaultColorTables.ts`
imports each file with Vite's `?raw` suffix to get its exact original text
(the same bytes `RadarWebRenderer.loadColorTable` would accept), used both
as the color-table editor's selectable presets and as a readable reference
copy -- never hand-edited or transformed.

To refresh after `crates/radar-render/color_tables/*.json` changes, re-copy:

```sh
cp crates/radar-render/color_tables/*.json apps/web/src/data/color_tables/
```
