// Storm-label placement: put each cell's label next to its dot, clear of every
// track, meso circle, dot and already-placed label, not just its own track.

export type Pt = { x: number; y: number };
export type Rect = { x: number; y: number; w: number; h: number };
/** `ring`: only the outline counts (a meso circle); otherwise the whole disc (a dot). */
export type Circle = { x: number; y: number; r: number; ring: boolean };
export type Obstacles = { segments: [Pt, Pt][]; circles: Circle[]; rects: Rect[] };

/** Half the width of a drawn track plus its casing, so a label can't touch the line. */
const SEGMENT_PAD = 4;
/** Half a meso ring's drawn width (6px casing). */
const RING_PAD = 3;

function segmentHitsRect(a: Pt, b: Pt, r: Rect): boolean {
  // Liang-Barsky clip of segment ab against r.
  let t0 = 0;
  let t1 = 1;
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const edges: [number, number][] = [
    [-dx, a.x - r.x],
    [dx, r.x + r.w - a.x],
    [-dy, a.y - r.y],
    [dy, r.y + r.h - a.y],
  ];
  for (const [p, q] of edges) {
    if (p === 0) {
      if (q < 0) return false;
      continue;
    }
    const t = q / p;
    if (p < 0) t0 = Math.max(t0, t);
    else t1 = Math.min(t1, t);
    if (t0 > t1) return false;
  }
  return true;
}

function circleHitsRect(c: Circle, r: Rect): boolean {
  const nearest = Math.hypot(c.x - Math.max(r.x, Math.min(c.x, r.x + r.w)), c.y - Math.max(r.y, Math.min(c.y, r.y + r.h)));
  if (!c.ring) return nearest <= c.r;
  const farthest = Math.hypot(Math.max(c.x - r.x, r.x + r.w - c.x), Math.max(c.y - r.y, r.y + r.h - c.y));
  return nearest <= c.r + RING_PAD && farthest >= c.r - RING_PAD;
}

function rectsOverlap(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

function hits(rect: Rect, obstacles: Obstacles): number {
  const padded = { x: rect.x - SEGMENT_PAD, y: rect.y - SEGMENT_PAD, w: rect.w + 2 * SEGMENT_PAD, h: rect.h + 2 * SEGMENT_PAD };
  let n = 0;
  for (const [a, b] of obstacles.segments) if (segmentHitsRect(a, b, padded)) n++;
  for (const c of obstacles.circles) if (circleHitsRect(c, rect)) n++;
  for (const other of obstacles.rects) if (rectsOverlap(rect, other)) n++;
  return n;
}

/**
 * The label's box for a `w` x `h` label beside `at`. Tries `preferred` (a unit
 * direction) first, then directions turning away from it, then the same ring
 * further out; takes the first clear spot, else the one with fewest hits.
 */
export function placeLabel(at: Pt, w: number, h: number, preferred: Pt, obstacles: Obstacles): Rect {
  const base = Math.atan2(preferred.y, preferred.x);
  let best: Rect | null = null;
  let bestHits = Infinity;
  for (const dist of [14, 26]) {
    for (const turn of [0, 1, -1, 2, -2, 3, -3, 4]) {
      const a = base + (turn * Math.PI) / 4;
      const nx = Math.cos(a);
      const ny = Math.sin(a);
      // The box's near edge sits `dist` from the dot, along the direction.
      const rect = { x: at.x + nx * dist - (w * (1 - nx)) / 2, y: at.y + ny * dist - (h * (1 - ny)) / 2, w, h };
      const n = hits(rect, obstacles);
      if (n === 0) return rect;
      if (n < bestHits) [best, bestHits] = [rect, n];
    }
  }
  return best!;
}
