// @ts-expect-error -- the app tsconfig has no Node types; vitest runs under Node.
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import { parseHail, parseMeso, parseNst, parseTvs } from "../storms/nst";
import { closestApproach, stormAlerts } from "../storms/nearbyHazards";
import { placeLabel } from "../storms/labelPlacement";
import type { StormCell, StormTracks } from "../storms/nst";
import { parseObjectKey } from "../nexrad/keys";
import { parsePlacefile } from "../placefiles/placefile";
import { drawPlacefiles } from "../placefiles/draw";
import { fetchMds, fetchWatches } from "../spc/spc";
import { lsrLayer } from "../spc/lsr";
import { fetchWpc } from "../spc/wpc";
import { glmKeyAt, glmLayer, mergeFlashes, nearestFlashKm } from "../lightning/glm";
import { alertsGeoJson, fetchArchivedAlerts } from "../alerts/archived";
import { formatTags } from "../alerts/types";
import { clampHistoryLimit, historyReducer, initialState } from "../scan/useScanHistory";
import { alertNearSite, pointInAlertGeometry } from "../alerts/geometryHitTest";
import { coveringAlerts } from "../location/useMyLocation";
import { raobLevels } from "../forecast/sounding";
import { parseColorTableJson } from "../colorTables/colorTableJson";
import { fetchOpenMeteoSounding, surfaceParcel } from "../forecast/sounding";
import { leadHourOptions, variableAvailable, variableOnMap } from "../forecast/useForecastProvider";
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
  it("counts zone-only alerts NWS lists for the point (watches)", () => {
    const watch = { key: "watch", alert: { id: "urn:oid:watch.1", geometry: null } } as never;
    const ids = new Set(["urn:oid:watch.1"]);
    expect(coveringAlerts({ lat: 50, lon: 5 }, [...active, watch], new Set(), ids).covering.map((a) => a.key)).toEqual(["watch"]);
    expect(coveringAlerts({ lat: 50, lon: 5 }, [...active, watch], new Set()).covering).toEqual([]);
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
    const alerts = stormAlerts(owensboro, [{ icao: "KVWX", tracks }]);
    // L8 carries the TVS, its 2.75" hail and (within 10 km) meso 677: one alert, ranked as a TVS.
    const l8 = alerts.find((a) => a.key === "KVWX:L8")!;
    expect(l8.rank).toBe(4);
    expect(l8.urgent).toBe(true);
    expect(l8.body).toMatch(/TVS/);
    expect(l8.body).toMatch(/2\.75″ hail/);
    expect(alerts[0].key).toBe("KVWX:L8"); // worst first
    expect(alerts.some((a) => a.body.includes("284"))).toBe(false); // 096 deg, well north-east of town
    expect(stormAlerts({ lat: 35.0, lon: -97.0 }, [{ icao: "KVWX", tracks }])).toEqual([]);
  });
});

describe("stormAlerts heading", () => {
  const here = { lat: 35, lon: -97 };
  // ~0.11 deg of longitude is 10 km at 35N.
  const lonKm = 1 / (111.32 * Math.cos((35 * Math.PI) / 180));
  const cell = (id: string, startKm: number, stepKm: number): StormCell => {
    const at: [number, number] = [here.lon + startKm * lonKm, here.lat];
    const forecast = [0, 1, 2, 3, 4].map((i) => [here.lon + (startKm + i * stepKm) * lonKm, here.lat] as [number, number]);
    return { id, at, past: [], forecast, motion: null, hail: { severeChance: 90, chance: 100, sizeIn: 1.5 } };
  };
  const tracks = (...cells: StormCell[]): StormTracks => ({ scanTimeMs: 0, cells, avgMotion: null, tvs: [], mesos: [] });

  it("finds the closest point along the track and when", () => {
    const c = cell("A1", -30, 15); // 30 km west, moving east 15 km per 15 min
    const a = closestApproach(here, c.forecast);
    expect(a.km).toBeLessThan(0.5);
    expect(a.min).toBeCloseTo(30, 0);
    // Only 30 minutes ahead count: one that takes 40 minutes is still 10 km out at 30.
    expect(closestApproach(here, cell("A2", -40, 15).forecast).km).toBeCloseTo(10, 0);
  });

  it("alerts on a storm heading your way, with an arrival time", () => {
    const [alert] = stormAlerts(here, [{ icao: "KTLX", tracks: tracks(cell("A1", -40, 20)) }]);
    expect(alert.title).toBe("1.50″ hail storm heading your way");
    expect(alert.body).toMatch(/arrives in ~\d+ min, passing over you/);
  });

  it("ignores a storm moving away, even one that is fairly close", () => {
    expect(stormAlerts(here, [{ icao: "KTLX", tracks: tracks(cell("B2", 30, 20)) }])).toEqual([]);
  });

  it("reports a storm two radars both see once, from the nearer radar", () => {
    const alerts = stormAlerts(here, [
      { icao: "KTLX", tracks: tracks(cell("C3", -10, 15)) },
      { icao: "KFDR", tracks: tracks(cell("Q8", -12, 15)) },
    ]);
    expect(alerts.map((a) => a.key)).toEqual(["KTLX:C3"]);
    expect(alerts[0].title).toMatch(/near you$/);
  });
});

