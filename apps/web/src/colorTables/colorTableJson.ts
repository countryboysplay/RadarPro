// Plain TypeScript mirror of `crates/radar-render/COLOR_TABLE_FORMAT.md`'s
// JSON shape -- used only to *read* a table for display (the legend, the
// editor's syntax help) and to build a CSS approximation of its gradient.
// The authoritative parse/validate/apply step is always
// `RadarWebRenderer.loadColorTable` (Rust); this module never substitutes
// for that -- it only reads back JSON the Rust side already accepted (via
// `activeColorTableJson`) or is about to be sent to it.

export type ColorRgba = [number, number, number, number];

export interface ColorTableStop {
  value: number;
  color: ColorRgba;
}

export interface ColorTableJson {
  format_version: number;
  name: string;
  moments: string[];
  units: string;
  mode: "gradient" | "stepped";
  domain: { min: number; max: number };
  cyclic?: boolean;
  stops: ColorTableStop[];
  missing_color: ColorRgba;
  range_folded_color: ColorRgba;
}

/** Parse a color-table JSON string into the plain shape above, or `null` on
 * any parse/shape failure -- used only for display (legend), so a failure
 * here just means "no legend to show," never a crash. The authoritative
 * error surface for a load *attempt* is `useRadarRenderer`'s
 * `loadColorTable`, which reports the Rust validator's real message. */
export function parseColorTableJson(json: string): ColorTableJson | null {
  try {
    const parsed: unknown = JSON.parse(json);
    if (!isColorTableJsonShape(parsed)) return null;
    return parsed;
  } catch {
    return null;
  }
}

function isColorRgba(value: unknown): value is ColorRgba {
  return Array.isArray(value) && value.length === 4 && value.every((v) => typeof v === "number");
}

function isColorTableJsonShape(value: unknown): value is ColorTableJson {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.name === "string" &&
    Array.isArray(v.moments) &&
    typeof v.units === "string" &&
    (v.mode === "gradient" || v.mode === "stepped") &&
    typeof v.domain === "object" &&
    v.domain !== null &&
    typeof (v.domain as Record<string, unknown>).min === "number" &&
    typeof (v.domain as Record<string, unknown>).max === "number" &&
    Array.isArray(v.stops) &&
    v.stops.every(
      (s) =>
        typeof s === "object" &&
        s !== null &&
        typeof (s as Record<string, unknown>).value === "number" &&
        isColorRgba((s as Record<string, unknown>).color),
    ) &&
    isColorRgba(v.missing_color) &&
    isColorRgba(v.range_folded_color)
  );
}

export function rgbaToCss([r, g, b, a]: ColorRgba): string {
  return `rgba(${r}, ${g}, ${b}, ${(a / 255).toFixed(3)})`;
}

/** Build a CSS `linear-gradient()` background string from a table's own
 * stops/domain/mode -- reusing the loaded table's real stop colors and
 * positions directly rather than hardcoding a second copy of the color
 * mapping. `"stepped"` mode uses hard edges (two color-stops at the same
 * percentage) so no blending is introduced where the source table has
 * none; `"gradient"` mode lets the browser interpolate linearly between
 * percentage-positioned stops, a reasonable legend approximation of the
 * renderer's own linear interpolation (this is a display-only legend, not
 * a second source of truth for any actual gate value). */
export function buildLegendGradientCss(table: ColorTableJson): string {
  const { min, max } = table.domain;
  const span = max - min || 1;
  const pct = (value: number) => `${(((value - min) / span) * 100).toFixed(3)}%`;

  if (table.stops.length === 0) return "transparent";
  if (table.stops.length === 1) return rgbaToCss(table.stops[0].color);

  const parts: string[] = [];
  if (table.mode === "stepped") {
    for (let i = 0; i < table.stops.length; i++) {
      const stop = table.stops[i];
      const next = table.stops[i + 1];
      const startPct = pct(stop.value);
      const endPct = next ? pct(next.value) : "100%";
      parts.push(`${rgbaToCss(stop.color)} ${startPct} ${endPct}`);
    }
  } else {
    for (const stop of table.stops) {
      parts.push(`${rgbaToCss(stop.color)} ${pct(stop.value)}`);
    }
  }
  return `linear-gradient(to right, ${parts.join(", ")})`;
}
