//! Where the review opens: a split beside the agent's block, a floating layer over its window,
//! or a tab (a Rex window) of its own. Each is built here as API arguments; only `open` calls
//! Rex.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::command::Rex;
use crate::config::RexPlacement;

/// The popup's `[x, y, w, h]`, as fractions of the window: the size Herdr's popup defaults to.
const POPUP_BOUNDS: [f64; 4] = [0.05, 0.075, 0.9, 0.85];

/// Open a block running `argv` in `cwd` beside or over `block`, or in a tab called `label`,
/// and focus it.
pub(crate) fn open(
    rex: &Rex,
    block: &str,
    placement: RexPlacement,
    cwd: Option<&Path>,
    argv: &[String],
    label: &str,
) -> Result<()> {
    let review = review_block(cwd, argv);
    match placement {
        RexPlacement::Split => rex.api("session.new_split", &split(block, &review))?,
        RexPlacement::Popup => {
            let described = rex.api("session.describe_block", &json!({ "block_id": block }))?;
            let window = described.pointer("/block/window_id").and_then(Value::as_str);
            let window = window.with_context(|| format!("block {block} is not in a window"))?;
            rex.api("session.new_layer", &layer(window, &review))?
        }
        RexPlacement::Tab => rex.api("session.new_window", &tab(label, &review))?,
    };
    Ok(())
}

/// The review's block: `argv` run directly, with no shell between Rex and the app.
fn review_block(cwd: Option<&Path>, argv: &[String]) -> Value {
    let mut options = serde_json::Map::new();
    options.insert("shell".to_owned(), json!("none"));
    options.insert("command".to_owned(), json!(argv));
    if let Some(cwd) = cwd {
        options.insert("cwd".to_owned(), json!(cwd.display().to_string()));
    }
    json!({ "block": { "flavor": "com.superlogical.terminal.shell", "label": "Annotate", "options": options } })
}

/// `session.new_split` arguments: `review` to the right of `block`.
fn split(block: &str, review: &Value) -> Value {
    json!({ "anchor_id": block, "direction": "horizontal", "side": "after", "focus": true, "layout": review })
}

/// `session.new_layer` arguments: `review` floating over `window`, the agent's own window, so
/// a launch from a block in a background tab doesn't land over whatever tab is active.
fn layer(window: &str, review: &Value) -> Value {
    json!({ "window_id": window, "bounds": POPUP_BOUNDS, "focus": true, "layout": review })
}

/// `session.new_window` arguments: a tab in the agent's session holding only `review`, shown
/// at once. Rex closes the tab when the review exits; the review then shows the agent again.
fn tab(label: &str, review: &Value) -> Value {
    json!({ "window_label": label, "focus": true, "layout": review })
}

/// After a review in a tab of its own ends, show `block`, the pane it was opened from, and so
/// its tab. Only while the review's own tab (`own` is the review's block) is the one showing:
/// a review that ends while you are elsewhere moves nobody.
pub(crate) fn show_again(rex: &Rex, own: &str, block: &str) {
    let view = rex.api("session.view", &json!({}));
    if view.is_ok_and(|view| showing(&view, own)) {
        let _ = rex.api("session.focus_block", &json!({ "block_id": block }));
    }
}

/// Whether `block` is in the session's active window, from `session.view`.
fn showing(view: &Value, block: &str) -> bool {
    let active = view.get("active_window_id").and_then(Value::as_str);
    let windows = view.get("windows").and_then(Value::as_array).into_iter().flatten();
    let blocks = windows
        .filter(|w| active.is_some() && w.get("window_id").and_then(Value::as_str) == active)
        .filter_map(|w| w.get("layers").and_then(Value::as_array))
        .flatten()
        .filter_map(|layer| layer.get("blocks").and_then(Value::as_array))
        .flatten();
    blocks.into_iter().any(|b| b.get("block_id").and_then(Value::as_str) == Some(block))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn the_review_opens_to_the_right_of_the_agents_block_and_runs_directly() {
        let split = split("block:b", &review_block(Some(Path::new("/tmp")), &["/bin/pt".to_owned()]));
        assert_eq!(split["anchor_id"], "block:b");
        assert_eq!((&split["direction"], &split["side"]), (&json!("horizontal"), &json!("after")));
        assert_eq!(split["layout"]["block"]["options"]["shell"], "none");
        assert_eq!(split["layout"]["block"]["options"]["command"], json!(["/bin/pt"]));
        assert_eq!(split["layout"]["block"]["options"]["cwd"], "/tmp");
    }

    #[test]
    fn a_popup_floats_inside_the_agents_window_and_takes_focus() {
        let layer = layer("window:w", &review_block(None, &["/bin/pt".to_owned()]));
        assert_eq!(layer["window_id"], "window:w");
        assert_eq!(layer["focus"], true);
        let bounds: Vec<f64> = serde_json::from_value(layer["bounds"].clone()).unwrap_or_default();
        let fits = matches!(bounds[..], [x, y, w, h] if x >= 0.0 && y >= 0.0 && x + w <= 1.0 && y + h <= 1.0);
        assert!(fits, "four bounds inside the window: {bounds:?}");
        assert_eq!(layer["layout"]["block"]["label"], "Annotate");
    }

    #[test]
    fn a_tab_holds_only_the_review_is_named_for_it_and_is_shown_at_once() {
        let tab = tab("Annotate · claude", &review_block(Some(Path::new("/tmp")), &["/bin/pt".to_owned()]));
        assert_eq!(tab["window_label"], "Annotate · claude");
        assert_eq!(tab["focus"], true);
        assert_eq!(tab["layout"]["block"]["options"]["command"], json!(["/bin/pt"]));
        assert_eq!(tab["layout"]["block"]["options"]["cwd"], "/tmp");
        assert_eq!(tab["layout"].get("split"), None, "the review is the tab's only block");
    }

    /// `session.view` as Rex 2063 returns it, cut to what `showing` reads.
    fn view(active: &str) -> Value {
        json!({
            "active_window_id": active,
            "windows": [
                { "window_id": "window:agent", "layers": [{ "kind": "tiled", "blocks": [{ "block_id": "block:claude" }] }] },
                { "window_id": "window:review", "layers": [{ "kind": "tiled", "blocks": [{ "block_id": "block:review" }] }] }
            ]
        })
    }

    #[test]
    fn the_agent_is_shown_again_only_while_the_reviews_tab_is_showing() {
        assert!(showing(&view("window:review"), "block:review"));
        assert!(!showing(&view("window:agent"), "block:review"), "you moved to another tab: stay there");
        assert!(!showing(&json!({}), "block:review"));
    }
}
