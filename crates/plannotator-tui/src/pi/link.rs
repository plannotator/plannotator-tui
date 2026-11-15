//! This plannotator-tui's side of the Pi link: its registry entry and the loopback
//! listener a Pi session sends notes to.
//!
//! The listener runs on its own thread and hands each note to the app through a channel;
//! the app answers when its event loop next polls (`App::poll_pi`), and the listener
//! writes that answer back. The app owns the document, so only it can say whether the
//! file is open and the quote is there.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::{MAX_LINE, VERSION, field};

/// How long the listener waits for the app to answer a note. The event loop polls every
/// 250 ms, and an editor or other blocking moment can hold it longer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a connected Pi gets to send its request line.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A note a Pi session asked to leave on a document.
#[derive(Debug)]
pub(crate) struct Note {
    pub(crate) file: PathBuf,
    pub(crate) quote: Option<String>,
    /// 1-based source line the note is about, when given.
    pub(crate) line: Option<usize>,
    pub(crate) body: String,
    pub(crate) author: String,
    reply: SyncSender<Value>,
}

impl Note {
    /// A note as the listener would hand it over, and the channel its answer arrives on.
    #[cfg(test)]
    pub(crate) fn for_test(
        file: PathBuf,
        quote: Option<&str>,
        line: Option<usize>,
        note: &str,
    ) -> (Self, Receiver<Value>) {
        let (reply, answer) = mpsc::sync_channel(1);
        let quote = quote.map(str::to_owned);
        (Self { file, quote, line, body: note.to_owned(), author: "pi · test".to_owned(), reply }, answer)
    }

    /// Answer the Pi session that sent this note. The reply is one of the spec's shapes.
    pub(crate) fn answer(self, reply: Value) {
        let _ = self.reply.send(reply);
    }
}

/// The registry entry and listener, for as long as the review runs. Dropping it removes
/// the entry; the listener thread ends with the process.
/// This process's entry while a link is open, so a hang-up, which exits without running
/// `Drop`, can still remove it.
static ENTRY: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Remove this process's registry entry now. The hang-up watch, unix only, calls it before
/// exiting.
#[cfg(unix)]
pub(crate) fn remove_entry() {
    if let Ok(mut entry) = ENTRY.lock()
        && let Some(path) = entry.take()
    {
        let _ = std::fs::remove_file(path);
    }
}

pub(crate) struct Link {
    entry: PathBuf,
    port: u16,
    token: Arc<str>,
    cwd: PathBuf,
    files: Mutex<Vec<PathBuf>>,
    notes: Receiver<Note>,
}

impl Link {
    /// Listen on loopback and list this process in `<root>/tui/<pid>.json`.
    pub(crate) fn start(root: &Path, cwd: PathBuf, files: Vec<PathBuf>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let token: Arc<str> = super::token()?.into();
        let (tx, notes) = mpsc::channel();
        let thread_token = Arc::clone(&token);
        std::thread::Builder::new()
            .name("pi-link".into())
            .spawn(move || serve(&listener, &thread_token, &tx))?;
        let link = Self {
            entry: root.join("tui").join(format!("{}.json", std::process::id())),
            port,
            token,
            cwd,
            files: Mutex::new(files),
            notes,
        };
        link.write()?;
        if let Ok(mut entry) = ENTRY.lock() {
            *entry = Some(link.entry.clone());
        }
        Ok(link)
    }

    /// Notes that arrived since the last call, oldest first.
    pub(crate) fn take_notes(&self) -> Vec<Note> {
        self.notes.try_iter().collect()
    }

    /// The documents this review can take notes for, open one first. Rewrites the entry
    /// when they changed.
    pub(crate) fn set_files(&self, files: Vec<PathBuf>) {
        let Ok(mut current) = self.files.lock() else { return };
        if *current == files {
            return;
        }
        *current = files;
        drop(current);
        let _ = self.write();
    }

    fn write(&self) -> std::io::Result<()> {
        let files = self.files.lock().map(|files| files.clone()).unwrap_or_default();
        super::write_entry(
            &self.entry,
            &json!({
                "v": VERSION,
                "host": "plannotator-tui",
                "pid": std::process::id(),
                "cwd": self.cwd,
                "port": self.port,
                "token": &*self.token,
                "files": files,
                "updated": super::now_ms(),
            }),
        )
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.entry);
        if let Ok(mut entry) = ENTRY.lock()
            && entry.as_ref() == Some(&self.entry)
        {
            *entry = None;
        }
    }
}

fn serve(listener: &TcpListener, token: &str, notes: &Sender<Note>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        // One connection at a time: notes are rare and each is answered within seconds.
        let reply = handle(&stream, token, notes);
        let mut line = serde_json::to_vec(&reply).unwrap_or_default();
        line.push(b'\n');
        let _ = (&stream).write_all(&line);
    }
}

fn handle(stream: &TcpStream, token: &str, notes: &Sender<Note>) -> Value {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
    let mut line = String::new();
    if BufReader::new(stream.take(MAX_LINE)).read_line(&mut line).is_err() {
        return error("bad_request", "could not read the request line");
    }
    let Ok(request) = serde_json::from_str::<Value>(&line) else {
        return error("bad_request", "the request is not JSON");
    };
    if field(&request, "token").as_str() != Some(token) {
        return error("bad_token", "wrong token");
    }
    match field(&request, "op").as_str() {
        Some("ping") => json!({"ok": true, "pid": std::process::id()}),
        Some("note") => note(&request, notes),
        _ => error("bad_request", "unknown op"),
    }
}

fn note(request: &Value, notes: &Sender<Note>) -> Value {
    let Some(file) = field(request, "file").as_str().map(PathBuf::from).filter(|file| file.is_absolute())
    else {
        return error("bad_request", "file must be an absolute path");
    };
    let body = field(request, "note").as_str().unwrap_or_default().trim().to_owned();
    if body.is_empty() {
        return error("empty", "the note is empty");
    }
    let quote = field(request, "quote").as_str().filter(|quote| !quote.is_empty()).map(str::to_owned);
    let line =
        field(request, "line").as_u64().and_then(|line| usize::try_from(line).ok()).filter(|&line| line > 0);
    let author = field(request, "author").as_str().map(str::trim).filter(|a| !a.is_empty()).unwrap_or("pi");
    let author: String = author.chars().take(120).collect();
    let (reply, answer) = mpsc::sync_channel(1);
    if notes.send(Note { file, quote, line, body, author, reply }).is_err() {
        return error("not_open", "this review has ended");
    }
    answer
        .recv_timeout(ANSWER_TIMEOUT)
        .unwrap_or_else(|_| error("busy", "plannotator-tui did not answer in time; try again"))
}

pub(crate) fn error(code: &str, message: &str) -> Value {
    json!({"ok": false, "code": code, "message": message})
}