describe("placeLabel", () => {
  const none = { segments: [], circles: [], rects: [] };
  const up = { x: 0, y: -1 };
  it("uses the preferred side when it is clear", () => {
    const box = placeLabel({ x: 100, y: 100 }, 30, 16, up, none);
    expect(box.y + box.h).toBeCloseTo(86);
    expect(box.x + box.w / 2).toBeCloseTo(100);
  });
  it("moves off a neighbour's track crossing the preferred spot", () => {
    const track: [{ x: number; y: number }, { x: number; y: number }] = [{ x: 50, y: 78 }, { x: 150, y: 78 }];
    const box = placeLabel({ x: 100, y: 100 }, 30, 16, up, { ...none, segments: [track] });
    expect(box.y + box.h < 74 || box.y > 82).toBe(true);
    expect(box.y).toBeGreaterThan(80); // the next clear spot, beside the dot, not further up past the line
  });
  it("keeps clear of a meso ring and an earlier label", () => {
    const ring = { x: 100, y: 70, r: 12, ring: true };
    const earlier = { x: 105, y: 80, w: 40, h: 16 };
    const box = placeLabel({ x: 100, y: 100 }, 30, 16, up, { segments: [], circles: [ring], rects: [earlier] });
    const overlapsEarlier = box.x < earlier.x + earlier.w && earlier.x < box.x + box.w && box.y < earlier.y + earlier.h && earlier.y < box.y + box.h;
    expect(overlapsEarlier).toBe(false);
    expect(Math.hypot(box.x + box.w / 2 - 100, box.y + box.h / 2 - 70)).toBeGreaterThan(20);
  });
});

describe("parsePlacefile", () => {
  it("reads a Spotter Network object with icon, text and hover", () => {
    const pf = parsePlacefile(
      [
        "Refresh: 1",
        "Title: Spotter Network Positions",
        'Font: 1, 11, 0, "Courier New"',
        'IconFile: 1, 30, 30, 15, 15, "/iconsheets/Spotternet_096.png"',
        "Object: 41.5,-88.25",
        'Icon: 0,0,000,1,6,"Jane Doe\\n2026-10-20 14:54:31 UTC"',
        'Text: 15, 10, 1, "Jane, Doe"',
        "End:",
      ].join("\n"),
      "https://www.spotternetwork.org/feeds/gr.txt",
    );
    expect(pf.title).toBe("Spotter Network Positions");
    expect(pf.refreshSeconds).toBe(60);
    expect(pf.iconFiles.get(1)?.url).toBe("https://www.spotternetwork.org/iconsheets/Spotternet_096.png");
    expect(pf.items[0]).toMatchObject({ kind: "icon", anchor: [-88.25, 41.5], at: [0, 0], file: 1, index: 6, hover: "Jane Doe\n2026-10-20 14:54:31 UTC" });
    expect(pf.items[1]).toMatchObject({ kind: "text", anchor: [-88.25, 41.5], at: [15, 10], text: "Jane, Doe" });
  });

  it("reads lines, polygon contours, colors and thresholds, and skips triangles", () => {
    const pf = parsePlacefile(
      [
        "; comment",
        "Threshold: 50",
        "Color: 255 0 0",
        'Line: 2, 0, "Track"',
        "35, -97",
        "36, -98",
        "End:",
        "Triangles:",
        "1, 2",
        "3, 4",
        "5, 6",
        "End:",
        "Polygon:",
        "30, -90, 0, 255, 0, 128",
        "31, -90",
        "31, -91",
        "30, -90",
        "End:",
        "Place: 40, -100, Some Town",
      ].join("\n"),
    );
    expect(pf.items.map((i) => i.kind)).toEqual(["line", "polygon", "text"]);
    expect(pf.items[0]).toMatchObject({ points: [[-97, 35], [-98, 36]], width: 2, color: "rgba(255, 0, 0, 1)", threshold: 50, hover: "Track", anchor: null });
    expect(pf.items[1]).toMatchObject({ contours: [[[-90, 30], [-90, 31], [-91, 31], [-90, 30]]], color: "rgba(0, 255, 0, 0.5019607843137255)" });
    expect(pf.items[2]).toMatchObject({ kind: "text", at: [-100, 40], text: "Some Town" });
  });
});

