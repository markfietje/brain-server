//! Subjects panel — the DSAR console (DESIGN §4.3). Locate / export / purge /
//! deletion certificate with client-side chain verify. Gates on v1.15.0 (/dsar,
//! /tombstones, /dsar/{{id}}/certificate).

use dioxus::prelude::*;

pub fn panel() -> Element {
    rsx! {
        h1 { "Subjects (DSAR)" }
        p { class: "text-gray-500",
            "DSAR console lands with brain-server v1.15.0 (POST /dsar, GET /tombstones, "
            "GET /dsar/{{id}}/certificate). The defining screenshot: a deletion certificate "
            "with a green chain-verified badge."
        }
    }
}
