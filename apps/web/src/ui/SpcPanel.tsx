import { useEffect, useState } from "react";
import { fetchProductText, OUTLOOK_NAMES, type MesoscaleDiscussion, type OutlookKind } from "../spc/spc";

export interface SpcPanelProps {
  outlook: OutlookKind;
  onOutlookChange: (kind: OutlookKind) => void;
  showWatches: boolean;
  onShowWatchesChange: (on: boolean) => void;
  showMds: boolean;
  onShowMdsChange: (on: boolean) => void;
  mds: MesoscaleDiscussion[];
  watchCount: number;
  selectedMd: string | null;
  onSelectMd: (productId: string) => void;
  error: string | null;
  outlookHiddenInArchive: boolean;
}

const time = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

/** SPC Day 1 outlook picker, watch and MD switches, and the active MDs. */
export function SpcPanel(props: SpcPanelProps) {
  return (
    <>
      <label className="map-layer-select">
        <span>Outlook</span>
        <select value={props.outlook} onChange={(e) => props.onOutlookChange(e.target.value as OutlookKind)}>
          {(Object.keys(OUTLOOK_NAMES) as OutlookKind[]).map((k) => (
            <option key={k} value={k}>
              {OUTLOOK_NAMES[k]}
            </option>
          ))}
        </select>
      </label>
      {props.outlookHiddenInArchive && <div className="settings-field-note">The outlook shows on live radar only.</div>}
      <label className="check-field">
        <input type="checkbox" checked={props.showWatches} onChange={(e) => props.onShowWatchesChange(e.target.checked)} />
        <span>Tornado and severe thunderstorm watches{props.showWatches ? ` (${props.watchCount})` : ""}</span>
      </label>
      <label className="check-field">
        <input type="checkbox" checked={props.showMds} onChange={(e) => props.onShowMdsChange(e.target.checked)} />
        <span>Mesoscale discussions{props.showMds ? ` (${props.mds.length})` : ""}</span>
      </label>
      {props.error && <div className="settings-field-status settings-field-status-none">SPC data failed: {props.error}</div>}
      {props.showMds && props.mds.length > 0 && (
        <ul className="alerts-list">
          {props.mds.map((md) => (
            <li key={md.productId}>
              <button
                type="button"
                className={`alert-row${md.productId === props.selectedMd ? " alert-row-selected" : ""}`}
                onClick={() => props.onSelectMd(md.productId)}
              >
                <span className="alert-row-text">
                  <span className="alert-row-event">MD {md.num}</span>
                  <span className="alert-row-headline">
                    {md.concerning}
                    {md.watchChance !== null && ` · watch chance ${md.watchChance}%`} · until {time(md.expiresMs)}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

/** The full text of one MD (or archived warning), for the right panel. */
export function MdText({ productId, source = "NWS Storm Prediction Center" }: { productId: string; source?: string }) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setText(null);
    setError(null);
    const controller = new AbortController();
    fetchProductText(productId, controller.signal).then(setText, (err) => {
      if (!controller.signal.aborted) setError(err instanceof Error ? err.message : String(err));
    });
    return () => controller.abort();
  }, [productId]);
  if (error) return <div className="settings-field-status settings-field-status-none">Could not load the text: {error}</div>;
  if (text === null) return <div className="settings-field-note">loading…</div>;
  return (
    <>
      <p className="alert-detail-text">{text.trim()}</p>
      <div className="alert-detail-attribution">Source: {source}, via Iowa Environmental Mesonet.</div>
    </>
  );
}