describe("SPC watches and MDs", () => {
  afterEach(() => vi.unstubAllGlobals());
  const square = [[[-98, 35], [-97, 35], [-97, 36], [-98, 36], [-98, 35]]];
  const serve = (body: unknown) => {
    const fetch = vi.fn<(url: string) => Promise<Response>>(async () => new Response(JSON.stringify(body)));
    vi.stubGlobal("fetch", fetch);
    return fetch;
  };

  it("turns an archived watch into a hoverable area and outline", async () => {
    const fetch = serve({
      features: [{ geometry: { type: "MultiPolygon", coordinates: [square] }, properties: { type: "TOR", num: "187", utc_expired: "2024-05-07T00:03:00Z" } }],
    });
    const layer = await fetchWatches(Date.UTC(2024, 4, 6, 23), new AbortController().signal);
    expect(String(fetch.mock.calls[0][0])).toContain("valid=2024-05-06T23:00:00Z");
    expect(layer.file.items.map((i) => i.kind)).toEqual(["polygon", "line"]);
    expect(layer.file.items[0].hover).toMatch(/^Tornado Watch 187\nuntil /);
  });

  it("reads MD number, concern and watch chance, with or without a chance", async () => {
    serve({
      features: [
        { geometry: { type: "Polygon", coordinates: square }, properties: { num: "0657", product_id: "P1", concerning: "SEVERE POTENTIAL...WATCH POSSIBLE", watch_confidence: "40.0", expire: "2024-05-06T23:30:00Z" } },
        { geometry: { type: "Polygon", coordinates: square }, properties: { num: "658", product_id: "P2", concerning: "TORNADO WATCH 187...190", watch_confidence: null, expire: "2024-05-07T00:00:00Z" } },
      ],
    });
    const { mds, layer } = await fetchMds(null, new AbortController().signal);
    expect(mds.map(({ num, concerning, watchChance }) => [num, concerning, watchChance])).toEqual([
      ["657", "Severe potential...watch possible", 40],
      ["658", "Tornado watch 187...190", null],
    ]);
    expect(layer.file.items[0].hover).toContain("Watch chance 40%");
    expect(layer.file.items[2].hover).not.toContain("Watch chance");
  });
});

describe("lsrLayer", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("keeps chaser reports, newest on top, with size and remarks in the hover", () => {
    // Node has no OffscreenCanvas; the icon sheet only needs to exist.
    const ctx = new Proxy({}, { get: () => () => {}, set: () => true });
    vi.stubGlobal("OffscreenCanvas", class { getContext = () => ctx; transferToImageBitmap = () => ({}) });
    const report = (typetext: string, valid: string, extra: object = {}) => ({
      properties: { typetext, valid, lon: -97, lat: 35, magf: null, unit: null, qualifier: null, city: "2 N Moore", county: "Cleveland", st: "OK", source: "Trained Spotter", remark: null, ...extra },
    });
    const layer = lsrLayer(
      [
        report("HAIL", "2024-05-06T23:10:00Z", { magf: 1.75, unit: "Inch", remark: "Golf ball hail." }),
        report("TORNADO", "2024-05-06T22:40:00Z"),
        report("NON-TSTM WND GST", "2024-05-06T22:00:00Z", { magf: 64, unit: "MPH" }),
      ],
      "test",
    );
    expect(layer.file.items.map((i) => (i.kind === "icon" ? i.index : 0))).toEqual([1, 3]);
    expect(layer.file.items[1].hover).toMatch(/^Hail 1.75 in · .+\n2 N Moore, Cleveland Co., OK\nSource: Trained Spotter\nGolf ball hail.$/);
    expect(layer.file.items[0].hover).toMatch(/^Tornado · /);
  });
});

