import { buildLegendGradientCss, parseColorTableJson, rgbaToCss } from "../colorTables/colorTableJson";

export interface LegendProps {
  /** The active color table's own JSON (from `activeColorTableJson`) --
   * the legend is built entirely from this, never a second hardcoded copy
   * of the palette. `null` while unavailable. */
  activeColorTableJson: string | null;
  momentCode: string;
}

/** Small color-scale swatch: a gradient bar spanning the active table's
 * domain, plus distinct swatches for "missing" and "range-folded" so those
 * two states never look like just differently-worded copies of each
 * other -- matching the data probe's own three-way distinction. */
export function Legend({ activeColorTableJson, momentCode }: LegendProps) {
  const table = activeColorTableJson ? parseColorTableJson(activeColorTableJson) : null;

  if (!table) {
    return (
      <div className="legend">
        <div className="legend-title">Legend -- {momentCode}</div>
        <div className="legend-empty">(no color table loaded yet)</div>
      </div>
    );
  }

  return (
    <div className="legend">
      <div className="legend-title">
        {table.name} ({momentCode})
      </div>
      <div className="legend-bar" style={{ background: buildLegendGradientCss(table) }} />
      <div className="legend-domain">
        <span>{table.domain.min}</span>
        <span>{table.units}</span>
        <span>{table.domain.max}</span>
      </div>
      <div className="legend-states">
        <span className="legend-swatch">
          <span className="legend-swatch-box" style={{ background: rgbaToCss(table.missing_color) }} />
          missing
        </span>
        <span className="legend-swatch">
          <span className="legend-swatch-box" style={{ background: rgbaToCss(table.range_folded_color) }} />
          range-folded
        </span>
      </div>
    </div>
  );
}
