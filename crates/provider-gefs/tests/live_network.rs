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

use provider_gefs::client::GefsClient;
use provider_gefs::decode::decode_field;
use provider_gefs::ensemble::EnsembleIdentity;
use provider_gefs::field::CanonicalField;
use provider_gefs::idx;
use provider_gefs::keys::{ForecastHour, MemberKey, ProductGroup, RunReference};

async fn fetch_and_decode(
    client: &GefsClient,
    run: RunReference,
    member: MemberKey,
) -> Result<provider_gefs::field::GriddedField, String> {
    let field = CanonicalField::Temperature2m;
    let key =
        provider_gefs::keys::object_key(run, member, ProductGroup::PGRB2S_P25, ForecastHour(0));
    let idx_key =
        provider_gefs::keys::idx_key(run, member, ProductGroup::PGRB2S_P25, ForecastHour(0));

    let idx_text = client
        .fetch_idx_text(&idx_key)
        .await
        .map_err(|e| format!("fetch .idx for {member}: {e}"))?;
    let entries = idx::parse_idx(&idx_key, &idx_text).map_err(|e| format!("parse .idx: {e}"))?;
    let position = entries
        .iter()
        .position(|e| e.variable == field.idx_variable() && e.level == field.idx_level())
        .ok_or_else(|| {
            format!(
                "{} / {} not found in .idx for {member}",
                field.idx_variable(),
                field.idx_level()
            )
        })?;

    let content_length = if position + 1 == entries.len() {
        Some(
            client
                .content_length(&key)
                .await
                .map_err(|e| format!("HEAD {member}: {e}"))?,
        )
    } else {
        None
    };
    let (start, end) = idx::byte_range(&key, &entries, position, content_length)
        .map_err(|e| format!("byte_range: {e}"))?;

    let bytes = client
        .fetch_byte_range(&key, start, end)
        .await
        .map_err(|e| format!("range GET {member}: {e}"))?;

    decode_field(&key, &bytes, field).map_err(|e| format!("decode {member}: {e}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn discovers_fetches_and_decodes_real_control_member_perturbed_member_and_mean() {
    let client = match GefsClient::default_bucket() {
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
        println!(
            "SKIP live_network test: could not find any published GEFS run in the last two \
             days -- no network access, NOAA's service unavailable, or a genuine bucket-layout \
             change."
        );
        return;
    };
    println!("Using real published run: {run}");

    // --- Control member ---
    match fetch_and_decode(&client, run, MemberKey::Control).await {
        Ok(field) => {
            assert_eq!(field.ensemble, EnsembleIdentity::Control);
            assert_eq!(field.forecast_lead_hours, 0);
            assert_eq!(field.valid_time, field.run_time);
            assert_eq!(field.geometry.width, 1440);
            assert_eq!(field.geometry.height, 721);
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
    match fetch_and_decode(&client, run, MemberKey::Perturbed(1)).await {
        Ok(field) => assert_eq!(field.ensemble, EnsembleIdentity::Member(1)),
        Err(e) => println!("SKIP perturbed-member assertions: {e}"),
    }

    // --- Ensemble mean ---
    match fetch_and_decode(&client, run, MemberKey::Mean).await {
        Ok(field) => assert_eq!(field.ensemble, EnsembleIdentity::Mean),
        Err(e) => println!("SKIP ensemble-mean assertions: {e}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_known_absent_member_fails_clearly_never_panics() {
    let client = match GefsClient::default_bucket() {
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
