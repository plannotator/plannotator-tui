//! plannotator-tui: annotate Markdown in the terminal.

mod app;
mod archive;
mod base64;
mod cli;
mod config;
mod delivery;
mod doc;
mod editor;
mod export;
mod file_search;
mod gate;
mod herdr;
mod keys;
mod last;
mod layout;
mod rex;
mod srcmap;
mod store;
mod terminal;
mod theme;
mod tree;
mod workspace_paths;
mod wrap;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::run(&args) {
        Ok(code) => code,
        Err(err) => {
            #[allow(clippy::print_stderr, reason = "the one place errors reach the user")]
            {
                eprintln!("plannotator_tui: {err:#}");
            }
            // A strict gate's own failure must not read as the reviewer's rejection.
            if err.is::<gate::GateError>() {
                std::process::ExitCode::from(gate::GATE_ERROR_EXIT)
            } else {
                std::process::ExitCode::FAILURE
            }
        }
    }
}
