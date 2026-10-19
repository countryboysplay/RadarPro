// @ts-expect-error -- the app tsconfig has no Node types; vitest runs under Node.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parseHail, parseMeso, parseNst, parseTvs } from "../storms/nst";
import { nearbyHazards } from "../storms/nearbyHazards";
import { parseObjectKey } from "../nexrad/keys";
import { clampHistoryLimit, historyReducer, initialState } from "../scan/useScanHistory";
import { alertNearSite, pointInAlertGeometry } from "../alerts/geometryHitTest";
import { coveringAlerts } from "../location/useMyLocation";
import { parseColorTableJson } from "../colorTables/colorTableJson";
import { legLengthAndBearing } from "../ui/CrossSectionPanel";
import { archiveKeyFor, newestVolumeNumber, nextVolumeNumber, parseChunkName, previousVolumeNumber } from "../nexrad/chunks";

describe("parseObjectKey", () => {
  it("parses a real Level II key", () => {
    const v = parseObjectKey("2026/09/12/KTLX/KTLX20260912_000110_V06");
    expect(v).toMatchObject({ icao: "KTLX", formatVersion: 6, startTimeMillis: Date.UTC(2026, 8, 12, 0, 1, 10) });
  });
  it("accepts gzipped archive keys", () => {
    expect(parseObjectKey("2013/05/20/KTLX/KTLX20130520_195004_V06.gz")).toMatchObject({
      icao: "KTLX",
      startTimeMillis: Date.UTC(2013, 4, 20, 19, 50, 4),
    });
  });
  it("rejects companion and garbage keys", () => {
    expect(parseObjectKey("2026/09/12/KTLX/KTLX20260912_000110_V06_MDM")).toBeNull();
    expect(parseObjectKey("")).toBeNull();
    expect(parseObjectKey("not/a/key")).toBeNull();
  });
});

describe("historyReducer", () => {
  const add = (s: ReturnType<typeof initialState>, n: number) =>
    historyReducer(s, { type: "SCAN_ADDED", key: `k${n}`, startTimeMillis: n * 1000 });

  it("live mode tracks the newest scan and evicts oldest past the cap", () => {
    let s = initialState(5);
    for (let i = 0; i < 7; i++) s = add(s, i);
    expect(s.entries.map((e) => e.key)).toEqual(["k2", "k3", "k4", "k5", "k6"]);
    expect(s.currentIndex).toBe(4);
  });

  it("a paused view stays on its frame when new scans arrive", () => {
    let s = initialState(5);
    for (let i = 0; i < 3; i++) s = add(s, i);
    s = historyReducer(s, { type: "STEP", delta: -1 });
    expect(s.playMode).toBe("paused");
    s = add(s, 3);
    expect(s.entries[s.currentIndex].key).toBe("k1");
  });

  it("backfilled older scans slot in by time, without duplicates or moving the view", () => {
    let s = add(initialState(5), 9);
    for (const n of [8, 7, 9]) s = add(s, n);
    expect(s.entries.map((e) => e.key)).toEqual(["k7", "k8", "k9"]);
    expect(s.entries[s.currentIndex].key).toBe("k9");
  });

  it("clamps the history limit", () => {
    expect(clampHistoryLimit(1)).toBe(5);
    expect(clampHistoryLimit(1000)).toBe(60);
    expect(clampHistoryLimit(Number.NaN)).toBe(15);
  });
});

describe("pointInAlertGeometry", () => {
  const square = { type: "Polygon" as const, coordinates: [[[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]]] };
  it("hits inside and misses outside", () => {
    expect(pointInAlertGeometry([5, 5], square as never)).toBe(true);
    expect(pointInAlertGeometry([15, 5], square as never)).toBe(false);
  });
  it("respects holes", () => {
    const holed = {
      type: "Polygon" as const,
      coordinates: [square.coordinates[0], [[4, 4], [6, 4], [6, 6], [4, 6], [4, 4]]],
    };
    expect(pointInAlertGeometry([5, 5], holed as never)).toBe(false);
    expect(pointInAlertGeometry([2, 2], holed as never)).toBe(true);
  });
});

describe("coveringAlerts", () => {
  const square = { type: "Polygon" as const, coordinates: [[[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]]] };
  const held = (key: string, geometry: unknown) => ({ key, alert: { geometry } }) as never;
  const active = [held("in", square), held("zone-only", null), held("far", { ...square, coordinates: [[[20, 20], [30, 20], [30, 30], [20, 20]]] })];
  it("finds polygons covering the point and only reports new ones as fresh", () => {
    const first = coveringAlerts({ lat: 5, lon: 5 }, active, new Set());
    expect(first.covering.map((a) => a.key)).toEqual(["in"]);
    expect(first.fresh.map((a) => a.key)).toEqual(["in"]);
    expect(coveringAlerts({ lat: 5, lon: 5 }, active, new Set(["in"])).fresh).toEqual([]);
    expect(coveringAlerts({ lat: 50, lon: 5 }, active, new Set()).covering).toEqual([]);
  });
});

