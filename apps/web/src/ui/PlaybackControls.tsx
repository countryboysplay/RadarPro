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
}

function formatTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "medium" });
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
        <span className={`play-mode play-mode-${playMode}`}>{playMode}</span>
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
