// RadarPro live-radar map view -- Stage S04 (Map Integration and Live
// Radar). See `Agent Context/context/stages/S04-map-live-radar.md`.
//
// Wires together three independent, already-proven-elsewhere pieces:
//   - `useRadarRenderer`: the `radar-web` wasm decode/render pipeline.
//   - `useScanPoller`: live NOAA S3 discovery + download, polled in the
//     background.
//   - `MapView`: MapLibre GL JS map + the radar canvas overlay adapter.
// This component only orchestrates state between them; none of the pieces
// above depend on each other directly.
import { useCallback, useEffect, useRef, useState } from "react";
import "./App.css";
import { MapView } from "./map/MapView";
import { RADAR_CANVAS_SIZE, useRadarRenderer } from "./radar/useRadarRenderer";
import { DEFAULT_SITE_ICAO, findSite, RADAR_SITES } from "./sites";
import { type PollEvent, useScanPoller } from "./scan/useScanPoller";

function describePollEvent(event: PollEvent): string {
  switch (event.type) {
    case "checking":
      return "checking for new scan…";
    case "downloaded":
      return `new scan received: ${event.volume.key.split("/").pop()}`;
    case "up-to-date":
      return `up to date (${event.volume.key.split("/").pop()})`;
    case "none-available":
      return "no scans available yet for today (UTC)";
    case "error":
      return `error: ${event.message}`;
  }
}

export default function App() {
  const [icao, setIcao] = useState(DEFAULT_SITE_ICAO);
  const site = findSite(icao) ?? RADAR_SITES[0];

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { status, error, adapterName, backend, sweepInfo, renderVolume } =
    useRadarRenderer(canvasRef);

  // Holds the most recently downloaded (but not yet successfully rendered)
  // volume's bytes -- a plain mutable ref, never React state, per
  // GLOBAL_CONTRACT's "large binary arrays do not live in React state".
  // Needed only to cover the startup race between GPU init and the first
  // discovery/download completing in whichever order.
  const pendingBytesRef = useRef<Uint8Array | null>(null);

  const handleVolumeBytes = useCallback(
    (bytes: Uint8Array) => {
      pendingBytesRef.current = bytes;
      if (renderVolume(bytes)) {
        pendingBytesRef.current = null;
      }
    },
    [renderVolume],
  );

  useEffect(() => {
    if (status === "ready" && pendingBytesRef.current) {
      renderVolume(pendingBytesRef.current);
      pendingBytesRef.current = null;
    }
  }, [status, renderVolume]);

  // Switching sites: drop anything pending for the old site so a late
  // GPU-ready transition can't render a stale site's bytes over the new
  // selection. `useScanPoller`'s own effect (keyed on `icao`) independently
  // cancels the old site's in-flight request/interval -- see its docs.
  useEffect(() => {
    pendingBytesRef.current = null;
  }, [icao]);

  const pollEvent = useScanPoller(icao, handleVolumeBytes);

  return (
    <div style={{ position: "fixed", inset: 0 }}>
      <MapView site={site} canvasRef={canvasRef} canvasSize={RADAR_CANVAS_SIZE} />

      <div className="hud-panel">
        <h1>RadarPro — live radar (S04)</h1>

        <label>
          Site{" "}
          <select value={icao} onChange={(e) => setIcao(e.target.value)}>
            {RADAR_SITES.map((s) => (
              <option key={s.icao} value={s.icao}>
                {s.icao} — {s.name}
              </option>
            ))}
          </select>
        </label>

        <dl>
          <dt>GPU</dt>
          <dd>
            {status === "loading" && "initializing…"}
            {status === "error" && `error: ${error}`}
            {status === "ready" && `${adapterName || "(adapter name withheld)"} (${backend})`}
          </dd>

          <dt>Scan feed</dt>
          <dd>{describePollEvent(pollEvent)}</dd>

          {sweepInfo && (
            <>
              <dt>Sweep</dt>
              <dd>
                {sweepInfo.siteIcao} · elevation {sweepInfo.elevationDeg.toFixed(2)}° ·{" "}
                {sweepInfo.radialCount} radials · {sweepInfo.sweepCount} sweeps in volume
              </dd>
            </>
          )}
        </dl>
      </div>
    </div>
  );
}
