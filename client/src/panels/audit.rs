//! Audit panel — the append-only hash-chain browser (DESIGN §4.5). GET /audit
//! exists today; read events appear when BRAIN_AUDIT_READ_EVENTS=on (v1.15.0 M1).

use dioxus::prelude::*;

pub fn panel() -> Element {
    rsx! {
        h1 { "Audit" }
        p { class: "text-gray-500",
            "Hash-chain browser over GET /audit (filter by principal/kind/since, "
            "/audit/export). Read events appear with BRAIN_AUDIT_READ_EVENTS=on (v1.15.0)."
        }
    }
}
