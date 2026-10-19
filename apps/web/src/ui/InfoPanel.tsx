export interface InfoPanelProps {
  elevationDeg: number | null;
  sweepIndex: number | null;
  sweepCount: number | null;
  momentCode: string | null;
  units: string | null;
}

/**
 * Small persistent info panel: the selected sweep's elevation angle and
 * the active moment/units. Scan time lives once, in the time strip.
 *
 * VCP (Volume Coverage Pattern) number is deliberately **not** shown here:
 * `radar_types::Volume::volume_coverage_pattern` is decoded natively but
 * `radar-web`'s wasm `VolumeSummary` does not currently expose it to JS
 * (only `siteIcao`/`sweepCount`/`elevationDegs`) -- see this task's final
 * report for that gap. Rather than guess at or fabricate a VCP value, this
 * panel just omits the field.
 */
export function InfoPanel({
  elevationDeg,
  sweepIndex,
  sweepCount,
  momentCode,
  units,
}: InfoPanelProps) {
  return (
    <dl className="info-panel">
      <dt>Elevation</dt>
      <dd>
        {elevationDeg !== null ? `${elevationDeg.toFixed(2)}°` : "—"}
        {sweepIndex !== null && sweepCount !== null && ` (sweep ${sweepIndex + 1} of ${sweepCount})`}
      </dd>

      <dt>Moment</dt>
      <dd>
        {momentCode ?? "—"}
        {units && ` (${units})`}
      </dd>
    </dl>
  );
}
