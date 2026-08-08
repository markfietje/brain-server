//! Security panel — quarantine review + auth-failure feed + audit-chain verify
//! (DESIGN §4.4). Quarantine + /audit already exist; chain-verify is a client
//! recompute over GET /audit.

use dioxus::prelude::*;

pub fn panel() -> Element {
    rsx! {
        h1 { "Security" }
        p { class: "text-gray-500",
            "Quarantine review (GET /quarantine exists), auth-failure feed (GET /audit "
            "kind=authz-denial), and the audit-chain verify button (client recomputes the "
            "chain over GET /audit). The post-CVE-2026-59726 control surface."
        }
    }
}
