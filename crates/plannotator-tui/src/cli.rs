//! Command-line entry points: the interactive viewer and the headless tools.
//!
//! This is the only module that prints to stdout.

#![allow(clippy::print_stdout, reason = "the CLI's job is to print")]

use std::io::{IsTerminal as _, Write as _, stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use plannotator_tui_schema::{DocumentSource, Kind};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;

use crate::app::App;
use crate::config::Config;
use crate::delivery::{Caller, Clipboard, Delivery, Discard, HerdrAgent};
use crate::doc::Document;
use crate::gate::{self, Decision, GateError, ReviewArgs};
use crate::herdr::context::HerdrEnv;
use crate::layout::DocLayout;
use crate::rex::agent::RexAgent;

pub(crate) const USAGE: &str = "usage:
  plannotator-tui <file.md | folder> [--gate] [--json] [--require-approval] [--result-file <path>]
  plannotator-tui --export <file.md>
  plannotator-tui --bench <file.md>
  plannotator-tui --blocks <file.md>
  plannotator-tui --annotate <file.md> <quote> <text> [comment|looks_good|delete] [--occurrence N]
  plannotator-tui --annotate-block <file.md> <block> <text>
  plannotator-tui --snapshot <file.md> [cols rows scroll] [select-quote] [menu]
  plannotator-tui config
  plannotator-tui --version
  plannotator-tui herdr open [file.md | folder] [--placement overlay|split|popup] [--deliver-to <pane>]
                             [--gate] [--result-file <path>]
  plannotator-tui herdr last [--placement P] [--deliver-to <pane>] [--newest]
  plannotator-tui herdr terminal [--lines N] [--placement P] [--deliver-to <pane>] [--print]
  plannotator-tui herdr pane
  plannotator-tui rex open [file.md | folder] [--block <id>] [--session <id>] [--paste]
                           [--gate] [--result-file <path>]
  plannotator-tui rex last [--block <id>] [--session <id>] [--newest] [--paste]
  plannotator-tui rex terminal [--block <id>] [--session <id>] [--lines N] [--paste] [--print]
  plannotator-tui rex find [--block <id>] [--session <id>] [--lines N] [--paste]
  plannotator-tui rex pane (open <path> [--gate [--result-file <path>]] | last <last args>
                            | terminal --block <id> [--lines N])
  plannotator-tui last [--host claude|codex|pi|omp|copilot|droid|hermes|opencode|cmd|agy] [--pid N] [--session <transcript>]
                       [--session-id <id>] [--stdin] [--print] [--pick N] [--newest]";

/// Width the document gets when nothing else is known: gutter + rail + gap subtracted.
fn doc_width(cols: u16) -> usize {
    usize::from(cols.saturating_sub(2 + 1 + 36).max(20))
}

fn open_file(path: &PathBuf) -> Result<DocumentSource> {
    let content = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(DocumentSource::file(path.clone(), content))
}

/// A file opens on its own; a folder opens in folder mode on its first markdown file.
/// Interactive runs send to the Rex block or Herdr agent pane named by the environment, else the
/// clipboard; headless runs send nowhere.
pub(crate) fn delivery(interactive: bool) -> Box<dyn Delivery> {
    if !interactive {
        return Box::new(Discard);
    }
    agent_pane().unwrap_or_else(|| Box::new(Clipboard))
}

/// A gated review sends to the same agent pane; without one its decision goes to the caller.
fn gate_delivery() -> Box<dyn Delivery> {
    agent_pane().unwrap_or_else(|| Box::new(Caller))
}

/// The Rex block or Herdr agent pane the environment names, if any.
fn agent_pane() -> Option<Box<dyn Delivery>> {
    if let Some(agent) = RexAgent::from_lookup(|k| std::env::var(k).ok()) {
        return Some(Box::new(agent));
    }
    let env = HerdrEnv::from_env();
    match env.delivery_target() {
        Some(target) if env.in_herdr => {
            let agent = target.agent.or_else(|| env.agent_in_pane(&target.pane));
            Some(Box::new(HerdrAgent::new(env.bin, target.pane, agent)))
        }
        _ => None,
    }
}

fn open_app(path: &PathBuf, width: usize, interactive: bool) -> Result<App> {
    open_review(path, width, delivery(interactive))
}

fn open_review(path: &PathBuf, width: usize, delivery: Box<dyn Delivery>) -> Result<App> {
    if path.is_dir() {
        App::open_folder(path, width, delivery)
    } else {
        App::open(open_file(path)?, width, delivery)
    }
}

fn parse_kind(s: Option<&str>) -> Kind {
    match s {
        Some("looks_good" | "approve") => Kind::LooksGood,
        Some("delete") => Kind::Delete,
        _ => Kind::Comment,
    }
}

/// What follows `--annotate <file> <quote> <text>`: an optional kind and `--occurrence N`
/// (1-based; the first occurrence when absent), in either order.
fn annotate_options(rest: &[String]) -> Result<(Kind, usize)> {
    let mut kind = None;
    let mut occurrence = 1;
    let mut items = rest.iter().map(String::as_str);
    while let Some(item) = items.next() {
        match item {
            "--occurrence" => {
                occurrence = items
                    .next()
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|n| *n >= 1)
                    .context("--occurrence takes a number from 1")?;
            }
            flag if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}\n{USAGE}"),
            other if kind.is_none() => kind = Some(parse_kind(Some(other))),
            other => anyhow::bail!("unexpected argument {other:?}\n{USAGE}"),
        }
    }
    Ok((kind.unwrap_or(Kind::Comment), occurrence))
}

