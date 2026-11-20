//! A panic on a layout thread ends the app the way a panic on the main thread does.
//!
//! ratatui's panic hook restores the terminal. Run on a layout thread, it would hand the
//! screen back while the main thread is still drawing. So the hook is kept off layout
//! threads: their panic is only noted, and the thread that waits for the layout panics
//! with that note, which runs the hook there, once.

use std::any::Any;
use std::cell::Cell;
use std::panic;
use std::sync::Mutex;

thread_local! {
    static LAYOUT_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// The first layout thread's panic, as the hook would have printed it.
static NOTED: Mutex<Option<String>> = Mutex::new(None);

/// Keep the current panic hook off layout threads. Install after any hook that must not
/// run there (ratatui's); installing again only adds a layer that changes nothing.
pub(crate) fn keep_hook_off_layout_threads() {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if LAYOUT_THREAD.with(Cell::get) {
            if let Ok(mut noted) = NOTED.lock() {
                noted.get_or_insert_with(|| info.to_string());
            }
        } else {
            hook(info);
        }
    }));
}

/// Run `work` as a layout thread.
pub(super) fn as_layout_thread<R>(work: impl FnOnce() -> R) -> R {
    LAYOUT_THREAD.with(|flag| flag.set(true));
    work()
}

/// Carry a layout thread's panic on to this thread: on as a layout thread, else as a
/// panic of this thread's own, so the hook runs here with the original message.
pub(super) fn carry_on(payload: Box<dyn Any + Send>) -> ! {
    if LAYOUT_THREAD.with(Cell::get) {
        panic::resume_unwind(payload);
    }
    let noted = NOTED.lock().ok().and_then(|mut noted| noted.take());
    let message = noted.unwrap_or_else(|| {
        let text = payload.downcast_ref::<&str>().copied();
        text.or_else(|| payload.downcast_ref::<String>().map(String::as_str)).unwrap_or("panicked").to_owned()
    });
    #[allow(clippy::panic, reason = "the layout thread panicked; the app ends as on its own panic")]
    {
        panic!("layout thread {message}")
    }
}
