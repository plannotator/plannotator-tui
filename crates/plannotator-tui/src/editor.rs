//! The user's own editor, for changing the reviewed file by hand: which one `$VISUAL` or
//! `$EDITOR` names, and the command that opens a file in it. The viewer hands the terminal
//! over while it runs (see `cli::run_editor`) and reloads the file when it exits.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Editors that open a file at a line when it is preceded by `+N`.
const PLUS_LINE: &[&str] =
    &["vi", "vim", "nvim", "gvim", "nano", "hx", "helix", "emacs", "emacsclient", "micro", "kak"];

/// A request to open `path` in `editor` (the raw `$VISUAL`/`$EDITOR` value) at `line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditorRequest {
    pub(crate) editor: String,
    pub(crate) path: PathBuf,
    /// 1-based line of the document the reviewer was on.
    pub(crate) line: usize,
}

/// `$VISUAL`, then `$EDITOR`; a blank value counts as unset.
pub(crate) fn from_env(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["VISUAL", "EDITOR"].into_iter().filter_map(var).find(|value| !value.trim().is_empty())
}

/// Split an editor value into words the way a shell would for the simple cases people
/// write: whitespace separates, single and double quotes group, and a backslash escapes a
/// quote, a backslash or a space. Any other backslash is literal, so a Windows path keeps
/// its separators. `None` when the value is empty or a quote is left open.
pub(crate) fn split(value: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('\''), c) => word.get_or_insert_default().push(c),
            (_, '\\') if chars.peek().is_some_and(|next| matches!(next, '"' | '\'' | '\\' | ' ' | '\t')) => {
                word.get_or_insert_default().extend(chars.next());
            }
            (None, '"' | '\'') => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            (None, c) if c.is_whitespace() => words.extend(word.take()),
            (_, c) => word.get_or_insert_default().push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    words.extend(word);
    (!words.is_empty()).then_some(words)
}

/// The program's name without its directory or a Windows extension, for display and for
/// telling which editors take `+N`.
pub(crate) fn program_name(program: &str) -> String {
    let name = Path::new(program).file_name().map_or_else(|| program.into(), |n| n.to_string_lossy());
    let lower = name.to_lowercase();
    lower.strip_suffix(".exe").or_else(|| lower.strip_suffix(".cmd")).unwrap_or(&lower).to_owned()
}

/// The program and arguments that open the request's file: the editor's own words, `+N`
/// when the editor understands it, then the path. `None` when the editor value is empty
/// or unbalanced.
pub(crate) fn argv(request: &EditorRequest) -> Option<Vec<String>> {
    let mut words = split(&request.editor)?;
    let program = words.first().map(|program| program_name(program))?;
    if PLUS_LINE.contains(&program.as_str()) {
        words.push(format!("+{}", request.line.max(1)));
    }
    words.push(request.path.to_string_lossy().into_owned());
    Some(words)
}

/// The command to run, inheriting this terminal's stdin, stdout and stderr.
pub(crate) fn command(request: &EditorRequest) -> Option<Command> {
    let words = argv(request)?;
    let (program, args) = words.split_first()?;
    let mut command = Command::new(program);
    command.args(args);
    Some(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(editor: &str, line: usize) -> EditorRequest {
        EditorRequest { editor: editor.into(), path: PathBuf::from("/notes/plan.md"), line }
    }

    fn owned(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn visual_wins_over_editor_and_a_blank_value_is_unset() {
        let env = |visual: Option<&str>, editor: Option<&str>| {
            from_env(|key| match key {
                "VISUAL" => visual.map(str::to_owned),
                "EDITOR" => editor.map(str::to_owned),
                _ => None,
            })
        };
        assert_eq!(env(Some("code -w"), Some("vim")).as_deref(), Some("code -w"));
        assert_eq!(env(None, Some("vim")).as_deref(), Some("vim"));
        assert_eq!(env(Some("  "), Some("nano")).as_deref(), Some("nano"));
        assert_eq!(env(None, Some("")), None);
        assert_eq!(env(None, None), None);
    }

    #[test]
    fn an_editor_value_splits_into_words_with_quotes_grouping() {
        assert_eq!(split("code -w"), Some(owned(&["code", "-w"])));
        assert_eq!(split("  nvim   -u x  "), Some(owned(&["nvim", "-u", "x"])));
        assert_eq!(
            split("'/Applications/My Editor/bin/ed' --wait"),
            Some(owned(&["/Applications/My Editor/bin/ed", "--wait"]))
        );
        assert_eq!(
            split(r#""C:\Program Files\Notepad++\notepad++.exe" -multiInst"#),
            Some(owned(&[r"C:\Program Files\Notepad++\notepad++.exe", "-multiInst"]))
        );
        assert_eq!(split(r"my\ editor -f"), Some(owned(&["my editor", "-f"])));
        assert_eq!(split(r#"vim -c "set tw=0""#), Some(owned(&["vim", "-c", "set tw=0"])));
        assert_eq!(split(r#"ed """#), Some(owned(&["ed", ""])));
    }

    #[test]
    fn an_empty_or_unbalanced_value_is_no_editor() {
        assert_eq!(split(""), None);
        assert_eq!(split("   "), None);
        assert_eq!(split("vim 'unclosed"), None);
        assert_eq!(argv(&request("\"code", 3)), None);
    }

    #[test]
    fn editors_that_take_plus_n_open_at_the_line() {
        assert_eq!(
            argv(&request("nvim -u x", 12)),
            Some(owned(&["nvim", "-u", "x", "+12", "/notes/plan.md"]))
        );
        for editor in
            ["vim", "/usr/bin/vi", "nano", "hx", "emacs -nw", "emacsclient -t", "micro", "kak", "NVIM.EXE"]
        {
            let argv = argv(&request(editor, 7)).unwrap_or_default();
            assert!(argv.contains(&"+7".to_owned()), "{editor}: {argv:?}");
        }
    }

    #[test]
    fn other_editors_get_the_path_alone() {
        assert_eq!(argv(&request("code -w", 12)), Some(owned(&["code", "-w", "/notes/plan.md"])));
        assert_eq!(
            argv(&request("subl --wait", 5)).and_then(|argv| argv.last().cloned()).as_deref(),
            Some("/notes/plan.md")
        );
        // A name that merely contains `vim` is not vim.
        assert!(!argv(&request("vimdiff-wrapper", 5)).unwrap_or_default().contains(&"+5".to_owned()));
    }

    #[test]
    fn the_line_is_never_zero() {
        assert!(argv(&request("vim", 0)).unwrap_or_default().contains(&"+1".to_owned()));
    }
}