describe("archived warnings and SPS", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("asks IEM for the scan's minute and lists a tornado warning first, with its tags", async () => {
    const square = [[[-98, 35], [-97, 35], [-97, 36], [-98, 36], [-98, 35]]];
    const sbw = (phenomena: string, significance: string, ps: string, tags: object = {}) => ({
      geometry: { type: "MultiPolygon", coordinates: [square] },
      properties: { phenomena, significance, ps, wfo: "OUN", eventid: 12, year: 2024, product_id: `P-${phenomena}`, issue: "2024-05-06T22:50:00Z", polygon_begin: "2024-05-06T22:55:00Z", polygon_end: "2024-05-06T23:30:00Z", tornadotag: null, hailtag: null, windtag: null, damagetag: null, ...tags },
    });
    const fetch = vi.fn<(url: string) => Promise<Response>>(async (url) =>
      new Response(
        JSON.stringify({
          features: url.includes("sbw")
            ? [sbw("FL", "Y", "Flood Advisory"), sbw("TO", "W", "Tornado Warning", { tornadotag: "RADAR INDICATED", hailtag: 1.75 })]
            : [{ id: "SPS1", geometry: { type: "Polygon", coordinates: square }, properties: { issue: "2024-05-06T22:32:00Z", expire: "2024-05-06T23:15:00Z", max_hail_size: "0.00", max_wind_gust: "50", wfo: "OUN" } }],
        }),
      ),
    );
    vi.stubGlobal("fetch", fetch);
    const alerts = await fetchArchivedAlerts(Date.UTC(2024, 4, 6, 23), new AbortController().signal);
    expect(fetch.mock.calls.map((c) => String(c[0]))).toEqual([
      "https://mesonet.agron.iastate.edu/geojson/sbw.geojson?ts=2024-05-06T23:00Z",
      "https://mesonet.agron.iastate.edu/geojson/sps.geojson?valid=2024-05-06T23:00Z",
    ]);
    expect(alerts.map(({ alert }) => [alert.event, alert.severity])).toEqual([
      ["Tornado Warning", "Extreme"],
      ["Special Weather Statement", "Moderate"],
      ["Flood Advisory", "Minor"],
    ]);
    expect(alerts[0].key).toBe("OUN.TO.W.12.2024");
    expect(alerts[0].alert.id).toBe("P-TO");
    expect(alerts[0].alert.expires).toBe(Date.UTC(2024, 4, 6, 23, 30));
    expect(alerts[0].alert.headline).toMatch(/^until /);
    expect(formatTags(alerts[0].alert.tags)).toBe("Tornado radar indicated · Hail 1.75 in");
    // SPS hail "0.00" means none.
    expect(formatTags(alerts[1].alert.tags)).toBe("Wind 50 mph");
    // Live SPSs often say "Up to".
    expect(formatTags([["maxHailSize", "Up to .75"], ["maxWindGust", "Up to 50 MPH"]])).toBe("Hail up to .75 in · Wind up to 50 mph");
    expect(formatTags([["maxHailSize", "0.00"], ["maxWindGust", "0 MPH"]])).toBe("");
    expect(alertsGeoJson(alerts).features[0]).toMatchObject({ id: "OUN.TO.W.12.2024", properties: { severity: "Extreme" }, geometry: { type: "MultiPolygon" } });
  });
});

