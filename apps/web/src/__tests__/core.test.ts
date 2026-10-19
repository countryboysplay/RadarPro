import { describe, expect, it } from "vitest";
import { parseObjectKey } from "../nexrad/keys";
import { clampHistoryLimit, historyReducer, initialState } from "../scan/useScanHistory";
import { pointInAlertGeometry } from "../alerts/geometryHitTest";
import { parseColorTableJson } from "../colorTables/colorTableJson";

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

describe("parseColorTableJson", () => {
  it("returns null on malformed input instead of throwing", () => {
    expect(parseColorTableJson("{")).toBeNull();
    expect(parseColorTableJson('{"name": 3}')).toBeNull();
  });
});
