//! [`ForecastProvider`]: the trait every forecast data source implements --
//! discover a published run, then fetch/decode one field for it.
//!
//! # Shape and why
//!
//! ```text
//! discover_latest_run(lookback_days) -> Run
//! fetch_field(&Run, &FieldRequest)   -> ForecastGrid
//! ```
//!
//! `Run` is an **associated type**, not [`crate::model::ModelRun`] itself:
//! each provider's real run identity is genuinely provider-specific (GEFS's
//! `RunReference` is a UTC date + one of four run hours; a different
//! provider's real run key could be shaped differently), and the S08 stage
//! file's own instruction is for `provider-gefs` to keep "the
//! `noaa-gefs-pds` bucket/key layout" as its own concern, not something
//! this crate generalizes away. [`Self::run_metadata`] converts a
//! provider's own `Run` into the canonical, display-ready [`crate::model::ModelRun`]
//! any UI code can show without knowing the concrete `Run` type. This
//! trait is used generically (`fn use_provider<P: ForecastProvider>(p: &P)`),
//! not as a trait object -- an associated type plus `async fn` (both
//! ordinary, stable Rust since 1.75, no `async-trait` dependency needed)
//! is not object-safe, but nothing in this stage's exit criteria (the
//! shared render call path, proven in `forecast-core`'s own live
//! cross-provider test) requires dynamic dispatch; a real UI provider
//! switcher is explicitly out of scope for this stage (`apps/web` wiring is
//! a separate follow-up task).
//!
//! `Error` is also an associated type rather than one shared error enum:
//! GEFS's and HRRR's real failure modes (a missing member vs. an
//! unsupported grid template, to pick one example) are provider-specific
//! detail GLOBAL_CONTRACT already asks to keep at the provider boundary
//! ("provider-specific names and formats stop at provider boundaries").
//!
//! # What this trait does *not* generalize
//!
//! Bucket/key layout, `.idx` parsing, and the GRIB2 decode wiring itself
//! all stay inside each concrete provider (`provider-gefs`, `provider-hrrr`)
//! -- this trait only names the three operations a provider-agnostic caller
//! needs, it does not prescribe *how* a provider implements them.

use crate::grid::ForecastGrid;
use crate::model::{ModelMetadata, ModelRun};
use crate::request::FieldRequest;
use std::future::Future;

/// `Send` on every target except `wasm32`, where it is a no-op marker
/// implemented for everything.
///
/// [`ForecastProvider`]'s async methods need to be usable from a
/// multi-threaded native async runtime (`Send` futures), but a provider's
/// real implementation (`provider-gefs`/`provider-hrrr`, via `reqwest`) is
/// built on `wasm-bindgen`'s `JsFuture` on `wasm32` -- a type that wraps a
/// `Rc<RefCell<..>>` and is therefore never `Send`, because `wasm32-unknown-
/// unknown` has no real threads to send anything to in the first place. A
/// blanket `+ Send` bound on [`ForecastProvider`]'s futures would make the
/// trait impossible to implement on `wasm32` at all for any HTTP-backed
/// provider; this marker lets the same trait definition require `Send`
/// only where it is actually meaningful (native), while wasm32's
/// inherently single-threaded model makes the requirement moot there. Same
/// idiom used across the wasm/native-async ecosystem (e.g.
/// `send_wrapper`-adjacent crates) for exactly this reason -- not a
/// speculative abstraction, just what wasm32 async requires.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// A forecast data source: discover a published run, then fetch and decode
/// one field for it.
pub trait ForecastProvider {
    /// This provider's own concrete run-identity type (e.g. GEFS's
    /// `RunReference`). Never a [`ModelRun`] itself -- see this module's
    /// doc comment.
    type Run: Clone;

    /// This provider's own error type for every fallible operation below.
    type Error: std::error::Error + Send + Sync + 'static;

    /// This provider's static identity/description.
    fn metadata(&self) -> ModelMetadata;

    /// Convert this provider's own run identity into the canonical,
    /// display-ready [`ModelRun`].
    fn run_metadata(&self, run: &Self::Run) -> ModelRun;

    /// Discover the most recently published run, looking back at most
    /// `lookback_days` UTC calendar days from today. A model run can take
    /// a few hours after its nominal run time to fully publish, so "most
    /// recent" may not be "today's latest nominal run hour".
    fn discover_latest_run(
        &self,
        lookback_days: u32,
    ) -> impl Future<Output = Result<Self::Run, Self::Error>> + MaybeSend;

    /// Fetch and decode one field (one variable, forecast lead, and
    /// ensemble statistic if this provider has one) for `run`. A
    /// deterministic provider (no ensemble at all, `ModelMetadata::is_ensemble
    /// == false`) must return an error rather than silently ignoring a
    /// request with `request.ensemble.is_some()`.
    fn fetch_field(
        &self,
        run: &Self::Run,
        request: &FieldRequest,
    ) -> impl Future<Output = Result<ForecastGrid, Self::Error>> + MaybeSend;
}
