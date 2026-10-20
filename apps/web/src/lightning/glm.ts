// GOES-East lightning (GLM) flashes from the last 15 minutes, live: polls
// NOAA's public bucket every 20 s, downloads each new 20-second file, decodes
// it with `crates/glm` (wasm) and draws flashes as a placefile layer coloured
// by age, so drawing and hover reuse `placefiles/draw.ts`.

import { useEffect, useState } from "react";
import { parseListObjectsV2 } from "../nexrad/xml";
import type { PlaceItem } from "../placefiles/placefile";
import type { PlacefileLayer } from "../placefiles/usePlacefiles";
import { BOLT } from "../mrms/useMrmsOverlay";
import { legLengthAndBearing } from "../ui/CrossSectionPanel";
import { decodeGlmFlashes, ensureGlmWasmModuleLoaded } from "./wasmModule";

const BUCKET = "https://noaa-goes19.s3.amazonaws.com";
export const GLM_WINDOW_MS = 15 * 60_000;
/** GLM publishes a file every 20 s. */
const POLL_MS = 20_000;
const REQUEST_TIMEOUT_MS = 20_000;
/** North America and nearby waters; most of GOES-East's flashes are over South America. */
const BOX = { west: -135, east: -55, south: 15, north: 60 };

/** Flat `[lon, lat, timeMs, ...]`, as `decodeGlmFlashes` returns. */
export type Flashes = Float64Array;

const pad = (n: number, width = 2) => String(n).padStart(width, "0");

/** The bucket key a file starting at `ms` would have, up to the seconds (for S3's `start-after`). */
export function glmKeyAt(ms: number): string {
  const d = new Date(ms);
  const year = d.getUTCFullYear();
  const doy = pad(Math.floor((ms - Date.UTC(year, 0, 1)) / 86_400_000) + 1, 3);
  const hour = pad(d.getUTCHours());
  return `GLM-L2-LCFA/${year}/${doy}/${hour}/OR_GLM-L2-LCFA_G19_s${year}${doy}${hour}${pad(d.getUTCMinutes())}${pad(d.getUTCSeconds())}`;
}

function inBox(all: Float64Array): Float64Array {
  const kept: number[] = [];
  for (let i = 0; i < all.length; i += 3) {
    const [lon, lat] = [all[i], all[i + 1]];
    if (lon >= BOX.west && lon <= BOX.east && lat >= BOX.south && lat <= BOX.north) kept.push(lon, lat, all[i + 2]);
  }
  return Float64Array.from(kept);
}

async function ok(response: Response): Promise<Response> {
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response;
}

