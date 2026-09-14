// This project's six shipped default color tables, vendored verbatim into
// `src/data/color_tables/` (see that directory's README) and imported here
// as raw text (Vite's `?raw` suffix) -- i.e. exactly the bytes
// `RadarWebRenderer.loadColorTable` would accept, never re-serialized or
// otherwise transformed. Used to populate the color-table editor's preset
// dropdown (`ColorTableEditor.tsx`).
import reflectivityRaw from "../data/color_tables/reflectivity.json?raw";
import velocityRaw from "../data/color_tables/velocity.json?raw";
import spectrumWidthRaw from "../data/color_tables/spectrum_width.json?raw";
import differentialReflectivityRaw from "../data/color_tables/differential_reflectivity.json?raw";
import correlationCoefficientRaw from "../data/color_tables/correlation_coefficient.json?raw";
import differentialPhaseRaw from "../data/color_tables/differential_phase.json?raw";
import stormRelativeVelocityRaw from "../data/color_tables/storm_relative_velocity.json?raw";

export interface DefaultColorTablePreset {
  /** The moment wire code this default targets, e.g. `"REF"`. */
  momentCode: string;
  /** Short label for the preset dropdown. */
  label: string;
  /** The table's exact original JSON text. */
  json: string;
}

export const DEFAULT_COLOR_TABLE_PRESETS: DefaultColorTablePreset[] = [
  { momentCode: "REF", label: "REF -- Reflectivity (default)", json: reflectivityRaw },
  { momentCode: "VEL", label: "VEL -- Radial Velocity (default)", json: velocityRaw },
  { momentCode: "SW", label: "SW -- Spectrum Width (default)", json: spectrumWidthRaw },
  { momentCode: "ZDR", label: "ZDR -- Differential Reflectivity (default)", json: differentialReflectivityRaw },
  { momentCode: "CC", label: "CC -- Correlation Coefficient (default)", json: correlationCoefficientRaw },
  { momentCode: "PHI", label: "PHI -- Differential Phase (default)", json: differentialPhaseRaw },
  { momentCode: "SRV", label: "SRV -- Storm-Relative Velocity (default)", json: stormRelativeVelocityRaw },
];
