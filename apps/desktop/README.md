# RadarPro Desktop (Tauri shell)

Stage **S10 (Desktop Beta), Phase 1** -- see
`Agent Context/context/stages/S10-desktop-beta.md`. This is the buildable
groundwork only: a native Windows shell wrapping `apps/web`'s existing
frontend, unsigned, for local dev/debug use. It is **not** a signed
installer, an auto-updater, or a cross-platform build -- see "Deferred
work" below.

## What this is

- A Tauri v2 app (`src-tauri/`) that loads `apps/web`'s frontend into a
  native WebView2 window. It does not reimplement any app logic --
  `apps/web`'s React/wasm/wgpu code is unchanged and still runs standalone
  in a plain browser exactly as before.
- Two load paths, both configured in `src-tauri/tauri.conf.json`:
  - `tauri dev`: proxies `apps/web`'s Vite dev server
    (`beforeDevCommand`/`devUrl`, `http://localhost:5173`) for iteration
    with hot reload.
  - `tauri build`: runs `apps/web`'s production build
    (`beforeBuildCommand`) and loads the built `apps/web/dist` output
    directly (`frontendDist`) -- no dev server involved.
- One real persisted setting: the default radar site (`icao`), via
  `tauri-plugin-store`, surviving an app restart. See
  `apps/web/src/platform/desktop.ts`.
- Structured logging to a real file in the OS log directory
  (`%LOCALAPPDATA%\org.radarpro.desktop\logs\radarpro.log` on Windows) via
  `tauri-plugin-log`, fed both by native-side log lines and the webview's
  own `console.*` calls (forwarded manually -- see the doc comment on
  `attachDesktopLogging` for why `tauri-plugin-log`'s own `attachConsole()`
  does *not* do this).
- A small `get_diagnostics` command (app/Tauri version, OS/arch, log file
  location) surfaced in the app's own Settings sidebar section
  ("Desktop diagnostics") -- native-shell-only, invisible in a browser tab.

## Tauri version

**Tauri v2**, CLI/crate `2.11.x` (`@tauri-apps/cli` `2.11.4`, `tauri` crate
`2.11.3`/`2.11.5` as resolved) -- verified against the live npm/crates.io
registries at the time this was built (2026-09-13), not assumed from
training data. Tauri v2 is the current stable major (v1 is EOL/legacy).

## Prerequisites (this machine)

- Node.js + npm (already used by `apps/web`).
- Rust stable via `rustup` (already installed for this workspace's own
  crates; this crate uses the same toolchain).
- Windows: the WebView2 Runtime, which ships with Windows 11 / current
  Edge and was already present on this machine -- nothing extra installed.

## Dev workflow

```sh
cd apps/desktop
npm install       # once
npm run dev       # runs `tauri dev`: starts apps/web's Vite dev server
                   # (npm --prefix ../web run dev) and opens the native window
```

## Local unsigned build

```sh
cd apps/desktop
npm run build     # runs `tauri build`: builds apps/web (npm --prefix ../web
                   # run build) then bundles a release binary + installer(s)
                   # under src-tauri/target/release/bundle/
```

This produces **unsigned** local artifacts only (Windows will show an
"unknown publisher" SmartScreen warning) -- see "Deferred work".

## Why `src-tauri` is its own Cargo workspace

`src-tauri/Cargo.toml` has an empty `[workspace]` table, deliberately
making it its own Cargo workspace root rather than joining the repo's root
`Cargo.toml` workspace. Tauri pulls in a large, Windows-specific dependency
graph (WebView2 bindings, `windows-rs`, etc.) that has nothing to do with
the core radar/forecast crates; without this, running any workspace-wide
Cargo command from `C:\RadarPro` would silently pull this crate in too.
The root workspace's `cargo build`/`clippy`/`test` are unaffected by this
crate (verified via `cargo metadata` during this stage).

## Settings persistence

Backed by `tauri-plugin-store`, writing plain JSON to
`%APPDATA%\org.radarpro.desktop\settings.json` (Windows' roaming app-data
directory). Currently stores exactly one key, `defaultSiteIcao`. See
`apps/web/src/platform/desktop.ts` (`loadPersistedDefaultSite`/
`persistDefaultSite`) and its use in `apps/web/src/App.tsx`. Deliberately
not a generic settings framework -- one real setting, per this project's
"avoid premature abstraction" rule; add more keys to the same store the
same way if/when a second real setting is needed.

## Deferred work (explicitly out of scope for this phase)

These need a real decision and/or credentials/accounts **from the user**
before they can be implemented -- not skipped silently, not faked:

- **Code signing (Windows) / notarization (macOS)**: requires a code-signing
  certificate (Windows: OV/EV cert, ideally via a cloud HSM) and, for
  macOS, an Apple Developer account + notarization credentials. Without
  this, any distributed build trains users to click through an "unknown
  publisher"/Gatekeeper warning -- acceptable for local dev/debug only.
- **Installer distribution**: `tauri build` produces local NSIS/MSI
  artifacts under `src-tauri/target/release/bundle/`; nothing is uploaded
  or hosted anywhere. Real distribution needs a decision on where builds
  are hosted and how users get them.
- **Auto-update strategy**: no update server, update manifest signing key,
  or staged-rollout plan exists yet. Needs an explicit decision on hosting
  (self-hosted vs. a service) before implementing Tauri's updater plugin.
- **macOS / Linux builds**: this phase is Windows-only, built and verified
  on this machine. macOS needs Apple hardware (or CI) and its own
  notarization setup; Linux needs its own packaging/dependency story
  (GTK/WebKitGTK version spread across distros).
- **Crash-reporting service**: no third-party crash SaaS (Sentry, etc.) is
  wired up. A local crash-handling strategy (Tauri/WebView2 crash dumps
  already land under the app's `EBWebView/Crashpad` directory) is a
  reasonable starting point for a future phase, but integrating a hosted
  service is a separate decision (data-handling/cost implications).
