//! A reply review that has opened more than one reply is still one review. Each reply
//! keeps its notes in memory (the open one in `open`, the rest in `pick_cache`), and
//! sending, counting and the quit question cover all of them.

use super::feedback::{Feedback, SendScope};
use super::{App, Open};
use crate::store::Store;

impl App {
    /// Every reply this review has opened, with its candidate index, in picker order.
    /// A file, folder or single-document review has just the open document.
    pub(super) fn replies(&self) -> Vec<(usize, &Open)> {
        let mut replies: Vec<(usize, &Open)> = std::iter::once((self.pick_open, &self.open))
            .chain(self.pick_cache.iter().map(|(index, open)| (*index, open)))
            .collect();
        replies.sort_by_key(|(index, _)| *index);
        replies
    }

    pub(super) fn reply_store_mut(&mut self, index: usize) -> Option<&mut Store> {
        if index == self.pick_open {
            Some(&mut self.open.store)
        } else {
            self.pick_cache.get_mut(&index).map(|open| &mut open.store)
        }
    }

    /// Notes from every reply that has any. One such reply is sent exactly as a review
    /// that never used the picker; two or more are sent one block per reply, each
    /// headed with which reply it is so the agent can tell them apart. A reply that is
    /// not open and whose notes were all delivered already is left out, so sending from
    /// another reply does not repeat it.
    pub(super) fn reply_feedback(&self, scope: SendScope) -> Feedback {
        let annotated: Vec<(usize, &Open)> = self
            .replies()
            .into_iter()
            .filter(|(_, open)| !open.store.placed().is_empty())
            .filter(|(index, open)| *index == self.pick_open || !open.store.all_delivered())
            .collect();
        let mut feedback = Feedback::default();
        let named = annotated.len() > 1;
        for (index, open) in annotated {
            let name = if named { self.reply_name(index, open) } else { open.source.name.clone() };
            feedback.add(None, Some(index), &name, &open.doc, open.store.clone(), scope);
        }
        feedback
    }

    /// `claude · message 2 of 3 ("first line")`, numbered as the picker lists them.
    fn reply_name(&self, index: usize, open: &Open) -> String {
        let host = &self.message_host;
        let total = self.candidates.len();
        let first = open
            .doc
            .source
            .lines()
            .map(|line| line.trim().trim_start_matches('#').trim())
            .find(|line| !line.is_empty())
            .unwrap_or("");
        let first: String = if first.chars().count() > 60 {
            first.chars().take(59).chain(std::iter::once('…')).collect()
        } else {
            first.to_owned()
        };
        format!("{host} · message {} of {total} (\"{first}\")", index + 1)
    }
}
