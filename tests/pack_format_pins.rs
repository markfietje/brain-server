//! G-1 — the Tier C pack format's structural law (plan 28 §4).
//!
//! The behavioral battery lives in `src/packs/` (the validator's own
//! module, arm by arm: unknown file, executable content, the consuming
//! machinery's own errors). This file holds the pins a validator cannot
//! state about itself:
//!
//! * the format module is PURE — no filesystem, no network, no clock. The
//!   installer owns I/O; the format owns the law. A validator that reads
//!   the world is a second installer.
//! * `procedures/` rides the REGISTERED wizard-pack schema — the
//!   compile-time-embedded catalog's validator, called by name. C2 does
//!   not change that law (READ FIRST ②): the drift gate stays byte-green,
//!   and this pin is what keeps a second, looser schema from growing.

use std::path::PathBuf;

fn packs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/packs")
}

#[test]
fn the_pack_format_module_is_pure_no_fs_no_net_no_clock() {
    let source =
        std::fs::read_to_string(packs_dir().join("mod.rs")).expect("the pack format module exists");
    let code: String = source
        .lines()
        .filter(|l| {
            let t = l.trim();
            !(t.starts_with("//") || t.starts_with('*') || t.starts_with("/*"))
        })
        .collect();
    for banned in ["std::fs", "std::net", "SystemTime", "reqwest", "ureq"] {
        assert!(
            !code.contains(banned),
            "the pack format module must stay I/O-free: it names `{banned}` — \
             the format owns the law, the installer owns the world"
        );
    }
}

#[test]
fn pack_procedures_ride_the_registered_wizard_schema_not_a_second_one() {
    let source =
        std::fs::read_to_string(packs_dir().join("mod.rs")).expect("the pack format module exists");
    assert!(
        source.contains("crate::workflow::wizard::validate_wizard_pack"),
        "a pack's procedures/ must validate through the registered wizard-pack \
         schema (the drift gate's own validator) — a second, looser schema is \
         the drift the gate exists to refuse"
    );
    // and no OTHER pack-file validator may invent wizard vocabulary: the
    // module names `qtype` nowhere in its PRODUCTION half (the schema's
    // business, not the format's). The test module below the fence builds
    // fixtures in that vocabulary — that is what fixtures are for.
    let production = source
        .split("#[cfg(test)]")
        .next()
        .expect("non-empty source");
    let code: String = production
        .lines()
        .filter(|l| {
            let t = l.trim();
            !(t.starts_with("//") || t.starts_with('*') || t.starts_with("/*"))
        })
        .collect();
    assert!(
        !code.contains("qtype"),
        "the format module must not re-implement wizard-pack vocabulary — the \
         registered schema is the ONE validator"
    );
}

#[test]
fn the_file_set_allowlist_is_the_format_the_plan_names() {
    // plan 28 §4's tree, one line each — an entry added or dropped here
    // without the plan is a format change, and format changes are named acts
    let source = std::fs::read_to_string(packs_dir().join("mod.rs")).expect("the format module");
    for name in [
        "\"manifest.json\"",
        "\"domain.json\"",
        "\"roles.json\"",
        "\"taxonomy.json\"",
        "\"rules.json\"",
        "\"ui/manifest.json\"",
        "\"procedures/\"",
        "\"i18n/\"",
    ] {
        assert!(
            source.contains(name),
            "the allowlist must carry {name} — the file-set allowlist IS the format"
        );
    }
}
