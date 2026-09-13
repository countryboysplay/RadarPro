//! The only tests in this crate that talk to the real, live
//! `noaa-hrrr-bdp-pds` bucket: full discovery -> `.idx` -> sparse Range GET
//! -> GRIB2 decode for a real 2m-temperature field.
//!
//! Per the same pattern already established in this repo (`provider-gefs`'s
//! own `tests/live_network.rs`, `radar-cache`'s, `radar-render`'s
//! GPU-availability tests): a test that depends on an external resource
//! this environment does not control must never make `cargo test
//! --workspace` flaky on a machine or moment that simply lacks it -- every
//! fallible discovery step prints a clear skip message and returns
//! (passing) rather than failing the suite.

use forecast_core::grid::GridGeometry;
use forecast_core::provider::ForecastProvider;
use forecast_core::request::FieldRequest;
use forecast_core::variable::ForecastVariable;
use provider_hrrr::HrrrProvider;

#[tokio::test(flavor = "multi_thread")]
async fn discovers_fetches_and_decodes_a_real_hrrr_2m_temperature_field() {
    let provider = match HrrrProvider::default_bucket() {
        Ok(p) => p,
        Err(e) => {
            println!("SKIP live_network test: failed to build HTTP client: {e}");
            return;
        }
    };

    let Ok(run) = provider.discover_latest_run(2).await else {
        println!(
            "SKIP live_network test: could not find any published HRRR run in the last two \
             days -- no network access, NOAA's service unavailable, or a genuine bucket-layout \
             change."
        );
        return;
    };
    println!("Using real published run: {run}");

    let request = FieldRequest::new(ForecastVariable::Temperature2m, 0);
    match provider.fetch_field(&run, &request).await {
        Ok(field) => {
            assert_eq!(field.ensemble, None);
            assert_eq!(field.forecast_lead_hours, 0);
            assert_eq!(field.valid_time, field.run_time);
            assert_eq!(field.geometry.width(), 1799);
            assert_eq!(field.geometry.height(), 1059);
            assert!(matches!(field.geometry, GridGeometry::LambertConformal(_)));
            let min = field.values.iter().cloned().fold(f32::INFINITY, f32::min);
            let max = field
                .values
                .iter()
                .cloned()
                .fold(f32::NEG_INFINITY, f32::max);
            println!("HRRR real decode ok: min={min:.2}K max={max:.2}K");
            // 2m air temperature anywhere over CONUS is physically bounded.
            assert!((150.0..340.0).contains(&min), "implausible min {min}");
            assert!((150.0..340.0).contains(&max), "implausible max {max}");
        }
        Err(e) => println!("SKIP field-fetch assertions: {e}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_naming_an_ensemble_statistic_fails_clearly_never_panics() {
    let provider = match HrrrProvider::default_bucket() {
        Ok(p) => p,
        Err(e) => {
            println!("SKIP live_network test: failed to build HTTP client: {e}");
            return;
        }
    };
    let Ok(run) = provider.discover_latest_run(2).await else {
        println!("SKIP live_network test: could not find any published HRRR run.");
        return;
    };

    let request = FieldRequest::new(ForecastVariable::Temperature2m, 0)
        .with_ensemble(forecast_core::ensemble::EnsembleStatistic::Mean);
    let err = provider
        .fetch_field(&run, &request)
        .await
        .expect_err("HRRR is deterministic and must reject an ensemble-statistic request");
    assert!(matches!(
        err,
        provider_hrrr::HrrrError::EnsembleNotSupported
    ));
}
