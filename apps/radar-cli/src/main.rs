//! `radar-cli` — RadarPro command-line workstation utility.
//!
//! Implements `inspect <file>`: decode a NEXRAD Level II (Archive II) file
//! via `nexrad-level2` and print a concise summary of its structure for
//! manual validation. Never panics on a bad file — I/O errors and decode
//! errors are both reported as a clear message on stderr with a non-zero
//! exit code.

use clap::{Parser, Subcommand};
use radar_types::{GateValue, MomentKind, Radial, Sweep, Volume};
use std::path::PathBuf;
use std::process::ExitCode;

/// RadarPro command-line workstation utility.
#[derive(Debug, Parser)]
#[command(name = "radar-cli", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Decode a NEXRAD Level II (Archive II) file and print its structure.
    Inspect {
        /// Path to a NEXRAD Level II (Archive II) file.
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Inspect { file }) => run_inspect(&file),
        None => {
            println!("radar-cli: no subcommand given. Run with --help for usage.");
            ExitCode::SUCCESS
        }
    }
}

fn run_inspect(path: &PathBuf) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("radar-cli: failed to read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    let volume = match nexrad_level2::decode_volume(&bytes) {
        Ok(volume) => volume,
        Err(e) => {
            eprintln!("radar-cli: failed to decode {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    print_volume_summary(&volume);
    ExitCode::SUCCESS
}

fn print_volume_summary(volume: &Volume) {
    println!(
        "Site: {} ({:.4}, {:.4}), height {:.0} m MSL",
        volume.site.icao, volume.site.latitude_deg, volume.site.longitude_deg, volume.site.height_m
    );
    println!("Volume start: {}", volume.start_time);
    println!("VCP: {}", volume.volume_coverage_pattern);
    println!("Sweeps: {}", volume.sweeps.len());
    println!();

    for (index, sweep) in volume.sweeps.iter().enumerate() {
        print_sweep_summary(index, sweep);
    }

    if let Some(first_sweep) = volume.sweeps.first() {
        if let Some(first_radial) = first_sweep.radials.first() {
            print_sample_gates(first_radial);
        }
    }
}

fn print_sweep_summary(index: usize, sweep: &Sweep) {
    let moments = present_moments(sweep);
    let moment_names: Vec<&str> = moments.iter().map(|m| moment_short_name(*m)).collect();
    println!(
        "  Sweep {index}: elevation {:.2} deg, {} radials, moments: [{}]",
        sweep.elevation_angle_deg,
        sweep.radials.len(),
        moment_names.join(", ")
    );
}

/// The union of moment kinds present on any radial in this sweep, in
/// canonical (REF, VEL, SW, ZDR, CC, PHI) order.
fn present_moments(sweep: &Sweep) -> Vec<MomentKind> {
    let mut present = std::collections::BTreeSet::new();
    for radial in &sweep.radials {
        for kind in radial.moments.keys() {
            present.insert(*kind);
        }
    }
    present.into_iter().collect()
}

fn moment_short_name(kind: MomentKind) -> &'static str {
    match kind {
        MomentKind::Reflectivity => "REF",
        MomentKind::Velocity => "VEL",
        MomentKind::SpectrumWidth => "SW",
        MomentKind::DifferentialReflectivity => "ZDR",
        MomentKind::CorrelationCoefficient => "CC",
        MomentKind::DifferentialPhase => "PHI",
    }
}

/// Print the first few REF gates of `radial`, if present, to demonstrate
/// that Missing/RangeFolded/Value states are decoded and preserved
/// distinctly.
fn print_sample_gates(radial: &Radial) {
    let Some(moment) = radial.moments.get(&MomentKind::Reflectivity) else {
        println!("(first radial has no REF moment to sample)");
        return;
    };

    let sample_count = moment.gates.len().min(10);
    println!(
        "Sample REF gates (radial az={:.2} deg, first radial of first sweep, first {sample_count} of {} gates, first gate range {:.3} km, spacing {:.3} km):",
        radial.azimuth_angle_deg,
        moment.gates.len(),
        moment.first_gate_range_km,
        moment.gate_spacing_km,
    );
    for (i, gate) in moment.gates.iter().take(sample_count).enumerate() {
        let description = match gate {
            GateValue::Missing => "Missing".to_string(),
            GateValue::RangeFolded => "RangeFolded".to_string(),
            GateValue::Value(v) => format!("Value({v:.2} dBZ)"),
        };
        println!("  gate {i}: {description}");
    }
}