describe("WPC rainfall layers", () => {
  afterEach(() => vi.unstubAllGlobals());
  const square = [[[-98, 35], [-97, 35], [-97, 36], [-98, 36], [-98, 35]]];
  const serve = (features: object[]) => vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({ features }))));

  it("draws excessive rainfall risks lowest first, coloured by risk", async () => {
    serve([
      { geometry: { type: "Polygon", coordinates: square }, properties: { outlook: "Slight (At Least 15%)", dn: 2, valid_time: "12Z 10/21/26 - 12Z 10/22/26" } },
      { geometry: { type: "Polygon", coordinates: square }, properties: { outlook: "Marginal (At Least 5%)", dn: 1, valid_time: "12Z 10/21/26 - 12Z 10/22/26" } },
    ]);
    const layer = await fetchWpc("ero2", new AbortController().signal);
    const polygons = layer.file.items.filter((i) => i.kind === "polygon");
    expect(polygons.map((i) => i.hover)).toEqual([
      "WPC excessive rainfall Day 2: Marginal (At Least 5%)\n12Z 10/21/26 - 12Z 10/22/26",
      "WPC excessive rainfall Day 2: Slight (At Least 15%)\n12Z 10/21/26 - 12Z 10/22/26",
    ]);
  });

  it("labels each rain band with its own range, the top one open-ended", async () => {
    serve([
      { geometry: { type: "Polygon", coordinates: square }, properties: { qpf: 1.5, valid_time: "v" } },
      { geometry: { type: "Polygon", coordinates: square }, properties: { qpf: 0.01, valid_time: "v" } },
    ]);
    const layer = await fetchWpc("qpf72", new AbortController().signal);
    const fills = layer.file.items.filter((i) => i.kind === "polygon");
    expect(fills.map((i) => i.hover)).toEqual(["WPC rain forecast Days 1-3: 1.5 in or more\nv", "WPC rain forecast Days 1-3: 0.01 to 1.5 in\nv"]);
    // Bands don't overlap, so each fill is drawn from its own rings only.
    expect(fills.map((i) => (i.kind === "polygon" ? i.contours.length : 0))).toEqual([1, 1]);
    expect(layer.file.items.find((i) => i.kind === "line")?.color).toBe("#8968cd");
  });
});

