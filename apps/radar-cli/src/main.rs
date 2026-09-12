//! `radar-cli` — RadarPro command-line workstation utility.
//!
//! # Status: S00 placeholder
//!
//! This binary is a minimal, real (non-panicking) CLI skeleton created
//! during the S00 "foundation" stage. It parses arguments and supports
//! `--help`/`--version`, but does not yet implement the `inspect <file>`
//! subcommand that will decode a NEXRAD Level II file via
//! `nexrad-level2` and print its structure — that arrives in stage S01.

use clap::{Parser, Subcommand};

/// RadarPro command-line workstation utility.
#[derive(Debug, Parser)]
#[command(name = "radar-cli", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect a NEXRAD Level II file and print its structure.
    ///
    /// TODO(S01): not yet implemented. Decoding Archive II / Message 31
    /// data is out of scope for S00; see `nexrad-level2` for the current
    /// placeholder decoder.
    Inspect {
        /// Path to a NEXRAD Level II (Archive II) file.
        file: String,
    },
}

fn main() {
    let cli = Cli::parse();

    // A placeholder use of `radar-types` so the workspace dependency is
    // exercised; the real domain model arrives in S01.
    let _site_placeholder = radar_types::Site::new("UNSET", 0.0, 0.0);

    match cli.command {
        Some(Command::Inspect { file }) => {
            println!(
                "radar-cli: `inspect` is not implemented yet (S01 scope). Requested file: {file}"
            );
        }
        None => {
            println!("radar-cli: no subcommand given. Run with --help for usage.");
        }
    }
}
