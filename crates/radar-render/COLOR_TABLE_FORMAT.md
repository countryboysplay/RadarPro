# RadarPro Color Table Format (version 1)

This is RadarPro's own, original color-table format: a JSON file describing
how to map one radar moment's physical values to RGBA colors for display.
It is implemented and validated by `crates/radar-render/src/color_table.rs`
(`ColorTable::from_json`), and is precise enough that a person can
hand-write a valid file from reading this document alone. See
`docs/adr/0009-original-color-table-format.md` for the design rationale.

It is not a port of any specific commercial radar tool's proprietary color
table format — see that ADR's "Alternatives" section for how it differs.

## Top-level shape

```json
{
  "format_version": 1,
  "name": "string",
  "moments": ["REF"],
  "units": "dBZ",
  "mode": "gradient",
  "domain": { "min": -10.0, "max": 75.0 },
  "cyclic": false,
  "stops": [
    { "value": -10.0, "color": [0, 0, 0, 0] },
    { "value": 75.0, "color": [200, 0, 0, 255] }
  ],
  "missing_color": [0, 0, 0, 0],
  "range_folded_color": [255, 0, 255, 255]
}
```

## Fields

| Field | Type | Required | Meaning |
|---|---|---|---|
| `format_version` | integer | yes | Must be `1` (this document's version). A file with any other value is rejected, never guessed at. |
| `name` | string | yes | Human-readable display name. Must not be empty (or all whitespace). |
| `moments` | array of strings | yes | Which moment(s) this table is compatible with ("product compatibility"). Each entry must be one of the wire codes below. Must contain at least one entry. |
| `units` | string | yes | The physical unit `stops[].value` is expressed in, e.g. `"dBZ"`, `"m/s"`, `"dB"`, `"deg"`, `"dimensionless"`. Documentation only — this format does not convert units; display-unit conversion is a presentation-layer concern, never applied to source data (`GLOBAL_CONTRACT.md`). Must not be empty. |
| `mode` | `"gradient"` \| `"stepped"` | yes | See "Mapping modes" below. |
| `domain` | object `{min, max}` | yes | The physical-value range the built color lookup table (LUT) covers. `min`/`max` must both be finite numbers with `min < max`. |
| `cyclic` | boolean | no (default `false`) | Whether `domain`/`stops` wrap around (e.g. PHI's 0–360 degrees, where 360 and 0 are the same angle). See "Cyclic quantities" below. |
| `stops` | array of `{value, color}` | yes | Control points. Must contain at least one entry, sorted **strictly ascending** by `value` (no duplicate values), each `value` finite and within `[domain.min, domain.max]`. |
| `missing_color` | `[r, g, b, a]` | yes | Color for a gate with no data (below signal threshold). See "Missing and range-folded colors" below. |
| `range_folded_color` | `[r, g, b, a]` | yes | Color for a gate whose range is ambiguous (range-folded). See "Missing and range-folded colors" below. |

Any field not listed above is rejected (the parser uses strict, "no unknown
fields" validation) — this catches a typo'd field name as a load error
instead of silently ignoring it.

### Moment wire codes

`moments` entries must be one of these six strings (matching
`radar_types::MomentKind::wire_code`):

| Code | Moment |
|---|---|
| `REF` | Reflectivity |
| `VEL` | Radial velocity |
| `SW` | Spectrum width |
| `ZDR` | Differential reflectivity |
| `CC` | Correlation coefficient |
| `PHI` | Differential phase |

A table may name more than one moment when the same ramp is meaningfully
shared; each of this project's own shipped defaults (`color_tables/`) names
exactly one.

### Colors

A color is always a 4-element array `[r, g, b, a]` of integers `0`–`255`
(RGBA8, alpha included — so a table can express partial transparency at
low-signal values, not just fully-opaque-or-fully-transparent). A value
outside `0`–`255`, or an array of the wrong length, is a load error.

### Mapping modes

- **`"gradient"`**: continuous linear interpolation (component-wise,
  including alpha) between the two stops bracketing a value. A value at or
  before the first stop clamps to the first stop's color (unless `cyclic`
  is `true`); a value at or after the last stop clamps to the last stop's
  color (same exception). This is the mode every one of this project's own
  shipped defaults uses.
- **`"stepped"`**: discrete color per value bucket — the traditional
  NWS-style reflectivity look. A value `v` in `[stops[i].value,
  stops[i + 1].value)` returns `stops[i].color` **unchanged** (no
  blending). Same first/last-stop clamping behavior outside the stop range
  as gradient mode (subject to the same `cyclic` exception).

A single-stop table (in either mode) is valid and produces one constant
color everywhere in `domain`.

### Cyclic quantities

Some moments are inherently cyclic — differential phase (`PHI`) is
0–360 degrees, where 360 and 0 name the same physical angle. Set
`"cyclic": true` for these. It changes sampling in two ways:

1. A physical value outside `[domain.min, domain.max)` is wrapped back into
   that range (`value = domain.min + (value - domain.min) mod (domain.max -
   domain.min)`) before mapping, rather than clamped to the nearest
   endpoint stop.
2. In `"gradient"` mode, the ramp interpolates **smoothly across the
   `domain.max`/`domain.min` seam** — e.g. a PHI value of 350 degrees
   blends toward whatever color is assigned to 360 (== 0) degrees, instead
   of clamping flatly to the last stop's color. In `"stepped"` mode this
   distinction does not apply (stepped mode never blends); `cyclic` only
   affects how an out-of-range input value is wrapped before stepping.

For a clean, seamless-looking cyclic gradient, give your first and last
stops (`value == domain.min` and `value == domain.max`) the same color, as
this project's own `PHI` default does (`color_tables/differential_phase.json`:
both `0.0` and `360.0` map to the same red).

### Missing and range-folded colors

`missing_color` and `range_folded_color` are **not** stops — they are
dedicated top-level fields, structurally separate from `stops`. This is
deliberate: the code that maps a *physical value* to a color
(`color_table::build_lut_from_table` and everything it calls) only ever
reads `stops`/`domain`/`mode`/`cyclic`; it never reads `missing_color` or
`range_folded_color` at all. There is no way for a real value lookup to
produce either sentinel color — a gate is only ever assigned
`missing_color`/`range_folded_color` via a completely separate code path
that checks the gate's *state* (`radar_types::GateValue::Missing` /
`RangeFolded`) before a value lookup is even attempted, exactly the same
"check the flag before ever touching the palette" discipline
`shaders/radar_sweep.wgsl` already uses.

