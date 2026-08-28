//! Document versions as git blob shas, byte-identical to the Workspaces API's `ETag`.

use sha1::{Digest, Sha1};

/// `sha1("blob " + len + "\0" + bytes)`, hex — what `git hash-object` prints.
pub fn blob_sha(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(b"blob ");
    hasher.update(bytes.len().to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(bytes);
    hex(&hasher.finalize())
}

/// Strip the weak marker and quotes an HTTP edge may add: `W/"abc"` → `abc`.
pub fn normalize_etag(etag: &str) -> &str {
    etag.trim().trim_start_matches("W/").trim_matches('"')
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn matches_git_hash_object() {
        // `printf 'hello\n' | git hash-object --stdin`
        assert_eq!(blob_sha(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
        // `git hash-object` of the empty blob
        assert_eq!(blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn etag_forms_normalize_to_the_bare_sha() {
        assert_eq!(normalize_etag("\"abc\""), "abc");
        assert_eq!(normalize_etag("W/\"abc\""), "abc");
        assert_eq!(normalize_etag("abc"), "abc");
    }
}
