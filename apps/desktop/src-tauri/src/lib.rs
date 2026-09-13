// RadarPro desktop shell -- Stage S10 (Desktop Beta), Phase 1 groundwork.
// See `Agent Context/context/stages/S10-desktop-beta.md`.
//
// This crate is deliberately thin: it hosts `apps/web`'s existing frontend
// in a native window and adds exactly the native-only capabilities that
// frontend cannot provide for itself in a browser -- structured log-file
// output and a settings store backed by the OS app-data directory. Every
// other feature (radar rendering, alerts, forecast, MRMS/Rainbow, the
// wasm/GPU pipeline) is unchanged `apps/web` code; this shell does not
// reimplement any of it (GLOBAL_CONTRACT: "the desktop shell is a new host
// for the exact same `apps/web` frontend, not a reimplementation").
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};

/// Diagnostics surfaced to the frontend via `invoke("get_diagnostics")` --
/// intentionally small and read-only (no filesystem/shell passthrough; see
/// GLOBAL_CONTRACT's IPC-boundary rules). This is the native-shell half of
/// this stage's "GPU/backend reporting" requirement: `apps/web`'s own
/// top-bar status line already reports the wgpu adapter/backend it
/// negotiated (see `useRadarRenderer`); this adds the *host* facts that
/// only the privileged side knows (app/Tauri version, OS/arch, and where
/// the real log file for this run lives on disk).
#[derive(Serialize)]
struct DesktopDiagnostics {
    app_version: String,
    tauri_version: String,
    os: String,
    arch: String,
    /// Absolute path to the directory `tauri-plugin-log` is writing this
    /// run's log file into, or `None` if it could not be resolved (e.g. a
    /// sandboxed/unsupported platform) -- never fabricated.
    log_dir: Option<String>,
}

#[tauri::command]
fn get_diagnostics(app: tauri::AppHandle) -> DesktopDiagnostics {
    let log_dir = app
        .path()
        .app_log_dir()
        .ok()
        .map(|p| p.display().to_string());
    DesktopDiagnostics {
        app_version: app.package_info().version.to_string(),
        tauri_version: tauri::VERSION.to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        log_dir,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // S10 Phase 2 crash-handling strategy (see `apps/desktop/CRASH_HANDLING.md`):
    // log a structured "about to crash" entry via the same `log::` macros
    // `tauri_plugin_log` below is already wired to (stdout + the on-disk
    // log file), then still run the previous/default hook (Rust's own
    // stderr backtrace printer) so nothing about the existing panic
    // behavior is lost. Installed before `tauri::Builder::default()` runs
    // so it is active for this thread's whole lifetime, including any
    // panic during setup, not just after the window is up.
    //
    // This only covers a Rust panic in this crate's own (currently very
    // small) native code -- it cannot and does not catch a WebView2/
    // renderer-side crash (already handled by WebView2's own Crashpad
    // dumps, see CRASH_HANDLING.md) or a wasm trap inside the webview
    // (surfaces as a JS exception in that process, not a native panic
    // here).
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("PANIC (native shell about to crash): {info}");
        default_panic_hook(info);
    }));

    tauri::Builder::default()
        // Structured logging to a real file in the OS app-data/log
        // directory (Windows: `%LOCALAPPDATA%/org.radarpro.desktop/logs`),
        // plus stdout for `tauri dev`. Enabled unconditionally (not gated
        // to debug builds) -- diagnosing a field report from an unsigned
        // local build is exactly when this matters most. The frontend
        // forwards its own `console.*` calls into this same pipeline by
        // calling this plugin's `trace`/`info`/`warn`/`error` JS functions
        // from a small `console.*` wrapper -- see
        // `apps/web/src/platform/desktop.ts`'s `attachDesktopLogging` doc
        // comment for why that plugin's own `attachConsole()` is NOT what
        // does this (it forwards the opposite direction) -- so wasm/GPU
        // init messages land in the same file as native-side log lines.
        .plugin(
            tauri_plugin_log::Builder::new()
                // `Builder::new()` already seeds its own default target list
                // (stdout + an app-named log file) -- `.target()` *appends*
                // to that list rather than replacing it (confirmed against
                // the crate's source during this stage's own verification,
                // after an earlier `.target()`-chaining version silently
                // produced duplicate log entries from two targets that
                // both resolved to the same file). `.targets([...])`
                // replaces the list outright, which is what we want here.
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir {
                        file_name: Some("radarpro".into()),
                    }),
                ])
                .level(log::LevelFilter::Info)
                .build(),
        )
        // Settings persistence (S10 Phase 1 proof-of-concept: the default
        // radar site). Backed by a JSON file in the OS app-data directory
        // via the official `tauri-plugin-store` -- see that crate's docs
        // for the on-disk format; deliberately not a hand-rolled
        // fs-read/write IPC command (GLOBAL_CONTRACT: narrowest verb
        // possible, no generic filesystem passthrough to the renderer).
        .plugin(tauri_plugin_store::Builder::default().build())
        // S10 Phase 4 update strategy: Tauri's own updater plugin, checking
        // the GitHub Releases endpoint configured in `tauri.conf.json`
        // (`plugins.updater.endpoints`) and verifying downloaded artifacts
        // against that config's `pubkey` (a self-generated Ed25519/minisign
        // keypair -- see `apps/desktop/README.md`'s "Releases and updates"
        // section for how that keypair is generated/rotated). This is a
        // completely separate, free, no-CA mechanism from the Windows
        // code-signing this project has explicitly decided against. The
        // frontend calls it via `@tauri-apps/plugin-updater` -- see
        // `apps/web/src/platform/desktop.ts`'s `checkForUpdate`/
        // `installPendingUpdate` and their use in `SettingsPanel.tsx`.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Supplies `relaunch()` (via `@tauri-apps/plugin-process`), used to
        // restart the app into the newly-installed version once an update
        // finishes downloading+installing. Only `process:allow-restart` is
        // granted in `capabilities/default.json` -- not `process:default`,
        // which would also hand the webview `allow-exit` (killing the whole
        // app), unneeded surface for what this feature actually needs (see
        // GLOBAL_CONTRACT's "narrowest verb possible" IPC rule).
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![get_diagnostics])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