The one required rule is that `missing_color` and `range_folded_color`
**must differ from each other**, so a legend can always show them as two
distinct states. They are *not* required to differ from any of `stops`'
colors: a table may legitimately choose the same visual appearance (e.g.
fully transparent) for `missing_color` and a low-end "no signal" stop, as
this project's own `REF`/`SW` defaults do — a below-threshold real reading
and "no data" both rendering as invisible is an intentional design choice,
not an ambiguity, since the underlying missing/range-folded distinction is
preserved through the separate state channel regardless of what color
anything is drawn in.

## Worked example

`crates/radar-render/color_tables/reflectivity.json` (this project's
default REF table — reproduced here in full since it is the canonical
example this document promises):

```json
{
  "format_version": 1,
  "name": "RadarPro Default Reflectivity",
  "moments": ["REF"],
  "units": "dBZ",
  "mode": "gradient",
  "domain": { "min": -10.0, "max": 75.0 },
  "cyclic": false,
  "stops": [
    { "value": -10.0, "color": [0, 0, 0, 0] },
    { "value": 5.0, "color": [0, 0, 0, 0] },
    { "value": 5.01, "color": [64, 170, 64, 255] },
    { "value": 25.0, "color": [0, 220, 0, 255] },
    { "value": 40.0, "color": [230, 220, 0, 255] },
    { "value": 55.0, "color": [230, 90, 0, 255] },
    { "value": 75.0, "color": [200, 0, 0, 255] }
  ],
  "missing_color": [0, 0, 0, 0],
  "range_folded_color": [255, 0, 255, 255]
}
```

Reading: below -10 dBZ or above 75 dBZ clamps to the nearest end color.
From -10 to 5 dBZ the table is fully transparent (a "no signal" floor,
deliberately the same appearance as `missing_color`). Just above 5 dBZ it
becomes a faint green, then interpolates smoothly through green → yellow →
orange → red as reflectivity increases toward 75 dBZ, fully opaque
throughout the visible band. A range-folded gate always renders opaque
magenta, a color this particular ramp never otherwise produces.

Five more worked examples (one per remaining moment:
`velocity.json`, `spectrum_width.json`, `differential_reflectivity.json`,
`correlation_coefficient.json`, `differential_phase.json`) live alongside
this file in `crates/radar-render/color_tables/` and are this project's own
built-in defaults — see that directory and
`color_table::default_color_table`'s doc comment for how they are embedded
and loaded.

## Loading a table

Rust: `radar_render::color_table::ColorTable::from_json(json_str)` — always
returns a `Result`, never panics on malformed input.

From a browser (via `radar-web`'s wasm API): `renderer.loadColorTable(json)`
parses, validates, and — on success — makes the table the active palette
for every moment it names, replacing that moment's default until a new
volume/session resets it or another table is loaded for the same moment.
