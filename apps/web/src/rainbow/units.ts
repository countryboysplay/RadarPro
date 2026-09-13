// UI polish pass on S09d Part C: Rainbow Weather's `/weather/v1/forecast`
// is always metric -- KB: "No unit-selection parameter is documented;
// convert client-side." This is the one shared helper every Rainbow
// temperature display converts through, rather than each call site
// inlining `* 9 / 5 + 32` on its own -- kept here (not in
// `RainbowWeatherPanel.tsx`) in case Nowcast or another Rainbow surface
// ever needs the same conversion.
//
// Deliberately narrow: temperature-only. Precipitation amount/mm, wind
// speed, pressure, visibility, etc. are never touched by this module or by
// anything that imports it -- those stay exactly as the API reports them,
// labeled with their own `units.*` string from the response. This is not a
// general unit-system switch.

/** `°C -> °F`. Callers round for display themselves (this module returns
 * the exact value, not a display string, so a future caller that wants
 * more precision than "nearest whole degree" isn't forced to re-derive the
 * raw conversion). */
export function celsiusToFahrenheit(celsius: number): number {
  return (celsius * 9) / 5 + 32;
}

/** `°C -> "<rounded °F>"` for direct use in a table cell -- nearest whole
 * degree, this stage's explicit "sensible display precision" call. */
export function formatFahrenheit(celsius: number): string {
  return Math.round(celsiusToFahrenheit(celsius)).toString();
}
