//! `.gitattributes` `linguist-generated` in the file list's kinds: git's answer wins both
//! ways once it comes, as in Plannotator.

use std::time::{Duration, Instant};

use super::*;
use crate::review::files::kind::Kind;

/// Tick until git's answer about the attributes is in.
fn attributes_read(screen: &mut Screen) {
    let started = Instant::now();
    while !screen.files.attributes_read() {
        screen.tick().expect("tick");
        assert!(started.elapsed() < Duration::from_secs(10), "git never answered");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn linguist_generated_marks_a_file_and_its_negation_unmarks_a_lockfile() {
    let fixture = Fixture::new("files-attributes");
    std::fs::create_dir_all(fixture.repo().join("api")).expect("folder");
    fixture.write(".gitattributes", "api/*.ts linguist-generated\nCargo.lock -linguist-generated\n");
    fixture.write("api/client.ts", "export const client = 1;\n");
    fixture.write("Cargo.lock", "# lock\n");
    let mut screen = fixture.open();
    attributes_read(&mut screen);
    assert_eq!(screen.files.kind_of("api/client.ts"), Some(Kind::Generated));
    assert_eq!(screen.files.kind_of("Cargo.lock"), Some(Kind::Other), "not generated");
    assert_eq!(screen.files.kind_of("app.py"), Some(Kind::Code));
    // The picker counts follow: Generated alone shows the client.
    keys(&mut screen, "t");
    keys(&mut screen, "jjjjj");
    press(&mut screen, KeyCode::Enter);
    settle(&mut screen);
    assert_eq!(screen.diff.paths(), ["api/client.ts"]);
}
