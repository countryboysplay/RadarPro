import type { RainbowStatus } from "../rainbow/useRainbowOverlay";

export interface RainbowToggleProps {
  configured: boolean;
  enabled: boolean;
  status: RainbowStatus;
  error: string | null;
  onToggle: () => void;
}

function describeStatus(status: RainbowStatus, error: string | null): string | null {
  switch (status) {
    case "unconfigured":
      return "not configured -- set VITE_RAINBOW_API_KEY (see .env.example) to enable";
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
 * S09b: toggle for Rainbow Weather's precipitation tile overlay
 * (doc.rainbow.ai) -- an ML/blended *nowcast* product, a third category
 * distinct from both this app's live NEXRAD radar sweep and its GEFS/HRRR
 * NWP forecasts (Global Contract: these must stay distinguishable). Labeled
 * "Rainbow nowcast" everywhere, never "radar" or "forecast", for exactly
 * that reason.
 *
 * Always rendered, even with no key configured (`configured === false`):
 * Global Contract requires the app fail independently for any optional
 * keyed provider -- a visibly-present-but-disabled control with a clear
 * "not configured" reason, never a silently-missing one that leaves a user
 * wondering whether the feature exists at all.
 */
export function RainbowToggle({ configured, enabled, status, error, onToggle }: RainbowToggleProps) {
  const description = describeStatus(status, error);
  return (
    <div className="rainbow-toggle">
      <label className={`rainbow-toggle-label${configured ? "" : " rainbow-toggle-label-disabled"}`}>
        <input type="checkbox" checked={enabled} disabled={!configured} onChange={onToggle} />{" "}
        Rainbow nowcast{!configured && " (not configured)"}
      </label>
      {description && <div className={`rainbow-toggle-status rainbow-toggle-status-${status}`}>{description}</div>}
    </div>
  );
}
