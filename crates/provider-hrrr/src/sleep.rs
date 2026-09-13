//! Per-target async sleep shim, used only by
//! [`crate::client::HrrrClient::object_exists_with_retries`]'s bounded
//! retry backoff.
//!
//! `tokio::time::sleep` needs `tokio`'s time driver, which in turn needs
//! OS timers/threads unavailable on `wasm32-unknown-unknown` -- this crate
//! has no `tokio` dependency at all on that target (see `Cargo.toml`). The
//! retry/backoff *algorithm* in `client.rs` must behave identically on
//! both targets (a transient discovery-loop error must still be retried,
//! not silently swallowed as "not found," on wasm32 exactly as natively --
//! see that function's own doc comment), so only the actual sleep
//! primitive is swapped per target here, not the retry logic itself.

/// Sleep for `duration` on the native target, via `tokio::time::sleep`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn sleep(duration: std::time::Duration) {
    tokio::time::sleep(duration).await;
}

/// Sleep for `duration` on `wasm32`, via the browser's own
/// `window.setTimeout` -- the standard hand-rolled shim for this (the same
/// technique the `gloo-timers` crate wraps) rather than a full async
/// runtime, which this crate otherwise has no need for on this target.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn sleep(duration: std::time::Duration) {
    let millis = i32::try_from(duration.as_millis()).unwrap_or(i32::MAX);
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let window = web_sys::window()
            .expect("provider-hrrr's wasm32 target always runs inside a browser window");
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
