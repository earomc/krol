//! `cargo xtask` — krol development automation.
//!
//! ```text
//! cargo xtask parity run      [--filter <substr>]   run scenarios against krol, diff vs golden
//!                             [--protocol 5] [--jobs N] (4 scenarios at once by default)
//! cargo xtask parity record   [--filter <substr>]   record vanilla observations (needs KROL_VANILLA_JAR)
//! cargo xtask parity diff     <scenario>            show the diff for one scenario from the last report
//! cargo xtask parity coverage                       feature coverage from parity/coverage.toml
//! ```

mod parity;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("parity") => parity::main(&args[1..]),
        _ => {
            eprintln!("usage: cargo xtask parity <run|record|diff|coverage> [options]");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
