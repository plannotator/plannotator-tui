//! Which changed files the list and the diff show: a name filter typed after `/`, the kinds
//! picked with `t`, and whether viewed files are hidden (`H`).

use super::kind::{KINDS, Kind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Filter {
    /// Words the path must contain, any case.
    pub(crate) query: String,
    /// Whether each kind shows, by [`Kind::index`].
    pub(crate) kinds: [bool; KINDS.len()],
    pub(crate) hide_viewed: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Self { query: String::new(), kinds: [true; KINDS.len()], hide_viewed: false }
    }
}

impl Filter {
    /// Whether the filter hides anything at all.
    pub(crate) fn is_active(&self) -> bool {
        !self.query.trim().is_empty() || !self.all_kinds() || self.hide_viewed
    }

    pub(crate) fn all_kinds(&self) -> bool {
        self.kinds.iter().all(|shown| *shown)
    }

    pub(crate) fn shows(&self, kind: Kind) -> bool {
        self.kinds.get(kind.index()).copied().unwrap_or(true)
    }

    /// The kinds shown, by name, when some are not: `Code, Tests`.
    pub(crate) fn kinds_label(&self) -> Option<String> {
        if self.all_kinds() {
            return None;
        }
        let names: Vec<&str> = KINDS.iter().filter(|k| self.shows(**k)).map(|k| k.label()).collect();
        Some(if names.is_empty() { "no kinds".to_owned() } else { names.join(", ") })
    }

    /// Which of `paths` pass: their kind is shown, they are not hidden as viewed, and they
    /// match the query.
    pub(crate) fn keep(&self, paths: &[&str], kinds: &[Kind], viewed: impl Fn(&str) -> bool) -> Vec<bool> {
        let named = matches(paths, &self.query);
        paths
            .iter()
            .zip(kinds)
            .zip(named)
            .map(|((path, kind), named)| named && self.shows(*kind) && !(self.hide_viewed && viewed(path)))
            .collect()
    }
}

/// Which of `paths` `query` matches. Every word of it must be in the path, any case; when
/// no path has them all, each word's characters in order will do (`rvwdraw` finds
/// `review/draw.rs`), as the document's quick open matches names. An empty query matches
/// every path.
pub(crate) fn matches(paths: &[&str], query: &str) -> Vec<bool> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return vec![true; paths.len()];
    }
    let lower: Vec<String> = paths.iter().map(|p| p.to_lowercase()).collect();
    let exact: Vec<bool> = lower.iter().map(|p| words.iter().all(|w| p.contains(w.as_str()))).collect();
    if exact.iter().any(|hit| *hit) {
        return exact;
    }
    lower.iter().map(|p| words.iter().all(|w| crate::file_search::rank(p, w).is_some())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATHS: [&str; 4] = [
        "crates/plannotator-tui/src/review/draw.rs",
        "crates/plannotator-tui/src/review/tests/look.rs",
        "packages/review-editor/components/ReviewAgentJobDetailPanel.tsx",
        "Cargo.lock",
    ];

    fn kept(filter: &Filter, viewed: &[&str]) -> Vec<&'static str> {
        let kinds: Vec<Kind> = PATHS.iter().map(|p| super::super::kind::classify(p)).collect();
        let keep = filter.keep(&PATHS, &kinds, |p| viewed.contains(&p));
        PATHS.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
    }

    #[test]
    fn every_word_must_be_in_the_path_in_any_case() {
        assert_eq!(matches(&PATHS, "REVIEW"), [true, true, true, false]);
        assert_eq!(matches(&PATHS, "review draw"), [true, false, false, false]);
        assert_eq!(matches(&PATHS, "  "), [true; 4]);
    }

    #[test]
    fn characters_in_order_match_only_when_no_path_has_the_words() {
        assert_eq!(matches(&PATHS, "rvwdraw"), [true, false, false, false]);
        // `panel` is in one path, so the looser match does not add others.
        assert_eq!(matches(&PATHS, "panel"), [false, false, true, false]);
        assert_eq!(matches(&PATHS, "zzz"), [false; 4]);
    }

    #[test]
    fn kinds_viewed_marks_and_the_query_all_apply() {
        let mut filter = Filter::default();
        assert!(!filter.is_active());
        assert_eq!(kept(&filter, &[]), PATHS);
        for kind in [Kind::Generated, Kind::Test] {
            if let Some(shown) = filter.kinds.get_mut(kind.index()) {
                *shown = false;
            }
        }
        assert_eq!(filter.kinds_label().as_deref(), Some("Code, Docs, Config, Other"));
        assert_eq!(kept(&filter, &[]), [PATHS[0], PATHS[2]]);
        filter.hide_viewed = true;
        assert_eq!(kept(&filter, &[PATHS[0]]), [PATHS[2]]);
        filter.query = "draw".into();
        assert!(filter.is_active());
        assert_eq!(kept(&filter, &[]), [PATHS[0]]);
    }
}
