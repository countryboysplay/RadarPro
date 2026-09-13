import type { PlayMode } from "../scan/useScanHistory";

export interface PlaybackControlsProps {
  playMode: PlayMode;
  currentIndex: number;
  entriesCount: number;
  /** The currently-displayed frame's real timestamp, or `null` if none is
   * selected -- S09: "use real timestamps, not frame indexes." The frame
   * index is still shown alongside it (useful for "how many scans held"),
   * just no longer the only thing displayed. */
  currentTimeMillis: number | null;
  frameMs: number;
  onPrevious: () => void;
  onNext: () => void;
  onTogglePlay: () => void;
  onJumpLatest: () => void;
  onFrameMsChange: (ms: number) => void;
  /** S10 Phase 3: `true` once the displayed "live" frame's own volume start
   * time is old enough that it should no longer read as fresh -- see
   * `useScanHistory`'s `isLiveStale` doc comment for why this exists
   * (verified live against a real blocked feed: the badge below used to
   * keep reading "live" unchanged no matter how old the data actually
   * got). Always `false` outside `"live"` mode. */
  isLiveStale: boolean;
  /** Age (ms) of the currently-displayed frame's own volume start time, for
   * the "no new scan in Xm" label -- `null` when nothing is selected. */
  currentVolumeAgeMillis: number | null;
}

function formatTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "medium" });
}

/** Round an age in milliseconds down to a human "Xm"/"Xh Ym" label -- only
 * ever shown once an age is already well past `STALE_LIVE_AFTER_MS` (tens of
 * minutes), so minute granularity (no seconds) is the right precision. */
function formatAge(ms: number): string {
  const totalMinutes = Math.max(1, Math.floor(ms / 60_000));
  if (totalMinutes < 60) return `${totalMinutes}m`;
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`;
}

/**
 * Previous/next scan, loop-play/pause, and speed control over the bounded
 * scan-history cache (`useScanHistory`). None of these buttons ever
 * trigger a network request -- they only move `currentIndex` through
 * already-downloaded frames; see that hook's docs.
 */
export function PlaybackControls({
  playMode,
  currentIndex,
  entriesCount,
  currentTimeMillis,
  frameMs,
  onPrevious,
  onNext,
  onTogglePlay,
  onJumpLatest,
  onFrameMsChange,
  isLiveStale,
  currentVolumeAgeMillis,
}: PlaybackControlsProps) {
  const disabled = entriesCount === 0;
  return (
    <div className="playback-controls">
      <div className="playback-buttons">
        <button type="button" onClick={onPrevious} disabled={disabled} title="Previous scan (Left arrow)">
          ⏮ Prev
        </button>
        <button type="button" onClick={onTogglePlay} disabled={disabled} title="Play/Pause (Space)">
          {playMode === "playing" ? "⏸ Pause" : "▶ Play"}
        </button>
        <button type="button" onClick={onNext} disabled={disabled} title="Next scan (Right arrow)">
          Next ⏭
        </button>
        <button
          type="button"
          onClick={onJumpLatest}
          disabled={disabled || playMode === "live"}
          title="Jump to latest and resume live polling (L)"
        >
          ⏭⏭ Latest
        </button>
      </div>
      <div className="playback-status">
        <span className={`play-mode play-mode-${playMode}${isLiveStale ? " play-mode-stale" : ""}`}>{playMode}</span>
        {isLiveStale && currentVolumeAgeMillis !== null && (
          <span className="play-mode-stale-warning" title="The live feed has not produced a newer scan in a while -- see Settings > Network diagnostics.">
            ⚠ no new scan in {formatAge(currentVolumeAgeMillis)}
          </span>
        )}
        <span className="playback-time">
          {currentTimeMillis !== null ? formatTime(currentTimeMillis) : "no scan loaded"}
        </span>
        <span className="playback-frame-index">
          (frame {entriesCount === 0 ? 0 : currentIndex + 1} / {entriesCount})
        </span>
      </div>
      <label className="playback-speed">
        Speed{" "}
        <input
          type="range"
          min={100}
          max={2000}
          step={50}
          value={frameMs}
          onChange={(e) => onFrameMsChange(Number(e.target.value))}
        />{" "}
        {frameMs} ms/frame
      </label>
    </div>
  );
}
