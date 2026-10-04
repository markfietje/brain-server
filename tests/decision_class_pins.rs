//! R53a — decision-class instrumentation: the permanent pin suite.
//!
//! **What this round is.** The plan (`IMPL_R53A_…`) opens by treating "routing is
//! an attack surface" — content-influenced signals steering which model runs a
//! call — and closes with a six-row enforcement table defending it. Measured at
//! `3bac3cb` and re-derived this round: **that surface does not exist in this
//! tree.** All three model surfaces bind the model to the call site or the
//! process. So the round is a **pin, not a control**: it turns an accidental
//! safety property into a machine-checked one, so a later round cannot quietly
//! open it.
//!
//! **The property under test, stated once:** *which model runs a call is a
//! function of the call site, never of the call's content.* Content may steer
//! cost, never safety.
//!
//! **The one behavioural pin lives elsewhere.** `R53a.2`/`D53a.4` — driving one
//! provider with adversarial content and reading the model off every body that
//! actually left the process — is at
//! `src/agentloop/provider_http.rs::the_bound_model_is_not_a_function_of_the_call_content`.
//! It needs the loopback recording harness that only in-crate tests can reach.
//! Everything here is structural: source-level, comment-stripped, and about the
//! shape of the code rather than one run of it.
//!
//! **Comment-stripping is the F7-07 house rule** (v1.28.87). A symbol named in a
//! comment must never produce a false pass, so every source assert below reads
//! code, not prose.
//!
//! Preregistrations: `P53a.1`–`P53a.3`, recorded in
//! `plans/R53A_DECISION_CLASS_INSTRUMENTATION_EVIDENCE_2026-09-29.md` §3 BEFORE
//! the first row was emitted. The telemetry-name pin below asserts the *emitted*
//! string equals the *preregistered* string — not merely that a row exists.

use brain_server::decision_class::{ClassObservation, DecisionClass, observations};
use std::path::PathBuf;

// ── source reading + the comment stripper ───────────────────────────────────

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_src(rel: &str) -> String {
    let p = crate_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

/// Strip comments so a symbol named in a comment cannot produce a false pass.
///
/// Line-wise and deliberately conservative, following the `r51_gate_law_pins`
/// form: it only ever REMOVES text, so it can under-report a site, never invent
/// one. Residual ceiling (shared with the `main_suite.rs` F7-07 extractor): forms
/// outside `//`, `/* */`, `"…"` and `'…'` could desync a scan. These pins are
/// regression locks, not parsers.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_block = false;
    for line in src.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut res = String::new();
        let (mut in_str, mut in_chr) = (false, false);
        let mut i = 0usize;
        while i < chars.len() {
            if in_block {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    in_block = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if !in_str && !in_chr && chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
                break;
            }
            if !in_str && !in_chr && chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                in_block = true;
                i += 2;
                continue;
            }
            let c = chars[i];
            if c == '"' && !in_chr {
                in_str = !in_str;
            } else if c == '\'' && !in_str {
                in_chr = !in_chr;
            }
            res.push(c);
            i += 1;
        }
        out.push_str(&res);
        out.push('\n');
    }
    out
}

