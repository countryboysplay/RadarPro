import { useEffect, useId, useState } from "react";
import { useRainbowApiKey } from "../rainbow/useRainbowApiKey";
import {
  checkForUpdate,
  getDesktopDiagnostics,
  installPendingUpdate,
  isDesktop,
  type DesktopDiagnostics,
  type UpdateCheckResult,
  type UpdateDownloadProgress,
} from "../platform/desktop";
import type { PollEvent } from "../scan/useScanPoller";
import { MIN_HISTORY_SCANS, MAX_HISTORY_SCANS_LIMIT } from "../scan/useScanHistory";
import type { AlertPollStatus } from "../alerts/useAlertPoller";
import { overallNetworkStatus, useNetworkDiagnostics } from "../diagnostics/useNetworkDiagnostics";

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
 * S10 Phase 4 update strategy: a "Check for updates" affordance backed by
 * `@tauri-apps/plugin-updater` (see `checkForUpdate`/`installPendingUpdate`
 * in `apps/web/src/platform/desktop.ts` for what these actually call and
 * why this is unrelated to code signing). Renders nothing in a browser tab.
 *
 * Current version is read from the same `get_diagnostics` command
 * `DesktopDiagnosticsPanel` already uses, rather than adding a second way
 * to ask the native side for the app version.
 */
function UpdatePanel() {
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [check, setCheck] = useState<UpdateCheckResult | null>(null);
  const [phase, setPhase] = useState<"idle" | "checking" | "downloading" | "done">("idle");
  const [progress, setProgress] = useState<UpdateDownloadProgress | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    let cancelled = false;
    getDesktopDiagnostics().then((d) => {
      if (!cancelled && d) setAppVersion(d.appVersion);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  if (!isDesktop()) return null;

  async function handleCheck() {
    setPhase("checking");
    setInstallError(null);
    const result = await checkForUpdate();
    setCheck(result);
    setPhase("idle");
  }

  async function handleInstall() {
    setPhase("downloading");
    setInstallError(null);
    setProgress({ downloadedBytes: 0, totalBytes: null });
    const result = await installPendingUpdate((p) => setProgress(p));
    // A successful install normally relaunches the app before this
    // resolves at all -- reaching here with `ok: true` (rather than the
    // process just ending) is harmless, but `ok: false` means it genuinely
    // failed (e.g. a dropped connection mid-download) and the user is
    // still looking at this panel.
    if (!result.ok) {
      setInstallError(result.error ?? "install failed");
      setPhase("idle");
    } else {
      setPhase("done");
    }
  }

  return (
    <div className="settings-field" style={{ marginTop: "1em" }}>
      <label>Updates</label>
      <dl className="settings-field-note" style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "0.15em 0.6em", margin: 0 }}>
        <dt>Current version</dt>
        <dd>{appVersion ?? "…"}</dd>
      </dl>

      <div className="settings-field-row" style={{ marginTop: "0.5em" }}>
        <button type="button" onClick={handleCheck} disabled={phase === "checking" || phase === "downloading"}>
          {phase === "checking" ? "Checking…" : "Check for updates"}
        </button>
        {check?.status === "available" && phase !== "downloading" && phase !== "done" && (
          <button type="button" onClick={handleInstall} style={{ marginLeft: "0.5em" }}>
            Download and install {check.version}
          </button>
        )}
      </div>

      {check?.status === "up-to-date" && phase === "idle" && (
        <div className="settings-field-status settings-field-status-active">up to date (v{appVersion ?? "?"})</div>
      )}
      {check?.status === "available" && (phase === "idle" || phase === "downloading" || phase === "done") && (
        <div className="settings-field-status settings-field-status-active">
          update available: v{check.version}
          {check.notes ? ` -- ${check.notes}` : ""}
        </div>
      )}
      {check?.status === "error" && (
        <div className="settings-field-status settings-field-status-none">check failed: {check.error}</div>
      )}

      {phase === "downloading" && (
        <div className="settings-field-status">
          downloading…
          {progress
            ? progress.totalBytes
              ? ` ${Math.round((progress.downloadedBytes / progress.totalBytes) * 100)}%`
              : ` ${Math.round(progress.downloadedBytes / 1024)} KB`
            : ""}
        </div>
      )}
      {phase === "done" && <div className="settings-field-status settings-field-status-active">installed -- restarting…</div>}
      {installError && <div className="settings-field-status settings-field-status-none">install failed: {installError}</div>}

      <p className="settings-field-note">
        Checks this project's GitHub Releases for a newer signed update package (verified against a bundled public
        key -- unrelated to Windows code signing, which this build does not use). Installing relaunches the app into
        the new version.
      </p>
    </div>
  );
}

