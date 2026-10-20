import { useState } from "react";
import { Icon } from "./Icon";
import { SUGGESTED_PLACEFILES, type PlacefileEntry, type PlacefileStatus } from "../placefiles/usePlacefiles";

export interface PlacefilesPanelProps {
  entries: PlacefileEntry[];
  onChange: (entries: PlacefileEntry[]) => void;
  status: Map<string, PlacefileStatus>;
}

function statusText(s: PlacefileStatus | undefined, on: boolean): string {
  if (!on) return "off";
  if (!s) return "loading…";
  const when =
    s.lastMs === null
      ? ""
      : new Date(s.lastMs).toLocaleTimeString(undefined, {
          hour: "numeric",
          minute: "2-digit",
        });
  if (!s.error) return `${s.items} items · ${when}`;
  return s.lastMs === null ? `failed: ${s.error}` : `refresh failed: ${s.error}; showing ${when}`;
}

/** GRLevelX placefile list: add by URL or a suggestion, switch each on or off, remove. */
export function PlacefilesPanel({ entries, onChange, status }: PlacefilesPanelProps) {
  const [draft, setDraft] = useState("");
  const url = draft.trim();
  const valid = /^https?:\/\/\S+$/.test(url) && !entries.some((e) => e.url === url);
  const add = (u: string) => onChange([...entries, { url: u, on: true }]);
  const suggestions = SUGGESTED_PLACEFILES.filter((s) => !entries.some((e) => e.url === s.url));

  return (
    <>
      {entries.map((entry) => {
        const s = status.get(entry.url);
        const name = s?.title || SUGGESTED_PLACEFILES.find((x) => x.url === entry.url)?.name || entry.url;
        return (
          <div key={entry.url} style={{ marginBottom: 8 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <label className="check-field" style={{ flex: 1, minWidth: 0, margin: 0 }} title={entry.url}>
                <input type="checkbox" checked={entry.on} onChange={(e) => onChange(entries.map((x) => (x.url === entry.url ? { ...x, on: e.target.checked } : x)))} />
                <span>{name}</span>
              </label>
              <button type="button" className="icon-button" aria-label={`Remove ${name}`} title="Remove" onClick={() => onChange(entries.filter((x) => x.url !== entry.url))}>
                <Icon name="close" size={16} />
              </button>
            </div>
            <div className={`settings-field-status${entry.on && s?.error ? " settings-field-status-none" : ""}`}>{statusText(s, entry.on)}</div>
          </div>
        );
      })}
      <form
        className="settings-field-row"
        onSubmit={(e) => {
          e.preventDefault();
          if (!valid) return;
          add(url);
          setDraft("");
        }}
      >
        <input type="url" placeholder="Placefile URL" aria-label="Placefile URL" value={draft} onChange={(e) => setDraft(e.target.value)} />
        <button type="submit" disabled={!valid}>
          Add
        </button>
      </form>
      {suggestions.map((s) => (
        <button key={s.url} type="button" onClick={() => add(s.url)}>
          Add {s.name}
        </button>
      ))}
    </>
  );
}
