# RadarPro Desktop — Crash-Handling Strategy

Stage **S10 (Desktop Beta), Phase 2** -- see
`Agent Context/context/stages/S10-desktop-beta.md`'s "crash-handling
strategy" item. This is a strategy document plus a small native-side hook,
**not** a crash-reporting pipeline -- a hosted crash SaaS (Sentry, etc.)
needs an account/credentials decision from the user (see Phase 1's
deferred-work list and `README.md`), which is out of scope for a
credential-free phase.

There are three separate things that can "crash" in this app, and today
each is handled differently. Concretely:

## 1. WebView2 renderer crash (the actual `apps/web` frontend)

Confirmed still true, live-verified this phase (not just re-stated from
Phase 1): WebView2 is Chromium-based and uses **Crashpad** for its own
process crash reporting, entirely independent of this app's code. Running
the app once and inspecting its real app-data directory shows:

```
%LOCALAPPDATA%\org.radarpro.desktop\EBWebView\Crashpad\
├── attachments\
├── reports\      <- .dmp minidumps land here if/when the renderer crashes
├── metadata
├── settings.dat
└── throttle_store.dat
```

(`org.radarpro.desktop` is this app's `identifier` from `tauri.conf.json`;
`EBWebView` is WebView2's own per-app user-data-folder name, created the
first time the app navigates to content -- this repo's `apps/web`
frontend, in this case.)

Today: these dumps are written locally and never leave the machine. No
code in this repo reads, uploads, or even lists them. This is a real (if
passive) starting point -- if a user reports a renderer crash, a developer
with access to that machine can pull the `.dmp` files from
`reports\` and symbolicate them the same way any Chromium/Edge crash dump
is symbolicated -- but nothing automatic happens today.

## 2. Native (Rust) shell panic

New this phase: `apps/desktop/src-tauri/src/lib.rs`'s `run()` installs a
panic hook (via `std::panic::take_hook`/`set_hook`) *before*
`tauri::Builder::default()` runs, so it is active for the whole process
lifetime, including any panic during setup. On a panic it:

1. Logs `PANIC (native shell about to crash): {info}` via `log::error!` --
   the same `log::` pipeline `tauri_plugin_log` already routes to stdout
   *and* the real on-disk log file (`%LOCALAPPDATA%\org.radarpro.desktop\logs\radarpro.log`,
   confirmed present and being written to live during this phase's
   verification), so a field report's log file has "what panicked, where,
   roughly when" without any additional tooling.
2. Then calls the previous (default) hook, so Rust's own stderr panic
   message/backtrace behavior is unchanged -- this is additive, not a
   replacement.

This is real but narrow: this crate's own native code is currently tiny
(one Tauri command, two plugin registrations, this hook) -- there just
isn't much surface for a *native*-side panic today. Its value grows as
this shell's own native code grows (e.g. if a future phase adds more
commands that touch the filesystem, native dialogs, etc.).

**What this does not cover:** a WebView2/renderer crash (case 1 above,
already handled independently by Crashpad) and a wasm trap inside the
webview (case 3 below) -- neither runs on this native thread, so this
hook cannot see either.

## 3. A wasm panic/trap inside the webview (radar-web / weather-alerts / etc.)

A Rust panic inside one of this repo's wasm crates (`radar-web`,
`weather-alerts`, `forecast-web`, `mrms-web`) surfaces as a normal
JavaScript exception in the webview's console (via wasm-bindgen's panic
hook, which by default prints to the console) -- it does not crash the
WebView2 process outright unless it happens during something Chromium
itself considers fatal. Console forwarding already exists
(`attachDesktopLogging` in `apps/web/src/platform/desktop.ts`, Phase 1),
so a wasm panic's `console.error` output already lands in the same
on-disk log file as everything else, *if* `console.error` is what
reported it (wasm-bindgen's default panic hook does use `console.error`).
No change needed here this phase -- confirmed as already covered by
existing Phase 1 work, not a new gap.

## What a future signed-and-distributed build should add

Deliberately not implemented yet -- each of these needs an explicit
decision (and usually an account/credential) from the user, consistent
with this phase staying credential-free:

- **Opt-in crash reporting to a real service** (e.g. Sentry, or Tauri's
  own crash-reporter ecosystem plugins). Must be:
  - **Off by default.** Per this project's Global Contract ("no
    telemetry/data collection the user hasn't explicitly opted into"), a
    first-run or Settings-panel toggle must be explicitly turned on before
    a single byte of crash data leaves the machine -- mirroring how the
    Rainbow API key field already treats a credential as
    browser-local-only until the user acts.
  - **Minimized in scope.** A reasonable target: the panic message,
    Rust backtrace, app/Tauri version, OS/arch (all already surfaced
    read-only via `get_diagnostics`), and the *existence* of a WebView2
    Crashpad dump (offering to attach it, not auto-uploading it silently)
    -- never radar/alert/forecast data being viewed at the time, never
    the Rainbow API key or any other locally-stored setting, never
    anything resembling telemetry beyond the crash event itself.
  - **Clearly disclosed.** The Settings panel's toggle should say in
    plain language what gets sent and where, the same register this
    project's existing Rainbow-key note already uses ("requests go
    directly from this browser to Rainbow's own API, never through any
    RadarPro server").
- **Symbol upload for native (Rust) crashes**, so a panic backtrace from a
  release (non-debug) build resolves to real function names/line numbers
  instead of addresses -- needs a decision on where `.pdb` (Windows) /
  `.dSYM` (macOS) symbols are stored and by which service.
- **WebView2 Crashpad dump collection**, i.e. actually watching
  `EBWebView\Crashpad\reports\` and offering to upload a dump after a
  detected renderer crash (WebView2 exposes a
  `ProcessFailed`/`CoreWebView2.ProcessFailed`-style event a host app can
  observe) -- today nothing in this repo watches for that event at all;
  a future phase should add a Tauri-side listener that at minimum logs
  "renderer process failed: {reason}" via the same `log::` pipeline (an
  easy, credential-free win closely related to this phase's panic-hook
  work), before tackling the harder "upload the dump" half.
- **Crash-triage grouping rules** once a real pipeline exists, so repeated
  reports of the same underlying bug don't each need individual manual
  review.

## Verified this phase

- `EBWebView\Crashpad\reports\` exists under
  `%LOCALAPPDATA%\org.radarpro.desktop\` after a normal run (checked live,
  not assumed from Phase 1's note).
- The native panic hook logs to the real on-disk log file
  (`%LOCALAPPDATA%\org.radarpro.desktop\logs\radarpro.log`) before the
  process goes down -- verified by temporarily adding a
  `#[tauri::command] fn trigger_test_panic() { panic!(...) }`, invoking it
  from the running app's webview console, observing the crash, then
  removing it (not shipped). The real, unexpected result: a panic
  reached from inside a Tauri command handler does not just unwind this
  thread -- it took down the **entire process** immediately
  (`thread caused non-unwinding panic. aborting.`, exit code
  `0xc0000409` / `STATUS_STACK_BUFFER_OVERRUN`), with no window/webview
  survival and no chance for any *later* cleanup code to run. This makes
  the panic hook logging *before* that abort more important, not less --
  it is the only chance to record anything about what happened, since
  nothing downstream of the panic gets to run at all. The log line was
  confirmed present (`PANIC (native shell about to crash): panicked at
  src\lib.rs:...`) immediately followed by the process's real exit in
  both the terminal output and the on-disk log file.
