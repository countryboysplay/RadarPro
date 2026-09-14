import { useState, type ReactNode } from "react";

export interface SidebarCategoryProps {
  title: string;
  /** A short, live one-line status summary shown next to the title even
   * while collapsed -- e.g. "Live Radar Sweep active" -- so the current
   * state of everything inside is visible without opening the category.
   * Recomputed fresh on every render from real state, never a separate
   * cached string. */
  summary: string;
  /** Initial expanded state for an *uncontrolled* category (the common
   * case). Categories default closed (per the sidebar-redesign audit --
   * "collapsed by default") unless a caller has a reason to start one
   * open, e.g. `App.tsx` opening Alerts by default only when there are
   * currently active alerts. */
  defaultOpen?: boolean;
  /** Controlled expanded state -- mirrors `SidebarSection`'s own
   * open/onToggle pair, for the same reason (a parent forcing a category
   * open in response to something other than the header click, e.g. an
   * alert getting selected). */
  open?: boolean;
  onToggle?: (open: boolean) => void;
  children: ReactNode;
}

/**
 * One top-level, collapsed-by-default *category* grouping several related
 * `SidebarSection`s/panels under one header with a live status summary --
 * introduced by the sidebar redesign to replace eight flat, equally-weighted
 * `SidebarSection`s in `App.tsx` with real hierarchy ("Map Layers", "Point
 * Forecasts", "Model Forecast", "Alerts", "Settings"). `Sidebar`'s pinned
 * "Live Radar" group deliberately does *not* use this component -- it is
 * used every session and is never collapsed at all -- so this is only for
 * the secondary, occasional-use tools.
 *
 * Visually a heavier/bolder header than a plain `SidebarSection` (see
 * `.sidebar-category-header` in `App.css`) so the hierarchy reads at a
 * glance: category > section > field, not three visually-identical rows.
 */
export function SidebarCategory({ title, summary, defaultOpen = false, open: controlledOpen, onToggle, children }: SidebarCategoryProps) {
  const [internalOpen, setInternalOpen] = useState(defaultOpen);
  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : internalOpen;

  function handleToggle() {
    const next = !open;
    if (!isControlled) setInternalOpen(next);
    onToggle?.(next);
  }

  return (
    <div className="sidebar-category">
      <button type="button" className="sidebar-category-header" onClick={handleToggle} aria-expanded={open}>
        <span className="sidebar-category-chevron" aria-hidden="true">
          {open ? "▾" : "▸"}
        </span>
        <span className="sidebar-category-title">{title}</span>
        <span className="sidebar-category-summary">— {summary}</span>
      </button>
      {open && <div className="sidebar-category-body">{children}</div>}
    </div>
  );
}
