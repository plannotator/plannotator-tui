//! Running inside Herdr: what Herdr tells us and how we open ourselves in a pane.
//!
//! Nothing here draws. `context` reads the environment Herdr sets; `launch` turns it into
//! one `herdr plugin pane open` invocation.

pub(crate) mod context;
pub(crate) mod launch;