/// The text from `needle`'s start through its closing brace: the signature AND
/// the body, brace-balanced and string-aware, read from comment-stripped
/// source.
///
/// The signature is included deliberately. Two of the properties this suite
/// pins live in the parameter list — `request_body(model: &str, …)` and
/// `record(…, class: DecisionClass)` — so a body-only extractor would have
/// reported both as missing the very thing they assert.
fn body_of(stripped: &str, needle: &str) -> String {
    let start = stripped
        .find(needle)
        .unwrap_or_else(|| panic!("{needle} not found — the symbol moved; re-derive the pin"));
    let open = stripped[start..]
        .find('{')
        .map(|i| start + i)
        .unwrap_or_else(|| panic!("no body after {needle}"));
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in stripped[open..].char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return stripped[start..open + i + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces after {needle}");
}

// ── P53a.1 — the closed class set ───────────────────────────────────────────

/// The plan's `R53a.1`: "an unclassified decision site is a hard error, not a
/// default". Here that is stronger than a default — the enum has no `Unknown`
/// arm, and the census is exactly the emitted set.
#[test]
fn the_class_set_is_closed_and_the_census_is_the_emitted_set() {
    assert_eq!(
        DecisionClass::ALL.len(),
        3,
        "three model surfaces in this tree: the LLM stream, the injection screen, \
         the embedder. A fourth model surface must arrive with a fourth class, or \
         its spend is invisible."
    );
    let emitted: Vec<&str> = DecisionClass::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(
        emitted,
        vec!["open_generate", "classify", "encode"],
        "P53a.1 froze this vocabulary BEFORE the first row was emitted"
    );
    let rows = observations();
    assert_eq!(
        rows.len(),
        DecisionClass::ALL.len(),
        "every DECLARED class emits a row, including the ones at zero"
    );
    for (row, class) in rows.iter().zip(DecisionClass::ALL) {
        assert_eq!(row.class, class);
    }
}

/// The plan's fail-closed default, made concrete: the default IS class 6, the
/// one whose output is not machine-checked. Code reaching for a default gets the
/// human-verified class, never one that would let it pass as if it were checked.
#[test]
fn the_default_class_is_the_human_verified_one() {
    assert_eq!(
        DecisionClass::default(),
        DecisionClass::OpenGenerate,
        "the fail-closed default must be the class with NO machine verifier"
    );
    assert_ne!(
        DecisionClass::default(),
        DecisionClass::Classify,
        "defaulting to a machine-checked class would fail OPEN"
    );
}

// ── the minimisation property: the security control and the privacy statement ─

/// `P53a.2`/§7's demand, as one pin: **no emitted label can vary with content.**
///
/// `as_str` is a total function of a FIELDLESS enum — it has nothing to read — so
/// this is not a runtime check but a structural consequence. The pin states the
/// consequence and pins the two things that could break it: a payload added to
/// the enum, and a label that stops coming from `as_str`.
///
/// This is simultaneously the least-privilege control (a metric label reveals
/// nothing about content) and the privacy statement (no personal data can reach
/// a label, because nothing derived from a principal or a payload ever enters
/// this type).
#[test]
fn no_emitted_label_can_vary_with_content() {
    // (1) The type has no payload to carry one.
    let decl = strip_comments(&read_src("src/decision_class.rs"));
    let body = body_of(&decl, "pub enum DecisionClass {");
    for variant in ["OpenGenerate", "Classify", "Encode"] {
        let line = body
            .lines()
            .find(|l| l.contains(variant))
            .unwrap_or_else(|| panic!("{variant} vanished from the census"));
        let is_payload = line.contains('(') || line.contains('{');
        assert!(
            !is_payload,
            "variant {variant} grew a payload: {line:?}. A label that can read a field \
             can read CONTENT, and every privacy and least-privilege claim above it \
             is void."
        );
    }
    // (2) And it is total — every declared class has a label, so a class can
    // never fall through to something content-derived.
    for class in DecisionClass::ALL {
        assert!(!class.as_str().is_empty());
        assert_eq!(class.as_str(), class.as_str());
    }
}

// ── the surfaces themselves: the model is bound to the call site ────────────

/// The LLM path, structurally. `ProviderRequest` is the type every caller
/// builds, so a `model` field on it would be a channel by which content — or a
/// caller — could steer the model. There is no such field, and the outbound body
/// builder takes the model as an ARGUMENT, with the one production call site
/// passing the provider's own field.
#[test]
fn the_provider_request_carries_no_model_and_the_body_takes_it_as_an_argument() {
    let provider = strip_comments(&read_src("src/agentloop/provider.rs"));
    let request = body_of(&provider, "pub(crate) struct ProviderRequest {");
    assert!(
        !request.contains("model"),
        "ProviderRequest grew a model field: {request:?} — that is the channel a \
         content-influenced router would use, and it must not exist."
    );
    // The three fields it does carry are the whole surface.
    for field in ["system_prompt", "messages", "tools"] {
        assert!(
            request.contains(field),
            "ProviderRequest lost {field}; re-derive this pin against the real shape"
        );
    }

    let http = strip_comments(&read_src("src/agentloop/provider_http.rs"));
    let builder = body_of(&http, "fn request_body(");
    // `model` is a PARAMETER, so the builder is a pure function of
    // `(model, request)` and cannot reach a model out of the request.
    assert!(
        builder.contains("model: &str"),
        "request_body must take the model as an argument"
    );
    // And the emitted `model` key is that PARAMETER, not a field of the
    // request. This is the arm that is falsifiable by a one-line edit — the
    // signature alone would still read as correct while the body had started
    // preferring the request's own idea of the model.
    assert!(
        builder.contains("\"model\": model,"),
        "the emitted model must be the argument, not something read off the request; \
         got:\n{builder}"
    );
    assert!(
        !builder.contains("req.model"),
        "the body builder read a model off the request: {builder:?}"
    );
}

/// The send seam. `stream` clones `self.model` and hands it to the builder —
/// the model is a property of the constructed provider, and the only thing that
/// could move it is a field on the request, which the pin above forbids.
#[test]
fn the_send_seam_reads_the_model_off_the_provider_not_the_request() {
    let http = strip_comments(&read_src("src/agentloop/provider_http.rs"));
    let stream = body_of(&http, "    fn stream(");
    assert!(
        stream.contains("let model = self.model.clone();"),
        "the send seam must take the model from the provider's own field; got:\n{stream}"
    );
    assert!(
        !stream.contains("req.model"),
        "the send seam read a model off the request — content can steer the model now"
    );
}

/// The screen. The classifier is a process-wide `LazyLock`, copied
/// unconditionally when a `Screen` is built. Content reaches the *verdict*
/// (the threshold comparison), never a model — there is no second model to
/// reach.
#[test]
fn the_classifier_is_process_wide_and_the_content_selects_only_the_verdict() {
    let screen = strip_comments(&read_src("src/screen.rs"));
    assert!(
        screen.contains("static CLASSIFIER: LazyLock<Option<Arc<dyn InjectionScorer>>>"),
        "the classifier must stay a process-wide LazyLock — a per-content classifier \
         would be a content-selected model"
    );
    let from_config = body_of(&screen, "    fn from_config() -> Screen {");
    assert!(
        from_config.contains("classifier: CLASSIFIER.clone()"),
        "Screen::from_config must copy the process-wide classifier unconditionally; got:\n{from_config}"
    );
    assert!(
        !from_config.contains("content") && !from_config.contains("title"),
        "Screen construction must not read the content it is about to screen; got:\n{from_config}"
    );
}

/// The embedder. It is chosen once at boot from the retrieval profile, and the
/// model is then a property of the concrete type — `encode` has no model to
/// choose. The truncation helper is a length bound, and a length is not a
/// routing signal.
#[test]
fn the_embedder_is_chosen_at_boot_from_the_profile_alone() {
    let embed = strip_comments(&read_src("src/embed.rs"));
    let factory = body_of(&embed, "pub fn embedder_for_profile(");
    // The only match arm is the profile.
    assert!(
        factory.contains("match profile"),
        "the embedder factory must switch on the profile; got:\n{factory}"
    );
    for leak in ["text", "content", "query", "tokens", "input"] {
        assert!(
            !factory.contains(leak),
            "the embedder factory reads {leak:?} — a content-dependent embedder is a \
             content-selected model; got:\n{factory}"
        );
    }

    // And the budget helper that every backend funnels through is a length
    // truncation, nothing more.
    let budget = body_of(&embed, "pub(crate) fn embed_input(");
    assert!(
        budget.contains("MAX_EMBED_CHARS") && budget.contains("char_indices"),
        "embed_input must remain a char-boundary length truncation; got:\n{budget}"
    );
    assert!(
        !budget.contains("model") && !budget.contains("profile"),
        "the input-budget helper started reading the model or the profile — that is \
         routing at the last possible seam; got:\n{budget}"
    );

    // The boot binding itself: the profile is read once, into AppState.
    let bootstrap = strip_comments(&read_src("src/server/bootstrap.rs"));
    assert!(
        bootstrap.contains("let model = crate::embed::embedder_for_profile(profile)?;"),
        "the embedder must be bound once at boot from the profile"
    );
}

// ── D53a.2 — zero unclassified sites, asserted as a site table ──────────────

/// The plan's `D53a.2`: "every decision site is classified, and the count of
/// unclassified sites is zero. The count is **asserted, not surveyed**."
///
/// Asserted, not surveyed: the spend-observation signature takes a REQUIRED
/// class (a source check that it is neither `Option<_>` nor defaulted), and the
/// three surfaces each name their class at a pinned site. A fourth model surface
/// that forgot to name a class would not compile; a fourth that named one would
/// have to edit this table, which is the point.
#[test]
fn every_model_surface_names_its_class_at_a_pinned_site() {
    // (1) The signature is required, not optional and not defaulted.
    let subs = strip_comments(&read_src("src/agentloop/subagents.rs"));
    let record = body_of(&subs, "pub(crate) fn record(");
    assert!(
        record.contains("class: DecisionClass"),
        "record must take a required class; got:\n{record}"
    );
    assert!(
        !record.contains("Option<DecisionClass>"),
        "the class became optional — that is the defaulting the plan forbade"
    );
    assert!(
        !record.contains("Default::default()"),
        "the class gained a default — an unclassified site must not compile"
    );

    // (2) The site table: one row per model surface, each naming its class.
    const SITES: [(&str, &str, &str); 3] = [
        (
            "src/agentloop/run_loop.rs",
            "DecisionClass::OpenGenerate",
            "the LLM provider stream",
        ),
        (
            "src/screen.rs",
            "DecisionClass::Classify",
            "the injection screen",
        ),
        ("src/embed.rs", "DecisionClass::Encode", "the embedder"),
    ];
    for (file, class, what) in SITES {
        let src = strip_comments(&read_src(file));
        assert!(
            src.contains(class),
            "{what} ({file}) no longer names {class} — a model surface stopped \
             declaring its class, so its spend became unattributable"
        );
    }
}

// ── P53a.3 — the emitted telemetry names ────────────────────────────────────

/// `P53a.3`, as the prompt requires it: the pin asserts the **emitted** name
/// matches the **preregistered** name. Not "a row exists" — the literal.
#[test]
fn the_emitted_metric_names_are_the_preregistered_ones() {
    let core = strip_comments(&read_src("src/server/router/core.rs"));
    for name in [
        "brain_model_calls_total",
        "brain_model_tokens_total",
        "brain_model_incomplete_total",
    ] {
        assert!(
            core.contains(&format!("# TYPE {name} counter")),
            "{name} must be exported as a counter, and its name is the preregistered \
             one — a rename here breaks every downstream reader of the schema"
        );
    }
    // The label is not a literal in the source — it is `as_str()` on the
    // census row, which is what makes the minimisation property (above) hold on
    // the wire too. So the assertion is that the emitted row is BOUND to the
    // class's own label, not that a hand-typed string is present: a literal
    // here would be a fourth place to update when a class is added, and a
    // fourth place to forget.
    assert!(
        core.contains("let class = row.class.as_str();"),
        "the emitted label must come from `DecisionClass::as_str`, not a literal"
    );
    for template in [
        "brain_model_calls_total{{class=\\\"{class}\\\"}}",
        "brain_model_tokens_total{{class=\\\"{class}\\\"}}",
        "brain_model_incomplete_total{{class=\\\"{class}\\\"}}",
    ] {
        assert!(
            core.contains(template),
            "the emitted row template changed: {template} not found. The preregistered \
             name is part of the frozen schema (P53a.3)."
        );
    }
    // And the family iterates the census, so the emitted set cannot drift from
    // the declared set in either direction.
    assert!(
        core.contains("for row in crate::decision_class::observations() {"),
        "the family must iterate the census, so every declared class emits a row"
    );
    for forbidden in [
        "class=\"model",
        "class=\"domain",
        "class=\"user",
        "class=\"tenant",
    ] {
        assert!(
            !core.contains(forbidden),
            "a model/domain/user/tenant label reached the decision-class family: \
             {forbidden}. The class is derived from the call site and carries no \
             content, principal, or scope."
        );
    }
}

/// The counters move, and only on the surface that moved. The process-global
/// sharing (every sibling test in this binary can increment) makes the
/// cross-class assertion racy, so what is asserted here is what is deterministic:
/// a note lands on its own class, and no counter ever moves backwards.
///
/// Separation is pinned non-racily by the in-crate test
/// `r53a_recorded_spend_is_attributed_to_its_class_and_the_total_is_unchanged`,
/// which records two different classes on one budget — had they shared a bucket,
/// the second class's delta would have been zero.
#[test]
fn a_class_counter_moves_for_the_class_that_was_noted_and_never_backwards() {
    let read = |class: DecisionClass| -> ClassObservation {
        observations()
            .into_iter()
            .find(|r| r.class == class)
            .expect("the census emits every declared class")
    };
    let before: Vec<ClassObservation> = observations();
    brain_server::decision_class::note_call(DecisionClass::Classify);
    let after: Vec<ClassObservation> = observations();

    let classify_delta = read(DecisionClass::Classify).calls.saturating_sub(
        before
            .iter()
            .find(|r| r.class == DecisionClass::Classify)
            .expect("census is stable")
            .calls,
    );
    assert!(
        classify_delta >= 1,
        "noting a Classify call did not move the Classify counter"
    );
    for (b, a) in before.iter().zip(&after) {
        assert!(
            a.calls >= b.calls,
            "{}: the call counter went backwards — a counter that decreases is a \
             counter an operator cannot trust",
            a.class.as_str()
        );
        assert!(
            a.tokens >= b.tokens,
            "{}: the token counter went backwards",
            a.class.as_str()
        );
        assert!(
            a.incomplete >= b.incomplete,
            "{}: the incomplete counter went backwards",
            a.class.as_str()
        );
    }
}