describe("parseColorTableJson", () => {
  it("returns null on malformed input instead of throwing", () => {
    expect(parseColorTableJson("{")).toBeNull();
    expect(parseColorTableJson('{"name": 3}')).toBeNull();
  });
});


describe("legLengthAndBearing", () => {
  it("measures a due-east leg along the equator", () => {
    const { km, deg } = legLengthAndBearing({ lat: 0, lon: 0 }, { lat: 0, lon: 1 });
    expect(km).toBeCloseTo(111.2, 1);
    expect(deg).toBeCloseTo(90, 5);
  });
  it("reports a northbound leg as 000", () => {
    expect(legLengthAndBearing({ lat: 35, lon: -97 }, { lat: 36, lon: -97 }).deg).toBeCloseTo(0, 5);
  });
});

describe("alertNearSite", () => {
  const ktlx = { lat: 35.333, lon: -97.278 };
  const square = (lon: number, lat: number) => ({
    type: "Polygon" as const,
    coordinates: [[[lon, lat], [lon + 0.5, lat], [lon + 0.5, lat + 0.5], [lon, lat]]] as [number, number][][],
  });
  it("keeps polygons in radar range and drops distant or zone-only alerts", () => {
    expect(alertNearSite(square(-97.5, 35.5), ktlx)).toBe(true);
    expect(alertNearSite(square(-134.5, 58.3), ktlx)).toBe(false); // Juneau
    expect(alertNearSite(null, ktlx)).toBe(false);
  });
});

describe("real-time chunk feed", () => {
  it("parses chunk names", () => {
    expect(parseChunkName("20261019-172040-003-I")).toEqual({
      startTimeMillis: Date.UTC(2026, 9, 19, 17, 20, 40),
      index: 3,
      kind: "I",
    });
    expect(parseChunkName("20261019-172040-003")).toBeNull();
  });

  it("finds the newest volume across the 999 wrap and lingering old folders", () => {
    // Live KTLX shape: an old run 211-258, a stray 269, then 322-676 current.
    const live = [...Array.from({ length: 48 }, (_, i) => 211 + i), 269, ...Array.from({ length: 355 }, (_, i) => 322 + i)];
    expect(newestVolumeNumber(live)).toBe(676);
    // Just wrapped: 995-999 then 1-3.
    expect(newestVolumeNumber([995, 996, 997, 998, 999, 1, 2, 3])).toBe(3);
    expect(newestVolumeNumber([])).toBeNull();
    expect(nextVolumeNumber(999)).toBe(1);
    expect(previousVolumeNumber(1)).toBe(999);
  });

  it("names a real-time volume with the archive's key", () => {
    expect(archiveKeyFor("KTLX", Date.UTC(2026, 9, 19, 17, 20, 40), 6)).toBe("2026/10/19/KTLX/KTLX20261019_172040_V06");
  });

  it("updates a held volume in place when its bytes grow", () => {
    let s = historyReducer(initialState(5), { type: "SCAN_ADDED", key: "k", startTimeMillis: 1, revision: 100 });
    s = historyReducer(s, { type: "SCAN_ADDED", key: "k", startTimeMillis: 1, revision: 250 });
    expect(s.entries).toEqual([{ key: "k", startTimeMillis: 1, revision: 250 }]);
    expect(historyReducer(s, { type: "SCAN_ADDED", key: "k", startTimeMillis: 1, revision: 250 })).toBe(s);
  });
});

describe("parseNst", () => {
  // Real KBYX product; expected values come from its own text table.
  const bytes = new Uint8Array(readFileSync(new URL("../../../../fixtures/nexrad-level3/KBYX_NST_20261019_2005", import.meta.url)));
  const tracks = parseNst(bytes);
  const radar = { lat: 24.597, lon: -81.703 };

  it("reads every cell, the scan time and the average motion", () => {
    expect(tracks.cells.map((c) => c.id)).toEqual(["Q8", "S3", "H9", "F2", "Z3", "D6", "M9", "X6", "N4", "H8"]);
    expect(tracks.scanTimeMs).toBe(Date.UTC(2026, 9, 19, 20, 5, 30));
    expect(tracks.avgMotion).toEqual({ speedKts: 8, fromDeg: 50 });
  });

  it("places cells where the table says (Q8 at 108 deg / 121 nm)", () => {
    const q8 = tracks.cells[0];
    const { km, deg } = legLengthAndBearing(radar, { lon: q8.at[0], lat: q8.at[1] });
    expect(deg).toBeCloseTo(108, 0);
    expect(km / 1.852).toBeCloseTo(121, 0);
    expect(q8.forecast).toHaveLength(5);
    expect(q8.forecast[0]).toEqual(q8.at);
    // M9 moves from 28 deg, so its 60-minute forecast is farther south.
    const m9 = tracks.cells[6];
    expect(m9.forecast[4][1]).toBeLessThan(m9.at[1]);
  });

  it("gives new cells no track", () => {
    const h9 = tracks.cells[2];
    expect(h9.past).toEqual([]);
    expect(h9.forecast).toEqual([]);
    expect(h9.motion).toBeNull();
  });

  it("derives each cell's motion close to the table's FCST MVT", () => {
    // Table: M9 28/8, X6 3/11, N4 75/28, H8 37/8 (deg from / kt).
    const table: Record<string, [number, number]> = { M9: [28, 8], X6: [3, 11], N4: [75, 28], H8: [37, 8] };
    for (const cell of tracks.cells.filter((c) => c.id in table)) {
      const [fromDeg, speedKts] = table[cell.id];
      const dDeg = Math.abs(((cell.motion!.fromDeg - fromDeg + 540) % 360) - 180);
      expect(dDeg, cell.id).toBeLessThanOrEqual(10);
      expect(Math.abs(cell.motion!.speedKts - speedKts), cell.id).toBeLessThanOrEqual(2);
    }
  });

  it("rejects other products", () => {
    expect(() => parseNst(new Uint8Array(100))).toThrow(/NST/);
  });
});