/** Flashes in the last `GLM_WINDOW_MS` over North America, refreshed every 20 s while `enabled`. */
export function useGlmFlashes(enabled: boolean) {
  const [state, setState] = useState<{ flashes: Flashes | null; updatedMs: number; error: string | null }>({ flashes: null, updatedMs: 0, error: null });
  useEffect(() => {
    if (!enabled) {
      setState({ flashes: null, updatedMs: 0, error: null });
      return;
    }
    const controller = new AbortController();
    const { signal } = controller;
    const files = new Map<string, Float64Array>();
    let lastKey: string | null = null;
    let busy = false;
    // A request stuck on a dropped connection would otherwise hold `busy` and stop the feed.
    const get = (url: string) => fetch(url, { signal: AbortSignal.any([signal, AbortSignal.timeout(REQUEST_TIMEOUT_MS)]) }).then(ok);
    const poll = async () => {
      if (busy) return;
      busy = true;
      try {
        await ensureGlmWasmModuleLoaded();
        const now = Date.now();
        const oldest = glmKeyAt(now - GLM_WINDOW_MS);
        // Only keys after the newest one already seen: a few per poll, after the first.
        const after = lastKey && lastKey > oldest ? lastKey : oldest;
        const listing = await get(`${BUCKET}/?list-type=2&prefix=GLM-L2-LCFA/&start-after=${encodeURIComponent(after)}`);
        const keys = parseListObjectsV2(await listing.text()).keys;
        const results = await Promise.allSettled(
          keys.map(async (key) => {
            const bytes = await get(`${BUCKET}/${key}`).then((r) => r.arrayBuffer());
            files.set(key, inBox(decodeGlmFlashes(new Uint8Array(bytes))));
          }),
        );
        if (signal.aborted) return;
        // A file that failed is skipped rather than retried: 20 s of flashes, not a stuck feed.
        if (keys.length > 0) lastKey = keys[keys.length - 1];
        for (const key of files.keys()) if (key < oldest) files.delete(key);
        const failed = results.filter((r) => r.status === "rejected").length;
        const flashes = new Float64Array([...files.values()].reduce((n, f) => n + f.length, 0));
        let at = 0;
        for (const f of files.values()) {
          flashes.set(f, at);
          at += f.length;
        }
        setState({ flashes, updatedMs: now, error: failed > 0 ? `${failed} of ${keys.length} lightning files failed` : null });
      } catch (err) {
        // Ages still advance, so old flashes fade and drop off instead of looking new.
        if (!signal.aborted) setState((s) => ({ ...s, updatedMs: Date.now(), error: err instanceof Error ? err.message : String(err) }));
      } finally {
        busy = false;
      }
    };
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [enabled]);
  return state;
}

/** Age classes, newest first: under 1, 5, 10 and 15 minutes. Same four colours as the MRMS bolts. */
const AGES = [
  { maxMin: 1, color: "#ffffff" },
  { maxMin: 5, color: "#e6dc00" },
  { maxMin: 10, color: "#e67800" },
  { maxMin: 15, color: "#dc0000" },
];
/** The MRMS bolt for one flash is 20 css px tall (half-height 10). */
const BOLT_R = 10;
const ICON_PX = 2 * BOLT_R + 2;
let sheet: ImageBitmap | null = null;

/** One row of bolts, one per age class, drawn like the MRMS lightning layer's (`mrms/useMrmsOverlay.ts`). */
function iconSheet(): ImageBitmap {
  if (sheet) return sheet;
  const canvas = new OffscreenCanvas(ICON_PX * AGES.length, ICON_PX);
  const ctx = canvas.getContext("2d")!;
  ctx.lineWidth = 1;
  ctx.lineJoin = "round";
  ctx.strokeStyle = "rgba(0, 0, 0, 0.85)";
  AGES.forEach(({ color }, i) => {
    const [cx, cy] = [(i + 0.5) * ICON_PX, ICON_PX / 2];
    ctx.beginPath();
    for (const [bx, by] of BOLT) ctx.lineTo(cx + bx * BOLT_R, cy + by * BOLT_R);
    ctx.closePath();
    ctx.fillStyle = color;
    ctx.fill();
    ctx.stroke();
  });
  sheet = canvas.transferToImageBitmap();
  return sheet;
}

const time = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit" });

/** Flashes from the last 15 min as icons, oldest first so the newest draw on top. */
export function glmLayer(flashes: Flashes, nowMs: number): PlacefileLayer {
  const order: number[] = [];
  for (let i = 0; i < flashes.length; i += 3) if (nowMs - flashes[i + 2] <= GLM_WINDOW_MS) order.push(i);
  order.sort((a, b) => flashes[a + 2] - flashes[b + 2]);
  const items = order.map((i): PlaceItem => {
    const ageMin = (nowMs - flashes[i + 2]) / 60_000;
    const index = AGES.findIndex((a) => ageMin < a.maxMin) + 1 || AGES.length;
    return { kind: "icon", anchor: null, threshold: 999, color: "", at: [flashes[i], flashes[i + 1]], angle: 0, file: 1, index, hover: `Lightning ${time(flashes[i + 2])}` };
  });
  return {
    url: "glm",
    file: {
      title: "GOES lightning",
      refreshSeconds: 0,
      iconFiles: new Map([[1, { url: "", width: ICON_PX, height: ICON_PX, hotX: ICON_PX / 2, hotY: ICON_PX / 2 }]]),
      fonts: new Map(),
      items,
    },
    sheets: new Map([[1, iconSheet()]]),
  };
}

/** Distance (km) to the nearest flash since `sinceMs`, or null if none. */
export function nearestFlashKm(flashes: Flashes, here: { lat: number; lon: number }, sinceMs: number): number | null {
  let best: number | null = null;
  for (let i = 0; i < flashes.length; i += 3) {
    if (flashes[i + 2] < sinceMs) continue;
    const km = legLengthAndBearing(here, { lat: flashes[i + 1], lon: flashes[i] }).km;
    if (best === null || km < best) best = km;
  }
  return best;
}
