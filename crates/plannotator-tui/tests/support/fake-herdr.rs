//! Standalone argv recorder compiled by tests with `rustc`. `pane read` prints the
//! `pane-read.txt` fixture, or fails with `pane-read.error` on stderr when that exists.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn fixture(root: &Path, name: &str, fallback: &str) -> String {
    std::fs::read_to_string(root.join(name)).unwrap_or_else(|_| fallback.to_owned())
}

fn main() {
    let executable = std::env::current_exe().unwrap_or_default();
    let root = executable.parent().unwrap_or_else(|| Path::new("."));
    let program = executable
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json_args = args
        .iter()
        .map(|arg| json_string(arg))
        .collect::<Vec<_>>()
        .join(",");
    let line = format!(
        "{{\"program\":{},\"argv\":[{}]}}\n",
        json_string(program),
        json_args
    );
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("calls.jsonl"))
        .expect("open call log");
    log.write_all(line.as_bytes()).expect("write call log");

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let command_args = match arg_refs.as_slice() {
        ["--session", _, rest @ ..] => rest,
        rest => rest,
    };

    match command_args {
        ["agent", "get", _] => print!(
            "{}",
            fixture(
                root,
                "agent-get.json",
                r#"{"result":{"agent":{"agent":"codex","agent_session":{"kind":"id","value":"11111111-1111-4111-8111-111111111111"}}}}"#,
            )
        ),
        ["pane", "process-info", ..] => print!(
            "{}",
            fixture(
                root,
                "process-info.json",
                r#"{"result":{"process_info":{"foreground_process_group_id":42,"foreground_processes":[{"name":"codex","pid":42}]}}}"#,
            )
        ),
        ["pane", "read", ..] => match std::fs::read_to_string(root.join("pane-read.error")) {
            Ok(error) => {
                eprint!("{error}");
                std::process::exit(1);
            }
            Err(_) => print!("{}", fixture(root, "pane-read.txt", "")),
        },
        _ => print!(r#"{{"result":{{}}}}"#),
    }
}
