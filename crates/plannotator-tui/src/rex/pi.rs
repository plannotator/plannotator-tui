//! Read the live Pi session snapshot from its per-process Unix socket.

use std::path::PathBuf;

use anyhow::Result;
#[cfg(unix)]
use rustix::event::{PollFd, PollFlags, Timespec, poll};
#[cfg(unix)]
use rustix::io::Errno;
#[cfg(unix)]
use serde_json::Value;

/// A validated Pi session that has a persisted transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PiSession {
    pub(crate) session_id: String,
    pub(crate) session_path: PathBuf,
    pub(crate) leaf_id: Option<String>,
}

/// Read the snapshot for the Pi process in the Rex foreground.
pub(crate) fn current_session(pid: u32) -> Result<PiSession> {
    #[cfg(unix)]
    {
        let home =
            std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| extension_error("HOME is not set"))?;
        read_from_home(&home, pid, READ_TIMEOUT)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err(extension_error("Pi session sockets require a Unix platform"))
    }
}

#[cfg(unix)]
const MAX_RESPONSE_BYTES: usize = 16 * 1024;
#[cfg(unix)]
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(unix)]
fn read_from_home(home: &std::path::Path, pid: u32, timeout: std::time::Duration) -> Result<PiSession> {
    use std::io::{ErrorKind, Read as _};
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    anyhow::ensure!(pid > 0, "{}", extension_error("Pi process ID must be positive"));
    anyhow::ensure!(home.is_absolute(), "{}", extension_error("HOME is not an absolute path"));

    let socket_path = home.join(".cache/plannotator-tui/pi").join(format!("{pid}.sock"));
    let mut stream = UnixStream::connect(&socket_path).map_err(|error| {
        extension_error(format!("cannot connect to Pi session socket {}: {error}", socket_path.display()))
    })?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| extension_error("Pi session socket read timeout is too large"))?;

    // Read through EOF so a truncated or multi-line response cannot be mistaken for a snapshot.
    // The monotonic deadline keeps an endpoint from extending the timeout by trickling bytes.
    // Polling avoids a Darwin race: SO_RCVTIMEO can return EINVAL if the peer closes after connect.
    let mut response = Vec::with_capacity(256);
    let mut chunk = [0; 1024];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        anyhow::ensure!(
            !remaining.is_zero(),
            "{}",
            extension_error(format!("could not read Pi session socket {}: timed out", socket_path.display()))
        );
        let poll_timeout = Timespec::try_from(remaining)
            .map_err(|_| extension_error("Pi session socket read timeout is too large"))?;
        {
            let mut poll_fds = [PollFd::new(&stream, PollFlags::IN)];
            match poll(&mut poll_fds, Some(&poll_timeout)) {
                Ok(0) | Err(Errno::INTR) => continue,
                Ok(_) => {
                    anyhow::ensure!(
                        poll_fds.iter().any(|entry| {
                            entry.revents().intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR)
                        }),
                        "{}",
                        extension_error(format!(
                            "Pi session socket {} became invalid",
                            socket_path.display()
                        ))
                    );
                }
                Err(error) => {
                    return Err(extension_error(format!(
                        "could not wait for Pi session socket {}: {error}",
                        socket_path.display()
                    )));
                }
            }
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "{}",
            extension_error(format!("could not read Pi session socket {}: timed out", socket_path.display()))
        );
        match stream.read(&mut chunk) {
            Ok(count) => {
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "{}",
                    extension_error(format!(
                        "could not read Pi session socket {}: timed out",
                        socket_path.display()
                    ))
                );
                if count == 0 {
                    break;
                }
                anyhow::ensure!(
                    response.len().saturating_add(count) <= MAX_RESPONSE_BYTES,
                    "{}",
                    extension_error("Pi session endpoint response exceeded the size limit")
                );
                response.extend(chunk.iter().take(count).copied());
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(extension_error(format!(
                    "could not read Pi session socket {}: {error}",
                    socket_path.display()
                )));
            }
        }
    }

    let Some(json) = response.strip_suffix(b"\n") else {
        anyhow::bail!(
            "{}",
            extension_error("Pi session endpoint did not send a newline-terminated snapshot")
        );
    };
    anyhow::ensure!(
        !json.contains(&b'\n') && !json.contains(&b'\r'),
        "{}",
        extension_error("Pi session endpoint returned more than one line")
    );
    parse_snapshot(json, pid)
}

