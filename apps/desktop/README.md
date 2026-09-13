# RadarPro Desktop (Tauri shell)

Stage **S10 (Desktop Beta)** -- see
`Agent Context/context/stages/S10-desktop-beta.md`. A native **Windows**
shell wrapping `apps/web`'s existing frontend: settings persistence,
structured logging/diagnostics, crash handling, and (as of Phase 4) a real
release pipeline and self-update mechanism. **Windows-only and
deliberately unsigned** -- both explicit user decisions, not gaps; see
"Releases and updates" below for what that does and doesn't mean.

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
- Three real persisted settings, all via `tauri-plugin-store`, surviving an
  app restart: the default radar site (`icao`), favorited/starred moment
  codes (surfaced as a star toggle + quick-select row next to the Moment
  picker), and the scan-history cache-size limit (Settings sidebar
  section, wired through to `useScanHistory`'s actual eviction cap). See
  `apps/web/src/platform/desktop.ts`.
- Structured logging to a real file in the OS log directory
  (`%LOCALAPPDATA%\org.radarpro.desktop\logs\radarpro.log` on Windows) via
  `tauri-plugin-log`, fed both by native-side log lines and the webview's
  own `console.*` calls (forwarded manually -- see the doc comment on
  `attachDesktopLogging` for why `tauri-plugin-log`'s own `attachConsole()`
  does *not* do this). A native panic hook logs a structured
  "about to crash" entry to this same file before unwinding -- see
  `CRASH_HANDLING.md`.
- A small `get_diagnostics` command (app/Tauri version, OS/arch, log file
  location) surfaced in the app's own Settings sidebar section
  ("Desktop diagnostics") -- native-shell-only, invisible in a browser tab.
- A network diagnostics panel (Settings sidebar section) showing
  `navigator.onLine` plus the real outcome (success/failure + when) of the
  radar scan and alerts pollers' own most recent live requests -- works in
  both the desktop shell and a plain browser tab, since it needs no Tauri
  API.
- A real update-check mechanism (Settings sidebar section, "Updates") via
  `tauri-plugin-updater`, and a GitHub Actions release pipeline that builds
  and publishes the unsigned Windows installer on a tag push -- see
  "Releases and updates" below.

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
"unknown publisher" SmartScreen warning) -- deliberately, see "Releases
and updates" below. For a real, distributable release (attached to a
GitHub Release, discoverable by the in-app updater), see that section's
"Cutting a release" instead of running this by hand.

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
directory). Currently stores three keys: `defaultSiteIcao`,
`favoriteMoments` (a string array, e.g. `["REF","VEL"]`), and
`cacheLimitScans` (an integer, bounded to
`useScanHistory`'s `MIN_HISTORY_SCANS`/`MAX_HISTORY_SCANS_LIMIT`). See
`apps/web/src/platform/desktop.ts` (`loadPersistedDefaultSite`/
`persistDefaultSite` and the `FavoriteMoments`/`CacheLimit` equivalents)
and their use in `apps/web/src/App.tsx`. Deliberately not a generic
settings framework -- add more keys to the same store the same way
if/when a new real setting is needed, rather than inventing a second
mechanism.

## Releases and updates (S10 Phase 4)

### Code signing: deliberately not done, ever

RadarPro does **not** pursue a Windows code-signing certificate (or Apple
notarization) -- this is an explicit, permanent user decision (see
`Agent Context/context/stages/S10-desktop-beta.md`'s "Code signing"
section), not a gap waiting to be filled. Every installer this project
publishes is unsigned: Windows will show an "unknown publisher"
SmartScreen warning, which a user clicks through ("More info" -> "Run
anyway"). RadarPro is open source; anyone who wants a signed build is free
to fork and sign it themselves with their own certificate.

The updater below uses a **completely different, free, no-CA mechanism**
(a self-generated Ed25519/minisign keypair) to verify update *packages*
are unmodified and came from this project's own release pipeline. That is
not code signing and does not make SmartScreen trust the app's publisher
identity -- don't confuse the two.

### Cutting a release

1. Bump `"version"` in `apps/desktop/src-tauri/tauri.conf.json` (the
   updater compares this value against what's already installed --
   forgetting to bump it means an already-installed copy won't see the
   new release as an update).
2. Commit that bump, then tag and push:
   ```sh
   git tag desktop-v0.2.0   # match the version you just set, prefixed "desktop-v"
   git push origin desktop-v0.2.0
   ```
3. Pushing a `desktop-v*` tag triggers `.github/workflows/desktop-release.yml`
   (a **separate** workflow from `.github/workflows/ci.yml`, which still
   only runs on every push/PR to `main` and does not touch the desktop
   app). It runs on `windows-latest` only (this project is Windows-only --
   see this stage's "Platforms" decision), builds `apps/web` then
   `apps/desktop` exactly like the local `npm run build` flow above, and
   uses `tauri-apps/tauri-action` to create a **draft** GitHub Release
   with the built `.msi`/`.exe` installer(s) plus a signed `latest.json`
   updater manifest attached.
4. The release is a **draft** on purpose -- go to the repo's Releases page,
   sanity-check the attached installer (download and run it once), then
   publish the draft manually. Until it's published, `.../releases/latest`
   doesn't see it (GitHub's API/download-shortcut behavior, not something
   this workflow adds), so no installed copy will offer to update to it --
   publishing the draft *is* the rollout gate.
5. To pull a bad release back: unpublish/delete it (or delete just the
   `latest.json` asset) on GitHub -- installed copies simply stop seeing an
   update until a corrected release is published. There's no separate
   "rollback" mechanism to run; GitHub Releases as the update source makes
   the currently-published release the entire state to manage.

### One-time setup: the updater signing key (you must do this)

The updater plugin needs an Ed25519/minisign keypair to sign/verify update
packages. **A keypair has already been generated for this repo** (via
`tauri signer generate`); its **public** key is committed in
`apps/desktop/src-tauri/tauri.conf.json` (`plugins.updater.pubkey` --
public keys are meant to be public, safe to commit). Its **private** key
was written to a local file *outside* this repo and was never committed
(verify yourself with `git log -p -- apps/desktop/src-tauri` and
`git status` if you want to double-check) -- you (the repo owner) need to
add it as a GitHub Actions secret before `desktop-release.yml` can produce
a working updater manifest:

1. Get the private key content -- it was printed once when generated and
   saved to a local file kept outside version control. If you no longer
   have it, generate a **new** keypair yourself:
   ```sh
   cd apps/desktop
   npx tauri signer generate -w /somewhere/outside/the/repo/radarpro-updater.key
   ```
   then replace `plugins.updater.pubkey` in `tauri.conf.json` with the new
   `.pub` file's content and commit that (only the public key -- see
   above) -- any existing installs' updater will only trust packages
   signed by whichever private key matches the pubkey they shipped with,
   so rotating the key means older installs stop auto-updating until they
   manually reinstall the new version once.
2. In the GitHub repo (`countryboysplay/RadarPro`) go to **Settings ->
   Secrets and variables -> Actions -> New repository secret**.
3. Add a secret named exactly `TAURI_SIGNING_PRIVATE_KEY` whose value is
   the full contents of the private key file (the whole file, not a path
   -- paste it as-is).
4. This key was generated **without a password** (the CLI warns about this
   -- acceptable here because the raw key material never leaves GitHub's
   encrypted secret storage and is never in the repo). If you'd rather
   protect it with a password, regenerate with `tauri signer generate -p
   <password> -w ...` and also add a second secret named
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` with that password -- the workflow
   already reads both secrets unconditionally (an unset/empty
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secret is fine for a
   no-password key).
5. That's it -- no other secrets are needed. `GITHUB_TOKEN` (used to
   create the release and upload assets) is provided automatically by
   GitHub Actions, not something you set up.

Nothing above is a code-signing certificate or costs money -- see "Code
signing: deliberately not done, ever" above.

### How the app checks for updates

`apps/web/src/platform/desktop.ts`'s `checkForUpdate`/`installPendingUpdate`
wrap `@tauri-apps/plugin-updater`, surfaced as an "Updates" section in the
Settings sidebar (`apps/web/src/ui/SettingsPanel.tsx`'s `UpdatePanel`):
"Check for updates" polls the endpoint configured in
`plugins.updater.endpoints` in `tauri.conf.json`
(`https://github.com/countryboysplay/RadarPro/releases/latest/download/latest.json`,
generated and signed by the release workflow above); if a newer version is
found, "Download and install" downloads it, verifies its signature against
the committed `pubkey`, installs it, and relaunches the app
(`@tauri-apps/plugin-process`'s `relaunch()`) -- all no-ops in a plain
browser tab, same as every other function in that module.

## Deferred work (explicitly out of scope for this stage)

These need a real decision and/or credentials/accounts **from the user**
before they can be implemented -- not skipped silently, not faked:

- **macOS / Linux builds**: out of scope for this whole stage, not just
  this phase -- an explicit, permanent user decision (Windows-only; see
  this stage's "Platforms" section), not a gap. macOS would need Apple
  hardware (or CI) and its own notarization setup; Linux would need its
  own packaging/dependency story (GTK/WebKitGTK version spread across
  distros). Neither is planned.
- **Crash-reporting service**: no third-party crash SaaS (Sentry, etc.) is
  wired up. See `CRASH_HANDLING.md` for the concrete strategy: what
  already exists today (WebView2's own Crashpad dumps under
  `EBWebView\Crashpad\reports\`, a native panic hook logging to the same
  file `tauri-plugin-log` writes to) and what a future distributed build
  should add (opt-in only, per this project's no-telemetry-without-
  consent rule).
- **Percentage-based staged rollout**: GitHub Releases as the update
  source is all-or-nothing (a release is either published, and every
  installed copy sees it as "latest", or it's a draft and no one does) --
  there's no built-in 1%/10%/100% ramp. The draft-first release process
  above is the staging gate this project actually has: nothing reaches
  users until a human reviews and publishes.
