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
  onToggle: () => void;
  phase: MrmsPhase;
  error: string | null;
  snapshot: MrmsSnapshotMetadata | null;
  grid: MrmsGridMetadata | null;
  onRefresh: () => void;
}

/**
 * S09 Phase 3: NOAA MRMS national radar-mosaic overlay control -- product
 * picker (reflectivity / precip rate) + on/off toggle + live status/
 * attribution, mirroring `ForecastPanel`'s status-line shape and
 * `RainbowToggle`'s always-visible-control convention (never hidden just
 * because a feature is off).
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
  onToggle,
  phase,
  error,
  snapshot,
  grid,
  onRefresh,
}: MrmsPanelProps) {
  const busy = phase !== "ready" && phase !== "error" && phase !== "idle" && phase !== "no-coverage-in-view";

  return (
    <div className="mrms-panel">
      <label className="mrms-toggle-label">
        <input type="checkbox" checked={enabled} onChange={onToggle} /> NOAA MRMS national mosaic (observation)
      </label>

      <div className="mrms-product-switcher" role="group" aria-label="MRMS product">
        {PRODUCTS.map((p) => (
          <button
            key={p.id}
            type="button"
            className={`mrms-product-button${productId === p.id ? " mrms-product-button-active" : ""}`}
            onClick={() => onProductChange(p.id)}
            disabled={!enabled || (busy && productId === p.id)}
          >
            {p.label}
          </button>
        ))}
      </div>

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
