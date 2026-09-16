//! Feedback export: annotations as numbered Markdown a coding agent reads without a schema.
//!
//! `# Annotations on <name>`, then one `## Annotation N (line X)` per annotation in document
//! order: what kind of note it is, the quoted text, the body as a blockquote, and any image
//! attachments as Markdown links. Deleted text is fenced so quoted markdown cannot escape.

use std::fmt::Write as _;
use std::ops::Range;

use plannotator_tui_schema::{Annotation, Kind};

/// One annotation placed in the document, as the exporter needs it.
pub(crate) struct Entry<'a> {
    pub(crate) annotation: &'a Annotation,
    pub(crate) range: Range<usize>,
    /// 1-based source line span of the annotated range.
    pub(crate) lines: (usize, usize),
}

pub(crate) fn feedback(source: &str, name: &str, entries: &[Entry<'_>]) -> String {
    if entries.is_empty() {
        return "No annotations.".to_owned();
    }
    let mut out = format!("# Annotations on {name}\n\n");
    for (i, entry) in entries.iter().enumerate() {
        let quoted = source.get(entry.range.clone()).unwrap_or("");
        let line_label = match entry.lines {
            (a, b) if a == b => format!("line {a}"),
            (a, b) => format!("lines {a}\u{2013}{b}"),
        };
        let _ = writeln!(out, "## Annotation {} ({line_label})", i + 1);
        let body = entry.annotation.body.trim();
        match entry.annotation.anchor.kind() {
            Kind::Delete => {
                out.push_str("Remove this:\n");
                out.push_str(&fenced(quoted));
                let _ = writeln!(out, "> {}", if body.is_empty() { "I don't want this." } else { body });
            }
            Kind::LooksGood => {
                let _ = writeln!(out, "Looks good: \"{}\"", single_line(quoted));
                if !body.is_empty() {
                    let _ = writeln!(out, "> {}", quote_lines(body));
                }
            }
            Kind::Comment => {
                let _ = writeln!(out, "Comment on: \"{}\"", single_line(quoted));
                let _ = writeln!(out, "> {}", quote_lines(body));
            }
        }
        write_attachments(&mut out, entry.annotation);
        for reply in &entry.annotation.replies {
            let who = reply.author.as_deref().unwrap_or("reply");
            let _ = writeln!(out, "- **Reply ({who}):** {}", reply.body.replace('\n', "\n  "));
        }
        out.push('\n');
    }
    out
}

/// A fence longer than any backtick run inside the text, so quoted markdown cannot escape.
fn fenced(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}\n{text}\n{fence}\n")
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn quote_lines(text: &str) -> String {
    text.replace('\n', "\n> ")
}

fn write_attachments(out: &mut String, annotation: &Annotation) {
    let total = annotation.attachments.len()
        + annotation.plannotator_tui.attachments.iter().filter(|a| a.is_local_image()).count();
    if total == 0 {
        return;
    }
    out.push_str("Attachments:\n");
    let mut index = 1;
    for url in &annotation.attachments {
        let alt = format!("image {index}");
        let _ = writeln!(out, "- Image {index}: {url}");
        let _ = writeln!(out, "  ![{}]({})", markdown_alt(&alt), markdown_target(url));
        index += 1;
    }
    for attachment in annotation.plannotator_tui.attachments.iter().filter(|a| a.is_local_image()) {
        let alt = attachment.alt.as_deref().filter(|alt| !alt.trim().is_empty()).unwrap_or("attached image");
        let target = file_uri(&attachment.path);
        let _ = writeln!(out, "- Image {index}: `{}`", attachment.path);
        let _ = writeln!(out, "  ![{}]({target})", markdown_alt(alt));
        index += 1;
    }
}

fn markdown_alt(text: &str) -> String {
    text.replace(['[', ']'], "")
}

fn markdown_target(text: &str) -> String {
    text.replace(' ', "%20").replace('(', "%28").replace(')', "%29")
}

fn file_uri(path: &str) -> String {
    let normalized = if cfg!(windows) { path.replace('\\', "/") } else { path.to_owned() };
    let normalized = if cfg!(windows) {
        if let Some(unc) = normalized.strip_prefix("//?/UNC/") {
            format!("//{unc}")
        } else {
            normalized.strip_prefix("//?/").unwrap_or(&normalized).to_owned()
        }
    } else {
        normalized
    };
    let path = if cfg!(windows) && normalized.as_bytes().get(1) == Some(&b':') {
        format!("/{normalized}")
    } else {
        normalized
    };
    format!("file://{}", percent_encode_path(&path))
}

fn percent_encode_path(path: &str) -> String {
    let mut out = String::new();
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(char::from(byte));
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// 1-based line numbers of the first and last byte of `range`.
pub(crate) fn line_span(source: &str, range: &Range<usize>) -> (usize, usize) {
    let line_at = |offset: usize| source.get(..offset).map_or(1, |s| s.matches('\n').count() + 1);
    (line_at(range.start), line_at(range.end.saturating_sub(1).max(range.start)))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    use plannotator_tui_schema::{Anchor, SourceRange, State};

    #[cfg(windows)]
    #[test]
    fn canonical_attachment_paths_export_as_file_uris() {
        assert_eq!(file_uri(r"\\?\C:\captures\shot.png"), "file:///C:/captures/shot.png");
        assert_eq!(file_uri(r"\\?\UNC\server\captures\shot.png"), "file:////server/captures/shot.png");
    }

    fn annotation(source: &str, quote: &str, kind: Kind, body: &str) -> (Annotation, Range<usize>) {
        let start = source.find(quote).expect("present");
        let range = start..start + quote.len();
        let source_range = SourceRange { start, end: range.end, version: "v".into() };
        let annotation = Annotation {
            id: "a".into(),
            document_id: String::new(),
            anchor: Anchor::new(quote, source, source_range, kind, None),
            body: body.into(),
            author: None,
            author_name: None,
            state: State::Open,
            attachments: Vec::new(),
            plannotator_tui: plannotator_tui_schema::AnnotationExtras::default(),
            created_at: String::new(),
            updated_at: String::new(),
            replies: Vec::new(),
            other: std::collections::BTreeMap::default(),
        };
        (annotation, range)
    }

    #[test]
    fn export_matches_the_agent_facing_shape() {
        let source = "# Title\n\nShip the login page by Friday.\n\nDrop the `legacy` path.\n";
        let (comment, r1) = annotation(source, "login page", Kind::Comment, "Which page?\nBe specific.");
        let (delete, r2) = annotation(source, "Drop the `legacy` path.", Kind::Delete, "");
        let entries = [
            Entry { annotation: &comment, lines: line_span(source, &r1), range: r1 },
            Entry { annotation: &delete, lines: line_span(source, &r2), range: r2 },
        ];
        let out = feedback(source, "plan.md", &entries);
        assert_eq!(
            out,
            "# Annotations on plan.md\n\n\
             ## Annotation 1 (line 3)\nComment on: \"login page\"\n> Which page?\n> Be specific.\n\n\
             ## Annotation 2 (line 5)\nRemove this:\n```\nDrop the `legacy` path.\n```\n> I don't want this.\n\n"
        );
    }

    #[test]
    fn images_are_rendered_as_clear_markdown_attachments() {
        let source = "see screenshot\n";
        let (mut note, range) = annotation(source, "screenshot", Kind::Comment, "Compare this.");
        note.attachments.push("https://cdn.example.com/upload.png".into());
        note.plannotator_tui.attachments.push(plannotator_tui_schema::LocalAttachment::image(
            "/tmp/screen shot.png".into(),
            Some("screen shot.png".into()),
            Some("image/png".into()),
        ));
        let out = feedback(source, "reply", &[Entry { annotation: &note, lines: (1, 1), range }]);
        assert!(out.contains("Attachments:\n- Image 1: https://cdn.example.com/upload.png"), "{out}");
        assert!(out.contains("![image 1](https://cdn.example.com/upload.png)"), "{out}");
        assert!(out.contains("- Image 2: `/tmp/screen shot.png`"), "{out}");
        assert!(out.contains("![screen shot.png](file:///tmp/screen%20shot.png)"), "{out}");
    }

    #[test]
    fn remote_image_destinations_preserve_markdown_delimiters() {
        let source = "screenshot";
        let (mut note, range) = annotation(source, source, Kind::Comment, "Compare this.");
        note.attachments = vec![
            "https://cdn.example.com/screenshot).png".into(),
            "https://cdn.example.com/screen(shot).png?q=a%20b".into(),
        ];
        let out = feedback(source, "reply", &[Entry { annotation: &note, lines: (1, 1), range }]);
        let destinations: Vec<_> = pulldown_cmark::Parser::new(&out)
            .filter_map(|event| match event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { dest_url, .. }) => {
                    Some(dest_url.into_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            destinations,
            [
                "https://cdn.example.com/screenshot%29.png",
                "https://cdn.example.com/screen%28shot%29.png?q=a%20b",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_image_destinations_preserve_unix_backslashes() {
        let source = "screenshot";
        let (mut note, range) = annotation(source, source, Kind::Comment, "Compare this.");
        note.plannotator_tui.attachments.push(plannotator_tui_schema::LocalAttachment::image(
            "/tmp/screen\\shot.png".into(),
            None,
            Some("image/png".into()),
        ));
        let out = feedback(source, "reply", &[Entry { annotation: &note, lines: (1, 1), range }]);
        let destinations: Vec<_> = pulldown_cmark::Parser::new(&out)
            .filter_map(|event| match event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { dest_url, .. }) => {
                    Some(dest_url.into_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(destinations, ["file:///tmp/screen%5Cshot.png"]);
    }

    #[test]
    fn fences_grow_past_embedded_backticks() {
        assert!(fenced("has ``` inside").starts_with("````\n"));
    }
}
