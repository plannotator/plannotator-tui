//! Laying out many blocks at once: across the machine's cores, and on a thread of its own
//! for the part of a long document below the first screen. Blocks are independent, so the
//! work splits by block and the results join back in document order.

use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use super::RenderedBlock;
use super::worker_panic::{as_layout_thread, carry_on};
use crate::doc::Document;

/// Fewest blocks worth a thread of their own; below this, starting one costs more.
const MIN_CHUNK: usize = 64;

/// Run `work` on every item, split across the machine's cores. Falls back to the calling
/// thread for short slices, or when a thread cannot be started.
pub(super) fn each_in_parallel<T: Send>(items: &mut [T], work: impl Fn(&mut T) + Sync) {
    let threads = thread::available_parallelism().map_or(1, NonZeroUsize::get);
    let chunk = items.len().div_ceil(threads).max(MIN_CHUNK);
    if items.len() <= chunk {
        items.iter_mut().for_each(work);
        return;
    }
    let work = &work;
    let (unstarted, panic) = thread::scope(|scope| {
        let mut threads = Vec::new();
        let mut unstarted = Vec::new();
        for (index, part) in items.chunks_mut(chunk).enumerate() {
            match thread::Builder::new()
                .spawn_scoped(scope, move || as_layout_thread(|| part.iter_mut().for_each(work)))
            {
                Ok(thread) => threads.push(thread),
                Err(_) => unstarted.push(index),
            }
        }
        // Joined here, a worker's panic is not reported a second time by `scope`.
        let panics: Vec<_> = threads.into_iter().filter_map(|thread| thread.join().err()).collect();
        (unstarted, panics.into_iter().next())
    });
    if let Some(payload) = panic {
        carry_on(payload);
    }
    for index in unstarted {
        if let Some(part) = items.chunks_mut(chunk).nth(index) {
            part.iter_mut().for_each(work);
        }
    }
}

/// Render and wrap blocks `range` of `doc` for `width`, in document order.
pub(super) fn lay_out(doc: &Document, range: Range<usize>, width: usize) -> Vec<RenderedBlock> {
    let mut slots: Vec<(usize, Option<RenderedBlock>)> = range.map(|index| (index, None)).collect();
    each_in_parallel(&mut slots, |(index, block)| *block = Some(RenderedBlock::new(doc, *index, width)));
    slots.into_iter().filter_map(|(_, block)| block).collect()
}

/// Blocks `first..` of a document, being laid out for `width` on a background thread.
#[derive(Debug)]
pub(super) struct Rest {
    doc: Arc<Document>,
    first: usize,
    width: usize,
    /// `None` when no thread could be started; [`Rest::join`] then does the work itself.
    thread: Option<JoinHandle<Vec<RenderedBlock>>>,
}

impl Rest {
    pub(super) fn spawn(doc: &Document, first: usize, width: usize) -> Self {
        let doc = Arc::new(doc.clone());
        let shared = Arc::clone(&doc);
        let thread = thread::Builder::new()
            .name("layout".to_owned())
            .spawn(move || as_layout_thread(|| lay_out(&shared, first..shared.blocks.len(), width)))
            .ok();
        Self { doc, first, width, thread }
    }

    /// Whether [`Rest::join`] would return without waiting.
    pub(super) fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// The laid-out blocks and the width they were wrapped for. Waits for the thread, or
    /// lays them out here when no thread could be started.
    ///
    /// A panic on the thread continues on this one (`worker_panic`), so the app ends as on
    /// a panic of its own; laying the same blocks out again would only panic a second time.
    pub(super) fn join(self) -> (Vec<RenderedBlock>, usize) {
        let blocks = match self.thread.map(JoinHandle::join) {
            Some(Ok(blocks)) => blocks,
            Some(Err(payload)) => carry_on(payload),
            None => lay_out(&self.doc, self.first..self.doc.blocks.len(), self.width),
        };
        (blocks, self.width)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, reason = "the tests are about a panicking worker")]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn message(payload: &(dyn std::any::Any + Send)) -> String {
        payload.downcast_ref::<String>().cloned().unwrap_or_default()
    }

    /// A panic while laying out the rest reaches the thread that waits for it as a panic of
    /// its own, with the message; the blocks are not laid out a second time.
    #[test]
    fn a_panic_on_the_layout_thread_continues_on_the_waiting_thread() {
        let doc = Document::parse("one\n\ntwo\n".to_owned());
        let rest = Rest {
            doc: Arc::new(doc),
            first: 0,
            width: 40,
            thread: Some(thread::spawn(|| panic!("layout failed"))),
        };
        let payload = catch_unwind(AssertUnwindSafe(|| rest.join())).expect_err("the panic carries on");
        assert_eq!(message(payload.as_ref()), "layout thread layout failed");
    }

    /// A worker's panic leaves `each_in_parallel` as one panic with its message, not as
    /// `scope`'s own "a scoped thread panicked", and the other workers still finish.
    #[test]
    fn a_panicking_worker_ends_each_in_parallel_with_its_own_panic() {
        let visited = AtomicUsize::new(0);
        let mut items: Vec<usize> = (0..MIN_CHUNK * 4).collect();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            each_in_parallel(&mut items, |item| {
                assert!(*item != MIN_CHUNK + 1, "block failed");
                visited.fetch_add(1, Ordering::Relaxed);
            });
        }));
        let payload = outcome.expect_err("the panic carries on");
        assert_eq!(message(payload.as_ref()), "layout thread block failed");
        if thread::available_parallelism().map_or(1, NonZeroUsize::get) > 1 {
            assert!(visited.load(Ordering::Relaxed) > MIN_CHUNK * 2);
        }
    }
}
