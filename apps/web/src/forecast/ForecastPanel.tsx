import type { RefObject } from "react";
import {
  FORECAST_LEAD_HOUR_OPTIONS,
  FORECAST_RENDER_HEIGHT,
  FORECAST_RENDER_WIDTH,
  FORECAST_VARIABLE,
  type ForecastPhase,
  type useForecastProvider,
} from "./useForecastProvider";
import type { ForecastEnsembleSpec, ForecastProviderId } from "./types";

const PROVIDERS: { id: ForecastProviderId; label: string }[] = [
  { id: "gefs", label: "GEFS" },
  { id: "hrrr", label: "HRRR" },
];

function describePhase(phase: ForecastPhase, providerId: ForecastProviderId | null, error: string | null): string {
  const name = providerId?.toUpperCase() ?? "";
  switch (phase) {
    case "idle":
      return "select a model to load a forecast";
    case "loading-module":
      return `loading ${name} forecast module…`;
    case "discovering":
      return `discovering latest ${name} run…`;
    case "fetching":
      return `fetching ${name} field…`;
    case "rendering":
      return `rendering ${name} grid…`;
    case "ready":
      return `${name} render up to date`;
    case "error":
      return `${name} error: ${error ?? "unknown error"}`;
  }
}

function formatTime(iso: string | undefined): string {
  if (!iso) return "—";
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return iso;
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/**
 * S08 follow-up: a minimal proof-of-concept UI switching between the GEFS
 * (ensemble) and HRRR (deterministic) forecast providers through
 * `forecast-web`'s shared `ProviderHandle` API/`useForecastProvider` hook,
 * satisfying this stage's exit criteria ("the same forecast UI and grid
 * renderer switch between GEFS and HRRR through the shared provider API").
 *
 * # Dedicated panel, not a `MapView` overlay -- and why
 *
 * `MapView`'s doc comment explains why alert polygons (and range rings) had
 * to move to a manual `<canvas>` overlay layered above the opaque radar
 * sweep canvas: a native MapLibre layer paints into the map's own canvas,
 * *underneath* that opaque sweep image, so anything geographically inside
 * the sweep's footprint would be invisible. A forecast grid render is a
 * raster RGBA image, not vector polygons -- if it were added as a native
 * MapLibre raster/image source, it would sit in that same "underneath the
 * opaque sweep canvas" layer as the basemap itself, alongside vector alert
 * layers, so the occlusion problem those two features hit would not apply
 * to it (a raster *image* source is not something the radar canvas needs to
 * sit above the way a live sweep does relative to alert shading -- both are
 * just imagery at that point).
 *
 * The reason this stage still keeps the forecast render in its own panel
 * rather than reprojecting it onto the live map is a different one:
 * `renderCurrentGrid` takes a fixed orthographic camera
 * (center lon/lat + half-extent lon/lat, decimal degrees) that this hook
 * renders once and does not recompute reactively as the map pans/zooms --
 * doing that properly would mean (a) deriving that camera from the map's
 * current viewport on every `move`/`zoom` event the way `MapView` already
 * does for the radar canvas's on-screen box, *and* (b) re-invoking an async
 * GPU render (network-free, but still a real GPU round trip) on every such
 * event rather than just repositioning a CSS box the way the already-
 * rendered radar/rings/alerts overlays do. That is meaningfully more
 * design than this task's scope ("a working switcher + shared render
 * path", not a speculative live-georeferenced overlay) -- a dedicated panel
 * with its own fixed-size canvas needs none of that, and still exercises
 * the exact same shared `ProviderHandle` render path a future map
 * integration would reuse. A follow-up stage doing real map integration
 * should extend `MapView`'s existing `updateOverlay` pattern to also
 * reposition/re-render this canvas from the live viewport.
 *
 * # Model forecast, not observed radar
 *
 * Every label in this panel says "forecast"/model name explicitly and the
 * canvas carries its own caption -- this must never be mistaken for the
 * live NEXRAD radar sweep `MapView` renders (Global Contract).
 *
 * # S09: state lifted to `App.tsx`
 *
 * This panel used to own its `useForecastProvider(canvasRef)` call (and the
 * canvas ref it renders onto) entirely internally. S09's unified timeline
 * (`../timeline/UnifiedTimeline`) needs to both *read* forecast run/lead/
 * grid metadata and *drive* `setLeadHours` from outside this panel, so both
 * the hook instance and its canvas ref now live in `App.tsx` and are passed
 * down as props -- there is still exactly one `useForecastProvider` handle
 * for the whole app, just no longer instantiated in here. This component
 * keeps every other responsibility (provider switcher, phase/status text,
 * ensemble picker, canvas paint target, "not observed radar" labeling)
 * unchanged.
 */
export interface ForecastPanelProps {
  canvasRef: RefObject<HTMLCanvasElement>;
  forecast: ReturnType<typeof useForecastProvider>;
}

export function ForecastPanel({ canvasRef, forecast }: ForecastPanelProps) {
  const isEnsemble = forecast.metadata?.isEnsemble ?? false;
  const busy = forecast.phase !== "ready" && forecast.phase !== "error" && forecast.phase !== "idle";

  return (
    <div className="forecast-panel">
      <div className="forecast-panel-title">Model Forecast (GEFS/HRRR) — not observed radar</div>

      <label className="forecast-provider-select">
        <span>Model</span>
        <select
          value={forecast.providerId ?? ""}
          disabled={busy}
          onChange={(e) => forecast.selectProvider(e.target.value as ForecastProviderId)}
        >
          {PROVIDERS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
      </label>

      <div className={`forecast-status forecast-status-${forecast.phase}`}>
        {describePhase(forecast.phase, forecast.providerId, forecast.error)}
      </div>

      {forecast.phase === "error" && (
        <button type="button" className="forecast-retry-button" onClick={forecast.retry}>
          Retry {forecast.providerId?.toUpperCase()}
        </button>
      )}

      {forecast.metadata && (
        <div className="forecast-model-kind">
          {isEnsemble
            ? `Ensemble model (${forecast.metadata.resolutionDescription})`
            : `Deterministic model (${forecast.metadata.resolutionDescription})`}
        </div>
      )}

      <div className="forecast-controls">
        <label>
          Lead time{" "}
          <select
            value={forecast.leadHours}
            disabled={busy}
            onChange={(e) => forecast.setLeadHours(Number(e.target.value))}
          >
            {FORECAST_LEAD_HOUR_OPTIONS.map((h) => (
              <option key={h} value={h}>
                +{h}h
              </option>
            ))}
          </select>
        </label>

        {/* Ensemble-statistic picker only ever shown for an ensemble
            provider (GEFS) -- Global Contract: never offered for a
            deterministic provider (HRRR). */}
        {isEnsemble && (
          <label>
            Ensemble{" "}
            <select
              value={forecast.ensemble ?? "mean"}
              disabled={busy}
              onChange={(e) => forecast.setEnsemble(e.target.value as ForecastEnsembleSpec)}
            >
              <option value="mean">Mean</option>
              <option value="control">Control</option>
            </select>
          </label>
        )}
      </div>

      <dl className="forecast-meta">
        <dt>Run</dt>
        <dd>{forecast.run?.label ?? "—"}</dd>
        <dt>Valid</dt>
        <dd>{formatTime(forecast.grid?.validTime)}</dd>
        <dt>Variable</dt>
        <dd>
          {FORECAST_VARIABLE}
          {forecast.grid ? ` (${forecast.grid.unit})` : ""}
        </dd>
      </dl>

      <canvas
        ref={canvasRef}
        width={FORECAST_RENDER_WIDTH}
        height={FORECAST_RENDER_HEIGHT}
        className="forecast-canvas"
        aria-label="Model forecast render (not observed radar)"
      />
      <div className="forecast-canvas-caption">Model forecast render — not live radar</div>
    </div>
  );
}