/** Round a relative-time label out of an epoch-ms timestamp against a
 * live-ticking `now` (see `NetworkDiagnosticsPanel`'s own 1s timer) -- so
 * "3s ago" doesn't silently go stale between poll events, which can be
 * tens of seconds apart. */
function relativeTime(ms: number | null, now: number): string {
  if (ms === null) return "never";
  const deltaSec = Math.max(0, Math.round((now - ms) / 1000));
  if (deltaSec < 5) return "just now";
  if (deltaSec < 60) return `${deltaSec}s ago`;
  const min = Math.round(deltaSec / 60);
  if (min < 60) return `${min}m ago`;
  return `${Math.round(min / 60)}h ago`;
}

/**
 * S10 Phase 2 network diagnostics -- see `useNetworkDiagnostics`'s doc
 * comment for what this derives and why (no synthetic pings; reuses the
 * scan/alert pollers' own real outcomes, plus `navigator.onLine` as a
 * second, independently-tracked signal since the two can legitimately
 * disagree). Renders in both the browser and desktop builds -- unlike
 * `DesktopDiagnosticsPanel` below, nothing here depends on a Tauri API.
 */
function NetworkDiagnosticsPanel({
  scanPollEvent,
  alertStatus,
  alertError,
  alertLastPolledAt,
}: {
  scanPollEvent: PollEvent;
  alertStatus: AlertPollStatus;
  alertError: string | null;
  alertLastPolledAt: number | null;
}) {
  const diagnostics = useNetworkDiagnostics(scanPollEvent, alertStatus, alertError, alertLastPolledAt);
  const overall = overallNetworkStatus(diagnostics);

  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  const overallLabel: Record<typeof overall, string> = {
    online: "online",
    degraded: "degraded — recent requests failing",
    offline: "offline (browser reports no network)",
    checking: "checking…",
  };
  const overallClass =
    overall === "online" ? " settings-field-status-active" : overall === "checking" ? "" : " settings-field-status-none";

  function pollLine(o: (typeof diagnostics)["scan"]): string {
    if (o.lastFailureAt !== null && (o.lastSuccessAt === null || o.lastFailureAt > o.lastSuccessAt)) {
      return `failing, ${relativeTime(o.lastFailureAt, now)}: ${o.lastFailureDetail}`;
    }
    if (o.lastSuccessAt !== null) return `ok, ${relativeTime(o.lastSuccessAt, now)}`;
    return "no poll yet";
  }

  return (
    <div className="settings-field" style={{ marginTop: "1em" }}>
      <label>Network diagnostics</label>
      <div className={`settings-field-status${overallClass}`}>{overallLabel[overall]}</div>
      <dl className="settings-field-note" style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "0.15em 0.6em", margin: "0.4em 0 0" }}>
        <dt>Browser reports</dt>
        <dd>{diagnostics.browserOnline ? "online" : "offline"}</dd>
        <dt>Radar scan poll</dt>
        <dd>{pollLine(diagnostics.scan)}</dd>
        <dt>Alerts poll</dt>
        <dd>{pollLine(diagnostics.alerts)}</dd>
      </dl>
      <p className="settings-field-note">
        "Browser reports" is only <code>navigator.onLine</code>, which is unreliable alone -- a captive portal or a
        DNS-only outage can still report "online". The poll rows above reflect the real outcome of this app's own
        most recent live requests instead.
      </p>
    </div>
  );
}

