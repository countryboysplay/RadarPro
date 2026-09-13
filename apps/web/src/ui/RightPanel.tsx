// UI polish pass on S09d: a generic right-docked results panel.
//
// Before this, `RainbowNowcastPanel`/`RainbowWeatherPanel` rendered their
// input controls *and* their result tables both inline in the left
// sidebar's narrow control-rail column -- fine for a few dropdowns, but
// cramped/hard to read for tabular data (live user-testing feedback). The
// fix keeps each panel's inputs in their existing `SidebarSection`s and
// moves only the results into this dock instead.
//
// Deliberately a single reusable container (sections rendered into it, not
// two near-identical floating panels) so a third Rainbow (or non-Rainbow)
// result surface can plug in later without inventing a second dock --
// per this task's explicit "plan for reusable, don't over-engineer"
// instruction. Reuses `.hud-panel`'s existing dark-HUD visual language
// (background/border/color/font, `App.css`) the same way `.hud-panel.sidebar`
// already does for the left dock -- see `.right-panel`'s CSS doc comment
// for the position/size rules specific to this side.
import type { ReactNode } from "react";

export interface RightPanelSection {
  /** Stable React key -- also doubles as a natural id if this ever needs
   * one (e.g. deep-linking to a section). */
  key: string;
  title: string;
  /** Called when the user dismisses this one section (its own close
   * button), not the whole dock -- a second section, if present, stays put.
   * Omit to render a section with no close button. */
  onDismiss?: () => void;
  children: ReactNode;
}

export interface RightPanelProps {
  sections: RightPanelSection[];
}

/**
 * Renders nothing at all when `sections` is empty -- this dock must never
 * reserve permanent empty screen space; it only exists once there is
 * something to show (this task's explicit requirement).
 */
export function RightPanel({ sections }: RightPanelProps) {
  if (sections.length === 0) return null;

  return (
    <div className="hud-panel right-panel">
      {sections.map((section) => (
        <section key={section.key} className="right-panel-section">
          <div className="right-panel-section-header">
            <h2 className="right-panel-section-title">{section.title}</h2>
            {section.onDismiss && (
              <button
                type="button"
                className="right-panel-dismiss"
                aria-label={`Dismiss ${section.title}`}
                title="Dismiss"
                onClick={section.onDismiss}
              >
                ×
              </button>
            )}
          </div>
          {section.children}
        </section>
      ))}
    </div>
  );
}
