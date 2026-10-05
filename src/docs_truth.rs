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

    /// Every README badge that states a version, a count, or a file path, plus
    /// the OpenAPI version pair — each DERIVED from its source, never from a
    /// constant written in this test.
    ///
    /// Hand-maintenance is **disproven** as a mechanism here: the spec version
    /// sat at 1.23.0 while the server was serving `1.29.2`, the SBOM badge
    /// linked a file two releases behind, and the test-count badge drifted —
    /// across two consecutive releases (1.29.1 and 1.29.2). Each was invisible
    /// because nothing compared the displayed value to its source. This is the
    /// replacement.
    ///
    /// The decisive one is `x-api-version`. The router serves that header from
    /// `env!("CARGO_PKG_VERSION")` (`src/server/router/mod.rs:258`), so a spec
    /// claiming `1.23.0` is not stale — it is **factually wrong about the
    /// server's own wire behaviour**, and any client reconciling the header
    /// against the spec would compute a false version.
    #[test]
    fn readme_badges_and_openapi_version_are_derived_not_hand_typed() {
        let readme = doc("README.md");
        let cargo = doc("Cargo.toml");
        let spec = doc("openapi.yaml");

        // SOURCE 1 — the crate version. One read, everything else keys off it.
        let version = cargo
            .lines()
            .find_map(|l| l.strip_prefix("version = \""))
            .and_then(|l| l.split('"').next())
            .unwrap_or_else(|| panic!("Cargo.toml must carry a `version = \"x.y.z\"` line"));
        assert!(
            !version.is_empty() && version.starts_with(|c: char| c.is_ascii_digit()),
            "parsed crate version is not a version: {version:?}"
        );

        // Badge: the version, as displayed.
        assert!(
            readme.contains(&format!("badge/version-{version}-blue.svg")),
            "README version badge does not show the crate version {version} — run \
             scripts/badges.sh and paste the block"
        );

        // SOURCE 2 — the OpenAPI version pair must equal the served version.
        // Both halves: `info.version` and `x-api-version`.
        let info_version = spec
            .lines()
            .find_map(|l| l.strip_prefix("  version: "))
            .map(str::trim)
            .unwrap_or_else(|| panic!("openapi.yaml must carry an `info.version`"));
        let api_version = spec
            .lines()
            .find_map(|l| l.strip_prefix("  x-api-version: \""))
            .and_then(|l| l.split('"').next())
            .unwrap_or_else(|| panic!("openapi.yaml must carry an `x-api-version`"));
        assert_eq!(
            info_version, version,
            "openapi.yaml info.version ({info_version}) disagrees with the crate \
             version ({version})"
        );
        assert_eq!(
            api_version, version,
            "openapi.yaml x-api-version ({api_version}) disagrees with the crate \
             version ({version}). The router serves this header from \
             env!(\"CARGO_PKG_VERSION\"), so a stale value here is a spec that \
             misdescribes the running server."
        );

        // Badge: the spec version, as displayed — must follow BOTH.
        assert!(
            readme.contains(&format!("OpenAPI-{version}-")),
            "README OpenAPI badge does not show {version} (the spec's own version)"
        );
        assert!(
            !readme.contains("OpenAPI-1.23.0-"),
            "the README still advertises the pre-1.29 spec version 1.23.0"
        );

        // SOURCE 3 — the SBOM file for THIS version must exist and be the one
        // the badge links. The path is derived, so a release cannot point the
        // badge at a stale file.
        let sbom = format!("sbom/brain-server-{version}.cdx.json");
        assert!(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(&sbom)
                .exists(),
            "{sbom} is missing — run scripts/sbom.sh and commit it. The README \
             links a specific SBOM, so the link and the file must be the same \
             release."
        );
        assert!(
            readme.contains(&format!("href=\"{sbom}\"")),
            "the README SBOM link is not {sbom} — a badge pointing at another \
             release's file is a stale link, not a cosmetic one"
        );

        // The test-count badge cannot be derived inside a unit test (it needs a
        // full cargo run), so this pins the SHAPE rather than the number, and
        // `scripts/badges.sh --verify-count` owns the number — it re-derives the
        // count with the same feature set this script's callers use and refuses
        // on drift. `--selfcheck` deliberately does NOT, because the comparison
        // costs a full compile and it runs on every push.
        //
        // Asserting a literal number here would be exactly the hand-maintenance
        // this test replaces; what it DOES pin is the disclosure: a count this
        // file cannot prove must never read as proven.
        assert!(
            readme.contains("img.shields.io/badge/tests-"),
            "the README test-count badge vanished"
        );
        assert!(
            readme.contains("not selfcheck-verified"),
            "the test-count badge must carry its 'not selfcheck-verified' \
             disclaimer — a count this file cannot prove must never read as proven"
        );
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

        // ── the ROUTE BINDING, which the key-set check above cannot see ──
        // Key-set equality proves the two descriptions agree on FIELDS. It says
        // nothing about WHICH handler serves them: a schema could be perfectly
        // shaped, referenced from the advance path, and served by a different
        // type. So assert the chain end to end, in source:
        //
        //   spec path -> $ref -> schema name
        //   handler fn -> delivery::advance() -> its return type -> that struct
        //
        // Names are expected to DIFFER (Advanced vs DeliveryRunAdvanced) — that
        // is the recorded house convention, so the check compares the RETURN
        // TYPE of the call, not a string equality.
        let handler = std::fs::read_to_string(format!(
            "{}/src/handlers/delivery.rs",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("read the delivery handler");
        let core = std::fs::read_to_string(format!(
            "{}/src/workflow/delivery.rs",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("read the delivery core");

        assert!(
            handler.contains("pub async fn post_delivery_advance"),
            "the advance route's handler must exist — the spec documents a route \
             the handler tree no longer implements"
        );
        assert!(
            handler.contains("delivery::advance("),
            "post_delivery_advance must call delivery::advance — a route bound to \
             a different core fn serves a different contract than the spec states"
        );
        assert!(
            handler.contains("serde_json::to_value(advanced)"),
            "the handler must serialize the advance result directly; a projection \
             or hand-built object would break the field-for-field binding this \
             schema claims"
        );
        assert!(
            core.contains("pub(crate) fn advance(conn: &mut Connection, req: &Advance<'_>) -> Result<Advanced, DeliveryError>"),
            "delivery::advance must return Result<Advanced, _> — if the return \
             type changes, the served shape changes and this schema's key set \
             must be re-pinned with it"
        );
        assert!(
            core.contains("pub(crate) struct Advanced {"),
            "the Advanced struct this route serves must exist at the site the \
             schema description names"
        );

        // The description must name the divergence, so a reader who greps the
        // spec for `Advanced` is not left to wonder whether it is a typo.
        assert!(
            schema.contains("struct Advanced"),
            "the DeliveryRunAdvanced description must name the Rust type it is \
             served from, and say the names deliberately differ — an undocumented \
             name divergence reads as a mistake"
        );
    }

    /// the attestation round: the attestation READ surface's wire contract, and its two
    /// NON-CLAIMS, pinned against the spec and the handler together.
    ///
    /// The field-set half applies an earlier round's lesson to this new route: the route
    /// guards are path-level, so nothing else compares a Rust response struct to
    /// its schema. The non-claim half is different in kind — a reader who
    /// greps this route for `DSSE` or `in-toto` must find a NEGATION, because
    /// the envelope's field names deliberately mirror the in-toto Statement
    /// model and a description that named the lineage without the negation would
    /// read as a conformance claim. The pin therefore fails if the words appear
    /// WITHOUT a negation nearby, which is the only way to make a negative
    /// machine-checked.
    #[test]
    fn attestation_wire_schema_matches_the_handler() {
        let spec = doc("openapi.yaml");

        // The path exists and is a GET.
        let path = spec
            .find("  /workflow/delivery/runs/{id}/attestations:")
            .unwrap_or_else(|| panic!("openapi.yaml lost the attestation read path"));
        let block = &spec[path..];
        let end = block.find("\n  /workflow/").unwrap_or(block.len());
        let block = &block[..end];
        assert!(
            block.contains("get:"),
            "the attestation route is a GET — it re-derives from stored bytes and takes no body"
        );
        assert!(
            block.contains("#/components/schemas/DeliveryAttestationChain"),
            "the route must $ref the chain schema"
        );
        assert!(
            spec.contains("    DeliveryAttestationChain:"),
            "the route $refs a schema openapi.yaml does not define — a dangling ref is not a \
             contract"
        );
        assert!(
            block.contains("verify"),
            "`?verify=1` must be documented on the route: the verdict is UNCONDITIONAL and the \
             parameter is an explicit request for the identical payload"
        );

        // The response schema's property keys, collected by INDENT (a
        // `contains("field:")` test is vacuous — a rename satisfies it).
        let lines: Vec<&str> = spec.lines().collect();
        let start = lines
            .iter()
            .position(|l| l.trim_end() == "    DeliveryAttestationChain:")
            .unwrap_or_else(|| panic!("openapi.yaml lost the DeliveryAttestationChain schema"));
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.starts_with("    ") && !l.starts_with("     ") && !l.trim().is_empty())
            .map_or(lines.len(), |i| start + 1 + i);
        let schema = lines[start..end].join("\n");
        assert!(
            schema.contains("additionalProperties: false"),
            "anti-vacuity: the response must still be closed, or this pin is moot"
        );
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
            declared.len() >= 3,
            "the chain response property scan found only {} keys ({declared:?}) — the scanner \
             or the schema changed shape",
            declared.len()
        );
        for field in ["run_id", "chain", "verdict"] {
            assert!(
                declared.contains(&field),
                "the chain response schema omits `{field}` (declared: {declared:?}) — with \
                 additionalProperties:false a client that trusts the spec rejects the real \
                 response"
            );
        }

        // ── the two NON-CLAIMS, which must be present as negations ──
        // YAML `>` folds newlines into spaces, so a real reader sees one line.
        // The pin normalizes whitespace for the same reason: a phrase split
        // across a folded line is still present to every other reader, and
        // failing it would make the pin about line breaks rather than claims.
        let lower = block
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        for (claim, negation) in [
            ("dsse", "not dsse"),
            ("in-toto", "not an in-toto"),
            ("slsa", "no slsa"),
        ] {
            assert!(
                lower.contains(claim),
                "the route description should name `{claim}` in order to DENY it — an \
                 unmentioned standard is not a claim, and a mentioned one without its \
                 negation is"
            );
            assert!(
                lower.contains(negation),
                "the route description mentions `{claim}` without the explicit negation \
                 `{negation}` — the non-claim is not optional"
            );
        }
        // And the no-key operational consequence, which is a refusal, not a
        // degraded mark.
        assert!(
            lower.contains("refuses") || lower.contains("refuse"),
            "the route description must state the key posture: on a host with no operator key \
             every delivery phase pass refuses"
        );
        // The words that would turn an adjacency into a claim.
        for banned in [
            "dsse-compatible",
            "dsse conformant",
            "slsa-compliant",
            "in-toto compliant",
        ] {
            assert!(
                !lower.contains(banned),
                "the route description must never assert `{banned}` — the envelope is a project \
                 convention and verifies against none of their verifiers"
            );
        }
    }

    /// The Philippines posture is statute PLUS the Commission's guidance, and
    /// the guidance moves far faster than the statute. RA 10173 (2012) is still
    /// the operative law — re-verified 2026-09-26 against the NPC's own
    /// issuances index, which lists no amending or replacing Republic Act — but
    /// the component's §6.3 previously cited the statute alone and said nothing
    /// about the issuances that actually bear on it, including one squarely on
    /// AI systems processing personal data.
    ///
    /// This is a **docs-truth** pin, not a compliance claim: it freezes the
    /// issuances the doc must carry so they cannot go stale, and it pins the
    /// no-amending-RA finding so a future editor cannot quietly "correct" the
    /// statute line against a real but inapplicable instrument. The legal
    /// readings above it — whether this component is an "AI system", its
    /// provider/deployer role, whether the Advisory's scope reaches it — are
    /// operator and counsel determinations and are deliberately NOT asserted
    /// here.
    #[test]
    fn philippines_posture_cites_the_npc_issuances_not_just_the_statute() {
        let compliance = doc("COMPLIANCE.md");
        let ph = compliance
            .split("### 6.3 Jurisdiction Posture")
            .nth(1)
            .and_then(|s| s.split("### 6.4").next())
            .expect("COMPLIANCE.md must carry a §6.3 Jurisdiction Posture section");
        let ph_line = ph
            .lines()
            .find(|l| l.contains("Philippines"))
            .expect("§6.3 must carry a Philippines posture line");

        // The statute stays cited, and the pin states the no-amending-RA finding
        // as a dated claim rather than leaving it implied.
        assert!(
            ph_line.contains("RA 10173"),
            "the Philippines posture must cite the operative statute, RA 10173"
        );
        assert!(
            ph.contains("no amending or replacing Republic Act"),
            "the statute-currency finding must be stated IN the doc, dated — a \
             reader must not have to assume RA 10173 is still current"
        );

        // The issuances that bear on this component. Each carries its number, so
        // a re-baseline is a visible diff rather than a silent scope change.
        for issuance in [
            "NPC Advisory No. 2024-04", // AI systems processing personal data
            "NPC Advisory No. 2026-01", // data scraping of public personal data
            // 2026-03 is the newest issuance and the one a STALE NPC index
            // mirror silently omits: on 2026-09-26 the /pips-and-pics/ path
            // served a 2026-05-21 copy topping out at 2026-02, while
            // /lawphil/advisories/ served a 2026-09-21 copy carrying it. Pinning
            // only the two that both mirrors agree on would let a re-baseline
            // against the stale copy drop the newest item and still go green.
            "NPC Advisory No. 2026-03", // ASIR 2025 additional submission period
            "NPC Circular No. 2023-04", // guidelines on consent
        ] {
            assert!(
                ph.contains(issuance),
                "the Philippines posture omits {issuance} — the issuances index \
                 moved and the jurisdiction posture did not. Add the issuance and \
                 its date in the same change."
            );
        }
        assert!(
            ph.contains("guidance, not statute"),
            "the issuances must be classified as guidance rather than law — an \
             Advisory is issued under §7(g)/§9 IRR and does not amend the DPA"
        );
        assert!(
            ph.contains("not decided here") || ph.contains("NOT decided here"),
            "the AI-scope reading must remain an operator/counsel question, not a \
             conclusion written into a compliance surface"
        );

        // The draft-under-consultation item is named as a watch item, with its
        // consultation dates, so it is visible BEFORE adoption rather than
        // discovered after.
        assert!(
            ph.contains("DRAFT NPC Circular on Data Subject Rights"),
            "the open DS-Rights consultation must stay registered as a watch item"
        );
        assert!(
            ph.contains("05 Oct 2026") && ph.contains("19 Oct 2026"),
            "the consultation's comment deadline and hearing date must stay \
             stamped — a watch item without its clock is not a watch item"
        );
        // The re-verification route AND its staleness trap. An index that
        // silently omits the newest issuance is worse than no index, because it
        // reads as authoritative.
        assert!(
            ph.contains("pips-and-pics/advisories-circulars"),
            "the Philippines block must name the authoritative NPC issuances index \
             so the next re-baseline has a starting URL"
        );
        assert!(
            ph.contains("article:modified_time"),
            "the block must record that the NPC index is served from caches of \
             differing freshness, and that the timestamp — not the path — is what \
             distinguishes a current copy from a stale one"
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
