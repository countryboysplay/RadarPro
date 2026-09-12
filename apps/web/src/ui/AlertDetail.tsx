import { useState } from "react";
import type { AlertJson } from "../alerts/types";
import { SEVERITY_COLORS, formatAlertTime } from "../alerts/types";

export interface AlertDetailProps {
  alert: AlertJson;
  /** Whether this alert is still in `weather-alerts`' active set (read
   * directly from the store's own decision -- see `App.tsx`) -- `false`
   * means it was cancelled or expired since last selected. Never computed
   * by this component itself. */
  isActive: boolean;
  onClose: () => void;
}

/** Long-text fields (description/instruction) are shown in full by
 * default; collapsed only past this length, purely as a "show more"
 * *display* affordance -- the full, untruncated text is always in the DOM
 * once expanded, never actually cut or paraphrased (`GLOBAL_CONTRACT.md` /
 * S06: official text must be shown as NWS wrote it). */
const COLLAPSE_THRESHOLD = 400;

function ExpandableText({ text, className }: { text: string; className?: string }) {
  const [expanded, setExpanded] = useState(text.length <= COLLAPSE_THRESHOLD);
  return (
    <div className={className}>
      <p className="alert-detail-text">{expanded ? text : `${text.slice(0, COLLAPSE_THRESHOLD)}…`}</p>
      {text.length > COLLAPSE_THRESHOLD && (
        <button type="button" className="alert-detail-showmore" onClick={() => setExpanded((v) => !v)}>
          {expanded ? "show less" : "show more"}
        </button>
      )}
    </div>
  );
}

/**
 * S06 details panel: an active (or last-known, if since removed) alert's
 * full normalized fields, exactly as `weather-alerts` parsed them from
 * NWS's own CAP/GeoJSON feed -- no paraphrasing, summarizing, or silent
 * truncation of `headline`/`description`/`instruction` (only a "show
 * more" *display* affordance for a long `description`/`instruction`, per
 * this stage's own rule).
 */
export function AlertDetail({ alert, isActive, onClose }: AlertDetailProps) {
  return (
    <div className="alert-detail">
      <div className="alert-detail-header">
        <span className="alert-detail-severity" style={{ background: SEVERITY_COLORS[alert.severity] }}>
          {alert.severity}
        </span>
        <span className="alert-detail-event">{alert.event}</span>
        <button type="button" className="alert-detail-close" onClick={onClose} aria-label="Close">
          ×
        </button>
      </div>

      {!isActive && (
        <div className="alert-detail-inactive-banner">
          No longer active (cancelled or expired) — showing last-known content.
        </div>
      )}

      {alert.headline && <div className="alert-detail-headline">{alert.headline}</div>}

      <dl className="alert-detail-fields">
        <dt>Certainty</dt>
        <dd>{alert.certainty}</dd>
        <dt>Urgency</dt>
        <dd>{alert.urgency}</dd>
        <dt>Issued</dt>
        <dd>{formatAlertTime(alert.issued)}</dd>
        <dt>Effective</dt>
        <dd>{formatAlertTime(alert.effective)}</dd>
        <dt>Expires</dt>
        <dd>{formatAlertTime(alert.expires)}</dd>
        {alert.ends !== null && (
          <>
            <dt>Hazard est. ends</dt>
            <dd>{formatAlertTime(alert.ends)}</dd>
          </>
        )}
        <dt>Affected areas</dt>
        <dd>
          {alert.affectedAreas.length > 0 ? (
            <ul className="alert-detail-areas">
              {alert.affectedAreas.map((area) => (
                <li key={area}>{area}</li>
              ))}
            </ul>
          ) : (
            "—"
          )}
        </dd>
      </dl>

      {alert.description && (
        <>
          <div className="alert-detail-section-title">Description</div>
          <ExpandableText text={alert.description} />
        </>
      )}

      {alert.instruction && (
        <>
          <div className="alert-detail-section-title">Instructions</div>
          <ExpandableText text={alert.instruction} className="alert-detail-instruction" />
        </>
      )}

      <div className="alert-detail-attribution">
        Source: National Weather Service — {alert.sender} ({alert.senderId}). Official NWS product; not generated
        or inferred by RadarPro.
      </div>
    </div>
  );
}