#[cfg(unix)]
fn parse_snapshot(json: &[u8], expected_pid: u32) -> Result<PiSession> {
    let value: Value = serde_json::from_slice(json)
        .map_err(|error| extension_error(format!("Pi session endpoint returned invalid JSON: {error}")))?;
    anyhow::ensure!(value.is_object(), "{}", extension_error("Pi session snapshot must be a JSON object"));
    anyhow::ensure!(
        value.get("version").and_then(Value::as_u64) == Some(1),
        "{}",
        extension_error("Pi session snapshot has an unsupported protocol version")
    );
    anyhow::ensure!(
        value.get("pid").and_then(Value::as_u64) == Some(u64::from(expected_pid)),
        "{}",
        extension_error(format!("Pi session snapshot PID does not match foreground PID {expected_pid}"))
    );

    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|id| valid_identifier(id))
        .map(str::to_owned)
        .ok_or_else(|| extension_error("Pi session snapshot has an empty or invalid session_id"))?;
    let session_path = value
        .get("session_path")
        .and_then(Value::as_str)
        .filter(|path| !path.contains('\0') && std::path::Path::new(path).is_absolute())
        .map(PathBuf::from)
        .ok_or_else(|| {
            extension_error(
                "Pi session has no absolute persisted session_path; save the session before reviewing it",
            )
        })?;
    let leaf_id = match value.get("leaf_id") {
        Some(Value::Null) => None,
        Some(Value::String(id)) if valid_identifier(id) => Some(id.clone()),
        _ => return Err(extension_error("Pi session snapshot leaf_id must be null or a nonempty valid ID")),
    };

    Ok(PiSession { session_id, session_path, leaf_id })
}

