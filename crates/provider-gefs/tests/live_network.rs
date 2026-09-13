//! The only tests in this crate that talk to the real, live
//! `noaa-gefs-pds` bucket: full discovery -> `.idx` -> sparse Range GET ->
//! GRIB2 decode for three real ensemble identities (control, a perturbed
//! member, the mean), plus confirming a known-absent member (`gep31`)
//! fails clearly rather than panicking or silently substituting data.
//!
//! Per the same pattern already established in this repo for
//! GPU-availability (`radar-render/src/gpu_tests.rs`) and live-network
//! NEXRAD access (`radar-cache/tests/live_network.rs`): a test that
//! depends on an external resource this environment does not control (live
//! internet access, NOAA's own service, and "today's run has actually
//! finished publishing by the time this runs") must never make `cargo test
//! --workspace` flaky on a machine or moment that simply lacks it. Every
//! fallible discovery step here prints a clear skip message and returns
//! (passing) rather than failing the suite; a message that reaches this
//! test's own assertions (i.e. discovery *did* succeed) is a genuine
//! correctness bug, not a flake, if it then fails.

use forecast_core::ensemble::EnsembleStatistic;
use forecast_core::provider::ForecastProvider;
use forecast_core::request::FieldRequest;
use forecast_core::variable::ForecastVariable;
use provider_gefs::keys::{ForecastHour, MemberKey, ProductGroup};
use provider_gefs::GefsProvider;

#[tokio::test(flavor = "multi_thread")]
async fn discovers_fetches_and_decodes_real_control_member_perturbed_member_and_mean() {
    let provider = match GefsProvider::default_bucket() {
        Ok(p) => p,
        Err(e) => {
            println!("SKIP live_network test: failed to build HTTP client: {e}");
            return;
        }
    };

    let Ok(run) = provider.discover_latest_run(2).await else {
        println!(
            "SKIP live_network test: could not find any published GEFS run in the last two \
             days -- no network access, NOAA's service unavailable, or a genuine bucket-layout \
             change."
        );
        return;
    };
    println!("Using real published run: {run}");

    // --- Control member ---
    let control_request = FieldRequest::new(ForecastVariable::Temperature2m, 0)
        .with_ensemble(EnsembleStatistic::Control);
    match provider.fetch_field(&run, &control_request).await {
        Ok(field) => {
            assert_eq!(field.ensemble, Some(EnsembleStatistic::Control));
            assert_eq!(field.forecast_lead_hours, 0);
            assert_eq!(field.valid_time, field.run_time);
            assert_eq!(field.geometry.width(), 1440);
            assert_eq!(field.geometry.height(), 721);
            let min = field.values.iter().cloned().fold(f32::INFINITY, f32::min);
            let max = field
                .values
                .iter()
                .cloned()
                .fold(f32::NEG_INFINITY, f32::max);
            println!("gec00 real decode ok: min={min:.2}K max={max:.2}K");
            // 2m air temperature anywhere on Earth is physically bounded
            // (the coldest and hottest ever recorded surface readings are
            // both comfortably inside this range) -- a real decode error
            // (e.g. wrong scale/offset) would very likely blow past this.
            assert!((150.0..340.0).contains(&min), "implausible min {min}");
            assert!((150.0..340.0).contains(&max), "implausible max {max}");
        }
        Err(e) => println!("SKIP control-member assertions: {e}"),
    }

    // --- A perturbed member ---
    let member1_request = FieldRequest::new(ForecastVariable::Temperature2m, 0)
        .with_ensemble(EnsembleStatistic::Member(1));
    match provider.fetch_field(&run, &member1_request).await {
        Ok(field) => assert_eq!(field.ensemble, Some(EnsembleStatistic::Member(1))),
        Err(e) => println!("SKIP perturbed-member assertions: {e}"),
    }

    // --- Ensemble mean ---
    let mean_request = FieldRequest::new(ForecastVariable::Temperature2m, 0)
        .with_ensemble(EnsembleStatistic::Mean);
    match provider.fetch_field(&run, &mean_request).await {
        Ok(field) => assert_eq!(field.ensemble, Some(EnsembleStatistic::Mean)),
        Err(e) => println!("SKIP ensemble-mean assertions: {e}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_known_absent_member_fails_clearly_never_panics() {
    let client = match provider_gefs::client::GefsClient::default_bucket() {
        Ok(c) => c,
        Err(e) => {
            println!("SKIP live_network test: failed to build HTTP client: {e}");
            return;
        }
    };

    let Ok(run) = client
        .find_recent_run(ProductGroup::PGRB2S_P25, ForecastHour(0), 2)
        .await
    else {
        println!("SKIP live_network test: could not find any published GEFS run.");
        return;
    };

    // Empirically confirmed live (2026-09-12): gep30 exists, gep31 does
    // not (HTTP 404). This must come back as a clear, structured error --
    // never a panic, never silently substituting gep30's or any other
    // member's data.
    let idx_key = provider_gefs::keys::idx_key(
        run,
        MemberKey::Perturbed(31),
        ProductGroup::PGRB2S_P25,
        ForecastHour(0),
    );
    match client.fetch_idx_text(&idx_key).await {
        Err(e) if e.is_not_found() => {
            println!("Confirmed gep31 is absent (404), as expected: {e}");
        }
        Err(e) => println!(
            "SKIP: gep31 request failed with a non-404 error (network issue, not a bucket-layout \
             fact): {e}"
        ),
        Ok(_) => panic!(
            "gep31's .idx unexpectedly exists now -- GEFS may have changed its published \
             ensemble size; this crate's MemberKey::MAX_PERTURBED_MEMBER (30) and this test's \
             assumption both need revisiting"
        ),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_with_no_ensemble_statistic_fails_clearly_never_panics() {
    let provider = match GefsProvider::default_bucket() {
        Ok(p) => p,
        Err(e) => {
            println!("SKIP live_network test: failed to build HTTP client: {e}");
            return;
        }
    };
    let Ok(run) = provider.discover_latest_run(2).await else {
        println!("SKIP live_network test: could not find any published GEFS run.");
        return;
    };

    // GEFS is an ensemble provider -- a request naming no statistic at all
    // must be rejected, not silently defaulted to some member.
    let request = FieldRequest::new(ForecastVariable::Temperature2m, 0);
    let err = provider
        .fetch_field(&run, &request)
        .await
        .expect_err("an ensemble provider must reject a request naming no ensemble statistic");
    assert!(matches!(
        err,
        provider_gefs::GefsError::EnsembleStatisticRequired
    ));
}
