//! Where the review opens: a split beside the agent's block, or a floating layer over its
//! window. Both are built here as API arguments; only `open` calls Rex.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::command::Rex;
use crate::config::RexPlacement;

/// The popup's `[x, y, w, h]`, as fractions of the window: the size Herdr's popup defaults to.
const POPUP_BOUNDS: [f64; 4] = [0.05, 0.075, 0.9, 0.85];

/// Open a block running `argv` in `cwd` beside or over `block`, and focus it.
pub(crate) fn open(
    rex: &Rex,
    block: &str,
    placement: RexPlacement,
    cwd: Option<&Path>,
    argv: &[String],
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
}
