import { useRef, useState } from "react";
import { parseColorTableJson, rgbaToCss, type ColorTableJson } from "../colorTables/colorTableJson";

export interface ExportPanelProps {
  /** The map as one canvas at device resolution (`MapView`'s `exportRef`), or null before the map exists. */
  capture: () => HTMLCanvasElement | null;
  /** "KTLX Norman · REF 0.5°" */
  title: string;
  timeMs: number | null;
  tableJson: string | null;
  /** Start times of the held scans, oldest first; empty when there is nothing to loop. */
  frameTimes: number[];
  frameMs: number;
  /** The scan on screen is drawn and the time-keyed overlays (archive warnings, storm tracks, SPC) match it. */
  frameReady: boolean;
  /** Shows the scan at this time and pauses playback. */
  goTo: (ms: number) => void;
  /** Puts playback back where it was before the loop export. */
  restore: () => void;
}

/** Image (PNG) and loop (MP4, else WebM) export of the map with a legend and site/time stamp. */
export function ExportPanel({ capture, title, timeMs, tableJson, frameTimes, frameMs, frameReady, goTo, restore }: ExportPanelProps) {
  const [busy, setBusy] = useState<string | null>(null);
  // Read inside the loop's awaits, which would otherwise see the value from when it started.
  const readyRef = useRef(frameReady);
  readyRef.current = frameReady;
  const table = tableJson ? parseColorTableJson(tableJson) : null;

  function frame(ms: number | null) {
    const canvas = capture();
    if (canvas) stamp(canvas, title, ms, table);
    return canvas;
  }

  function saveImage() {
    frame(timeMs)?.toBlob((blob) => blob && save(blob, fileName(title, timeMs, "png")));
  }

  async function saveLoop() {
    setBusy("Capturing frames…");
    const frames: HTMLCanvasElement[] = [];
    try {
      for (const ms of frameTimes) {
        goTo(ms);
        // One render to switch the scan, then wait for it to be drawn and its overlays fetched (up to 15 s).
        await sleep(100);
        for (let waited = 0; !readyRef.current && waited < 15_000; waited += 100) await sleep(100);
        const f = frame(ms);
        if (f) frames.push(f);
      }
    } finally {
      restore();
    }
    if (frames.length) {
      setBusy("Encoding…");
      const { blob, ext } = await encode(frames, frameMs);
      save(blob, fileName(title, frameTimes[0], ext));
    }
    setBusy(null);
  }

  return (
    <>
      <div className="export-actions">
        <button type="button" onClick={saveImage} disabled={busy !== null}>
          Save image
        </button>
        <button type="button" onClick={saveLoop} disabled={busy !== null || frameTimes.length < 2} title="Scans held in the loop">
          Save loop
        </button>
      </div>
      {busy && <div className="cross-section-hint">{busy}</div>}
    </>
  );
}

function stamp(canvas: HTMLCanvasElement, title: string, ms: number | null, table: ColorTableJson | null) {
  const ctx = canvas.getContext("2d")!;
  const s = window.devicePixelRatio || 1;
  const pad = 10 * s;
  ctx.globalAlpha = 1;
  ctx.textBaseline = "top";

  const lines = [title, ms === null ? "" : formatTime(ms), "RadarPro"].filter(Boolean);
  ctx.font = `600 ${14 * s}px system-ui, sans-serif`;
  const w = Math.max(...lines.map((l) => ctx.measureText(l).width)) + 2 * pad;
  const h = lines.length * 18 * s + 2 * pad;
  ctx.fillStyle = "rgba(0, 0, 0, 0.7)";
  ctx.fillRect(pad, canvas.height - h - pad, w, h);
  ctx.fillStyle = "#fff";
  lines.forEach((l, i) => ctx.fillText(l, 2 * pad, canvas.height - h + i * 18 * s));

  if (!table) return;
  const barW = 260 * s;
  const x = canvas.width - barW - 2 * pad;
  const y = canvas.height - 46 * s - pad;
  ctx.fillStyle = "rgba(0, 0, 0, 0.7)";
  ctx.fillRect(x - pad, y - pad, barW + 2 * pad, 46 * s);
  const gradient = ctx.createLinearGradient(x, 0, x + barW, 0);
  for (const [offset, color] of legendStops(table)) gradient.addColorStop(offset, color);
  ctx.fillStyle = gradient;
  ctx.fillRect(x, y, barW, 12 * s);
  ctx.fillStyle = "#fff";
  ctx.font = `600 ${12 * s}px system-ui, sans-serif`;
  ctx.fillText(String(table.domain.min), x, y + 16 * s);
  ctx.textAlign = "center";
  ctx.fillText(table.units, x + barW / 2, y + 16 * s);
  ctx.textAlign = "right";
  ctx.fillText(String(table.domain.max), x + barW, y + 16 * s);
  ctx.textAlign = "left";
}

