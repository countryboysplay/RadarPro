import { useState, type ReactNode } from "react";

export interface SidebarSectionProps {
  title: string;
  /** Initial expanded state for an *uncontrolled* section (the common
   * case -- most sections just remember their own open/closed state). */
  defaultOpen?: boolean;
  /** Controlled expanded state -- give this (with `onToggle`) when a
   * parent needs to force a section open itself, e.g. `App.tsx` opening
   * the Alerts section whenever an alert gets selected (from the map or
   * the list), not just when the user clicks its header. Omit both for
   * a normal self-contained section. */
  open?: boolean;
  onToggle?: (open: boolean) => void;
  children: ReactNode;
}

/**
 * One stacked, independently collapsible group inside `Sidebar`.
 * Deliberately plain stacked groups rather than a single-open accordion:
 * more than one section (e.g. Radar and Alerts) can be expanded at once,
 * which matters for a HUD workstation where a user often wants two tools
 * visible together once the sidebar itself is open.
 */
export function SidebarSection({ title, defaultOpen = false, open: controlledOpen, onToggle, children }: SidebarSectionProps) {
  const [internalOpen, setInternalOpen] = useState(defaultOpen);
  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : internalOpen;

  function handleToggle() {
    const next = !open;
    if (!isControlled) setInternalOpen(next);
    onToggle?.(next);
  }

  return (
    <div className="sidebar-section">
      <button type="button" className="sidebar-section-header" onClick={handleToggle} aria-expanded={open}>
        <span className="sidebar-section-chevron" aria-hidden="true">
          {open ? "▾" : "▸"}
        </span>
        <span className="sidebar-section-title">{title}</span>
      </button>
      {open && <div className="sidebar-section-body">{children}</div>}
    </div>
  );
}
