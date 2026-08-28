//! plannotui: annotate Markdown in the terminal.

mod app;
mod cli;
mod doc;
mod layout;
mod srcmap;
mod store;
mod wrap;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::run(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            #[allow(clippy::print_stderr, reason = "the one place errors reach the user")]
            {
                eprintln!("plannotui: {err:#}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}
