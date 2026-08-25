//! The care engine core: inquiry and account-change dialogs. ZERO new
//! concepts — this crate is a thin, worktype-typed facade over
//! brain-interview-core's ambiguity / draft / repair machinery; the only
//! addition is the vocabulary binding (which run kinds may open a dialog).

#![deny(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Tests read as sibling cores' tests do (assert + unwrap on known-good
// fixtures); the production-code denies stay absolute.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod dialog;

pub use brain_interview_core::{ambiguity, draft, repair, state};
pub use dialog::CareDialog;
