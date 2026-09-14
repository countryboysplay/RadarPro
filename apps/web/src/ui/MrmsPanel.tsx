import type { MrmsPhase } from "../mrms/useMrmsOverlay";
import type { MrmsGridMetadata, MrmsProductId, MrmsSnapshotMetadata } from "../mrms/types";

const PRODUCTS: { id: MrmsProductId; label: string }[] = [
  { id: "reflectivity", label: "Reflectivity" },
  { id: "precip_rate", label: "Precip Rate" },
];

function formatTime(iso: string | undefined): string {
  if (!iso) return "—";
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return iso;
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function describePhase(phase: MrmsPhase, error: string | null): string {
  switch (phase) {
    case "idle":
      return "off";
    case "loading-module":
      return "loading MRMS module…";
    case "discovering":
      return "discovering latest snapshot…";
    case "fetching":
      return "fetching snapshot…";
    case "rendering":
      return "rendering…";
    case "ready":
      return "up to date";
    case "no-coverage-in-view":
      return "no MRMS coverage in the current map view (CONUS only) -- pan/zoom to the continental US";
    case "error":
      return `error: ${error ?? "unknown error"}`;
  }
}

export interface MrmsPanelProps {
  productId: MrmsProductId;
  onProductChange: (id: MrmsProductId) => void;
  enabled: boolean;
  phase: MrmsPhase;
  error: string | null;
  snapshot: MrmsSnapshotMetadata | null;
  grid: MrmsGridMetadata | null;
  onRefresh: () => void;
}

/**
 * S09 Phase 3: NOAA MRMS national radar-mosaic overlay control -- product
 * picker (reflectivity / precip rate) + live status/attribution, mirroring
 * `ForecastPanel`'s status-line shape.
 *
 * Sidebar redesign: this panel's own enable/disable checkbox is gone --
 * enabling MRMS is now done from the single "Active Map Layer" dropdown in
 * the "Map Layers" category (`App.tsx`), which enforces the live-radar/
 * Rainbow/MRMS mutual exclusivity as the control itself. This component is
 * rendered only while MRMS *is* the active map layer, so `enabled` is
 * always `true` whenever it appears -- kept as an explicit prop (rather
 * than assumed) so the status/meta block below stays exactly as
 * conditional as it always was. Its own title line carries a teal accent
 * (`--` distinct from Model Forecast's amber `#ffd27a`, App.css) since an
 * observation must never share a forecast's accent color (Global
 * Contract).
 *
 * The product picker itself converged from a button row to a `<select>`
 * dropdown to match `RainbowToggle`'s/`ForecastPanel`'s own single-choice
 * convention -- same `onProductChange` handler, just a different control.
 *
 * # An observation, never a forecast, never live-sweep radar
 *
 * Global Contract requires observations/forecasts stay distinguishable and
 * forbids implying model precipitation is radar -- the attribution line
 * below states the source and its nature explicitly on every render:
 * "NOAA MRMS, valid at <time> -- observation, not forecast". Unlike the
 * live NEXRAD sweep `MapView` renders directly from a single site, MRMS is
 * a *national gridded mosaic* built from many radars -- labeled "MRMS
 * national mosaic" (never bare "radar") so it is never mistaken for the
 * live single-site sweep either.
 */
export function MrmsPanel({
  productId,
  onProductChange,
  enabled,
  phase,
  error,
  snapshot,
  grid,
  onRefresh,
}: MrmsPanelProps) {
  const busy = phase !== "ready" && phase !== "error" && phase !== "idle" && phase !== "no-coverage-in-view";

  return (
    <div className="mrms-panel">
      <div className="mrms-panel-title">NOAA MRMS National Mosaic (observation)</div>

      <label className="mrms-product-select">
        <span>Product</span>
        <select
          value={productId}
          disabled={!enabled || busy}
          onChange={(e) => onProductChange(e.target.value as MrmsProductId)}
        >
          {PRODUCTS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
      </label>

      {enabled && (
        <>
          <div className={`mrms-status mrms-status-${phase}`}>{describePhase(phase, error)}</div>

          {phase === "error" && (
            <button type="button" className="mrms-retry-button" onClick={onRefresh}>
              Retry
            </button>
          )}
          {phase !== "error" && (
            <button type="button" className="mrms-refresh-button" onClick={onRefresh} disabled={busy}>
              Refresh snapshot
            </button>
          )}

          <dl className="mrms-meta">
            <dt>Snapshot</dt>
            <dd>{formatTime(snapshot?.snapshotTime)}</dd>
            <dt>Unit</dt>
            <dd>{grid?.unit ?? "—"}</dd>
          </dl>

          <div className="mrms-attribution">
            NOAA MRMS, valid at {formatTime(grid?.validTime)} -- observation, not forecast
          </div>
        </>
      )}
    </div>
  );
}