describe("GOES lightning", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("builds the bucket key of a file start, sorting before that file's real key", () => {
    const at = Date.UTC(2026, 9, 20, 17, 15, 0);
    expect(glmKeyAt(at, "G19")).toBe("GLM-L2-LCFA/2026/293/17/OR_GLM-L2-LCFA_G19_s2026293171500");
    const real = "GLM-L2-LCFA/2026/293/17/OR_GLM-L2-LCFA_G19_s20262931715000_e20262931715200_c20262931715218.nc";
    expect(glmKeyAt(at, "G19") < real && real < glmKeyAt(at + 20_000, "G19")).toBe(true);
    expect(glmKeyAt(Date.UTC(2026, 0, 1, 0, 0, 5), "G18")).toBe("GLM-L2-LCFA/2026/001/00/OR_GLM-L2-LCFA_G18_s2026001000005");
  });

  it("colours flashes by age, drops ones past 15 min, newest on top", () => {
    const ctx = new Proxy({}, { get: () => () => {}, set: () => true });
    vi.stubGlobal("OffscreenCanvas", class { getContext = () => ctx; transferToImageBitmap = () => ({}) });
    const now = Date.UTC(2026, 9, 20, 17, 15);
    const min = 60_000;
    const flashes = Float64Array.from([-97, 35, now - 12 * min, -97, 35, now - 20 * min, -97, 35, now - 30_000, -97, 35, now - 3 * min, -97, 35, now - 7 * min]);
    const items = glmLayer(flashes, now).file.items;
    expect(items.map((i) => (i.kind === "icon" ? i.index : 0))).toEqual([4, 3, 2, 1]);
  });

  it("takes each satellite's flashes from its own side when both are on", () => {
    const east = Float64Array.from([-120, 40, 1, -97, 35, 2]);
    const west = Float64Array.from([-120, 40, 3, -97, 35, 4]);
    expect([...mergeFlashes(east, west)!]).toEqual([-97, 35, 2, -120, 40, 3]);
    expect(mergeFlashes(null, west)).toBe(west);
    expect(mergeFlashes(null, null)).toBeNull();
  });

  it("finds the nearest recent flash only", () => {
    const now = Date.UTC(2026, 9, 20, 17, 15);
    // 0.1 deg of latitude is about 11 km; the closer flash is too old to count.
    const flashes = Float64Array.from([-97, 35.1, now - 60_000, -97, 35.01, now - 10 * 60_000]);
    const km = nearestFlashKm(flashes, { lat: 35, lon: -97 }, now - 5 * 60_000);
    expect(km).toBeGreaterThan(10.5);
    expect(km).toBeLessThan(11.5);
    expect(nearestFlashKm(flashes, { lat: 35, lon: -97 }, now)).toBeNull();
  });

  it("draws at any zoom and only the topmost of stacked bolts", () => {
    const ctx = new Proxy({ drawImage: vi.fn() } as Record<string, unknown>, { get: (t, k) => t[k as string] ?? (() => {}), set: () => true });
    vi.stubGlobal("OffscreenCanvas", class { getContext = () => ctx; transferToImageBitmap = () => ({}) });
    // A 3000-mile-wide view: 1 px per 0.05 deg.
    const map = {
      getBounds: () => ({ getNorth: () => 50, getSouth: () => 20, getEast: () => -60, getWest: () => -125 }),
      project: ([lon, lat]: [number, number]) => ({ x: (lon + 125) * 20, y: (50 - lat) * 20 }),
    };
    const now = Date.UTC(2026, 9, 20, 17, 15);
    // Two bolts 0.01 deg apart (same pixel), the newer on top, and one far away.
    const flashes = Float64Array.from([-97, 35, now - 7 * 60_000, -97.01, 35, now - 30_000, -90, 40, now - 60_000]);
    const canvas = { width: 1300, height: 600, getContext: () => ctx } as unknown as HTMLCanvasElement;
    const layer = glmLayer(flashes, now);
    layer.sheets.set(1, { width: 88, height: 22 } as ImageBitmap);
    drawPlacefiles(map as never, canvas, [layer]);
    const drawn = (ctx.drawImage as ReturnType<typeof vi.fn>).mock.calls;
    expect(drawn.length).toBe(2);
    // Source x is the age colour: the far 1-min (yellow) bolt, then the 30-s (white) one over the hidden 7-min one.
    expect(drawn.map((c) => c[1])).toEqual([22, 0]);
    // An icon from another layer under the same bolt is still drawn.
    const under = glmLayer(Float64Array.from([-97, 35, now - 30_000]), now);
    under.sheets.set(1, { width: 88, height: 22 } as ImageBitmap);
    (ctx.drawImage as ReturnType<typeof vi.fn>).mockClear();
    drawPlacefiles(map as never, canvas, [under, layer]);
    expect((ctx.drawImage as ReturnType<typeof vi.fn>).mock.calls.length).toBe(3);
  });
});

describe("surfaceParcel", () => {
  const levels = [1000, 900, 850, 700, 500, 300, 200].map((p, i) => ({
    p,
    z: [0, 990, 1500, 3100, 5800, 9500, 12000][i],
    t: [30, 20, 16, 4, -12, -40, -55][i],
    td: [20, 15, 10, -5, -25, -50, -70][i],
    u: 0,
    v: 0,
  }));

  it("finds the LCL, moist-adiabatic lapse and CAPE of a warm, moist surface parcel", () => {
    const parcel = surfaceParcel(levels);
    // About 125 m per degree of dew point depression.
    expect(parcel.lclAgl).toBeGreaterThan(1100);
    expect(parcel.lclAgl).toBeLessThan(1400);
    const t500 = parcel.path.find((x) => x.p === 500)!.t;
    expect(t500).toBeGreaterThan(-8);
    expect(t500).toBeLessThan(2);
    expect(parcel.cape).toBeGreaterThan(500);
    expect(parcel.cape).toBeLessThan(5000);
  });

  it("has no CAPE when the air aloft is warmer than the parcel", () => {
    const stable = levels.map((l, i) => (i === 0 ? l : { ...l, t: l.t + 25 }));
    expect(surfaceParcel(stable).cape).toBe(0);
  });
});

