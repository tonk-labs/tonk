//! Binary entry point. Everything of substance is in the library, so
//! it can be tested without shelling out to this.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    match tonk_tui_poc::cli::run() {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("tonk-tui-poc: {error}");
            ExitCode::FAILURE
        }
    }
}
