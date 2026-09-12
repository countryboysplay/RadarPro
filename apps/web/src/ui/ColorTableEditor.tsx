import { useEffect, useRef, useState } from "react";
import { DEFAULT_COLOR_TABLE_PRESETS } from "../colorTables/defaultColorTables";
import type { ColorTableLoadResult } from "../radar/useRadarRenderer";

export interface ColorTableEditorProps {
  momentCode: string;
  /** The active table's JSON for `momentCode`, used to (re)seed the
   * textarea whenever the selected moment changes. */
  activeColorTableJson: string | null;
  onApply: (json: string) => ColorTableLoadResult;
  onResetToBuiltinDefault: () => void;
}

/**
 * Minimal color-table viewer/editor (S05 deliverable 10): a `<textarea>`
 * seeded with the active table's own JSON, a dropdown of this project's six
 * shipped defaults to load as a starting point, and an Apply button that
 * calls `RadarWebRenderer.loadColorTable` through the given callback.
 *
 * A malformed table never throws into the console or crashes the page --
 * `onApply` always returns a plain `{ ok, ... }` result (see
 * `useRadarRenderer.loadColorTable`); on failure this component shows the
 * Rust validator's exact message and leaves the textarea (and the
 * previously-working active table) untouched.
 */
export function ColorTableEditor({
  momentCode,
  activeColorTableJson,
  onApply,
  onResetToBuiltinDefault,
}: ColorTableEditorProps) {
  const [draft, setDraft] = useState(activeColorTableJson ?? "");
  const [error, setError] = useState<string | null>(null);
  const [appliedNote, setAppliedNote] = useState<string | null>(null);

  // Re-seed the draft only when the *selected moment* actually changes
  // (`null` initially guarantees the first render always seeds). Applying
  // a table for the *current* moment also changes `activeColorTableJson`
  // (the parent bumps a cache-buster after a successful apply) -- that
  // must NOT re-run this reset, or a just-shown "Applied ..." confirmation
  // would be wiped out the instant it appears.
  const previousMomentRef = useRef<string | null>(null);
  useEffect(() => {
    if (previousMomentRef.current === momentCode) return;
    previousMomentRef.current = momentCode;
    setDraft(activeColorTableJson ?? "");
    setError(null);
    setAppliedNote(null);
  }, [momentCode, activeColorTableJson]);

  function handleApply() {
    const result = onApply(draft);
    if (result.ok) {
      setError(null);
      setAppliedNote(`Applied "${result.name}" to: ${result.appliedTo.join(", ")}`);
    } else {
      setAppliedNote(null);
      setError(result.error);
    }
  }

  function handlePresetChange(json: string) {
    setDraft(json);
    setError(null);
    setAppliedNote(null);
  }

  return (
    <div className="color-table-editor">
      <div className="color-table-editor-row">
        <label>
          Preset{" "}
          <select
            defaultValue=""
            onChange={(e) => {
              if (e.target.value) handlePresetChange(e.target.value);
              e.target.value = "";
            }}
          >
            <option value="" disabled>
              load a shipped default…
            </option>
            {DEFAULT_COLOR_TABLE_PRESETS.map((preset) => (
              <option key={preset.momentCode} value={preset.json}>
                {preset.label}
              </option>
            ))}
          </select>
        </label>
        <button type="button" onClick={() => setDraft(activeColorTableJson ?? "")}>
          Reload active
        </button>
        <button type="button" onClick={onResetToBuiltinDefault}>
          Reset {momentCode} to built-in
        </button>
      </div>

      <textarea
        className="color-table-editor-textarea"
        spellCheck={false}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        rows={10}
      />

      <div className="color-table-editor-row">
        <button type="button" onClick={handleApply}>
          Apply to {momentCode}
        </button>
      </div>

      {error && <div className="color-table-editor-error">Error: {error}</div>}
      {appliedNote && <div className="color-table-editor-ok">{appliedNote}</div>}
    </div>
  );
}
