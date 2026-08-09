//! Process-wide sqlite-vec registration (v1.18.x hardening).
//!
//! Exposed from the lib crate so the server binary (`src/main.rs`) and the
//! standalone `brain-migrate-rehearse` binary share ONE audited, correctly-
//! typed FFI registration instead of duplicating it (which previously forced
//! a second `unsafe` block). See the lib crate root comment for why shared
//! modules live here.
//!
//! # Safety
//!
//! `sqlite3_auto_extension` expects a C-ABI function pointer. `sqlite-vec`
//! 0.1.9 declares its entrypoint as `sqlite3_vec_init()` with **no arguments**
//! (a stub), so naively passing it requires a `transmute`. That transmute is
//! a signature-mismatch cast — sound only by reasoning about the target ABI.
//!
//! To eliminate it, we re-declare the C symbol with its **true** signature
//! (the real `sqlite3_vec_init` C function takes three arguments, matching
//! `RawAutoExtension`), then hand that correctly-typed pointer straight to
//! `sqlite3_auto_extension`. No cast, no transmute, no stub — the remaining
//! `unsafe` is only the irreducible FFI call into SQLite's process-global
//! extension registry.
//!
//! The call is sound because:
//! 1. `sqlite3_vec_init` is a `extern "C"` symbol linked from the sqlite-vec
//!    static library — its ABI matches what `sqlite3_auto_extension` expects.
//! 2. The function pointer is a process-lifetime static; it's never
//!    deallocated.
//! 3. `sqlite3_auto_extension` stores the pointer for process lifetime and
//!    never calls it after the process exits.
//! 4. `sqlite3_vec_init` opens no SQLite connection itself, so it cannot
//!    recurse into the auto-extension handler (the one failure mode the FFI
//!    contract warns about).
//!
//! This must be called before any r2d2 pool is built (pool connections
//! inherit the registration). The function is idempotent — SQLite
//! deduplicates registered extensions, so calling it multiple times is safe.
//!
//! `#[link(name = "sqlite_vec0")]` must be at module scope: the attribute is
//! ignored inside a function body, which would silently drop the `-l
//! sqlite_vec0` link directive.

use std::ffi::{c_char, c_int};

#[link(name = "sqlite_vec0")]
unsafe extern "C" {
    #[link_name = "sqlite3_vec_init"]
    fn sqlite3_vec_init_typed(
        _db: *mut rusqlite::ffi::sqlite3,
        _pz_err_msg: *mut *mut c_char,
        _: *const rusqlite::ffi::sqlite3_api_routines,
    ) -> c_int;
}

pub fn register_sqlite_vec() {
    // SAFETY: see the safety proof in the doc comment above.
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(sqlite3_vec_init_typed));
    }
}