/** The table's stops as offsets 0-1; stepped tables repeat each color up to the next stop (hard edges). */
export function legendStops(table: ColorTableJson): [number, string][] {
  const { min, max } = table.domain;
  const at = (v: number) => Math.min(1, Math.max(0, (v - min) / (max - min || 1)));
  return table.stops.flatMap((stop, i) => {
    const color = rgbaToCss(stop.color);
    if (table.mode !== "stepped") return [[at(stop.value), color]];
    const next = table.stops[i + 1];
    return [
      [at(stop.value), color],
      [next ? at(next.value) : 1, color],
    ];
  });
}

/** Plays the frames into a MediaRecorder in real time, the last one held three times as long. */
async function encode(frames: HTMLCanvasElement[], frameMs: number) {
  const mp4 = MediaRecorder.isTypeSupported("video/mp4;codecs=avc1");
  const mimeType = mp4 ? "video/mp4;codecs=avc1" : "video/webm";
  const out = document.createElement("canvas");
  // At most 1920 wide (social sites and phones reject bigger video); H.264 needs even dimensions.
  const scale = Math.min(1, 1920 / frames[0].width);
  out.width = Math.round(frames[0].width * scale) & ~1;
  out.height = Math.round(frames[0].height * scale) & ~1;
  const ctx = out.getContext("2d")!;
  ctx.scale(scale, scale);
  const stream = out.captureStream(0);
  const track = stream.getVideoTracks()[0] as CanvasCaptureMediaStreamTrack;
  const recorder = new MediaRecorder(stream, { mimeType, videoBitsPerSecond: 8_000_000 });
  const chunks: Blob[] = [];
  recorder.ondataavailable = (e) => chunks.push(e.data);
  const stopped = new Promise((r) => (recorder.onstop = r));
  recorder.start();
  for (const [i, f] of frames.entries()) {
    ctx.drawImage(f, 0, 0);
    track.requestFrame();
    await sleep(i === frames.length - 1 ? frameMs * 3 : frameMs);
  }
  // A frame's length is the gap to the next one, so repeat the last frame to end the hold.
  // Redrawn because an unchanged canvas sends no frame.
  ctx.drawImage(frames[frames.length - 1], 0, 0);
  track.requestFrame();
  await sleep(100);
  recorder.stop();
  await stopped;
  return { blob: new Blob(chunks, { type: mimeType }), ext: mp4 ? "mp4" : "webm" };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function save(blob: Blob, name: string) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 10_000);
}

function formatTime(ms: number) {
  return new Date(ms).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZoneName: "short",
  });
}

function fileName(title: string, ms: number | null, ext: string) {
  const site = title.split(" ")[0];
  const when = ms === null ? "" : `-${new Date(ms).toISOString().slice(0, 16).replace(/[-:]/g, "").replace("T", "-")}Z`;
  return `RadarPro-${site}${when}.${ext}`;
}