/**
 * S10 Phase 2 cache-size control: a real, persisted, user-adjustable cap
 * on how many recently-downloaded scans `useScanHistory` keeps in memory
 * (see that module's `MIN_HISTORY_SCANS`/`MAX_HISTORY_SCANS_LIMIT`) --
 * replacing the previously hard-coded `MAX_HISTORY_SCANS` constant as the
 * *default*, not as the only possible value.
 */
function CacheControls({
  cacheLimit,
  entriesCount,
  onCacheLimitChange,
}: {
  cacheLimit: number;
  entriesCount: number;
  onCacheLimitChange: (limit: number) => void;
}) {
  const inputId = useId();
  return (
    <div className="settings-field" style={{ marginTop: "1em" }}>
      <label htmlFor={inputId}>Scan history cache size</label>
      <div className="settings-field-row">
        <input
          id={inputId}
          type="number"
          min={MIN_HISTORY_SCANS}
          max={MAX_HISTORY_SCANS_LIMIT}
          step={1}
          value={cacheLimit}
          onChange={(e) => {
            const parsed = Number(e.target.value);
            if (Number.isFinite(parsed)) onCacheLimitChange(parsed);
          }}
        />
        <span className="settings-field-note">scans ({entriesCount} currently held)</span>
      </div>
      <p className="settings-field-note">
        How many recently-downloaded radar volumes stay held in memory for previous/next/loop playback (min{" "}
        {MIN_HISTORY_SCANS}, max {MAX_HISTORY_SCANS_LIMIT}). Higher values give a longer loop history at the cost of
        more memory (a WSR-88D volume is typically a few MB); lowering it below the current count evicts the oldest
        held scans immediately.
      </p>
    </div>
  );
}

/**
 * S09c Settings section: the Rainbow API key field (the S09c addendum's
 * original whole scope), plus S10 Phase 2's cache-size control and network
 * diagnostics, plus the Phase 1 desktop diagnostics panel -- its own
 * top-level sidebar section, distinct from "Rainbow", so it reads as app
 * configuration rather than a Rainbow-specific control.
 *
 * The Rainbow key field is a credential, so it defaults to
 * `type="password"` with a show/hide toggle, even though (per
 * `useRainbowApiKey`'s doc comment) it never leaves this browser except in
 * a request straight to Rainbow's own API. Saves on every change (no
 * separate Save button/blur handler needed): `setSettingsKey` just writes
 * `localStorage` and notifies every subscriber, so this stays cheap and
 * keeps the Rainbow section's "configured" state in lockstep with what's
 * typed here, live.
 */
export interface SettingsPanelProps {
  scanPollEvent: PollEvent;
  alertStatus: AlertPollStatus;
  alertError: string | null;
  alertLastPolledAt: number | null;
  cacheLimit: number;
  cacheEntriesCount: number;
  onCacheLimitChange: (limit: number) => void;
}

export function SettingsPanel({
  scanPollEvent,
  alertStatus,
  alertError,
  alertLastPolledAt,
  cacheLimit,
  cacheEntriesCount,
  onCacheLimitChange,
}: SettingsPanelProps) {
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

      <CacheControls cacheLimit={cacheLimit} entriesCount={cacheEntriesCount} onCacheLimitChange={onCacheLimitChange} />
      <NetworkDiagnosticsPanel
        scanPollEvent={scanPollEvent}
        alertStatus={alertStatus}
        alertError={alertError}
        alertLastPolledAt={alertLastPolledAt}
      />
      <UpdatePanel />
      <DesktopDiagnosticsPanel />
    </div>
  );
}