pub(crate) fn run(args: &[String]) -> Result<ExitCode> {
    if wants_decision(args) {
        return review_command(args);
    }
    tool(args).map(|()| ExitCode::SUCCESS)
}

/// A review with a decision flag, which may come before the path. Without one the
/// invocation means what it always has.
fn wants_decision(args: &[String]) -> bool {
    let decision_flag = |arg: &String| gate::FLAGS.contains(&arg.as_str());
    match args.first() {
        Some(first) if decision_flag(first) => true,
        Some(first)
            if first.starts_with('-') || matches!(first.as_str(), "config" | "herdr" | "rex" | "last") =>
        {
            false
        }
        Some(_) => args.iter().skip(1).any(decision_flag),
        None => false,
    }
}

fn tool(args: &[String]) -> Result<()> {
    let arg = |i: usize| args.get(i).map(String::as_str);
    let path = |i: usize| arg(i).map(|p| crate::workspace_paths::absolute(Path::new(p))).context(USAGE);
    match arg(0) {
        Some("--bench") => bench(&path(1)?),
        Some("--export") => {
            let target = path(1)?;
            let app = open_app(&target, 100, false)?;
            print!("{}", if target.is_dir() { app.folder_feedback()? } else { app.feedback() });
            Ok(())
        }
        Some("--blocks") => {
            let app = open_app(&path(1)?, 100, false)?;
            for line in app.describe_blocks() {
                println!("{line}");
            }
            Ok(())
        }
        Some("--annotate") => {
            let mut app = open_app(&path(1)?, 100, false)?;
            let quote = arg(2).context(USAGE)?;
            let body = arg(3).context(USAGE)?.to_owned();
            let (kind, occurrence) = annotate_options(args.get(4..).unwrap_or_default())?;
            app.add_quote_annotation_at(quote, occurrence, kind, body)
        }
        Some("--annotate-block") => {
            let mut app = open_app(&path(1)?, 100, false)?;
            let block: usize = arg(2).and_then(|s| s.parse().ok()).context(USAGE)?;
            let body = arg(3).context(USAGE)?.to_owned();
            app.add_block_annotation(block, Kind::Comment, body)
        }
        Some("--snapshot") => {
            let cols: u16 = arg(2).and_then(|s| s.parse().ok()).unwrap_or(140);
            let rows: u16 = arg(3).and_then(|s| s.parse().ok()).unwrap_or(40);
            let scroll: i64 = arg(4).and_then(|s| s.parse().ok()).unwrap_or(0);
            // A trailing `menu` opens the Review menu; a quote may come before it.
            let menu = args.get(5..).is_some_and(|rest| rest.last().is_some_and(|last| last == "menu"));
            let select = arg(5).filter(|quote| *quote != "menu");
            snapshot(&path(1)?, cols, rows, scroll, select, menu)
        }
        Some("--version" | "-V") => {
            println!("plannotator-tui {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("config") => show_config(),
        Some("herdr") => herdr_command(args.get(1..).unwrap_or_default()),
        Some("rex") => crate::rex::cli::run(args.get(1..).unwrap_or_default()),
        Some("last") => last_command(args.get(1..).unwrap_or_default()),
        Some(flag) if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}\n{USAGE}"),
        Some(_) => interactive(&path(0)?),
        None => anyhow::bail!(USAGE),
    }
}

/// `plannotator-tui config`: where the file is and what is in effect.
fn show_config() -> Result<()> {
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let path = crate::config::config_path(|k| std::env::var(k).ok(), &home);
    let mut config = Config::load_from(&path)?;
    // `PLANNOTATOR_TUI_THEME` overrides the file, so the file's value is not the effective one.
    config.ui.theme = crate::theme::effective_setting(|key| std::env::var(key).ok(), config.ui.theme)?;
    let state = if path.is_file() { "" } else { " (not present; defaults)" };
    println!("# {}{state}", path.display());
    print!("{}", config.to_toml()?);
    Ok(())
}

/// `plannotator-tui <file | folder> [--gate] [--json] [--require-approval] [--result-file PATH]`:
/// the review ends with one decision, printed on stdout and published to the result file
/// (decision 17). Under a strict flag every failure is a [`GateError`].
fn review_command(args: &[String]) -> Result<ExitCode> {
    let strict = gate::is_strict(args);
    gated_review(args).map_err(|err| if strict { GateError(err).into() } else { err })
}

fn gated_review(args: &[String]) -> Result<ExitCode> {
    let options = ReviewArgs::parse(args).map_err(|err| anyhow::anyhow!("{err:#}\n{USAGE}"))?;
    let path = crate::workspace_paths::absolute(&options.path);
    anyhow::ensure!(path.exists(), "{}: no such file or folder", path.display());
    let result_file = options.result_file.as_deref().map(crate::workspace_paths::absolute);
    if let Some(file) = &result_file {
        gate::check_result_file(file)?;
    }
    // The review draws on stdout. Redirected, nobody could see it and the gate would wait
    // forever; the result file is how a script captures the decision.
    anyhow::ensure!(
        stdout().is_terminal(),
        "stdout is not a terminal: the review is drawn there; read the decision from --result-file instead"
    );
    let mode = options.mode();
    let decision = run_review(|width| {
        let mut app = open_review(&path, width, gate_delivery())?;
        app.gate = mode;
        Ok(app)
    })?
    .unwrap_or(Decision::Dismissed);
    // Stdout first, as Plannotator does: if the file cannot be published, the decision has
    // still reached the caller once.
    let mut out = stdout().lock();
    if options.json {
        let line = if options.strict() { decision.strict_json() } else { decision.json(&path) };
        writeln!(out, "{line}")?;
    } else if let Some(text) = decision.plain(&gate::context_line(&path, path.is_dir())) {
        writeln!(out, "{text}")?;
    }
    out.flush()?;
    if let Some(file) = &result_file {
        gate::publish(file, &decision.strict_json())?;
    }
    Ok(ExitCode::from(decision.exit_code(options.require_approval)))
}

/// `plannotator-tui herdr open [PATH] [--placement P] [--deliver-to PANE] [--gate]
/// [--result-file PATH]`; `herdr last` takes `--newest` on top, which `open` has nothing to
/// skip. A gated open with a result file fails as a [`GateError`], like the direct gate.
fn herdr_command(args: &[String]) -> Result<()> {
    let strict = args.first().is_some_and(|sub| sub == "open") && gate::is_strict(args);
    herdr_launch(args).map_err(|err| if strict { GateError(err).into() } else { err })
}

fn herdr_launch(args: &[String]) -> Result<()> {
    use crate::herdr::launch::{OpenArgs, agent_get, agent_identity, plan, plan_last, process_info, run};
    let sub = args.first().map(String::as_str);
    if sub == Some("pane") {
        return herdr_pane();
    }
    if sub == Some("terminal") {
        return herdr_terminal(args.get(1..).unwrap_or_default());
    }
    if !matches!(sub, Some("open" | "last")) {
        anyhow::bail!(USAGE);
    }
    let mut open = OpenArgs::default();
    let mut rest = args.get(1..).unwrap_or_default().iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--placement" => {
                let value = rest.next().context("--placement needs a value")?;
                open.placement = Some(value.parse()?);
            }
            "--deliver-to" => {
                open.deliver_to = Some(rest.next().context("--deliver-to needs a value")?.clone());
            }
            "--newest" if sub == Some("last") => open.newest = true,
            "--newest" => {
                anyhow::bail!("--newest is only for `herdr last`; `herdr open` has no picker to skip")
            }
            "--gate" if sub == Some("open") => open.gate = true,
            "--result-file" if sub == Some("open") => {
                let value = rest.next().filter(|v| !v.starts_with("--"));
                let value = value.context("missing value for --result-file")?;
                open.result_file = Some(crate::workspace_paths::absolute(Path::new(value)));
            }
            "--gate" | "--result-file" => anyhow::bail!("{arg} is only for `herdr open`"),
            flag if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}\n{USAGE}"),
            path if open.path.is_none() => open.path = Some(PathBuf::from(path)),
            extra => anyhow::bail!("unexpected argument {extra:?}\n{USAGE}"),
        }
    }
    if let Some(file) = &open.result_file {
        anyhow::ensure!(open.gate, "--result-file requires --gate");
        // The pane publishes when the review ends; a name that is already taken would only
        // fail then, after the review.
        gate::check_result_file(file)?;
    }
    let env = HerdrEnv::from_env();
    let config = Config::load()?;
    let cwd = std::env::current_dir().context("current directory")?;
    let launch = if sub == Some("last") {
        let probe = plan(&env, &config, OpenArgs { path: None, ..open.clone() }, &cwd)?;
        let pane = probe.deliver.as_ref().map(|t| t.pane.clone()).or(probe.target_pane);
        let pane = pane.context("no agent pane to read: not focused on one and no --deliver-to")?;
        let agent = agent_get(&env, &pane);
        let process = match agent.as_deref().and_then(agent_identity) {
            Some(_) => None,
            None => Some(process_info(&env, &pane)?),
        };
        plan_last(&env, &config, open, &cwd, process.as_deref(), agent.as_deref())?
    } else {
        plan(&env, &config, open, &cwd)?
    };
    run(&env, &launch)
}

