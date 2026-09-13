import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { ScanHistory } from "../scan/useScanHistory";
import { FORECAST_LEAD_HOUR_OPTIONS, type useForecastProvider } from "../forecast/useForecastProvider";

/**
 * S09 "Unified Timeline" -- see `Agent Context/context/stages/S09-timeline-mrms.md`.
 *
 * One shared, real-time-axis control spanning from the oldest currently-held
 * radar scan through "now" out to the forecast's furthest available lead
 * time, driving the *existing* radar-history index (`ScanHistory`) and
 * forecast lead-hour (`useForecastProvider`) selections. This is
 * deliberately **not** a rendering-pipeline merge -- see the stage
 * instructions' scope boundary: the radar map canvas and the forecast panel
 * canvas are untouched; this component only ever calls
 * `history.jumpToNearestByTimestamp` / `forecast.setLeadHours`.
 *
 * # Why a segmented (not single-scale) real-time axis
 *
 * A first version of this component used one linear scale across the whole
 * domain (`(t - domainMin) / (domainMax - domainMin)`). That broke down in
 * this task's own live browser verification: `MAX_HISTORY_SCANS` (15
 * WSR-88D volumes, roughly 1-2.5h of real time) is tiny next to a forecast
 * horizon reaching `Math.max(...FORECAST_LEAD_HOUR_OPTIONS)` hours (48h
 * today) -- two radar scans four minutes apart, at that scale, land under
 * 1px apart. Clicking the older of two such ticks actually hit the newer,
 * visually-overlapping one instead (confirmed live: `aria-pressed` ended up
 * on the wrong tick after the click). Real-time position alone cannot be
 * both "one shared axis" and individually clickable at this ratio of spans.
 *
 * The fix keeps every position derived from real absolute timestamps (still
 * "one real time axis" in the sense the stage spec cares about -- radar and
 * forecast never get independent index-based scales) but gives the past
 * and future *segments* their own fixed pixel budget on either side of a
 * "now" landmark that always sits at `PAST_FRACTION` of the track width:
 * `[domainMin, now] -> [0, PAST_FRACTION]` and
 * `[now, domainMax] -> [PAST_FRACTION, 1]`. Within each segment the mapping
 * is still linear in real time (so relative recency within, say, the held
 * radar history is still faithfully represented) -- only the *slope*
 * changes at the "now" boundary, which is exactly where the spec already
 * demands an unmistakable visual break. This is the standard technique
 * other observed+forecast timeline UIs use for the same reason, not a
 * one-off hack.
 *
 * # Why the "you are viewing" indicator always shows both sides
 *
 * Radar playback and forecast lead-hour are two independent selections (the
 * map canvas and the forecast panel canvas render side by side, per this
 * stage's scope boundary -- there is no single merged "playhead" position
 * that is *either* observed *or* forecast). This indicator therefore always
 * shows both halves at once -- "OBSERVED at <time> / FORECAST (<model>) at
 * <time>" -- each half updating live as its own side of the track is
 * dragged/clicked. That satisfies "always knowing the source" without
 * inventing a single position that would misleadingly imply the two panels
 * are showing the same moment in time.
 */

export interface UnifiedTimelineProps {
  history: ScanHistory;
  forecast: ReturnType<typeof useForecastProvider>;
}

const MS_PER_HOUR = 3_600_000;
/** Floor on each segment's real-time span so a near-empty history (0-1
 * entries) or a not-yet-discovered forecast run never produces a
 * degenerate (or divide-by-zero) segment. */
const MIN_PAST_SPAN_MS = 30 * 60_000;
const MIN_FUTURE_SPAN_MS = 30 * 60_000;
/** Fraction of the track's width given to the past (observed) segment --
 * see this module's doc comment on why the axis is segmented at "now"
 * rather than one single linear scale. Comfortably wide enough that up to
 * `MAX_HISTORY_SCANS` (15) held radar ticks stay individually clickable. */
const PAST_FRACTION = 0.35;
/** How often the "now" marker (and therefore the whole domain, since it
 * always includes "now") re-derives its position. Real-time precision to
 * the second would be pointless here and would re-render this component
 * constantly; a WSR-88D volume completes every several minutes, so a
 * once-a-minute tick is more than enough to keep "now" visibly live. */
