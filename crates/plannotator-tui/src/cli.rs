//! Command-line entry points: the interactive viewer and the headless tools.
//!
//! This is the only module that prints to stdout.

#![allow(clippy::print_stdout, reason = "the CLI's job is to print")]

use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use plannotator_tui_schema::{DocumentSource, Kind};
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;

use crate::app::App;
use crate::config::Config;
use crate::delivery::{Clipboard, Delivery, Discard, HerdrAgent};
use crate::doc::Document;
use crate::herdr::context::HerdrEnv;
use crate::layout::DocLayout;

const USAGE: &str = "usage:
  plannotator-tui <file.md | folder>
  plannotator-tui --export <file.md>
  plannotator-tui --bench <file.md>
  plannotator-tui --blocks <file.md>
  plannotator-tui --annotate <file.md> <quote> <text> [comment|looks_good|delete]
  plannotator-tui --annotate-block <file.md> <block> <text>
  plannotator-tui --snapshot <file.md> [cols rows scroll] [select-quote]
  plannotator-tui config
  plannotator-tui --version
  plannotator-tui herdr open [file.md | folder] [--placement overlay|split|popup] [--deliver-to <pane>]
  plannotator-tui herdr last [--placement P] [--deliver-to <pane>]
  plannotator-tui herdr pane
  plannotator-tui last [--host claude|codex] [--pid N] [--session <transcript>] [--stdin] [--print] [--pick N]";

/// Width the document gets when nothing else is known: gutter + rail + gap subtracted.
fn doc_width(cols: u16) -> usize {
    usize::from(cols.saturating_sub(2 + 1 + 36).max(20))
}

fn open_file(path: &PathBuf) -> Result<DocumentSource> {
    let content = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(DocumentSource::file(path.clone(), content))
}

/// A file opens on its own; a folder opens in folder mode on its first markdown file.
/// Interactive runs send to the Herdr agent pane named by the environment, else the
/// clipboard; headless runs send nowhere.
pub(crate) fn delivery(interactive: bool) -> Box<dyn Delivery> {
    if !interactive {
        return Box::new(Discard);
    }
    let env = HerdrEnv::from_env();
    match env.delivery_target() {
        Some(target) if env.in_herdr => {
            let agent = target.agent.or_else(|| env.agent_in_pane(&target.pane));
            Box::new(HerdrAgent::new(env.bin, target.pane, agent))
        }
        _ => Box::new(Clipboard),
    }
}

fn open_app(path: &PathBuf, width: usize, interactive: bool) -> Result<App> {
    let delivery = delivery(interactive);
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

pub(crate) fn run(args: &[String]) -> Result<()> {
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
            app.add_quote_annotation(quote, parse_kind(arg(4)), body)
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
            snapshot(&path(1)?, cols, rows, scroll, arg(5))
        }
        Some("--version" | "-V") => {
            println!("plannotator-tui {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("config") => show_config(),
        Some("herdr") => herdr_command(args.get(1..).unwrap_or_default()),
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
    let config = Config::load_from(&path)?;
    let state = if path.is_file() { "" } else { " (not present; defaults)" };
    println!("# {}{state}", path.display());
    print!("{}", config.to_toml()?);
    Ok(())
}

/// `plannotator-tui herdr open [PATH] [--placement P] [--deliver-to PANE]`.
fn herdr_command(args: &[String]) -> Result<()> {
    use crate::herdr::launch::{OpenArgs, agent_get, plan, plan_last, process_info, run};
    let sub = args.first().map(String::as_str);
    if sub == Some("pane") {
        return herdr_pane();
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
            flag if flag.starts_with("--") => anyhow::bail!("unknown flag {flag}\n{USAGE}"),
            path if open.path.is_none() => open.path = Some(PathBuf::from(path)),
            extra => anyhow::bail!("unexpected argument {extra:?}\n{USAGE}"),
        }
    }
    let env = HerdrEnv::from_env();
    let config = Config::load()?;
    let cwd = std::env::current_dir().context("current directory")?;
    let launch = if sub == Some("last") {
        let probe = plan(&env, &config, OpenArgs { path: None, ..open.clone() }, &cwd)?;
        let pane = probe.deliver.as_ref().map(|t| t.pane.clone()).or(probe.target_pane);
        let pane = pane.context("no agent pane to read: not focused on one and no --deliver-to")?;
        let agent = agent_get(&env, &pane);
        plan_last(&env, &config, open, &cwd, &process_info(&env, &pane)?, agent.as_deref())?
    } else {
        plan(&env, &config, open, &cwd)?
    };
    run(&env, &launch)
}

/// The pane entrypoint: Herdr runs this in the opened pane; the environment says what to show.
fn herdr_pane() -> Result<()> {
    let env = HerdrEnv::from_env();
    let result = if let Some(pid) = env.message_pid {
        crate::last::run(&crate::last::LastOptions {
            host: env.host.clone(),
            pid: Some(pid),
            session: env.session.clone(),
            pick: 25,
            ..crate::last::LastOptions::default()
        })
    } else {
        let path = env.file.clone().unwrap_or(std::env::current_dir().context("current directory")?);
        interactive(&path)
    };
    // The pane closes when we exit; an error that flashes by is an error nobody can read.
    if let Err(err) = &result {
        #[allow(clippy::print_stderr, reason = "the pane is the only place this can be read")]
        {
            eprintln!("plannotator-tui: {err:#}\n\npress enter to close");
        }
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
    result
}

/// `plannotator-tui last [--host H] [--pid N] [--session PATH] [--stdin] [--print] [--pick N]`.
fn last_command(args: &[String]) -> Result<()> {
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
            "--stdin" => options.stdin = true,
            "--print" => options.print = true,
            "--pick" => options.pick = rest.next().context("--pick needs a value")?.parse()?,
            other => anyhow::bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    crate::last::run(&options)
}

fn interactive(path: &PathBuf) -> Result<()> {
    run_ui(|width| open_app(path, width, true))
}

/// Own the terminal for one app: `build` gets the document width the screen allows.
pub(crate) fn run_ui(build: impl FnOnce(usize) -> Result<App>) -> Result<()> {
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    let width = doc_width(terminal.size().map_or(120, |s| s.width));
    let result = build(width).and_then(|app| event_loop(&mut terminal, app));
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, mut app: App) -> Result<()> {
    app.clipboard = true;
    let mut dirty = true;
    while !app.quit {
        if dirty {
            let started = Instant::now();
            terminal.draw(|frame| app.draw(frame))?;
            app.record_frame(started.elapsed().as_secs_f64() * 1000.0);
            dirty = false;
        }
        if event::poll(Duration::from_millis(250))? {
            app.handle_event(&event::read()?)?;
            dirty = true;
            // Coalesce bursts (wheel, drag) into one redraw.
            while event::poll(Duration::ZERO)? {
                app.handle_event(&event::read()?)?;
            }
        }
    }
    Ok(())
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
/// `#` comment, `+` looks good, `-` delete, `%` selected.
fn snapshot(path: &PathBuf, cols: u16, rows: u16, scroll: i64, select: Option<&str>) -> Result<()> {
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};
    let mut terminal = ratatui::Terminal::new(TestBackend::new(cols, rows))?;
    let mut app = open_app(path, doc_width(cols), false)?;
    terminal.draw(|frame| app.draw(frame))?;
    app.scroll_for_snapshot(scroll);
    if let Some(quote) = select {
        app.select_quote_for_snapshot(quote)?;
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
            } else if style.bg == Some(Color::Indexed(22)) {
                '+'
            } else if style.bg == Some(Color::Indexed(58)) {
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
