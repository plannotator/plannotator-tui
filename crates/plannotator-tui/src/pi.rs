//! The Pi link (`docs/spec-pi-link.md`): find running Pi sessions, send review feedback to
//! one, and take notes a Pi session leaves on the open document.
//!
//! Both sides list themselves in a registry folder and answer one JSON line over loopback
//! TCP, checked against a per-process token. The Pi side is `integrations/pi/`.

use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) mod cli;
pub(crate) mod install;
pub(crate) mod link;
pub(crate) mod target;

/// The wire and registry version this build speaks.
pub(crate) const VERSION: u64 = 1;
/// The longest request or reply line either side reads.
const MAX_LINE: u64 = 1 << 20;
/// A ping is a liveness probe; a dead session must not hold up the picker.
const PING_TIMEOUT: Duration = Duration::from_millis(800);
/// Connecting to a closed loopback port fails at once on macOS and Linux; Windows retries
/// for about two seconds before it reports the refusal. A live listener accepts at once,
/// so only a stale entry ever waits this long, and it is deleted after.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// A send waits for Pi to accept the message, which it does before any model call.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// The registry root: `PLANNOTATOR_TUI_SESSIONS_DIR`, else
/// `<data dir>/clients/plannotator-tui/sessions`, beside this tool's notes. Plannotator
/// keeps its own live sessions in `<data dir>/sessions`, so that name is taken.
pub(crate) fn root() -> PathBuf {
    root_from(std::env::var("PLANNOTATOR_TUI_SESSIONS_DIR").ok(), crate::workspace_paths::data_dir)
}

fn root_from(env: Option<String>, data_dir: impl FnOnce() -> PathBuf) -> PathBuf {
    match env.map(|dir| dir.trim().to_owned()).filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => data_dir().join("clients").join("plannotator-tui").join("sessions"),
    }
}

/// A Pi session as its registry entry describes it.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct PiEntry {
    pub(crate) v: u64,
    pub(crate) host: String,
    pub(crate) id: String,
    pub(crate) pid: u32,
    pub(crate) cwd: PathBuf,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) transcript: Option<PathBuf>,
    pub(crate) port: u16,
    pub(crate) token: String,
    #[serde(default)]
    pub(crate) busy: bool,
    #[serde(default)]
    pub(crate) updated: u64,
    /// The entry file this was read from.
    #[serde(skip)]
    pub(crate) path: PathBuf,
}

impl PiEntry {
    /// The session's name, or the first eight characters of its id.
    pub(crate) fn label(&self) -> String {
        match self.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
            Some(name) => name.to_owned(),
            None => self.id.chars().take(8).collect(),
        }
    }

    fn read(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut entry: Self = serde_json::from_str(&text).ok()?;
        (entry.v == VERSION && entry.host == "pi").then(|| {
            entry.path = path.to_path_buf();
            entry
        })
    }
}

/// Every Pi entry in the registry that parses, live or not, oldest first.
pub(crate) fn pi_entries(root: &Path) -> Vec<PiEntry> {
    let Ok(dir) = std::fs::read_dir(root.join("pi")) else { return Vec::new() };
    let mut entries: Vec<PiEntry> = dir
        .filter_map(Result::ok)
        .map(|item| item.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| PiEntry::read(&path))
        .collect();
    entries.sort_by_key(|entry| entry.updated);
    entries
}

/// The entry the Pi session with this pid wrote last.
pub(crate) fn entry_for_pid(root: &Path, pid: u32) -> Option<PiEntry> {
    pi_entries(root).into_iter().rev().find(|entry| entry.pid == pid)
}

/// Where a Pi session stands in its tree, from two more fields of its entry. `/tree` moves
/// Pi's leaf without writing to the transcript.
#[derive(Debug, Deserialize)]
pub(crate) struct TreePosition {
    /// The newest entry in the session when `leaf` was recorded. While it is still the
    /// transcript's newest entry, `leaf` is current.
    pub(crate) tip: String,
    /// Pi's current entry; `None` before the first.
    #[serde(default)]
    pub(crate) leaf: Option<String>,
}

/// The tree position the Pi session whose transcript is `transcript` recorded last, or
/// `None` when no entry names it or its extension records none.
pub(crate) fn tree_position(root: &Path, transcript: &Path) -> Option<TreePosition> {
    let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let wanted = real(transcript);
    let entry = pi_entries(root)
        .into_iter()
        .rev()
        .find(|entry| entry.transcript.as_deref().is_some_and(|path| real(path) == wanted))?;
    serde_json::from_str(&std::fs::read_to_string(&entry.path).ok()?).ok()
}

