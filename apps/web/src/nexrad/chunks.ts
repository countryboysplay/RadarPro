// The real-time Level II feed: each volume arrives as numbered chunks
// (`ICAO/<volume 1-999>/<YYYYMMDD-HHMMSS>-<NNN>-<S|I|E>`, about one every
// 12 s) in Unidata's chunks bucket, minutes before the finished volume
// lands in the archive bucket. Chunks concatenated in order are an
// ordinary (partial, until the E chunk) Archive II volume.

import { fetchWithRetry } from "./bucket";
import type { DiscoveredVolume } from "./keys";
import { parseListObjectsV2 } from "./xml";

export const NEXRAD_CHUNKS_BUCKET_URL = "https://unidata-nexrad-level2-chunks.s3.amazonaws.com";

/** Volume numbers cycle 1..999 per site. */
const VOLUME_NUMBER_MAX = 999;

export interface ChunkName {
  /** Volume start, as in the chunk name. */
  startTimeMillis: number;
  /** 1-based position within the volume. */
  index: number;
  /** S = start (volume header + metadata), I = intermediate, E = end. */
  kind: "S" | "I" | "E";
}

/** Parse a chunk object's file name (`20261019-172040-003-I`); null if it isn't one. */
export function parseChunkName(name: string): ChunkName | null {
  const m = /^(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})-(\d{3})-([SIE])$/.exec(name);
  if (!m) return null;
  const [, y, mo, d, h, mi, s, n, kind] = m;
  return {
    startTimeMillis: Date.UTC(+y, +mo - 1, +d, +h, +mi, +s),
    index: Number(n),
    kind: kind as ChunkName["kind"],
  };
}

/** The volume after `n`, wrapping 999 -> 1. */
export function nextVolumeNumber(n: number): number {
  return (n % VOLUME_NUMBER_MAX) + 1;
}

/** The volume before `n`, wrapping 1 -> 999. */
export function previousVolumeNumber(n: number): number {
  return n === 1 ? VOLUME_NUMBER_MAX : n - 1;
}

/**
 * The most recently written volume number among a site's folders. Numbers
 * wrap at 999 and old folders linger, so it's the number just before the
 * widest gap in the circular sequence, not simply the largest.
 */
export function newestVolumeNumber(numbers: number[]): number | null {
  const sorted = [...new Set(numbers)].sort((a, b) => a - b);
  if (sorted.length === 0) return null;
  let newest = sorted[sorted.length - 1];
  let widest = sorted[0] + VOLUME_NUMBER_MAX - newest; // gap across the wrap
  for (let i = 0; i + 1 < sorted.length; i++) {
    const gap = sorted[i + 1] - sorted[i];
    if (gap > widest) {
      widest = gap;
      newest = sorted[i];
    }
  }
  return newest;
}

/** The archive bucket's key for the same volume, so both feeds share one history entry. */
export function archiveKeyFor(icao: string, startTimeMillis: number, formatVersion: number): string {
  const iso = new Date(startTimeMillis).toISOString(); // 2026-10-19T17:20:40.000Z
  const [y, mo, d] = [iso.slice(0, 4), iso.slice(5, 7), iso.slice(8, 10)];
  const hms = iso.slice(11, 19).replaceAll(":", "");
  return `${y}/${mo}/${d}/${icao}/${icao}${y}${mo}${d}_${hms}_V0${formatVersion}`;
}

async function list(prefix: string, delimiter: boolean, signal: AbortSignal) {
  const url = new URL(NEXRAD_CHUNKS_BUCKET_URL);
  url.searchParams.set("list-type", "2");
  url.searchParams.set("prefix", prefix);
  if (delimiter) url.searchParams.set("delimiter", "/");
  const response = await fetchWithRetry(url.toString(), { signal });
  // shortcut: one page (1000 entries) -- a site has <=999 volume folders and ~80 chunks per volume.
  return parseListObjectsV2(await response.text());
}

/** The site's newest real-time volume number, or null if the feed has none. */
export async function discoverNewestVolume(icao: string, signal: AbortSignal): Promise<number | null> {
  const page = await list(`${icao}/`, true, signal);
  const numbers = page.prefixes.map((p) => Number(p.split("/")[1])).filter((n) => Number.isInteger(n) && n > 0);
  return newestVolumeNumber(numbers);
}

export interface ListedChunk extends ChunkName {
  key: string;
}

/** One volume's chunks so far, in order. */
export async function listVolumeChunks(icao: string, volume: number, signal: AbortSignal): Promise<ListedChunk[]> {
  const page = await list(`${icao}/${volume}/`, false, signal);
  return page.keys
    .map((key) => {
      const name = parseChunkName(key.split("/").pop() ?? "");
      return name && { ...name, key };
    })
    .filter((c): c is ListedChunk => c !== null)
    .sort((a, b) => a.index - b.index);
}

export async function downloadChunk(key: string, signal: AbortSignal): Promise<Uint8Array> {
  const response = await fetchWithRetry(`${NEXRAD_CHUNKS_BUCKET_URL}/${key}`, { signal });
  return new Uint8Array(await response.arrayBuffer());
}

/**
 * Follows one volume as its chunks arrive. `poll` downloads any new chunks
 * (strictly in order -- a missing index waits for the next poll) and
 * returns the assembled volume bytes when something changed.
 */
export class ChunkedVolume {
  private parts: Uint8Array[] = [];
  private start: ChunkName | null = null;
  complete = false;

  constructor(
    readonly icao: string,
    readonly volume: number,
  ) {}

  get chunkCount(): number {
    return this.parts.length;
  }

  /** Archive-style identity for the volume, once its S chunk is in. */
  describe(): DiscoveredVolume | null {
    if (!this.start) return null;
    // Archive II header: "AR2V00" + 2-digit format version.
    const version = Number(new TextDecoder().decode(this.parts[0].subarray(6, 8))) || 6;
    return {
      icao: this.icao,
      key: archiveKeyFor(this.icao, this.start.startTimeMillis, version),
      formatVersion: version,
      startTimeMillis: this.start.startTimeMillis,
    };
  }

  async poll(signal: AbortSignal): Promise<Uint8Array | null> {
    if (this.complete) return null;
    const chunks = await listVolumeChunks(this.icao, this.volume, signal);
    let changed = false;
    for (const chunk of chunks) {
      if (chunk.index !== this.parts.length + 1) {
        if (chunk.index <= this.parts.length) continue;
        break; // gap: wait for the missing chunk
      }
      if (chunk.index === 1 && chunk.kind !== "S") break;
      this.parts.push(await downloadChunk(chunk.key, signal));
      if (chunk.index === 1) this.start = chunk;
      if (chunk.kind === "E") this.complete = true;
      changed = true;
    }
    if (!changed || !this.start) return null;
    const total = this.parts.reduce((n, p) => n + p.length, 0);
    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const p of this.parts) {
      bytes.set(p, offset);
      offset += p.length;
    }
    return bytes;
  }
}
