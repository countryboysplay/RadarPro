import type { RainbowOverlay, RainbowStatus } from "../rainbow/useRainbowOverlay";
import { RAINBOW_FORECAST_TIME_STEPS, RAINBOW_PALETTES, RAINBOW_TILE_LAYERS, type RainbowTileLayer } from "../rainbow/types";

export interface RainbowToggleProps {
  overlay: RainbowOverlay;
  onToggle: () => void;
}

function describeStatus(status: RainbowStatus, error: string | null): string | null {
  switch (status) {
    case "unconfigured":
      return "not configured -- add your API key in Settings to enable";
    case "off":
      return null;
    case "resolving":
      return "resolving latest snapshot…";
    case "ready":
      return "showing current conditions -- live radar hidden while active";
    case "error":
      return `unavailable: ${error ?? "unknown error"}`;
  }
}

/**
 * S09b (precip-only) / S09d Part A (full Tiles API surface): control panel
 * for Rainbow Weather's tile overlay (doc.rainbow.ai) -- an ML/blended
 * *nowcast* product, a third category distinct from both this app's live
 * NEXRAD radar sweep and its GEFS/HRRR NWP forecasts (Global Contract:
 * these must stay distinguishable). Labeled "Rainbow nowcast" everywhere,
 * never "radar" or "forecast", for exactly that reason.
 *
 * Always rendered, even with no key configured (`configured === false`):
 * Global Contract requires the app fail independently for any optional
 * keyed provider -- a visibly-present-but-disabled control with a clear
 * "not configured" reason, never a silently-missing one that leaves a user
 * wondering whether the feature exists at all. Every option control below
 * follows the same convention: always rendered (when applicable to the
 * current layer), just `disabled` while unconfigured.
 *
 * Per this stage's explicit UI decision, every picker here is a plain
 * `<select>` dropdown, not a button row (contrast `MrmsProductId`'s
 * button-row UI, used only as this codebase's type-shape precedent for a
 * layer-id union, never as UI precedent). `layerInfo`
 * (`overlay.layerInfo`) decides which of the palette/forecast-time/coverage/
 * use-precip-type controls apply to the currently selected layer -- hidden
 * entirely when not, never shown-but-disabled, since a hidden-but-not-
 * applicable param (e.g. `color` for `clouds`) has no meaning to configure
 * at all (KB §4.4).
 */
export function RainbowToggle({ overlay, onToggle }: RainbowToggleProps) {
  const {
    configured,
    enabled,
    status,
    error,
    layer,
    setLayer,
    layerInfo,
    color,
    setColor,
    coverage,
    setCoverage,
    usePrecipType,
    setUsePrecipType,
    forecastTime,
    setForecastTime,
  } = overlay;
  const description = describeStatus(status, error);

  return (
    <div className="rainbow-toggle">
      <label className={`rainbow-toggle-label${configured ? "" : " rainbow-toggle-label-disabled"}`}>
        <input type="checkbox" checked={enabled} disabled={!configured} onChange={onToggle} />{" "}
        Rainbow nowcast{!configured && " (not configured)"}
      </label>

      <div className="rainbow-toggle-options">
        <label className="rainbow-toggle-option">
          <span>Layer</span>{" "}
          <select value={layer} disabled={!configured} onChange={(e) => setLayer(e.target.value as RainbowTileLayer)}>
            {RAINBOW_TILE_LAYERS.map((l) => (
              <option key={l.id} value={l.id}>
                {l.label}
              </option>
            ))}
          </select>
        </label>

        {layerInfo.supportsForecastTime && (
          <label className="rainbow-toggle-option">
            <span>Forecast time</span>{" "}
            <select value={forecastTime} disabled={!configured} onChange={(e) => setForecastTime(Number(e.target.value))}>
              {RAINBOW_FORECAST_TIME_STEPS.map((step) => (
                <option key={step.value} value={step.value}>
                  {step.label}
                </option>
              ))}
            </select>
          </label>
        )}

        {layerInfo.supportsColorCoverage && (
          <label className="rainbow-toggle-option">
            <span>Palette</span>{" "}
            <select value={color} disabled={!configured} onChange={(e) => setColor(e.target.value)}>
              {RAINBOW_PALETTES.map((palette) => (
                <option key={palette.id} value={palette.id}>
                  {palette.name}
                </option>
              ))}
            </select>
          </label>
        )}

        {layerInfo.supportsColorCoverage && (
          <label className="rainbow-toggle-option rainbow-toggle-checkbox">
            <input type="checkbox" checked={coverage} disabled={!configured} onChange={(e) => setCoverage(e.target.checked)} />{" "}
            Coverage mask
          </label>
        )}

        {layerInfo.supportsUsePrecipType && (
          <label className="rainbow-toggle-option rainbow-toggle-checkbox">
            <input
              type="checkbox"
              checked={usePrecipType}
              disabled={!configured}
              onChange={(e) => setUsePrecipType(e.target.checked)}
            />{" "}
            Show precip type (rain/snow)
          </label>
        )}
      </div>

      {description && <div className={`rainbow-toggle-status rainbow-toggle-status-${status}`}>{description}</div>}
    </div>
  );
}
