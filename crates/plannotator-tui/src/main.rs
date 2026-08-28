//! plannotator-tui: annotate Markdown in the terminal.

mod app;
mod base64;
mod cli;
mod config;
mod delivery;
mod doc;
mod export;
mod herdr;
mod last;
mod layout;
mod srcmap;
mod store;
mod tree;
mod workspace_paths;
mod wrap;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::run(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            #[allow(clippy::print_stderr, reason = "the one place errors reach the user")]
            {
                eprintln!("plannotator_tui: {err:#}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}