describe("TVS and hail (KVWX, Owensboro KY hail event, 2025-03-15 10:01Z)", () => {
  // Expected values come from each product's own text table.
  const fixture = (name: string) => new Uint8Array(readFileSync(new URL(`../../../../fixtures/nexrad-level3/${name}`, import.meta.url)));
  const radar = { lat: 38.26, lon: -87.725 };
  const scanTimeMs = Date.UTC(2025, 2, 15, 10, 1, 50);

  it("places the TVS where the table says (L8 at 131 deg / 47 nm)", () => {
    const { tvs, scanTimeMs: t } = parseTvs(fixture("KVWX_NTV_20250315_1001"));
    expect(t).toBe(scanTimeMs);
    expect(tvs).toHaveLength(1);
    expect(tvs[0].elevated).toBe(false);
    const { km, deg } = legLengthAndBearing(radar, { lon: tvs[0].at[0], lat: tvs[0].at[1] });
    expect(deg).toBeCloseTo(131, 0);
    expect(km / 1.852).toBeCloseTo(47, 0);
  });

  it("reads hail estimates by cell, including under-half-inch rows", () => {
    const { hail, scanTimeMs: t } = parseHail(fixture("KVWX_NHI_20250315_1001"));
    expect(t).toBe(scanTimeMs);
    expect(hail.get("L8")).toEqual({ severeChance: 100, chance: 100, sizeIn: 2.75 });
    expect(hail.get("A0")).toEqual({ severeChance: 100, chance: 100, sizeIn: 2.25 });
    expect(hail.get("L5")).toEqual({ severeChance: 70, chance: 100, sizeIn: 1.25 });
    expect(hail.get("U1")).toEqual({ severeChance: 0, chance: 80, sizeIn: 0 });
  });

  it("uses the same cell IDs as the storm tracks", () => {
    const ids = new Set(parseNst(fixture("KVWX_NST_20250315_1001")).cells.map((c) => c.id));
    expect(["L8", "A0", "F1"].every((id) => ids.has(id))).toBe(true);
  });

  it("rejects the wrong product", () => {
    expect(() => parseTvs(fixture("KVWX_NHI_20250315_1001"))).toThrow(/NTV/);
  });

  it("ties the TVS to its storm", () => {
    expect(parseTvs(fixture("KVWX_NTV_20250315_1001")).tvs[0].stormId).toBe("L8");
  });

  it("reads mesocyclones where the table says (284 at 096/57, 677 at 136/49, 350 at 126/51)", () => {
    const { mesos, scanTimeMs: t } = parseMeso(fixture("KVWX_NMD_20250315_1001"));
    expect(t).toBe(scanTimeMs);
    expect(mesos.map((m) => m.id)).toEqual(["284", "677", "350"]);
    expect(mesos.every((m) => m.lowLevel)).toBe(true);
    const table: Record<string, [number, number]> = { "284": [96, 57], "677": [136, 49], "350": [126, 51] };
    for (const m of mesos) {
      const { km, deg } = legLengthAndBearing(radar, { lon: m.at[0], lat: m.at[1] });
      expect(Math.abs(deg - table[m.id][0]), m.id).toBeLessThanOrEqual(1);
      expect(Math.abs(km / 1.852 - table[m.id][1]), m.id).toBeLessThanOrEqual(1);
    }
  });

  it("alerts on the TVS, mesocyclones and hail near Owensboro, and nothing far away", () => {
    const tracks = parseNst(fixture("KVWX_NST_20250315_1001"));
    tracks.tvs = parseTvs(fixture("KVWX_NTV_20250315_1001")).tvs;
    tracks.mesos = parseMeso(fixture("KVWX_NMD_20250315_1001")).mesos;
    const hail = parseHail(fixture("KVWX_NHI_20250315_1001")).hail;
    for (const cell of tracks.cells) cell.hail = hail.get(cell.id) ?? null;

    const owensboro = { lat: 37.774, lon: -87.113 };
    const keys = nearbyHazards(owensboro, tracks).map((h) => h.key);
    expect(keys).toContain("tvs:L8");
    expect(keys).toContain("meso:677");
    expect(keys).toContain("hail:L8");
    expect(keys).not.toContain("meso:284"); // 096 deg, well north-east of town
    expect(nearbyHazards({ lat: 35.0, lon: -97.0 }, tracks)).toEqual([]);
  });
});
