//! Running inside herdr-core (`hc`): find the agent in a terminal, open ourselves in a terminal
//! of our own beside it, and send the feedback back to that terminal.
//!
//! herdr-core is driven only through its `hc` CLI, and herdr-app only through `herdr-app do`.
//! `command` runs `hc`; `launch` starts the review terminal; `show` puts it on screen with
//! herdr-app; `agent` is the delivery target the review sends to; `cli` parses the `hc`
//! subcommands.

pub(crate) mod agent;
pub(crate) mod cli;
pub(crate) mod command;
pub(crate) mod launch;
pub(crate) mod show;