/// The Pi sessions that answer a ping, with their busy flag and name fresh from the
/// answer. An entry whose port refuses the connection is left behind by a session that
/// ended without cleaning up, and is deleted.
pub(crate) fn live_pi_sessions(root: &Path) -> Vec<PiEntry> {
    pi_entries(root)
        .into_iter()
        .filter_map(|mut entry| {
            match request(
                entry.port,
                &json!({"v": VERSION, "token": entry.token, "op": "ping"}),
                PING_TIMEOUT,
            ) {
                Ok(reply) if field(&reply, "ok") == true => {
                    entry.busy = field(&reply, "busy").as_bool().unwrap_or(entry.busy);
                    if let Some(name) = field(&reply, "name").as_str() {
                        entry.name = Some(name.to_owned());
                    }
                    Some(entry)
                }
                Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
                    let _ = std::fs::remove_file(&entry.path);
                    None
                }
                _ => None,
            }
        })
        .collect()
}

/// One request line to `127.0.0.1:<port>` and its one reply line.
pub(crate) fn request(port: u16, body: &Value, timeout: Duration) -> std::io::Result<Value> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT.max(timeout))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut line = serde_json::to_vec(body)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    let mut reply = String::new();
    BufReader::new((&stream).take(MAX_LINE)).read_line(&mut reply)?;
    serde_json::from_str(&reply).map_err(std::io::Error::other)
}

/// How Pi took a message it accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// Pi was idle and started a turn with it.
    Started,
    /// Pi was busy; the message waits in its queue and is read next.
    Queued,
}

/// Why a send to Pi did not land.
#[derive(Debug)]
pub(crate) enum SendError {
    /// The session has ended (its entry is gone or its port refused).
    Gone,
    /// Pi answered with an error, or the exchange failed part way.
    Failed(String),
}

/// Send `text` to the session `entry` names. A session that rewrote its entry (a new
/// token or port) is re-read once before giving up.
pub(crate) fn send(entry: &PiEntry, text: &str) -> Result<Disposition, SendError> {
    match send_once(entry, text) {
        Err(SendError::Failed(code)) if code.starts_with("bad_token") => {
            let fresh = PiEntry::read(&entry.path).ok_or(SendError::Gone)?;
            send_once(&fresh, text)
        }
        other => other,
    }
}

fn send_once(entry: &PiEntry, text: &str) -> Result<Disposition, SendError> {
    let body = json!({
        "v": VERSION,
        "token": entry.token,
        "op": "send",
        "text": text,
        "deliver": "auto",
        "source": "plannotator-tui",
    });
    let reply = match request(entry.port, &body, SEND_TIMEOUT) {
        Ok(reply) => reply,
        Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
            let _ = std::fs::remove_file(&entry.path);
            return Err(SendError::Gone);
        }
        Err(err) => return Err(SendError::Failed(format!("no answer from Pi: {err}"))),
    };
    if field(&reply, "ok") == true {
        return Ok(match field(&reply, "disposition").as_str() {
            Some("queued") => Disposition::Queued,
            _ => Disposition::Started,
        });
    }
    let code = field(&reply, "code").as_str().unwrap_or("failed");
    Err(SendError::Failed(match field(&reply, "message").as_str() {
        Some(message) => format!("{code}: {message}"),
        None => code.to_owned(),
    }))
}

/// `value[key]`, or `null` when it is not there.
pub(crate) fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// A fresh 32-byte token as 64 hex characters.
pub(crate) fn token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|err| std::io::Error::other(err.to_string()))?;
    Ok(bytes.iter().fold(String::with_capacity(64), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    }))
}

/// Write `value` to `path` through a temporary file in the same folder, so a reader never
/// sees half an entry. The folder is private to the user (0700) and so is the file (0600).
pub(crate) fn write_entry(path: &Path, value: &Value) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("entry has no folder"))?;
    create_private_dir(dir)?;
    let tmp = dir.join(format!(".{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("entry")));
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
    }
    std::fs::rename(&tmp, path)
}

/// Create `dir` and the registry root above it, each private to the user when this call
/// creates it. A folder that already exists keeps its permissions.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let root = dir.parent().unwrap_or(dir);
    for dir in [root, dir] {
        if dir.is_dir() {
            continue;
        }
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// Milliseconds since the epoch, for an entry's `updated`.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests;
