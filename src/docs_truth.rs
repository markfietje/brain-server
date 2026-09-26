//! Docs truth: standards watch items are pinned by test so a standards
//! revision cannot land silently. If one of these fails, the cited standard
//! moved — re-verify the mapping in the referenced doc and update it in the
//! same change.

#[cfg(test)]
mod pins {
    fn doc(rel: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{rel} must exist and be readable: {e}"))
    }

    /// ISO/AWI 18295-1 is under revision (verified 2026-08). The watch item
    /// must stay registered: when the revision publishes, every clause
    /// reference has to be re-mapped deliberately, not silently.
    #[test]
    fn iso_18295_revision_stays_a_registered_watch_item() {
        let standards = doc("docs/CONTACT_CENTER_STANDARDS.md");
        assert!(
            standards.contains("ISO/AWI 18295-1"),
            "the ISO 18295-1 revision watch item vanished from the standards inventory"
        );
        let compliance = doc("COMPLIANCE.md");
        assert!(
            compliance.contains("ISO 18295-1"),
            "the contact-centre clause map lost its ISO 18295-1 anchor"
        );
    }

    /// The conformance posture stays self-assessed: certification language
    /// must not creep into the compliance file.
    #[test]
    fn contact_centre_posture_is_self_assessed_not_certified() {
        let compliance = doc("COMPLIANCE.md");
        let section = compliance
            .split("### 6.7")
            .nth(1)
            .unwrap_or_else(|| panic!("COMPLIANCE.md §6.7 missing"));
        let head = section.split("## ").next().unwrap_or_default();
        assert!(
            head.to_ascii_lowercase().contains("self-assessed"),
            "§6.7 must state its self-assessed posture"
        );
    }

    /// The metrics dictionary covers every emitted scoreboard field and the
    /// FCR window config exists with its documented default.
    #[test]
    fn fcr_window_documented_matches_code_default() {
        let metrics = doc("docs/metrics.md");
        assert!(
            metrics.contains("BRAIN_FCR_WINDOW_DAYS") && metrics.contains("default 7"),
            "metrics dictionary must document BRAIN_FCR_WINDOW_DAYS (default 7)"
        );
        assert_eq!(
            crate::config::DEFAULT_FCR_WINDOW_DAYS,
            7,
            "code default drifted from the documented default"
        );
    }

