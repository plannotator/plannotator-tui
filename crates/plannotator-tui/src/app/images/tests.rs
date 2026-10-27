#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use plannotator_tui_schema::Kind;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

use super::{image_path, resolve};
use crate::app::review_test_support::{draw, file_app, press, reopen};
use crate::app::{Focus, Mode};
use crate::workspace_paths;
fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator-images-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("shots")).expect("folder");
    std::fs::write(dir.join("shots/screen shot.PNG"), b"png").expect("image");
    std::fs::write(dir.join("notes.txt"), b"text").expect("text");
    dir
}

#[test]
fn typed_pasted_and_dropped_spellings_name_the_same_absolute_file() {
    let dir = folder("spellings");
    let expected = dir.join("shots").join("screen shot.PNG").to_string_lossy().into_owned();
    let absolute = expected.clone();
    let mut inputs = vec![
        "shots/screen shot.PNG".to_owned(),
        "./shots/../shots/screen shot.PNG".to_owned(),
        format!("  '{absolute}'\t"),
        format!("\"{absolute}\""),
    ];
    if cfg!(unix) {
        inputs.push(format!("file://{}", absolute.replace(' ', "%20")));
        inputs.push(absolute.replace(' ', "\\ "));
    }
    for input in inputs {
        assert_eq!(image_path(&input, &dir).expect(&input), expected, "{input}");
    }
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn missing_files_and_other_extensions_are_refused_with_the_path() {
    let dir = folder("refused");
    let missing = image_path("shots/gone.png", &dir).expect_err("missing").to_string();
    assert!(missing.starts_with("no such file:") && missing.contains("gone.png"), "{missing}");
    let text = image_path("notes.txt", &dir).expect_err("not an image").to_string();
    assert!(text.starts_with("not an image") && text.contains("notes.txt"), "{text}");
    let folder_named_like_an_image = dir.join("dir.png");
    std::fs::create_dir_all(&folder_named_like_an_image).expect("dir");
    assert!(image_path("dir.png", &dir).is_err(), "a directory is not an image file");
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn a_tilde_starts_at_home() {
    let home = std::env::home_dir().expect("home");
    assert_eq!(resolve("~/a.png", Path::new("/elsewhere")), workspace_paths::absolute(&home.join("a.png")));
}

fn enter(app: &mut crate::app::App) {
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("enter");
}

#[test]
fn an_image_rides_with_its_note_into_feedback_and_back_out() {
    let (root, mut app, delivery) = file_app("attach");
    let image = root.join("docs").join("shots").join("before.png");
    std::fs::create_dir_all(image.parent().expect("parent")).expect("shots");
    std::fs::write(&image, b"png").expect("image");
    let image = image.to_string_lossy().into_owned();
    app.add_quote_annotation("one", Kind::Comment, "match this".into()).expect("note");
    press(&mut app, 'E');
    assert_eq!(app.send_count(), 0);

    // From the document, the key finds the selected block's note.
    app.select_block(1);
    press(&mut app, 'I');
    let id = app.open.store.placed()[0].annotation.id.clone();
    assert_eq!(app.mode, Mode::AttachImage(id));
    app.handle_event(&Event::Paste("shots/missing.png".into())).expect("paste");
    enter(&mut app);
    assert!(matches!(app.mode, Mode::AttachImage(_)), "a bad path keeps the prompt open");
    assert!(app.status.as_deref().is_some_and(|s| s.starts_with("no such file")), "{:?}", app.status);
    press(&mut app, 'x');
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Esc))).expect("esc");
    assert_eq!(app.mode, Mode::Browse);

    // From the notes rail, a relative path starts at the document's folder.
    app.focus = Focus::Rail;
    for _ in 0..2 {
        press(&mut app, 'I');
        app.handle_event(&Event::Paste("'shots/before.png'\n".into())).expect("paste");
        enter(&mut app);
    }
    assert_eq!(app.status.as_deref(), Some("already attached: before.png"));
    assert_eq!(app.open.store.placed()[0].annotation.plannotator_tui.images, std::slice::from_ref(&image));
    assert_eq!(app.send_count(), 1, "a sent note with a new image is pending again");
    assert!(draw(&mut app, 100, 24).contains("1 image"));

    press(&mut app, 'E');
    let sent = delivery.calls.borrow().last().cloned().expect("sent");
    assert!(sent.contains(&format!("> match this\nImage: {image}\n")), "{sent}");
    let history =
        std::fs::read_to_string(app.data_dir.join("feedback").join(&app.project).join("index.jsonl"))
            .expect("history");
    let last: serde_json::Value =
        serde_json::from_str(history.lines().last().expect("a line")).expect("history JSON");
    assert_eq!(last["counts"]["images"], 1);

    reopen(&mut app);
    assert_eq!(app.open.store.placed()[0].annotation.plannotator_tui.images, [image]);
    assert_eq!(app.send_count(), 0);
    app.focus = Focus::Rail;
    press(&mut app, 'D');
    assert_eq!(app.status.as_deref(), Some("image removed: before.png"));
    assert_eq!(app.open.store.placed()[0].annotation.plannotator_tui.images, Vec::<String>::new());
    assert_eq!(app.send_count(), 1, "removing an image is a change to send");
    std::fs::remove_dir_all(root).expect("cleanup");
}
