import { useEffect, useId, useState } from "react";
import { useRainbowApiKey } from "../rainbow/useRainbowApiKey";
import { getDesktopDiagnostics, isDesktop, type DesktopDiagnostics } from "../platform/desktop";

/**
 * S10 desktop shell: read-only diagnostics (app/Tauri version, OS/arch,
 * on-disk log file location) fetched from the native side's `get_diagnostics`
 * command -- see `apps/desktop/src-tauri/src/lib.rs`. Renders nothing at
 * all in a browser tab (`isDesktop()` false), so this is purely additive.
 */
function DesktopDiagnosticsPanel() {
  const [diagnostics, setDiagnostics] = useState<DesktopDiagnostics | null>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    let cancelled = false;
    getDesktopDiagnostics().then((d) => {
      if (!cancelled) setDiagnostics(d);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  if (!isDesktop()) return null;

  return (
    <div className="settings-field" style={{ marginTop: "1em" }}>
      <label>Desktop diagnostics</label>
      {diagnostics ? (
        <dl className="settings-field-note" style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "0.15em 0.6em", margin: 0 }}>
          <dt>App version</dt>
          <dd>{diagnostics.appVersion}</dd>
          <dt>Tauri version</dt>
          <dd>{diagnostics.tauriVersion}</dd>
          <dt>OS / arch</dt>
          <dd>
            {diagnostics.os} / {diagnostics.arch}
          </dd>
          <dt>Log file</dt>
          <dd>{diagnostics.logDir ? `${diagnostics.logDir}\\radarpro.log` : "(unavailable)"}</dd>
        </dl>
      ) : (
        <p className="settings-field-note">loading…</p>
      )}
    </div>
  );
}

/**
 * S09c Settings section: currently just the Rainbow API key field (the
 * addendum's whole scope), but its own top-level sidebar section --
 * distinct from "Rainbow" -- so it reads as app configuration rather than
 * a Rainbow-specific control, leaving room for other settings later.
 *
 * The field is a credential, so it defaults to `type="password"` with a
 * show/hide toggle, even though (per `useRainbowApiKey`'s doc comment) it
 * never leaves this browser except in a request straight to Rainbow's own
 * API. Saves on every change (no separate Save button/blur handler
 * needed): `setSettingsKey` just writes `localStorage` and notifies every
 * subscriber, so this stays cheap and keeps the Rainbow section's
 * "configured" state in lockstep with what's typed here, live.
 */
export function SettingsPanel() {
  const { settingsKey, setSettingsKey, source, envKeyPresent } = useRainbowApiKey();
  const [reveal, setReveal] = useState(false);
  const inputId = useId();

  return (
    <div className="settings-field">
      <label htmlFor={inputId}>Rainbow API key</label>
      <div className="settings-field-row">
        <input
          id={inputId}
          type={reveal ? "text" : "password"}
          autoComplete="off"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          value={settingsKey}
          placeholder={envKeyPresent ? "using env var (see below)" : "not set"}
          onChange={(e) => setSettingsKey(e.target.value)}
        />
        <button type="button" onClick={() => setReveal((v) => !v)} title={reveal ? "Hide key" : "Show key"}>
          {reveal ? "Hide" : "Show"}
        </button>
      </div>

      {source === "settings" && (
        <div className="settings-field-status settings-field-status-active">using the key saved here</div>
      )}
      {source === "env" && (
        <div className="settings-field-status">
          no key saved here -- currently falling back to the <code>VITE_RAINBOW_API_KEY</code> env var
        </div>
      )}
      {source === "none" && <div className="settings-field-status settings-field-status-none">not configured</div>}

      <p className="settings-field-note">
        Used only by the Rainbow nowcast overlay -- requests go directly from this browser to Rainbow's own API,
        never through any RadarPro server. Stored only in this browser (<code>localStorage</code>); clear the field
        to remove it. A <code>VITE_RAINBOW_API_KEY</code> build-time env var, if set, is used as a fallback whenever
        this field is empty -- handy for a self-hosted/Docker setup.
      </p>

      <DesktopDiagnosticsPanel />
    </div>
  );
}
