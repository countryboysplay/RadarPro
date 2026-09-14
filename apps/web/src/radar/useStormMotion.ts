import { useState } from "react";

/** A uniform storm-motion vector for Storm-Relative Velocity (S11 Phase 2b)
 * -- see `radar_geo::storm_relative_velocity::StormMotion`'s doc comment for
 * the exact convention `directionDeg` follows (a heading, degrees clockwise
 * from true north, toward which the storm is moving -- not the
 * meteorological "wind direction" from-convention). */
export interface StormMotion {
  /** Storm motion speed in meters per second (`>= 0`). */
  speedMps: number;
  /** Compass bearing, degrees clockwise from true north, the storm is
   * moving toward. */
  directionDeg: number;
}

/**
 * Owns the storm-motion input state for the SRV moment picker. Plain local
 * `useState` -- unlike `useRainbowPoint`, storm motion has no natural
 * external default to track (no map-center/site coordinate it should follow),
 * so there is no "reset to an external default" behavior to build here.
 *
 * Defaults to zero speed/direction: per
 * `storm-relative-velocity.md`/`radar_geo::storm_relative_velocity`'s own
 * tests, a zero storm-motion vector is the identity transform (SRV output
 * equals the input VEL unchanged), making it a safe, inert default rather
 * than an arbitrary guess at the user's storm of interest.
 */
export function useStormMotion() {
  const [speedMps, setSpeedMps] = useState(0);
  const [directionDeg, setDirectionDeg] = useState(0);

  return { speedMps, directionDeg, setSpeedMps, setDirectionDeg };
}
