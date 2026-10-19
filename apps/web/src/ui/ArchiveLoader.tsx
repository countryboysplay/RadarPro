import { useState } from "react";

export interface ArchiveLoaderProps {
  archiveStartMillis: number | null;
  status: string | null;
  onLoad: (startMillis: number) => void;
  onReturnToLive: () => void;
}

/** Load past NEXRAD scans for the selected site from the public Level II
 * archive. The picked time is UTC, matching every other time in the app. */
export function ArchiveLoader({ archiveStartMillis, status, onLoad, onReturnToLive }: ArchiveLoaderProps) {
  const [value, setValue] = useState("");
  // `datetime-local` has no zone; append Z so it parses as UTC.
  const millis = value ? Date.parse(`${value}:00Z`) : NaN;
  const valid = Number.isFinite(millis) && millis <= Date.now();

  return (
    <div className="archive-loader">
      <label>
        Archive (UTC){" "}
        <input type="datetime-local" value={value} onChange={(e) => setValue(e.target.value)} />
      </label>{" "}
      <button type="button" disabled={!valid} onClick={() => onLoad(millis)}>
        Load
      </button>
      {archiveStartMillis !== null && (
        <>
          {" "}
          <button type="button" onClick={onReturnToLive}>
            Back to live
          </button>
        </>
      )}
      {status && <div className="archive-status">{status}</div>}
    </div>
  );
}
