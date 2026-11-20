//! Laying out many blocks at once: across the machine's cores, and on a thread of its own
//! for the part of a long document below the first screen. Blocks are independent, so the
//! work splits by block and the results join back in document order.

use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use super::RenderedBlock;
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
    let unstarted: Vec<usize> = thread::scope(|scope| {
        let mut unstarted = Vec::new();
        for (index, part) in items.chunks_mut(chunk).enumerate() {
            if thread::Builder::new().spawn_scoped(scope, move || part.iter_mut().for_each(work)).is_err() {
                unstarted.push(index);
            }
        }
        unstarted
    });
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
            .spawn(move || lay_out(&shared, first..shared.blocks.len(), width))
            .ok();
        Self { doc, first, width, thread }
    }

    /// The laid-out blocks and the width they were wrapped for. Waits for the thread; if it
    /// failed, lays them out here instead.
    pub(super) fn join(self) -> (Vec<RenderedBlock>, usize) {
        let all = self.first..self.doc.blocks.len();
        let blocks = match self.thread.map(JoinHandle::join) {
            Some(Ok(blocks)) => blocks,
            Some(Err(_)) | None => lay_out(&self.doc, all, self.width),
        };
        (blocks, self.width)
    }
}