#[cfg(unix)]
fn valid_identifier(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn extension_error(reason: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!(
        "{reason}. Install and enable the Plannotator Pi extension with `pi install git:github.com/plannotator/plannotator-tui`, then restart Pi or run /reload."
    )
}

#[cfg(all(test, unix))]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write as _;

    #[cfg(unix)]
    use std::os::unix::net::UnixListener;

    #[cfg(unix)]
    struct Scratch(PathBuf);

    #[cfg(unix)]
    impl Scratch {
        fn new(label: &str) -> Self {
            let home = PathBuf::from("/tmp").join(format!("ptpi-{label}-{}", std::process::id()));
            std::fs::create_dir_all(&home).expect("create test home");
            Self(home)
        }

        fn home(&self) -> &std::path::Path {
            &self.0
        }
    }

    #[cfg(unix)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn listener(home: &std::path::Path, pid: u32) -> UnixListener {
        let path = home.join(".cache/plannotator-tui/pi").join(format!("{pid}.sock"));
        std::fs::create_dir_all(path.parent().expect("socket parent")).expect("create socket parent");
        UnixListener::bind(path).expect("bind Pi endpoint")
    }

    #[cfg(unix)]
    fn snapshot(pid: u32) -> String {
        serde_json::json!({
            "version": 1,
            "pid": pid,
            "session_id": "session-1",
            "session_path": "/sessions/session-1.jsonl",
            "leaf_id": "leaf-1"
        })
        .to_string()
            + "\n"
    }

    #[cfg(unix)]
    #[test]
    fn reads_the_live_snapshot_for_the_foreground_pid() {
        let scratch = Scratch::new("valid");
        let pid = 73139;
        let listener = listener(scratch.home(), pid);
        let response = snapshot(pid);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept client");
            stream.write_all(response.as_bytes()).expect("write snapshot");
        });

        let session =
            read_from_home(scratch.home(), pid, std::time::Duration::from_secs(1)).expect("snapshot");
        assert_eq!(session.session_id, "session-1");
        assert_eq!(session.session_path, PathBuf::from("/sessions/session-1.jsonl"));
        assert_eq!(session.leaf_id.as_deref(), Some("leaf-1"));
        server.join().expect("server thread");
    }
    #[test]
    fn reads_a_delayed_snapshot_before_the_total_timeout() {
        let scratch = Scratch::new("delayed");
        let pid = 73139;
        let listener = listener(scratch.home(), pid);
        let response = snapshot(pid);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept client");
            std::thread::sleep(std::time::Duration::from_millis(50));
            stream.write_all(response.as_bytes()).expect("write snapshot");
        });

        let session =
            read_from_home(scratch.home(), pid, std::time::Duration::from_secs(1)).expect("snapshot");
        assert_eq!(session.session_id, "session-1");
        server.join().expect("server thread");
    }

    #[cfg(unix)]
    #[test]
    fn a_snapshot_must_name_the_foreground_pid() {
        let scratch = Scratch::new("pid");
        let requested_pid = 73139;
        let listener = listener(scratch.home(), requested_pid);
        let response = snapshot(requested_pid + 1);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept client");
            stream.write_all(response.as_bytes()).expect("write snapshot");
        });

        let error = read_from_home(scratch.home(), requested_pid, std::time::Duration::from_secs(1))
            .expect_err("reject the mismatched process");
        assert!(error.to_string().contains("does not match foreground PID"), "{error:#}");
        assert!(error.to_string().contains("Install and enable the Plannotator Pi extension"), "{error:#}");
        server.join().expect("server thread");
    }

    #[cfg(unix)]
    #[test]
    fn repeatedly_reads_snapshots_from_peers_that_close_immediately() {
        let scratch = Scratch::new("immediate-close");
        let pid = 73139;
        let listener = listener(scratch.home(), pid);
        let response = snapshot(pid);
        let server = std::thread::spawn(move || {
            for _ in 0..64 {
                let (mut stream, _) = listener.accept().expect("accept client");
                stream.write_all(response.as_bytes()).expect("write snapshot");
            }
        });

        for _ in 0..64 {
            let session = read_from_home(scratch.home(), pid, std::time::Duration::from_secs(1))
                .expect("snapshot from peer that closes immediately");
            assert_eq!(session.session_id, "session-1");
        }
        server.join().expect("server thread");
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_pid_socket_fails_with_extension_install_guidance() {
        let scratch = Scratch::new("missing");
        let error = read_from_home(scratch.home(), 73139, std::time::Duration::from_millis(50))
            .expect_err("missing endpoint must fail");
        assert!(error.to_string().contains("cannot connect to Pi session socket"), "{error:#}");
        assert!(error.to_string().contains("Install and enable the Plannotator Pi extension"), "{error:#}");
    }

    #[cfg(unix)]
    #[test]
    fn a_silent_endpoint_times_out_and_fails_closed() {
        let scratch = Scratch::new("timeout");
        let pid = 73139;
        let listener = listener(scratch.home(), pid);
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept client");
            std::thread::sleep(std::time::Duration::from_millis(100));
        });

        let error = read_from_home(scratch.home(), pid, std::time::Duration::from_millis(20))
            .expect_err("silent endpoint must time out");
        assert!(error.to_string().contains("could not read Pi session socket"), "{error:#}");
        assert!(error.to_string().contains("Install and enable the Plannotator Pi extension"), "{error:#}");
        server.join().expect("server thread");
    }

    #[cfg(unix)]
    #[test]
    fn a_trickling_endpoint_cannot_extend_the_total_read_timeout() {
        let scratch = Scratch::new("trickle");
        let pid = 73139;
        let listener = listener(scratch.home(), pid);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept client");
            for _ in 0..8 {
                if stream.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        let error = read_from_home(scratch.home(), pid, std::time::Duration::from_millis(35))
            .expect_err("a slow response must exceed the total timeout");
        assert!(error.to_string().contains("could not read Pi session socket"), "{error:#}");
        server.join().expect("server thread");
    }

    #[cfg(unix)]
    #[test]
    fn a_response_over_the_read_limit_is_rejected() {
        let scratch = Scratch::new("large");
        let pid = 73139;
        let listener = listener(scratch.home(), pid);
        let response = vec![b'x'; MAX_RESPONSE_BYTES + 1];
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept client");
            stream.write_all(&response).expect("write oversized response");
        });

        let error = read_from_home(scratch.home(), pid, std::time::Duration::from_secs(1))
            .expect_err("oversized response must fail");
        assert!(error.to_string().contains("exceeded the size limit"), "{error:#}");
        server.join().expect("server thread");
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_fields_must_match_protocol_and_name_a_persisted_session() {
        let valid = serde_json::json!({
            "version": 1,
            "pid": 42,
            "session_id": "s",
            "session_path": "/s.jsonl",
            "leaf_id": null
        });
        let parsed = parse_snapshot(valid.to_string().as_bytes(), 42).expect("nullable leaf is valid");
        assert_eq!(parsed.leaf_id, None);

        for invalid in [
            serde_json::json!({"version": 2, "pid": 42, "session_id": "s", "session_path": "/s", "leaf_id": null}),
            serde_json::json!({"version": 1, "pid": 42, "session_id": " ", "session_path": "/s", "leaf_id": null}),
            serde_json::json!({"version": 1, "pid": 42, "session_id": "s", "session_path": null, "leaf_id": null}),
            serde_json::json!({"version": 1, "pid": 42, "session_id": "s", "session_path": "relative.jsonl", "leaf_id": null}),
            serde_json::json!({"version": 1, "pid": 42, "session_id": "s", "session_path": "/s", "leaf_id": ""}),
        ] {
            let error = parse_snapshot(invalid.to_string().as_bytes(), 42).expect_err("invalid snapshot");
            assert!(
                error.to_string().contains("Install and enable the Plannotator Pi extension"),
                "{error:#}"
            );
        }
    }
}
