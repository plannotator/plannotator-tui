//! Timings on a change of 3,000 files in deep folders, as in a large monorepo. Ignored by
//! default; run with
//! `cargo test --release -p plannotator-tui big_change -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use std::io::Write as _;

use super::*;

const FILES: usize = 3000;
const COLS: u16 = 200;
const ROWS: u16 = 50;

/// `FILES` committed files in deep folders, each then changed on one line.
fn big(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    let kinds = ["ts", "test.ts", "md", "json", "rs", "go", "_test.go", "tsx", "css", "py"];
    let paths: Vec<String> = (0..FILES)
        .map(|i| {
            let (package, area, deep) = (i % 40, (i / 40) % 6, (i / 240) % 4);
            let kind = kinds.get(i % kinds.len()).copied().unwrap_or("ts");
            format!(
                "packages/group-{}/package-{package}/src/area-{area}/level-{deep}/inner/file-{i}-with-a-long-name.{kind}",
                package % 5
            )
        })
        .collect();
    let body = (0..20).fold(String::new(), |body, n| body + &format!("line {n}\n"));
    for path in &paths {
        let full = fixture.repo().join(path);
        std::fs::create_dir_all(full.parent().expect("a folder")).expect("folders");
        std::fs::write(&full, &body).expect("write");
    }
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "big"]);
    for path in &paths {
        std::fs::write(fixture.repo().join(path), format!("{body}changed\n")).expect("write");
    }
    fixture
}

/// The mean of `rounds` runs of `step` followed by one frame.
fn per_step(screen: &mut Screen, rounds: u32, mut step: impl FnMut(&mut Screen)) -> Duration {
    let started = Instant::now();
    for _ in 0..rounds {
        step(screen);
        let _ = frame(screen, COLS, ROWS);
    }
    started.elapsed() / rounds
}

#[test]
#[ignore = "timings; run by hand"]
fn big_change() {
    let fixture = big("big");
    let started = Instant::now();
    let (repo, capture) =
        crate::review::capture(&fixture.repo(), &Request::Uncommitted { base: None }).expect("capture");
    let captured = started.elapsed();
    let (wake, _ready) = std::sync::mpsc::sync_channel(1);
    let mut screen =
        Screen::open(repo, capture, &fixture.data(), Box::new(fixture.agent.clone()), wake).expect("screen");
    let opened = started.elapsed();
    let _ = frame(&mut screen, COLS, ROWS);
    let painted = started.elapsed();
    while screen.diff.files_shown() < screen.files.shown().len()
        || screen.diff.progress() == crate::review::diff::Progress::Loading
    {
        screen.tick().expect("tick");
        let _ = frame(&mut screen, COLS, ROWS);
    }
    let loaded = started.elapsed();
    let idle = per_step(&mut screen, 50, |_| {});
    let diff_j = per_step(&mut screen, 200, |s| keys(s, "j"));
    keys(&mut screen, "f");
    let list_j = per_step(&mut screen, 200, |s| keys(s, "j"));
    press(&mut screen, KeyCode::Esc);
    keys(&mut screen, "/");
    let typing = per_step(&mut screen, 11, |s| {
        let next = "area-3 lev1".chars().nth(s.files.filter.query.chars().count()).unwrap_or('x');
        keys(s, &next.to_string());
    });
    let filtered = Instant::now();
    press(&mut screen, KeyCode::Enter);
    let _ = frame(&mut screen, COLS, ROWS);
    let applied = filtered.elapsed();
    keys(&mut screen, "f");
    press(&mut screen, KeyCode::Esc);
    let kind = per_step(&mut screen, 10, |s| {
        keys(s, "tj ");
        press(s, KeyCode::Esc);
    });
    let _ = writeln!(
        std::io::stderr(),
        "{FILES} files: capture {captured:?}, screen {opened:?}, first paint {painted:?}, loaded {loaded:?}\n\
         frame {idle:?}; j in the diff {diff_j:?}; j in the list {list_j:?}\n\
         typing in / {typing:?} a key; enter {applied:?}; a kind on or off {kind:?}"
    );
}
