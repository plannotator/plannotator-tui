//! `OpenCode` messages out of a generated `opencode.db` (the 1.18 schema subset), read-only.
//!
//! Fixtures and helpers live in `fixture`; each other module holds the tests for one
//! invariant, so a failure names the rule it broke.

#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

mod fixture;
mod parts;
mod session;
mod v1;
mod v2;
