import { useEffect, useMemo, useState } from "react";
import { isDesktop } from "../platform/desktop";
import { parsePlacefile, type Placefile } from "./placefile";

/** One-click suggestions; both are public and need no login. */
export const SUGGESTED_PLACEFILES = [
  {
    name: "Spotter Network positions",
    url: "https://www.spotternetwork.org/feeds/gr.txt",
  },
  {
    name: "Spotter Network storm reports",
    url: "https://www.spotternetwork.org/feeds/reports.txt",
  },
];

/** Some files ask for a refresh every few seconds; this is the fastest we poll. */
const MIN_REFRESH_MS = 30_000;
/** Files that don't say how often to refresh. */
const DEFAULT_REFRESH_MS = 5 * 60_000;
const STORAGE_KEY = "radarpro.placefiles";

export interface PlacefileEntry {
  url: string;
  on: boolean;
}

export interface PlacefileLayer {
  url: string;
  file: Placefile;
  /** Loaded icon sheets by IconFile number; a sheet that failed is missing. */
  sheets: Map<number, ImageBitmap>;
}

export interface PlacefileStatus {
  title: string;
  items: number;
  lastMs: number | null;
  error: string | null;
}

/** Raw bytes of `url`, through the desktop command or the dev-server relay (no CORS needed). */
export async function fetchUrlBytes(url: string, signal?: AbortSignal): Promise<Uint8Array> {
  if (isDesktop()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return new Uint8Array(await invoke<ArrayBuffer>("fetch_url", { url }));
  }
  const response = await fetch(`/fetch-url?url=${encodeURIComponent(url)}`, {
    signal,
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

/** Icon sheets rarely change; kept for the session by URL. */
const sheetCache = new Map<string, Promise<ImageBitmap>>();

/** GR's rule: a sheet with no alpha channel has solid black made transparent.
 * A decoded image can't say if it had one, so "every pixel opaque" stands in. */
function blackToTransparent(bitmap: ImageBitmap): Promise<ImageBitmap> | ImageBitmap {
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const ctx = canvas.getContext("2d");
  if (!ctx) return bitmap;
  ctx.drawImage(bitmap, 0, 0);
  const image = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
  const px = image.data;
  for (let i = 3; i < px.length; i += 4) if (px[i] < 255) return bitmap;
  for (let i = 0; i < px.length; i += 4) if (px[i] === 0 && px[i + 1] === 0 && px[i + 2] === 0) px[i + 3] = 0;
  ctx.putImageData(image, 0, 0);
  return createImageBitmap(canvas);
}

function loadSheet(url: string): Promise<ImageBitmap> {
  let sheet = sheetCache.get(url);
  if (!sheet) {
    sheet = fetchUrlBytes(url)
      .then((bytes) => createImageBitmap(new Blob([bytes as BlobPart])))
      .then(blackToTransparent);
    sheet.catch(() => sheetCache.delete(url));
    sheetCache.set(url, sheet);
  }
  return sheet;
}

async function loadLayer(url: string, signal: AbortSignal): Promise<PlacefileLayer> {
  const text = new TextDecoder().decode(await fetchUrlBytes(url, signal));
  const file = parsePlacefile(text, url);
  const sheets = new Map<number, ImageBitmap>();
  await Promise.all(
    [...file.iconFiles].map(([n, icons]) =>
      loadSheet(icons.url).then(
        (bitmap) => void sheets.set(n, bitmap),
        () => {},
      ),
    ),
  );
  return { url, file, sheets };
}

function loadEntries(): PlacefileEntry[] {
  try {
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]");
    return Array.isArray(stored) ? stored.filter((e) => typeof e?.url === "string").map((e) => ({ url: e.url, on: e.on !== false })) : [];
  } catch {
    return [];
  }
}

/**
 * The user's placefile list (remembered across launches) and, for each one
 * switched on, its latest contents, re-fetched as often as the file asks.
 */
export function usePlacefiles() {
  const [entries, setEntries] = useState<PlacefileEntry[]>(loadEntries);
  const [layers, setLayers] = useState<Map<string, PlacefileLayer>>(new Map());
  const [status, setStatus] = useState<Map<string, PlacefileStatus>>(new Map());

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries));
  }, [entries]);

  const onKey = entries
    .filter((e) => e.on)
    .map((e) => e.url)
    .join("\n");

  useEffect(() => {
    const urls = onKey ? onKey.split("\n") : [];
    // Drop layers that were switched off or removed.
    setLayers((prev) => new Map([...prev].filter(([url]) => urls.includes(url))));
    const controller = new AbortController();
    const timers: ReturnType<typeof setTimeout>[] = [];
    for (const url of urls) {
      const poll = async () => {
        let next = DEFAULT_REFRESH_MS;
        try {
          const layer = await loadLayer(url, controller.signal);
          if (controller.signal.aborted) return;
          setLayers((prev) => new Map(prev).set(url, layer));
          setStatus((prev) =>
            new Map(prev).set(url, {
              title: layer.file.title,
              items: layer.file.items.length,
              lastMs: Date.now(),
              error: null,
            }),
          );
          if (layer.file.refreshSeconds > 0) next = Math.max(MIN_REFRESH_MS, layer.file.refreshSeconds * 1000);
        } catch (err) {
          if (controller.signal.aborted) return;
          // A failed refresh keeps the last good contents on the map.
          const error = err instanceof Error ? err.message : String(err);
          setStatus((prev) =>
            new Map(prev).set(url, {
              title: prev.get(url)?.title ?? "",
              items: prev.get(url)?.items ?? 0,
              lastMs: prev.get(url)?.lastMs ?? null,
              error,
            }),
          );
          next = MIN_REFRESH_MS * 2;
        }
        timers.push(setTimeout(poll, next));
      };
      poll();
    }
    return () => {
      controller.abort();
      timers.forEach(clearTimeout);
    };
  }, [onKey]);

  // Stable between fetches: MapView redraws its overlay whenever this changes.
  const list = useMemo(() => [...layers.values()], [layers]);
  return { entries, setEntries, layers: list, status };
}
