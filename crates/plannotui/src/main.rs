//! plannotui: prove that Markdown can be rendered by an off-the-shelf renderer,
//! laid out per block, selected with the mouse, and commented on - inside a plain
//! terminal or a Herdr pane.

mod app;
mod comments;
mod doc;
mod layout;
mod srcmap;
mod wrap;

use std::io::stdout;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;

use crate::app::App;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--bench") => {
            let path = args.get(1).map(PathBuf::from).context("usage: plannotui --bench <file.md>")?;
            return bench(path);
        }
        Some("--blocks") => {
            let path = args.get(1).map(PathBuf::from).context("usage: plannotui --blocks <file.md>")?;
            let app = App::open(path, 100)?;
            for line in app.describe_blocks() {
                println!("{line}");
            }
            return Ok(());
        }
        Some("--add-comment") => {
            let path = args
                .get(1)
                .map(PathBuf::from)
                .context("usage: plannotui --add-comment <file.md> <block> <text>")?;
            let block: usize = args.get(2).and_then(|s| s.parse().ok()).context("block index")?;
            let body = args.get(3).cloned().context("comment text")?;
            let mut app = App::open(path, 100)?;
            return app.add_comment(block, comments::Kind::Comment, body);
        }
        Some("--add-quote-comment") => {
            let path = args.get(1).map(PathBuf::from).context(
                "usage: plannotui --add-quote-comment <file.md> <quote> <text> [comment|approve|delete]",
            )?;
            let quote = args.get(2).cloned().context("quote")?;
            let body = args.get(3).cloned().context("comment text")?;
            let kind = match args.get(4).map(String::as_str) {
                Some("approve") => comments::Kind::Approve,
                Some("delete") => comments::Kind::Delete,
                _ => comments::Kind::Comment,
            };
            let mut app = App::open(path, 100)?;
            return app.add_quote_comment(&quote, kind, body);
        }
        Some("--snapshot") => {
            let path = args
                .get(1)
                .map(PathBuf::from)
                .context("usage: plannotui --snapshot <file.md> [cols rows scroll] [select-quote]")?;
            let cols: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(140);
            let rows: u16 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(40);
            let scroll: i64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
            let select = args.get(5).cloned();
            return snapshot(path, cols, rows, scroll, select);
        }
        _ => {}
    }
    let path = args.first().map(PathBuf::from).context("usage: plannotui <file.md>")?;

    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal, path);
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

/// Headless timing of the expensive paths: parse, per-block render + align, and reflow.
fn bench(path: PathBuf) -> Result<()> {
    let source = std::fs::read_to_string(&path)?;
    let bytes = source.len();
    let t = Instant::now();
    let doc = doc::Document::parse(source);
    let parse_ms = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    let mut layout = layout::DocLayout::build(&doc, 100);
    let build_ms = t.elapsed().as_secs_f64() * 1000.0;

    let mut reflow_ms = Vec::new();
    for width in [60, 140, 80, 120] {
        let t = Instant::now();
        layout.reflow(width);
        reflow_ms.push(format!("{width}→{:.1}ms", t.elapsed().as_secs_f64() * 1000.0));
    }
    let t = Instant::now();
    let mut hits = 0usize;
    for row in (0..layout.total_rows).step_by(7) {
        hits += layout.block_at_row(row).is_some() as usize;
    }
    let lookup_us = t.elapsed().as_secs_f64() * 1e6 / (layout.total_rows / 7).max(1) as f64;

    let mapped: usize = layout
        .blocks
        .iter()
        .flat_map(|b| b.rows.iter())
        .flat_map(|r| r.cells.iter())
        .filter(|c| c.is_some())
        .count();
    let cells: usize = layout.blocks.iter().flat_map(|b| b.rows.iter()).map(|r| r.cells.len()).sum();

    println!("{}: {} bytes, {} blocks", path.display(), bytes, doc.blocks.len());
    println!("parse+split          {parse_ms:8.1} ms");
    println!("render+align+wrap    {build_ms:8.1} ms  ({} rows)", layout.total_rows);
    println!("reflow               {}", reflow_ms.join("  "));
    println!("row lookup           {lookup_us:8.3} µs avg ({hits} hits)");
    println!("cells mapped         {mapped}/{cells} ({:.1}%)", mapped as f64 * 100.0 / cells.max(1) as f64);
    Ok(())
}

/// Draw one frame into an in-memory backend and print it as plain text, followed by a
/// mark map: `#` comment, `+` approve, `-` delete, `%` selected.
fn snapshot(path: PathBuf, cols: u16, rows: u16, scroll: i64, select: Option<String>) -> Result<()> {
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};
    let mut terminal = ratatui::Terminal::new(TestBackend::new(cols, rows))?;
    let mut app = App::open(path, cols.saturating_sub(2 + 1 + 36).max(20) as usize)?;
    terminal.draw(|frame| app.draw(frame))?;
    app.scroll_for_snapshot(scroll);
    if let Some(quote) = select {
        app.select_quote_for_snapshot(&quote)?;
    }
    terminal.draw(|frame| app.draw(frame))?;
    let buffer = terminal.backend().buffer();
    let mut marks = Vec::new();
    let mut any_marks = false;
    for y in 0..buffer.area.height {
        let mut line = String::new();
        let mut mark = String::new();
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            line.push_str(cell.symbol());
            let style = cell.style();
            let m = if style.add_modifier.contains(Modifier::REVERSED) {
                '%'
            } else if style.add_modifier.contains(Modifier::CROSSED_OUT) {
                '-'
            } else if style.bg == Some(Color::Indexed(22)) {
                '+'
            } else if style.bg == Some(Color::Indexed(58)) {
                '#'
            } else {
                ' '
            };
            any_marks |= m != ' ';
            mark.push(m);
        }
        println!("{}", line.trim_end());
        marks.push(mark);
    }
    if any_marks {
        println!("--- marks ---");
        for m in marks {
            println!("{}", m.trim_end());
        }
    }
    Ok(())
}

fn run(terminal: &mut ratatui::DefaultTerminal, path: PathBuf) -> Result<()> {
    let width = terminal.size()?.width.saturating_sub(2 + 1 + 36).max(20) as usize;
    let mut app = App::open(path, width)?;
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
            app.handle_event(event::read()?)?;
            dirty = true;
            // Coalesce bursts (wheel, drag) into one redraw.
            while event::poll(Duration::ZERO)? {
                app.handle_event(event::read()?)?;
            }
        }
    }
    Ok(())
}