/// `plannotator-tui herdr terminal [--lines N] [--placement P] [--deliver-to PANE] [--print]`:
/// review the focused pane's recent output. The read happens here first so a pane with
/// nothing to show fails before a review pane opens; `--print` stops after it.
fn herdr_terminal(args: &[String]) -> Result<()> {
    use crate::herdr::launch::{OpenArgs, TerminalRead, plan_terminal, run};
    use crate::herdr::terminal;
    let mut open = OpenArgs::default();
    let mut lines = crate::terminal::DEFAULT_LINES;
    let mut print = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--lines" => lines = lines_value(rest.next())?,
            "--placement" => {
                let value = rest.next().context("--placement needs a value")?;
                open.placement = Some(value.parse()?);
            }
            "--deliver-to" => {
                open.deliver_to = Some(rest.next().context("--deliver-to needs a value")?.clone());
            }
            "--print" => print = true,
            other => anyhow::bail!("unexpected argument {other:?}\n{USAGE}"),
        }
    }
    let env = HerdrEnv::from_env();
    let pane = env.reviewed_pane().context("no focused pane to read")?;
    let text = terminal::read(&env, &pane, lines)?;
    if print {
        print!("{}", crate::terminal::document(&text));
        return Ok(());
    }
    let config = Config::load()?;
    let cwd = std::env::current_dir().context("current directory")?;
    let launch = plan_terminal(&env, &config, open, &cwd, TerminalRead { pane, lines })?;
    run(&env, &launch)
}

