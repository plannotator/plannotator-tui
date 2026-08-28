//! Where rendered feedback goes when the user sends it (decision 11).
//!
//! The app renders feedback with one function regardless of target; only the target
//! varies. Clipboard is the standalone default. The Herdr plugin adds an agent-pane target
//! later; nothing else in the app knows which is in use.

use std::io::Write as _;

use anyhow::Result;

pub(crate) trait Delivery {
    /// Shown in the footer, e.g. `send → clipboard`.
    fn describe(&self) -> String;
    fn deliver(&self, feedback: &str) -> Result<()>;
}

/// OSC 52: hand text to the terminal's clipboard so Cmd-V works outside the app.
#[derive(Debug, Default)]
pub(crate) struct Clipboard;

impl Delivery for Clipboard {
    fn describe(&self) -> String {
        "clipboard".to_owned()
    }

    fn deliver(&self, feedback: &str) -> Result<()> {
        let mut out = std::io::stdout().lock();
        write!(out, "\x1b]52;c;{}\x07", base64(feedback.as_bytes()))?;
        out.flush()?;
        Ok(())
    }
}

/// Headless runs: nothing leaves the process.
#[derive(Debug, Default)]
pub(crate) struct Discard;

impl Delivery for Discard {
    fn describe(&self) -> String {
        "nowhere (headless)".to_owned()
    }

    fn deliver(&self, _feedback: &str) -> Result<()> {
        Ok(())
    }
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            let ch =
                if i <= chunk.len() { TABLE.get(((n >> (18 - 6 * i)) & 63) as usize).copied() } else { None };
            out.push(ch.map_or('=', char::from));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