    /// Throughput v1.28.58: every `/metrics` series the core router emits
    /// carries a dictionary row in docs/metrics.md — the scoreboard
    /// docs↔code parity applied to ops telemetry. Scans the metrics handler's
    /// source for `brain_*` series literals (the substring-lock idiom: a
    /// series added in code without its dictionary row fails here).
    #[test]
    fn metrics_series_have_dictionary_rows() {
        let core = doc("src/server/router/core.rs");
        let metrics = doc("docs/metrics.md");
        // collect every `brain_[a-z0-9_]+` literal in the metrics surface
        let mut series: Vec<String> = Vec::new();
        let bytes = core.as_bytes();
        let mut i = 0usize;
        while let Some(rel) = core[i..].find("brain_") {
            let start = i + rel;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_lowercase()
                    || bytes[end].is_ascii_digit()
                    || bytes[end] == b'_')
            {
                end += 1;
            }
            let name = core[start..end].to_string();
            if !series.contains(&name) {
                series.push(name);
            }
            i = start + 6;
        }
        // anti-vacuous: the scan must see the real surface — the pre-Throughput
        // series floor. If this fires, the scanner (or the handler) is broken.
        assert!(
            series.len() >= 10,
            "metrics-series scan found only {} names — the scanner or the handler is broken",
            series.len()
        );
        for name in &series {
            assert!(
                metrics.contains(&format!("`{name}`")),
                "metrics dictionary is missing a row for series `{name}` — add it to \
                 docs/metrics.md §\"Server telemetry series\" in the same commit"
            );
        }
    }

    /// The interview repair mode is a MODE of the one interview state
    /// machine, not a fork: the repair module's own documentation must
    /// carry the upstream repair-policy cross-reference (pi
    /// `RepairPolicyMode` at `extensions.rs:2048`; gajae's
    /// `deep-interview-repair-cli.md`). If this fires, the cross-reference
    /// was dropped — restore it in the same change, never silently.
    #[test]
    fn repair_is_mode_not_fork() {
        let repair = doc("crates/brain-interview-core/src/repair.rs");
        for anchor in [
            "RepairPolicyMode",
            "extensions.rs:2048",
            "deep-interview-repair-cli.md",
            "not a fork",
        ] {
            assert!(
                repair.contains(anchor),
                "the interview repair module lost the `{anchor}` cross-reference — \
                 repair is a mode, not a fork, and the module doc must say so"
            );
        }
    }

    /// `docs/engine-sdk.md` classifies every engine crate as Filled or a
    /// Scaffold, and until now NOTHING read the file — the classification had
    /// rotted past the point where a reader could trust it: `brain-care-core`
    /// (80 lines, 1 test) was listed Filled beside `legal-rules-db` (1217
    /// lines, 11 tests) listed as a Scaffold, and `brain-engine-sdk` itself —
    /// the file's own subject, 13k+ lines — was not listed at all.
    ///
    /// This pin is deliberately NOT a line-count comparison. Size is a bad
    /// proxy (the smallest "Filled" crate is a third the size of the smallest
    /// "Scaffold" one), so asserting counts would be asserting a fiction. What
    /// it checks is the two things that were actually false:
    ///
    /// 1. every crate the doc names must exist on disk, and the doc must name
    ///    at least the engine SDK — anti-vacuity, so a renamed or deleted crate
    ///    fails rather than silently shrinking the list;
    /// 2. the consumed engines must not be classified as Scaffolds, because
    ///    the delivery run lifecycle now calls them.
    #[test]
    fn engine_sdk_crate_map_is_accurate() {
        let sdk = doc("docs/engine-sdk.md");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("crates");

        // Every backticked `brain-*` / `legal-rules-db` name in the doc must be
        // a real crate directory. This is the anti-vacuity arm: if the scan saw
        // nothing, the rest of this test would pass on an empty file.
        let mut named: Vec<String> = Vec::new();
        let mut rest = sdk.as_str();
        while let Some(open) = rest.find('`') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('`') else { break };
            let name = &after[..close];
            if name.starts_with("brain-") || name.starts_with("legal-rules-db") {
                named.push(name.to_string());
            }
            rest = &after[close + 1..];
        }
        assert!(
            named.len() >= 9,
            "the crate scan found only {} names — the scanner or the doc changed shape",
            named.len()
        );
        for name in &named {
            assert!(
                root.join(name).join("Cargo.toml").exists(),
                "docs/engine-sdk.md names `{name}`, which has no crates/{name}/Cargo.toml — \
                 rename the crate or the row, never both silently"
            );
        }
        assert!(
            named.iter().any(|n| n == "brain-engine-sdk"),
            "the SDK doc must classify the SDK itself"
        );

        // The scaffolds bullet, taken WITH its wrapped continuation. The first
        // cut of this pin searched only the text AFTER the scaffolds line,
        // which had already removed the very names it was checking — so
        // re-classifying a consumed engine as a Scaffold passed a green pin.
        // The bullet is the matching line plus its continuations.
        let lines: Vec<&str> = sdk.lines().collect();
        let start = lines
            .iter()
            .position(|l| l.contains("Scaffold"))
            .unwrap_or_else(|| panic!("docs/engine-sdk.md lost its Scaffold line"));
        let mut bullet = String::new();
        for line in &lines[start..] {
            if !bullet.is_empty() && !line.starts_with(char::is_whitespace) {
                break; // a new bullet/paragraph ends the wrapped continuation
            }
            bullet.push_str(line);
            bullet.push(' ');
            if line.ends_with('.') && bullet.contains('`') {
                break;
            }
        }
        for consumed in ["brain-executor-core", "brain-consensus-core"] {
            assert!(
                !bullet.contains(consumed),
                "`{consumed}` is consumed by the delivery phase pass and must not be \
                 classified as a Scaffold — the scaffolds bullet reads: {bullet}"
            );
        }
    }

    /// The delivery phase pass's WIRE contract, pinned field by field.
    ///
    /// The whole delivery battery went green while `openapi.yaml` described a
    /// route that no longer existed: the advance request body is
    /// `additionalProperties: false` and did not list the `artifact` field the
    /// handler accepts, and the `DeliveryRunAdvanced` response schema is
    /// `additionalProperties: false` and did not list the `proposal_id` the
    /// server returns on EVERY success. Spec-conformant clients rejected the
    /// response; strict validators rejected the body.
    ///
    /// It stayed green because the existing route guards are **path-level only** —
    /// `test_openapi_covers_routes` proves every path is documented, never that
    /// a documented path's FIELDS match the handler. Nothing in the repository
    /// compared a Rust response struct to its schema. This is that comparison,
    /// scoped to the one route this round changed.
    #[test]
    fn delivery_advance_wire_schema_matches_the_handler() {
        let spec = doc("openapi.yaml");

        // The response schema: every field the Rust struct serializes must be
        // listed, and `additionalProperties: false` means a missing one is a
        // hard contract violation rather than a nicety.
        // The schema's own block. The end boundary matters: `DeliveryArtifact`
        // is defined BETWEEN `DeliveryRunAdvanced` and `DeliveryRunAnswered`,
        // so slicing to "the next DeliveryRun* name" swallowed a second schema
        // whole and let its keys satisfy this schema's assertions. The block
        // ends at the next 4-space schema definition — every key at 8 spaces
        // belongs to THIS schema.
        let lines: Vec<&str> = spec.lines().collect();
        let start = lines
            .iter()
            .position(|l| l.trim_end() == "    DeliveryRunAdvanced:")
            .unwrap_or_else(|| panic!("openapi.yaml lost the DeliveryRunAdvanced schema"));
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.starts_with("    ") && !l.starts_with("     ") && !l.trim().is_empty())
            .map_or(lines.len(), |i| start + 1 + i);
        let schema = lines[start..end].join("\n");
        assert!(
            schema.contains("additionalProperties: false"),
            "anti-vacuity: the response must still be closed, or this pin is moot"
        );
        // Collect the schema's ACTUAL property keys, by INDENT: the keys sit at
        // 8 spaces and their nested values (`type:`, `description:`) at 10+. A
        // `contains("{field}:")` substring test is vacuous — renaming the field
        // to `xproposal_id` satisfies it — which is the same self-matching
        // failure the crate-root `forbid` scan had.
        let declared: Vec<&str> = schema
            .lines()
            .skip_while(|l| !l.trim_start().starts_with("properties:"))
            .skip(1)
            .take_while(|l| l.starts_with("        ") || l.trim().is_empty())
            .filter(|l| l.starts_with("        ") && !l.starts_with("          "))
            .filter_map(|l| l.get(8..)?.split(':').next())
            .filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .collect();
        assert!(
            declared.len() >= 8,
            "the response property scan found only {} keys ({declared:?}) — the \
             scanner or the schema changed shape",
            declared.len()
        );
        for field in [
            "run_id",
            "phase",
            "tier",
            "trace_mode",
            "trace_id",
            "state_revision",
            "step_id",
            "proposal_id",
        ] {
            assert!(
                declared.contains(&field),
                "the advance response schema omits `{field}` (declared: {declared:?}) \
                 — with additionalProperties:false a client that trusts the spec \
                 rejects the real response. Add the field in the same commit as the \
                 change."
            );
        }

        // The request body: the advance path's schema must carry `artifact`.
        let path = spec
            .find("  /workflow/delivery/runs/{id}/advance:")
            .unwrap_or_else(|| panic!("openapi.yaml lost the delivery advance path"));
        let block = &spec[path..];
        let end = block.find("\n  /workflow/").unwrap_or(block.len());
        let block = &block[..end];
        assert!(
            block.contains("artifact:"),
            "the advance request body omits `artifact` — with \
             additionalProperties:false a client that trusts the spec cannot send \
             the typed artifact the handler accepts"
        );
        assert!(
            block.contains("#/components/schemas/DeliveryArtifact"),
            "the advance request must $ref the DeliveryArtifact schema"
        );
        assert!(
            spec.contains("    DeliveryArtifact:"),
            "the advance request $refs a DeliveryArtifact schema that openapi.yaml \
             does not define — a dangling ref is not a contract"
        );
        assert!(
            block.contains("delivery_quality_gate_refused"),
            "the advance 409 list must carry the checkpoint-gate refusal — a client \
             cannot handle an error the spec does not name"
        );
    }

    /// The AI Act DEPLOYER horizons live in docs and are pinned nowhere in code
    /// — `reg_watch` holds the Art 50 marking clock and the general application
    /// clock, and its own comment says the deployer horizons are "tracked in
    /// docs, not in code". So the two dates the Omnibus deferral moved had no
    /// machine check at all: a well-meaning edit to the prose would have moved
    /// a statutory date with nothing failing.
    ///
    /// This is a DOCS-TRUTH pin, not a legal claim and not a conformity claim.
    /// It asserts that the docs carry the two horizons and cite the instrument
    /// that moved them, so the text cannot drift silently. Whether this system
    /// is an "AI system", whether it is high-risk, and which role it holds are
    /// operator and counsel determinations and are recorded as open questions,
    /// never decided here.
    #[test]
    fn ai_act_deployer_horizons_are_stamped_from_the_amending_instrument() {
        let compliance = doc("COMPLIANCE.md");
        let detail = doc("docs/compliance.md");

        // The instrument that moved them must be cited, in both surfaces.
        for (name, text) in [
            ("COMPLIANCE.md", &compliance),
            ("docs/compliance.md", &detail),
        ] {
            assert!(
                text.contains("2026/1744"),
                "{name} must cite Regulation (EU) 2026/1744 — the instrument that \
                 deferred the high-risk regime"
            );
        }

        // The two deployer horizons, in the readable form and the ISO form, so
        // a rewrite that drops one of the spellings is caught.
        assert!(
            compliance.contains("2 December 2027"),
            "COMPLIANCE.md lost the Annex III stand-alone high-risk horizon"
        );
        assert!(
            compliance.contains("2 August 2028"),
            "COMPLIANCE.md lost the Annex I product-embedded horizon"
        );
        assert!(
            detail.contains("2027-12-02"),
            "docs/compliance.md lost the Annex III deployer horizon stamp"
        );
        assert!(
            detail.contains("2028-08-02"),
            "docs/compliance.md lost the Annex I deployer horizon stamp"
        );

        // The superseded value must not reappear as a live claim. It survives
        // only as a struck-through correction marker.
        assert!(
            !detail.contains("Annex III high-risk obligations apply from 2 Dec 2026"),
            "docs/compliance.md states the PRE-Omnibus Annex III horizon as live — \
             2026-12-02 is the Art 50(2) legacy-marking grace END, not this start"
        );
    }
}
