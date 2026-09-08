//! Select the exact-session or fallback discovery path and read its assistant messages.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use plannotator_tui_hosts::{Host, HostError, Message, Role, detect_host, sniff};
use plannotator_tui_schema::{DocumentSource, Provenance};

use super::fallback::Discovery;
use super::roots::Roots;
use super::{LastOptions, exact, fallback, readers};

pub(crate) struct Located {
    pub(crate) host: Host,
    /// The transcript file (Claude) or the newest thread file (Codex); for the label.
    pub(crate) transcript: PathBuf,
    /// The host-assigned session id: the one given, else the one the transcript's name
    /// carries unambiguously. Never a path.
    pub(crate) session_id: Option<String>,
    /// Assistant messages, newest first, at most `options.pick`.
    pub(crate) messages: Vec<Message>,
    /// How the transcript was chosen; the UI says so when nothing identified it exactly.
    pub(crate) discovery: Discovery,
}

pub(crate) fn locate(options: &LastOptions) -> Result<Located> {
    let session = options.session.as_deref().map(expand_home);
    let host = match &session {
        // A path without a host name: Herdr hands us the exact transcript for agents it
        // integrates, whatever their format. Its first lines say which reader applies.
        Some(path) if !host_named(options) => sniff(&head(path)?).map_or_else(|| host_for(options), Ok)?,
        _ => host_for(options)?,
    };
    let pick = options.pick.max(1);
    let roots = Roots::from_env();
    let (transcript, messages, discovery) = if let Some(path) = session {
        let (transcript, messages) = readers::explicit(host, &path, options.session_id.as_deref(), pick)?;
        (transcript, messages, Discovery::Exact)
    } else if let Some(id) = options.session_id.as_deref() {
        // Validation precedes cwd lookup and every resolver filesystem access.
        let id = plannotator_tui_hosts::validate_session_id(id)?;
        let cwd = agent_cwd()?;
        let exact = exact::resolve(host, id, &cwd, &roots)?;
        let (transcript, messages) = readers::exact(host, id, exact, pick)?;
        (transcript, messages, Discovery::Exact)
    } else {
        let cwd = agent_cwd()?;
        fallback::read(host, options, &cwd, &roots, pick)?
    };
    let messages: Vec<Message> =
        messages.into_iter().filter(|message| message.role == Role::Assistant).collect();
    if messages.is_empty() {
        bail!("transcript {} has no assistant messages yet", transcript.display());
    }
    let session_id =
        options.session_id.clone().or_else(|| plannotator_tui_hosts::session_id_of(host, &transcript));
    Ok(Located { host, transcript, session_id, messages, discovery })
}

/// Was a host named explicitly, by flag or by the launcher?
fn host_named(options: &LastOptions) -> bool {
    options.host.as_deref().is_some_and(|host| !host.trim().is_empty())
        || std::env::var("PLANNOTATOR_TUI_HOST").is_ok_and(|host| !host.trim().is_empty())
}

/// The first 64 KiB of a transcript, enough for `sniff`.
fn head(path: &Path) -> Result<String> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut bytes = vec![0u8; 64 * 1024];
    let count = file.read(&mut bytes).with_context(|| format!("reading {}", path.display()))?;
    bytes.truncate(count);
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// `~/x` as Herdr reports some session paths.
fn expand_home(path: &Path) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix("~/")) {
        Some(rest) => Roots::from_env().home.join(rest),
        None => path.to_path_buf(),
    }
}

fn host_for(options: &LastOptions) -> Result<Host> {
    let override_host = options.host.clone();
    let lookup = |key: &str| match key {
        "PLANNOTATOR_TUI_HOST" => override_host.clone().or_else(|| std::env::var(key).ok()),
        _ => std::env::var(key).ok(),
    };
    match detect_host(lookup) {
        Ok(host) => Ok(host),
        Err(HostError::Unsupported(name)) => {
            let supported: Vec<&str> = Host::ALL.iter().map(|host| host.label()).collect();
            bail!("{name} is not supported yet; supported hosts: {} (or use --stdin)", supported.join(", "))
        }
        Err(err) => Err(err.into()),
    }
}

/// The agent pane's cwd when the Herdr launcher set it, else our own.
fn agent_cwd() -> Result<PathBuf> {
    match std::env::var_os("PLANNOTATOR_TUI_CWD") {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => std::env::current_dir().context("current directory"),
    }
}

/// The agent pane's recent output as a document, when Herdr and a target pane are known.
/// Lossy (no markdown structure survives a terminal), so only a fallback.
pub(crate) fn screen_fallback(env: &crate::herdr::context::HerdrEnv) -> Option<DocumentSource> {
    let target = env.delivery_target()?;
    if !env.in_herdr {
        return None;
    }
    let output = Command::new(&env.bin)
        .args(["agent", "read", &target.pane, "--source", "recent-unwrapped", "--format", "text"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let text = String::from_utf8_lossy(&output.stdout).trim_end().to_owned();
    if text.is_empty() {
        return None;
    }
    let host = env.host.clone().or(target.agent).unwrap_or_else(|| "agent".to_owned());
    Some(DocumentSource::new(
        text,
        format!("{host} · screen"),
        true,
        Provenance::AgentMessage { host, session: None, message_id: None },
    ))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    /// One user turn and one assistant turn, the shape `claude::parse_messages` reads.
    fn transcript(dir: &Path) -> PathBuf {
        let path = dir.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"parentUuid":null,"isSidechain":false,"type":"user","#,
                r#""message":{"role":"user","content":"prompt"},"uuid":"u1","#,
                r#""timestamp":"2026-08-28T10:00:00.000Z"}"#,
                "\n",
                r#"{"parentUuid":"u1","isSidechain":false,"type":"assistant","#,
                r#""message":{"role":"assistant","content":[{"type":"text","text":"reply"}]},"#,
                r#""uuid":"s1","timestamp":"2026-08-28T10:01:00.000Z"}"#,
                "\n",
            ),
        )
        .expect("transcript");
        path
    }

    #[test]
    fn a_transcript_named_on_the_command_line_is_never_reported_as_a_guess() {
        let dir = std::env::temp_dir().join(format!("plannotator locate ü-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let options = LastOptions {
            host: Some("claude".to_owned()),
            session: Some(transcript(&dir)),
            pick: 25,
            ..LastOptions::default()
        };

        let located = locate(&options).expect("the named transcript is read");

        assert_eq!(located.discovery, Discovery::Exact);
        assert_eq!(located.messages.first().map(|m| m.text.as_str()), Some("reply"));
        assert_eq!(located.session_id, None, "`session.jsonl` is not a session id");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_uuid_named_transcript_yields_its_id_and_an_explicit_id_wins() {
        let dir = std::env::temp_dir().join(format!("plannotator locate id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let id = "01a04583-a848-7b21-a890-f3ed0c9fef05";
        let named = dir.join(format!("{id}.jsonl"));
        std::fs::copy(transcript(&dir), &named).expect("copy");
        let options = LastOptions {
            host: Some("claude".to_owned()),
            session: Some(named),
            pick: 25,
            ..LastOptions::default()
        };

        let located = locate(&options).expect("the named transcript is read");
        assert_eq!(located.session_id.as_deref(), Some(id));

        let explicit = LastOptions { session_id: Some("given-by-herdr".to_owned()), ..options };
        let located = locate(&explicit).expect("the named transcript is read");
        assert_eq!(located.session_id.as_deref(), Some("given-by-herdr"));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
