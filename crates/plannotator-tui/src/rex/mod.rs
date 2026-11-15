//! Running inside Rex: find the agent in a block, open ourselves in a split beside it, and
//! send the feedback back to that block.
//!
//! Rex is driven only through its `rex` CLI. `command` runs it; `launch` finds the agent and
//! `place` opens the split or popup; `terminal` reads a block's output; `paths` finds the
//! Markdown files in it; `agent` is the delivery target the opened app sends to; `install`
//! writes the Rex Annotate Lua module; `cli` parses the `rex` subcommands.

pub(crate) mod agent;
pub(crate) mod cli;
pub(crate) mod command;
pub(crate) mod install;
pub(crate) mod launch;
pub(crate) mod paths;
pub(crate) mod pi;
pub(crate) mod place;
pub(crate) mod terminal;
