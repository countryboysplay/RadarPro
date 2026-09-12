// S3 object key parsing/filtering and UTC calendar-date handling for
// discovery -- a TypeScript re-implementation of
// `crates/radar-cache/src/keys.rs`'s logic (read for reference per this
// task's brief; not reused directly since that crate is native/tokio-only).
//
// Object keys look like
// `{year}/{month:02}/{day:02}/{ICAO}/{ICAO}{yyyyMMdd}_{HHmmss}_V0{2-7}`.
// `parseObjectKey` recognizes exactly that filename shape and rejects
// everything else, including `_MDM` supplemental-metadata companion objects
// and any other unrecognized suffix -- per the discovery contract, keys are
// untrusted remote-sourced strings, and a key that doesn't match is skipped,
// never treated as an error.

export interface DiscoveredVolume {
  /** Four-letter ICAO site identifier, from the object key itself. */
  icao: string;
  /** The full S3 object key, e.g. "2026/09/12/KTLX/KTLX20260912_000110_V06". */
  key: string;
  /** Archive II format version parsed from the key's `_V0N` suffix (2-7). */
  formatVersion: number;
  /** Volume start time (UTC epoch milliseconds), from the key's timestamp. */
  startTimeMillis: number;
}

const EXPECTED_FILE_NAME_LEN = 23; // ICAO(4) + yyyyMMdd(8) + _(1) + HHmmss(6) + _(1) + V0N(3)

function allAsciiDigits(s: string): boolean {
  return s.length > 0 && /^[0-9]+$/.test(s);
}

function isAscii(s: string): boolean {
  for (let i = 0; i < s.length; i++) {
    if (s.charCodeAt(i) > 0x7f) return false;
  }
  return true;
}

/**
 * Parse one S3 object key into a {@link DiscoveredVolume}, or return `null`
 * if it is not a recognized Level II volume-scan object (a `_MDM`
 * companion, an unsupported/garbled format-version suffix, or anything else
 * that doesn't match the expected shape exactly). Never throws on
 * malformed/adversarial input.
 */
export function parseObjectKey(key: string): DiscoveredVolume | null {
  const fileName = key.slice(key.lastIndexOf("/") + 1);

  if (fileName.length !== EXPECTED_FILE_NAME_LEN || !isAscii(fileName)) {
    return null;
  }

  const icao = fileName.slice(0, 4);
  const dateDigits = fileName.slice(4, 12);
  const sep1 = fileName[12];
  const timeDigits = fileName.slice(13, 19);
  const sep2 = fileName[19];
  const versionTag = fileName.slice(20, 23);

  if (!/^[A-Z]{4}$/.test(icao)) return null;
  if (sep1 !== "_" || sep2 !== "_") return null;
  if (!allAsciiDigits(dateDigits) || !allAsciiDigits(timeDigits)) return null;
  if (versionTag[0] !== "V" || versionTag[1] !== "0") return null;
  const versionDigit = versionTag.charCodeAt(2);
  if (versionDigit < "2".charCodeAt(0) || versionDigit > "7".charCodeAt(0)) return null;
  const formatVersion = versionDigit - "0".charCodeAt(0);

  const year = Number(dateDigits.slice(0, 4));
  const month = Number(dateDigits.slice(4, 6));
  const day = Number(dateDigits.slice(6, 8));
  const hour = Number(timeDigits.slice(0, 2));
  const minute = Number(timeDigits.slice(2, 4));
  const second = Number(timeDigits.slice(4, 6));

  if (month < 1 || month > 12 || day < 1 || day > 31) return null;
  // Tolerate a leap second (60), matching keys.rs's rationale.
  if (hour > 23 || minute > 59 || second > 60) return null;

  // `Date.UTC` normalizes out-of-range components (e.g. day 32) rather than
  // rejecting them, so validity was already checked above; this is purely
  // the calendar -> epoch-millis conversion.
  const startTimeMillis = Date.UTC(year, month - 1, day, hour, minute, Math.min(second, 59));

  return { icao, key, formatVersion, startTimeMillis };
}

/** A UTC calendar date (no time-of-day), used to select which day's Level II objects to discover. */
export interface CacheDate {
  year: number;
  month: number;
  day: number;
}

export function todayUtc(): CacheDate {
  const now = new Date();
  return { year: now.getUTCFullYear(), month: now.getUTCMonth() + 1, day: now.getUTCDate() };
}

/** The S3 key prefix for this date/site, e.g. "2026/09/12/KTLX/". */
export function keyPrefix(date: CacheDate, icao: string): string {
  const pad = (n: number, width: number) => String(n).padStart(width, "0");
  return `${pad(date.year, 4)}/${pad(date.month, 2)}/${pad(date.day, 2)}/${icao}/`;
}

/** Sort ascending by (startTimeMillis, key) -- chronological, key as tiebreaker. */
export function compareDiscoveredVolumes(a: DiscoveredVolume, b: DiscoveredVolume): number {
  if (a.startTimeMillis !== b.startTimeMillis) return a.startTimeMillis - b.startTimeMillis;
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}