/// The pane entrypoint: Herdr runs this in the opened pane; the environment says what to show.
fn herdr_pane() -> Result<()> {
    let env = HerdrEnv::from_env();
    let result = if let Some(pane) = env.terminal_pane.clone() {
        crate::herdr::terminal::run(&env, &pane)
    } else if env.has_message_source() {
        crate::last::run(&crate::last::LastOptions {
            host: env.host.clone(),
            pid: env.message_pid,
            session: env.session.clone(),
            session_id: env.session_id.clone(),
            pick: 25,
            newest: env.newest,
            ..crate::last::LastOptions::default()
        })
    } else {
        let path = env.file.clone().unwrap_or(std::env::current_dir().context("current directory")?);
        if env.gate { gated_pane(&path, env.result_file.as_deref()) } else { interactive(&path) }
    };
    // The pane closes when we exit; an error that flashes by is an error nobody can read.
    if let Err(err) = &result {
        wait_on_error(err);
    }
    result
}

/// `--lines`: a count from 1.
pub(crate) fn lines_value(value: Option<&String>) -> Result<u32> {
    value.and_then(|n| n.parse::<u32>().ok()).filter(|n| *n >= 1).context("--lines takes a number from 1")
}

/// Show `err` in a pane that is about to close, until Enter.
pub(crate) fn wait_on_error(err: &anyhow::Error) {
    #[allow(clippy::print_stderr, reason = "the pane is the only place this can be read")]
    {
        eprintln!("plannotator-tui: {err:#}\n\npress enter to close");
    }
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

/// `plannotator-tui last [--host H] [--pid N] [--session PATH] [--stdin] [--print] [--pick N]
/// [--newest]`.
pub(crate) fn last_command(args: &[String]) -> Result<()> {
    use crate::last::LastOptions;
    let mut options = LastOptions { pick: 25, ..LastOptions::default() };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--host" => options.host = Some(rest.next().context("--host needs a value")?.clone()),
            "--pid" => options.pid = Some(rest.next().context("--pid needs a value")?.parse()?),
            "--session" => {
                options.session = Some(PathBuf::from(rest.next().context("--session needs a value")?));
            }
            "--session-id" => {
                options.session_id = Some(rest.next().context("--session-id needs a value")?.clone());
            }
            "--stdin" => options.stdin = true,
            "--print" => options.print = true,
            "--pick" => options.pick = rest.next().context("--pick needs a value")?.parse()?,
            "--newest" => options.newest = true,
            other => anyhow::bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    crate::last::run(&options)
}

/// A gated review in a Herdr or Rex pane: the agent hears the decision through delivery, and the
/// result file, when asked for, records it. The pane has no caller reading its stdout.
pub(crate) fn gated_pane(path: &PathBuf, result_file: Option<&Path>) -> Result<()> {
    if let Some(file) = result_file {
        gate::check_result_file(file)?;
    }
    let decision = run_review(|width| {
        let mut app = open_review(path, width, gate_delivery())?;
        app.gate = gate::Gate::Approve;
        Ok(app)
    })?
    .unwrap_or(Decision::Dismissed);
    match result_file {
        Some(file) => gate::publish(file, &decision.strict_json()),
        None => Ok(()),
    }
}

pub(crate) fn interactive(path: &PathBuf) -> Result<()> {
    run_ui(|width| open_app(path, width, true))
}

/// Own the terminal for one app: `build` gets the document width the screen allows.
pub(crate) fn run_ui(build: impl FnOnce(usize) -> Result<App>) -> Result<()> {
    run_review(build).map(drop)
}

/// `run_ui`, returning how a gated review ended; `None` for an ordinary one.
fn run_review(build: impl FnOnce(usize) -> Result<App>) -> Result<Option<Decision>> {
    // Settle the palette before the screen is ours: the background-colour query talks to
    // the terminal directly, and it must not race the alternate screen or the event loop.
    // A config that fails to parse never kept the plain TUI from starting; it still does
    // not. `plannotator-tui config` is where the error is reported.
    let config = Config::load().unwrap_or_default();
    crate::theme::install(
        crate::theme::resolve(|key| std::env::var(key).ok(), config.ui.theme, crate::theme::detect)?,
        &config.colors,
    );
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    // Shift+Enter is only distinguishable from Enter where the kitty keyboard protocol
    // holds end to end (terminal, and any multiplexer in between). Ask for it; the flag
    // gates the compose box's hint so it never advertises a key that cannot arrive.
    let shift_enter = ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    claim_input(shift_enter);
    let width = doc_width(terminal.size().map_or(120, |s| s.width));
    let result = build(width).and_then(|mut app| {
        app.shift_enter = shift_enter;
        app.keys = config.keys;
        app.ui = config.ui;
        app.editor = crate::editor::from_env(|key| std::env::var(key).ok());
        event_loop(&mut terminal, app)
    });
    release_input(shift_enter);
    ratatui::restore();
    result
}

/// Bracketed paste and, where the terminal has it, the kitty keyboard protocol. Mouse
/// capture is the caller's: the first claim turns it on before the keyboard query.
fn claim_input(shift_enter: bool) {
    let _ = execute!(stdout(), EnableBracketedPaste);
    if shift_enter {
        let _ = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
}

/// Undo `claim_input` and mouse capture, in reverse order.
fn release_input(shift_enter: bool) {
    if shift_enter {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(stdout(), DisableBracketedPaste);
    let _ = execute!(stdout(), DisableMouseCapture);
}

/// Give the terminal to the editor until it exits, then take the screen back. The outer
/// error is the terminal failing to come back, which ends the app; the inner one is the
/// editor failing to start, which the app reports and survives.
fn run_editor(
    terminal: &mut ratatui::DefaultTerminal,
    shift_enter: bool,
    request: &crate::editor::EditorRequest,
) -> Result<std::io::Result<std::process::ExitStatus>> {
    use ratatui::crossterm::{cursor, terminal as term};
    release_input(shift_enter);
    ratatui::restore();
    let _ = execute!(stdout(), cursor::Show);
    let outcome = match crate::editor::command(request) {
        Some(mut command) => command.status(),
        None => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the editor command is empty")),
    };
    term::enable_raw_mode().context("taking the terminal back from the editor")?;
    execute!(stdout(), term::EnterAlternateScreen, EnableMouseCapture)
        .context("taking the terminal back from the editor")?;
    claim_input(shift_enter);
    // The editor drew over the screen ratatui remembers; draw the next frame in full.
    terminal.clear()?;
    Ok(outcome)
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, mut app: App) -> Result<Option<Decision>> {
    app.clipboard = true;
    let mut dirty = true;
    while !app.quit {
        if let Some(request) = app.take_editor_request() {
            let outcome = run_editor(terminal, app.shift_enter, &request)?;
            app.editor_returned(&request, outcome);
            dirty = true;
        }
        dirty |= app.poll_file_search();
        dirty |= app.poll_disk();
        if dirty {
            let started = Instant::now();
            terminal.draw(|frame| app.draw(frame))?;
            app.record_frame(started.elapsed().as_secs_f64() * 1000.0);
            dirty = false;
        }
        let wait_ms = if app.file_search_scanning() { 25 } else { 250 };
        if event::poll(Duration::from_millis(wait_ms))? {
            app.handle_event(&event::read()?)?;
            dirty = true;
            // Coalesce bursts (wheel, drag) into one redraw.
            // Keys typed after `open-in-editor` belong to the editor, not to this loop.
            while !app.wants_editor() && event::poll(Duration::ZERO)? {
                app.handle_event(&event::read()?)?;
            }
        }
    }
    Ok(app.take_decision())
}

/// Headless timing of the expensive paths: parse, per-block render + align, and reflow.
fn bench(path: &PathBuf) -> Result<()> {
    let source = open_file(path)?;
    let bytes = source.content.len();
    let t = Instant::now();
    let doc = Document::parse(source.content);
    let parse_ms = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    let mut layout = DocLayout::build(&doc, 100);
    let build_ms = t.elapsed().as_secs_f64() * 1000.0;

    let reflow: Vec<String> = [60usize, 140, 80, 120]
        .into_iter()
        .map(|width| {
            let t = Instant::now();
            layout.reflow(width);
            format!("{width}→{:.1}ms", t.elapsed().as_secs_f64() * 1000.0)
        })
        .collect();

    let t = Instant::now();
    let hits = (0..layout.total_rows).step_by(7).filter(|&row| layout.block_at_row(row).is_some()).count();
    let lookup_us = t.elapsed().as_secs_f64() * 1e6 / (layout.total_rows / 7).max(1) as f64;

    let cells: usize = layout.blocks.iter().flat_map(|b| b.rows.iter()).map(|r| r.cells.len()).sum();
    let mapped: usize =
        layout.blocks.iter().flat_map(|b| b.rows.iter()).flat_map(|r| r.cells.iter()).flatten().count();

    println!("{}: {bytes} bytes, {} blocks", path.display(), doc.blocks.len());
    println!("parse+split          {parse_ms:8.1} ms");
    println!("render+align+wrap    {build_ms:8.1} ms  ({} rows)", layout.total_rows);
    println!("reflow               {}", reflow.join("  "));
    println!("row lookup           {lookup_us:8.3} µs avg ({hits} hits)");
    println!("cells mapped         {mapped}/{cells} ({:.1}%)", mapped as f64 * 100.0 / cells.max(1) as f64);
    Ok(())
}

/// Draw one frame into an in-memory backend and print it as text, followed by a mark map:
/// `#` comment, `+` looks good, `-` delete, `%` selected or highlighted.
fn snapshot(
    path: &PathBuf,
    cols: u16,
    rows: u16,
    scroll: i64,
    select: Option<&str>,
    menu: bool,
) -> Result<()> {
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;
    let mut terminal = ratatui::Terminal::new(TestBackend::new(cols, rows))?;
    let mut app = open_app(path, doc_width(cols), false)?;
    terminal.draw(|frame| app.draw(frame))?;
    app.scroll_for_snapshot(scroll);
    if let Some(quote) = select {
        app.select_quote_for_snapshot(quote)?;
    }
    if menu {
        app.open_review_menu();
    }
    terminal.draw(|frame| app.draw(frame))?;
    let buffer = terminal.backend().buffer();
    let mut marks = Vec::new();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        let mut mark = String::new();
        for x in 0..buffer.area.width {
            let Some(cell) = buffer.cell((x, y)) else { continue };
            line.push_str(cell.symbol());
            let style = cell.style();
            mark.push(if style.add_modifier.contains(Modifier::REVERSED) {
                '%'
            } else if style.add_modifier.contains(Modifier::CROSSED_OUT) {
                '-'
            } else if style.bg == Some(crate::theme::palette().approve_bg) {
                '+'
            } else if style.bg == Some(crate::theme::palette().comment_bg) {
                '#'
            } else {
                ' '
            });
        }
        println!("{}", line.trim_end());
        marks.push(mark);
    }
    if marks.iter().any(|m| !m.trim().is_empty()) {
        println!("--- marks ---");
        for m in marks {
            println!("{}", m.trim_end());
        }
    }
    Ok(())
}
