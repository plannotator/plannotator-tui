//! Pieces of the screen that document reviews (`app/`) and code reviews (`review/`) share:
//! the comment box, the agent picker, the header's send button, the mouse drag's edge timer, the colours of the text we paint
//! on our own backgrounds, and the footer's key hints.

pub(crate) mod agent_picker;
pub(crate) mod chips;
pub(crate) mod compose;
pub(crate) mod drag;
pub(crate) mod hints;
pub(crate) mod send_button;