const NOW_TICK_MS = 60_000;

function formatTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function useNowMillis(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), NOW_TICK_MS);
    return () => clearInterval(timer);
  }, []);
  return now;
}

export function UnifiedTimeline({ history, forecast }: UnifiedTimelineProps) {
  const nowMs = useNowMillis();
  const trackRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);

  const currentEntry = history.entries[history.currentIndex] ?? null;
  const runInitMs = forecast.run ? Date.parse(forecast.run.initTime) : null;
  const maxLeadHours = Math.max(...FORECAST_LEAD_HOUR_OPTIONS);
  // Ticks live at `runInitTime + leadHours` -- an absolute time, not just
  // "+Nh", per the stage spec ("computed as the current forecast run's init
  // time + lead hours -- an absolute time -- so both halves share one real
  // time axis").
  const forecastValidMs = runInitMs !== null ? runInitMs + forecast.leadHours * MS_PER_HOUR : null;

  // A forecast pipeline in flight (discovering/fetching/rendering) should
  // not accept a new lead-hour request out from under itself -- same
  // `busy` definition `ForecastPanel` uses for its own lead-time <select>.
  const forecastBusy =
    forecast.phase !== "ready" && forecast.phase !== "error" && forecast.phase !== "idle";

  let domainMin = history.entries.length > 0 ? history.entries[0].startTimeMillis : nowMs - MIN_PAST_SPAN_MS;
  domainMin = Math.min(domainMin, nowMs - MIN_PAST_SPAN_MS);

  let domainMax = runInitMs !== null ? runInitMs + maxLeadHours * MS_PER_HOUR : nowMs + MIN_FUTURE_SPAN_MS;
  domainMax = Math.max(domainMax, nowMs + MIN_FUTURE_SPAN_MS);

  const pastSpanMs = nowMs - domainMin;
  const futureSpanMs = domainMax - nowMs;

  /** Real timestamp -> track fraction (0..1), segmented at "now" -- see
   * this module's doc comment. */
  const fractionOf = useCallback(
    (ms: number) => {
      if (ms <= nowMs) {
        return Math.max(0, Math.min(PAST_FRACTION, ((ms - domainMin) / pastSpanMs) * PAST_FRACTION));
      }
      return Math.min(
        1,
        PAST_FRACTION + Math.max(0, ((ms - nowMs) / futureSpanMs) * (1 - PAST_FRACTION)),
      );
    },
    [domainMin, nowMs, pastSpanMs, futureSpanMs],
  );

  /** Inverse of `fractionOf`: track fraction (0..1) -> real timestamp. */
  const timeFromFraction = useCallback(
    (frac: number): number => {
      if (frac <= PAST_FRACTION) {
        return domainMin + (frac / PAST_FRACTION) * pastSpanMs;
      }
      return nowMs + ((frac - PAST_FRACTION) / (1 - PAST_FRACTION)) * futureSpanMs;
    },
    [domainMin, nowMs, pastSpanMs, futureSpanMs],
  );

  const timeFromClientX = useCallback(
    (clientX: number): number => {
      const el = trackRef.current;
      if (!el) return nowMs;
      const rect = el.getBoundingClientRect();
      const frac = rect.width === 0 ? 0 : Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
      return timeFromFraction(frac);
    },
    [nowMs, timeFromFraction],
  );

  /** Apply a real timestamp picked on the track: nearest held radar scan on
   * the observed (<= now) side, nearest available lead-hour option on the
   * forecast (> now) side. This is the single place both drag and
   * click-on-empty-track go through -- individual tick `<button>`s below
   * bypass it and call the exact hook method directly (they already know
   * precisely which entry/lead-hour they represent). */
  const applyAtTime = useCallback(
    (ms: number) => {
      if (ms <= nowMs) {
        if (history.entries.length > 0) history.jumpToNearestByTimestamp(ms);
        return;
      }
      if (runInitMs === null || forecastBusy) return;
      const hoursFromRun = (ms - runInitMs) / MS_PER_HOUR;
      let nearestHours = FORECAST_LEAD_HOUR_OPTIONS[0];
      let bestDiff = Infinity;
      for (const h of FORECAST_LEAD_HOUR_OPTIONS) {
        const diff = Math.abs(h - hoursFromRun);
        if (diff < bestDiff) {
          bestDiff = diff;
          nearestHours = h;
        }
      }
      forecast.setLeadHours(nearestHours);
    },
    [nowMs, history, runInitMs, forecastBusy, forecast],
  );

  const handlePointerDown = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      // Individual tick buttons own their own click handling (an exact
      // entry/lead-hour, not "nearest to pixel position") -- let their
      // native click fire instead of also treating this as a drag-start.
      if ((e.target as HTMLElement).closest(".unified-timeline-tick")) return;
      draggingRef.current = true;
      e.currentTarget.setPointerCapture(e.pointerId);
      applyAtTime(timeFromClientX(e.clientX));
    },
    [applyAtTime, timeFromClientX],
  );

  const handlePointerMove = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      if (!draggingRef.current) return;
      applyAtTime(timeFromClientX(e.clientX));
    },
    [applyAtTime, timeFromClientX],
  );

  const handlePointerUp = useCallback((e: ReactPointerEvent<HTMLDivElement>) => {
    draggingRef.current = false;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
  }, []);

  const providerLabel = forecast.metadata?.displayName ?? forecast.providerId?.toUpperCase() ?? null;

  return (
    <div className="unified-timeline">
      <div className="unified-timeline-header">
        <span className="unified-timeline-source unified-timeline-source-observed">
          OBSERVED{currentEntry ? ` at ${formatTime(currentEntry.startTimeMillis)}` : " — no scan loaded"}
        </span>
        <span className="unified-timeline-divider" aria-hidden="true">
          /
        </span>
        <span className="unified-timeline-source unified-timeline-source-forecast">
          FORECAST{providerLabel ? ` (${providerLabel})` : ""}
          {forecastValidMs !== null ? ` at ${formatTime(forecastValidMs)}` : " — no run loaded"}
        </span>
      </div>

      <div
        ref={trackRef}
        className="unified-timeline-track"
        role="group"
        aria-label="Unified observed/forecast timeline"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerUp}
      >
        {/* The unmistakable present boundary, per the stage spec --
            positioned but non-interactive (`pointer-events: none` in CSS)
            so it never steals a drag/click from the track underneath it.
            Always sits at exactly `PAST_FRACTION` -- see this module's doc
            comment on the segmented scale. */}
        <div className="unified-timeline-now-marker" style={{ left: `${PAST_FRACTION * 100}%` }}>
          <span className="unified-timeline-now-label">NOW</span>
        </div>

        {history.entries.map((entry) => (
          <button
            key={entry.key}
            type="button"
            className={`unified-timeline-tick unified-timeline-tick-observed${
              entry === currentEntry ? " unified-timeline-tick-active" : ""
            }`}
            style={{ left: `${fractionOf(entry.startTimeMillis) * 100}%` }}
            title={`Observed radar scan at ${formatTime(entry.startTimeMillis)}`}
            aria-label={`Observed radar scan at ${formatTime(entry.startTimeMillis)}`}
            aria-pressed={entry === currentEntry}
            onClick={() => history.jumpToNearestByTimestamp(entry.startTimeMillis)}
          />
        ))}

        {runInitMs !== null &&
          FORECAST_LEAD_HOUR_OPTIONS.map((hours) => {
            const tickMs = runInitMs + hours * MS_PER_HOUR;
            const active = hours === forecast.leadHours;
            return (
              <button
                key={hours}
                type="button"
                className={`unified-timeline-tick unified-timeline-tick-forecast${
                  active ? " unified-timeline-tick-active" : ""
                }`}
                style={{ left: `${fractionOf(tickMs) * 100}%` }}
                title={`Forecast +${hours}h — valid ${formatTime(tickMs)} (model precipitation, not radar)`}
                aria-label={`Forecast lead +${hours} hours, valid ${formatTime(tickMs)}`}
                aria-pressed={active}
                disabled={forecastBusy}
                onClick={() => forecast.setLeadHours(hours)}
              />
            );
          })}
      </div>

      <div className="unified-timeline-labels">
        <span>{formatTime(domainMin)}</span>
        <span>{formatTime(domainMax)}</span>
      </div>
    </div>
  );
}
