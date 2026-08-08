//! Review panel — the approval queue (DESIGN §4.1). Context-rich approval
//! cards with novelty/conflict/salience. Gates on v1.14.0 (/proposals).

use dioxus::prelude::*;

pub fn panel() -> Element {
    rsx! {
        h1 { "Review" }
        p { class: "text-gray-500",
            "Approval queue lands with brain-server v1.14.0 (POST /ingest/proposal, "
            "GET /proposals, POST /proposals/{{id}}/approve|reject). Each card shows "
            "novelty / conflict / salience — the why, not a binary button."
        }
    }
}
