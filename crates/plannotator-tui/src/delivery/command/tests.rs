#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use super::*;

fn command(line: &str) -> DeliverCommand {
    DeliverCommand::from_lookup(|_| None, Some(line), None).expect("a command")
}

#[test]
fn the_environment_wins_over_the_config_and_a_blank_command_is_none() {
    let env = |line: &'static str| move |k: &str| (k == COMMAND_VAR).then(|| line.to_owned());
    let from_env = DeliverCommand::from_lookup(env("tmux-send"), Some("other"), None).expect("a command");
    assert_eq!(from_env.line, "tmux-send");
    let from_config = DeliverCommand::from_lookup(env("  "), Some("other"), None).expect("a command");
    assert_eq!(from_config.line, "other");
    assert!(DeliverCommand::from_lookup(|_| None, Some(""), None).is_none());
    assert!(DeliverCommand::from_lookup(|_| None, None, None).is_none());
}

#[test]
fn the_target_is_named_for_the_program_the_line_runs() {
    assert_eq!(command("tmux-send %1").describe(), "cmd · tmux-send");
    assert_eq!(command("/opt/bin/plannotator-tui pi send --session a").describe(), "cmd · plannotator-tui");
    assert_eq!(command("PANE=%1 \"C:\\tools\\send.exe\" -q").describe(), "cmd · send.exe");
}

#[test]
fn the_exit_status_is_the_outcome_and_a_failure_keeps_what_the_command_said() {
    assert_eq!(
        outcome("s", Some(0), "\nqueued: pi is busy\nmore\n", "").expect("sent"),
        Some("queued: pi is busy".to_owned())
    );
    assert_eq!(outcome("s", Some(0), "", "warning\n").expect("sent"), None);
    let blocked = outcome("s", Some(EXIT_BLOCKED), "", "at a permission prompt\n").expect_err("blocked");
    assert!(matches!(&blocked, DeliveryError::Blocked(m) if m == "at a permission prompt"), "{blocked}");
    let gone = outcome("s", Some(EXIT_UNAVAILABLE), "", "").expect_err("unavailable");
    assert!(matches!(&gone, DeliveryError::Unavailable(m) if m == "s found no agent"), "{gone}");
    let failed = outcome("s", Some(2), "", "s: no pane %9\nusage: s PANE\n").expect_err("failed");
    assert!(matches!(&failed, DeliveryError::Failed(_)), "{failed}");
    assert_eq!(failed.to_string(), "s exited with 2: s: no pane %9");
    assert_eq!(outcome("s", None, "", "").expect_err("killed").to_string(), "s was stopped by a signal");
}

/// `exit N` means the same to `sh` and to `cmd`, so this runs the real shell on every platform.
#[test]
fn the_shell_runs_the_line_and_its_exit_code_reaches_the_review() {
    command("exit 0").deliver("notes").expect("delivered");
    assert!(matches!(command("exit 75").deliver("notes"), Err(DeliveryError::Blocked(_))));
    assert!(matches!(command("exit 69").deliver("notes"), Err(DeliveryError::Unavailable(_))));
    let failed = command("exit 3").deliver("notes").expect_err("failed");
    assert_eq!(failed.to_string(), "exit exited with 3");
}

#[cfg(unix)]
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator-tui-deliver-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[cfg(unix)]
#[test]
fn a_pipeline_gets_the_feedback_on_stdin_and_the_reviewed_path() {
    let dir = scratch("pipe");
    let out = dir.join("got");
    let line = format!(
        "tr a-z A-Z > '{out}' && printf '%s\\n' \"$PLANNOTATOR_TUI_REVIEW_PATH\" >> '{out}' && echo queued for pi",
        out = out.display()
    );
    let plan = dir.join("plan.md");
    let target = DeliverCommand::from_lookup(|_| None, Some(&line), Some(&plan)).expect("a command");
    target.deliver("# annotations\n").expect("delivered");
    let got = std::fs::read_to_string(&out).expect("written");
    assert_eq!(got, format!("# ANNOTATIONS\n{}\n", plan.display()));
    assert_eq!(target.notice().as_deref(), Some("queued for pi"));
    assert_eq!(target.notice(), None, "a notice is shown once");
    std::fs::remove_dir_all(dir).ok();
}

#[cfg(unix)]
#[test]
fn a_blocked_or_missing_agent_is_reported_in_the_commands_words() {
    let blocked = command("cat > /dev/null; echo 'claude is at a permission prompt' >&2; exit 75");
    let err = blocked.deliver("notes").expect_err("blocked");
    assert!(matches!(&err, DeliveryError::Blocked(m) if m == "claude is at a permission prompt"), "{err}");
    let missing = command("echo 'no pane %4' >&2; exit 69").deliver("notes").expect_err("unavailable");
    assert!(matches!(&missing, DeliveryError::Unavailable(m) if m == "no pane %4"), "{missing}");
    let unknown = command("no-such-program-for-plannotator-tui").deliver("notes").expect_err("failed");
    let message = unknown.to_string();
    assert!(message.starts_with("no-such-program-for-plannotator-tui exited with 127: "), "{message}");
}

#[cfg(unix)]
#[test]
fn the_review_never_hangs_on_a_command() {
    // Never reads a feedback larger than a pipe holds.
    command("exit 0").deliver(&"x".repeat(1 << 20)).expect("delivered");
    // Leaves a child behind that holds stdout open.
    let started = Instant::now();
    let background = command("sleep 5 & echo sent");
    background.deliver("notes").expect("delivered");
    assert_eq!(background.notice().as_deref(), Some("sent"));
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    // Never finishes.
    let mut stuck = command("sleep 5");
    stuck.timeout = Duration::from_millis(200);
    let err = stuck.deliver("notes").expect_err("stopped");
    assert!(matches!(&err, DeliveryError::Failed(_)), "{err}");
    assert!(err.to_string().starts_with("sleep did not finish within 0.2 s"), "{err}");
}
