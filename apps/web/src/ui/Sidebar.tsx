import type { ReactNode } from "react";

export interface SidebarProps {
  id?: string;
  open: boolean;
  children: ReactNode;
}

/**
 * S09c UI shell: the single collapsible left sidebar that replaced the
 * always-on floating hud-panel-left/right/alerts/alert-detail/forecast
 * boxes -- every tool this app has (radar controls, alerts, forecast,
 * Rainbow) now lives in here, organized into `SidebarSection`s, instead of
 * N separate panels shown at once. Toggled open/closed by one button in
 * `App.tsx`'s top bar; closed by default so the map is the unobstructed
 * default view (see `Agent Context/context/stages/S09c-ui-shell-sidebar.md`
 * for the reasoning -- modeled on RadarScope's own hidden-by-default
 * sidebar).
 *
 * Deliberately reuses `App.css`'s existing `.hud-panel` base styling
 * (background/border/color/font-size, plus its `select`/`dl`/`h1`
 * descendant rules) rather than introducing a new visual language --
 * `.hud-panel.sidebar` overrides only the handful of properties that
 * actually differ for a fixed, full-height slide-in panel (position,
 * width, transform) instead of a small floating box.
 */
export function Sidebar({ id, open, children }: SidebarProps) {
  return (
    <aside id={id} className={`hud-panel sidebar${open ? " sidebar-open" : ""}`} aria-hidden={!open}>
      {children}
    </aside>
  );
}
