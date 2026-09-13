//! Per-target async sleep shim, used by each provider's own bounded retry
//! backoff around its discovery request (`provider_hrrr::client::HrrrClient::
//! object_exists_with_retries`, `provider_gefs::client::GefsClient::
//! discover_members_with_retries`) -- originally written for `provider-hrrr`
//! alone, moved here verbatim once `provider-gefs` needed the exact same
//! shim for the exact same reason, per this workspace's "prefer a second
//! concrete implementation before generalizing" rule.
//!
//! `tokio::time::sleep` needs `tokio`'s time driver, which in turn needs
//! OS timers/threads unavailable on `wasm32-unknown-unknown` -- neither
//! provider crate depends on `tokio` at all on that target (see their
//! `Cargo.toml`s). The retry/backoff *algorithm* in each provider's
//! `client.rs` must behave identically on both targets (a transient
//! discovery-loop error must still be retried, not silently swallowed as
//! "not found," on wasm32 exactly as natively), so only the actual sleep
//! primitive is swapped per target here, not the retry logic itself.

/// Sleep for `duration` on the native target, via `tokio::time::sleep`.
#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep(duration: std::time::Duration) {
    tokio::time::sleep(duration).await;
}

/// Sleep for `duration` on `wasm32`, via the browser's own
/// `window.setTimeout` -- the standard hand-rolled shim for this (the same
/// technique the `gloo-timers` crate wraps) rather than a full async
/// runtime, which neither provider crate otherwise needs on this target.
#[cfg(target_arch = "wasm32")]
pub async fn sleep(duration: std::time::Duration) {
    let millis = i32::try_from(duration.as_millis()).unwrap_or(i32::MAX);
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let window = web_sys::window()
            .expect("this shim's wasm32 target always runs inside a browser window");
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis)
            .expect("window.setTimeout should never fail for a plain numeric delay");
    });
    // A rejected promise is impossible here (nothing ever calls `_reject`),
    // so a `Result::Err` from `JsFuture` is unreachable in practice; still
    // handled (not `.unwrap()`ed) rather than risking a wasm trap on an
    // untested browser edge case -- either outcome just means "done
    // waiting."
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
