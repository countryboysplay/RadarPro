import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "./Icon";

export interface SettingsSection {
  id: string;
  title: string;
  content: ReactNode;
}

/** Set-once options, out of the rail: a floating window over the map, so
 * changes show on the radar while it is open. Esc or × closes it. */
export function SettingsWindow({ sections, onClose }: { sections: SettingsSection[]; onClose: () => void }) {
  const [id, setId] = useState(sections[0].id);
  const current = sections.find((s) => s.id === id) ?? sections[0];
  const nav = useRef<HTMLElement>(null);
  // Keyboard users land in the window, not back at the top of the rail.
  useEffect(() => nav.current?.querySelector<HTMLElement>("[aria-current=true]")?.focus(), []);
  useEffect(() => {
    // Not from a text field: Esc there would close the window and drop a half-typed color table.
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !(e.target as Element).matches?.("textarea, input[type=text]") && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="settings-window" role="dialog" aria-labelledby="settings-window-title">
      <header className="settings-window-head">
        <h2 id="settings-window-title" className="rail-heading">
          Settings
        </h2>
        <button type="button" className="settings-window-close" onClick={onClose} aria-label="Close settings">
          <Icon name="close" size={16} />
        </button>
      </header>
      <nav ref={nav} className="settings-window-nav" aria-label="Settings sections">
        {sections.map((s) => (
          <button key={s.id} type="button" aria-current={s.id === current.id} onClick={() => setId(s.id)}>
            {s.title}
          </button>
        ))}
      </nav>
      <div className="settings-window-body">{current.content}</div>
    </div>
  );
}