describe("fetchOpenMeteoSounding", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("builds the column above the model ground from a real Open-Meteo HRRR answer (OKC, 2026-10-20 19Z)", async () => {
    const hourly: Record<string, (number | null)[]> = {
      surface_pressure: [976.0], temperature_2m: [24.0], dew_point_2m: [9.0], wind_speed_10m: [15.0], wind_direction_10m: [180],
      // Under the 976 hPa ground: must be dropped.
      temperature_1000hPa: [23.0], dew_point_1000hPa: [8.7], wind_speed_1000hPa: [18.0], wind_direction_1000hPa: [181], geopotential_height_1000hPa: [176],
      temperature_975hPa: [22.0], dew_point_975hPa: [7.4], wind_speed_975hPa: [19.4], wind_direction_975hPa: [182], geopotential_height_975hPa: [399],
      temperature_850hPa: [12.1], dew_point_850hPa: [-0.5], wind_speed_850hPa: [24.2], wind_direction_850hPa: [181], geopotential_height_850hPa: [1564],
      temperature_500hPa: [-11.9], dew_point_500hPa: [-17.5], wind_speed_500hPa: [39.9], wind_direction_500hPa: [254], geopotential_height_500hPa: [5857],
      // The top level has no dew point.
      temperature_50hPa: [-65.5], dew_point_50hPa: [null], wind_speed_50hPa: [7.2], wind_direction_50hPa: [90], geopotential_height_50hPa: [20744.83],
    };
    const fetch = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({ elevation: 386, hourly: { time: ["2026-10-20T19:00"], ...hourly } }) }));
    vi.stubGlobal("fetch", fetch);
    const s = await fetchOpenMeteoSounding(35.4, -97.5, Date.parse("2026-10-20T19:20Z"), new AbortController().signal);
    expect(String((fetch.mock.calls[0] as unknown[])[0])).toContain("start_hour=2026-10-20T19:00");
    expect(s.levels.map((l) => l.p)).toEqual([976, 975, 850, 500, 50]);
    // Model ground ~8 m under the 975 hPa height, not the 386 m DEM elevation.
    expect(s.levels[0].z).toBeGreaterThan(385);
    expect(s.levels[0].z).toBeLessThan(395);
    // A south wind blows toward the north.
    expect(s.levels[0].u).toBeCloseTo(0, 5);
    expect(s.levels[0].v).toBeCloseTo(15, 5);
    expect(s.levels.at(-1)!.td).toBe(-100);
  });

  it("reports Open-Meteo's own error text", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => ({ ok: false, status: 429, json: async () => ({ error: true, reason: "Too many concurrent requests" }) })));
    await expect(fetchOpenMeteoSounding(35.4, -97.5, 0, new AbortController().signal)).rejects.toThrow("Too many concurrent requests");
  });
});

describe("forecast variables per model", () => {
  it("offers RTMA only the surface fields it publishes, at one analysis hour", () => {
    expect(variableAvailable("temperature_2m", "rtma")).toBe(true);
    expect(variableAvailable("wind_speed_10m", "rtma")).toBe(true);
    expect(variableAvailable("composite_reflectivity", "rtma")).toBe(false);
    expect(variableAvailable("mslp", "rtma")).toBe(false);
    expect(leadHourOptions("rtma")).toEqual([0]);
  });

  it("keeps GEFS fields as they were and its coarse grid off the map", () => {
    expect(variableAvailable("mslp", "gefs")).toBe(true);
    expect(variableAvailable("wind_speed_10m", "gefs")).toBe(false);
    expect(variableAvailable("sbcape", "gefs")).toBe(false);
    expect(variableOnMap("temperature_2m", "gefs")).toBe(false);
    expect(variableOnMap("temperature_2m", "hrrr")).toBe(true);
    expect(variableOnMap("mslp", "hrrr")).toBe(false);
  });
});

describe("raobLevels", () => {
  it("starts at the first full level and interpolates missing winds in height", () => {
    const levels = raobLevels([
      { pres: 1000, hght: 202, tmpc: null, dwpc: null, drct: null, sknt: null },
      { pres: 983, hght: 357, tmpc: 10.8, dwpc: 8.7, drct: 180, sknt: 0 },
      { pres: 970, hght: 457, tmpc: 16, dwpc: 6, drct: null, sknt: null },
      { pres: 950, hght: 557, tmpc: 17, dwpc: null, drct: 180, sknt: 20 },
    ]);
    expect(levels.map((l) => l.p)).toEqual([983, 970, 950]);
    expect(levels[1].v).toBeCloseTo(10);
    expect(levels[1].u).toBeCloseTo(0);
    expect(Number.isNaN(levels[2].td)).toBe(true);
  });
});
