//! v0.9.1 Milestone 5 — Recall quality eval.
//!
//! Measures recall@5 across the search configurations to prove each lever
//! (hybrid, PRF) adds marginal lift. The metric itself is unit-tested here;
//! the full model-backed eval runs against a live server (see BENCHMARKS.md
//! for recorded numbers).

#![cfg(test)]

// Eval document set — 10 topically distinct notes. Used by the manual eval
// harness (ingest these, then query and measure recall@5).
#[allow(dead_code)]
const DOCS: &[&str] = &[
    "Bignay is a tropical fruit and a good alternative to blueberry, rich in antioxidants.",
    "The Rust programming language guarantees memory safety without a garbage collector.",
    "Vitamin D3 supplementation improves immune function and bone density in deficient adults.",
    "The GDPR is a European regulation protecting the personal data of EU residents.",
    "Gut microbiome diversity affects inflammation markers and immune system regulation.",
    "SQLite is an embedded relational database with FTS5 full-text search support.",
    "ISO 9001 is the international standard for quality management systems.",
    "Ownership and borrowing are Rust's core concepts for compile-time memory safety.",
    "Antioxidants in tropical fruits like bignay help reduce oxidative stress.",
    "The GDPR covers any organization processing EU residents' data, with fines up to four percent of global revenue.",
    // ── migration vertical (indices 10-14) ──
    "VxRail LCM upgrades require a green RCM release certification manifest before any upgrade wave is scheduled.",
    "A stretched-cluster rolling reboot reboots one ESXi node at a time; never reboot two nodes concurrently.",
    "vSAN storage policies set FTT failures to tolerate and FTM failure tolerance method per virtual machine.",
    "PowerFlex protection domains map fault sets to failure boundaries across SDS storage pools.",
    "NSX-T managers push micro-segmentation firewall rules to transport nodes over the control plane.",
    // ── legal vertical (indices 15-19) ──
    "A DPA data processing agreement under GDPR Article 28 binds the processor to the controller's instructions.",
    "Standard Contractual Clauses 2021 are the approved EU transfer mechanism for processors outside the EEA.",
    "RA 10173 the Philippine Data Privacy Act requires NPC breach notification within 72 hours.",
    "Schrems II requires a transfer impact assessment before any personal-data transfer to a third country.",
    "Legal holds freeze erasure until every hold is explicitly released by the operator.",
    // ── troubleshoot vertical (indices 20-24) ──
    "Intermittent storage fabric latency usually traces to a failing SFP on one uplink port, not the array.",
    "High VM disk latency triage order: vSAN backend congestion, then host cache, then the physical disk group.",
    "A node flapping out of vCenter management is most often NTP drift breaking certificate validation.",
    "PSOD purple diagnostic screen dumps land in var log and must be collected before any reboot clears them.",
    "vMotion failing at ten percent points to VMkernel port mobility or a missing shared datastore.",
];

// Eval queries — each maps to the indices of relevant docs (0-based).
// A query passes recall@5 if ALL relevant docs appear in the top 5.
#[allow(dead_code)]
const QUERIES: &[(&str, &[usize])] = &[
    ("blueberry alternative fruit", &[0, 8]),
    ("memory safe programming language", &[1, 7]),
    ("vitamin supplements immune health", &[2]),
    ("EU data protection regulation", &[3, 9]),
    ("gut inflammation microbiome", &[4]),
    ("embedded database search", &[5]),
    ("quality management standard", &[6]),
    ("GDPR organization coverage", &[3, 9]),
    ("antioxidants tropical fruit stress", &[0, 8]),
    ("Rust ownership borrowing", &[1, 7]),
    // ── migration vertical gold set (docs 10-14) ──
    ("RCM green before upgrade wave", &[10]),
    ("release certification manifest VxRail", &[10]),
    ("stretched cluster rolling reboot", &[11]),
    ("reboot one node at a time", &[11]),
    ("vSAN policy failures to tolerate", &[12]),
    ("FTT FTM storage policy", &[12]),
    ("PowerFlex fault sets boundaries", &[13]),
    ("protection domain SDS pools", &[13]),
    ("micro-segmentation rules transport nodes", &[14]),
    ("NSX-T control plane firewall", &[14]),
    ("LCM wave scheduling prerequisite", &[10]),
    ("ESXi concurrent reboot rule", &[11]),
    // ── legal vertical gold set (docs 15-19) ──
    ("processor bound to controller instructions", &[15]),
    ("Article 28 data processing agreement", &[15]),
    ("EU transfer mechanism outside EEA", &[16]),
    ("standard contractual clauses 2021", &[16]),
    ("Philippine breach notification deadline", &[17]),
    ("NPC 72 hours data privacy act", &[17]),
    ("transfer impact assessment requirement", &[18]),
    ("third country data transfer ruling", &[18]),
    ("hold freezes erasure until released", &[19]),
    ("legal hold explicit release", &[19]),
    // ── troubleshoot vertical gold set (docs 20-24) ──
    ("storage latency failing SFP uplink", &[20]),
    ("intermittent fabric latency cause", &[20]),
    ("VM disk latency triage order", &[21]),
    ("vSAN congestion host cache check", &[21]),
    ("node flapping out of vCenter", &[22]),
    ("NTP drift certificate validation", &[22]),
    ("purple screen dump collection", &[23]),
    ("PSOD logs before reboot", &[23]),
    ("vMotion fails at ten percent", &[24]),
    ("VMkernel shared datastore check", &[24]),
    // ── cross-category additions (semantic/negation/freshness/code) ──
    ("which fruit lowers oxidative damage", &[0, 8]),
    ("garbage-collector-free memory model", &[1, 7]),
    ("bone density supplement recommendation", &[2]),
    ("who enforces European privacy law", &[3, 9]),
    ("diet and immune system link", &[4]),
    ("in-process database with text search", &[5]),
    ("international quality certification", &[6]),
    ("compile-time safety guarantees Rust", &[1, 7]),
    ("tropical superfruit antioxidants", &[0, 8]),
    ("privacy fines percentage of revenue", &[3, 9]),
    ("upgrade planning checklist", &[10]),
    ("maintenance window node procedure", &[11]),
    ("per-VM resilience settings", &[12]),
    ("scale-out storage fault domains", &[13]),
    ("distributed firewall distribution", &[14]),
    ("who signs the processing agreement", &[15]),
    ("cross-border contract for vendors", &[16]),
    ("manila privacy regulator timeline", &[17]),
    ("assessment before moving data abroad", &[18]),
    ("can we delete while litigation pending", &[19]),
    ("flaky network port hardware swap", &[20]),
    ("slow virtual machine diagnostics", &[21]),
    ("host lost from inventory causes", &[22]),
    ("kernel panic evidence preservation", &[23]),
    ("live migration stuck early stage", &[24]),
    ("NOT a cloud database embedded engine", &[5]),
    ("no garbage collector language", &[1, 7]),
    ("without EEA adequacy what mechanism", &[16, 18]),
    ("erasure blocked during investigation", &[19]),
    ("one at a time not parallel", &[11]),
    ("GDPR", &[3, 9]),
    ("FTT", &[12]),
    ("PSOD", &[23]),
    ("DPA", &[15]),
    ("SFP", &[20]),
    ("RCM", &[10]),
    ("NTP drift", &[22]),
    ("vMotion", &[24]),
    ("SnapSync", &[]), // negation probe: term absent from corpus -> empty relevant
    ("Kubernetes ingress", &[]),
    ("database without a network service", &[5]),
    ("language chosen for systems reliability", &[1, 7]),
    ("supplement for immune deficiency adults", &[2]),
    ("fruit similar to blueberry nutrition", &[0, 8]),
    ("microbiome diversity inflammation markers", &[4]),
    ("full text search embedded sqlite", &[5]),
    ("ISO certification for factories", &[6]),
    ("borrow checker memory safety", &[1, 7]),
    ("bignay health benefits", &[0, 8]),
    ("EU residents personal data rules", &[3, 9]),
    ("upgrade prerequisite manifest check", &[10]),
    ("cluster maintenance single node rule", &[11]),
    ("storage policy tolerance settings", &[12]),
    ("fault boundary mapping storage", &[13]),
    ("firewall rule propagation mechanism", &[14]),
    ("controller processor contract terms", &[15]),
    ("EEA exit data transfer compliance", &[16, 18]),
    ("philippines data privacy regulator", &[17]),
    ("schrems assessment obligation", &[18]),
    ("release of frozen records operator", &[19]),
    ("uplink port errors hardware fault", &[20]),
    ("disk performance investigation steps", &[21]),
    ("time sync breaks management connection", &[22]),
    ("collect diagnostics before restart", &[23]),
    ("shared datastore requirement migration", &[24]),
];

/// recall@k: fraction of relevant docs found in the top-k results.
fn recall_at_k(results: &[i64], relevant: &[usize], k: usize) -> f32 {
    if relevant.is_empty() {
        return 1.0;
    }
    let top_k: std::collections::HashSet<i64> = results.iter().take(k).copied().collect();
    let found = relevant
        .iter()
        .filter(|&&r| top_k.contains(&(r as i64)))
        .count();
    found as f32 / relevant.len() as f32
}

#[test]
fn test_recall_at_k_metric() {
    // Pure unit test of the recall metric — no model needed.
    let results = vec![0, 3, 5, 1, 7, 2, 9]; // ids returned in rank order
    // relevant = [0, 8]. 0 is in top-5 (rank 0); 8 is not in results at all.
    let r5 = recall_at_k(&results, &[0, 8], 5);
    assert!(
        (r5 - 0.5).abs() < 1e-6,
        "1 of 2 relevant in top-5 -> 0.5, got {r5}"
    );

    // All relevant in top-k
    let r = recall_at_k(&[1, 7, 0], &[1, 7], 5);
    assert!((r - 1.0).abs() < 1e-6, "both relevant in top-5 -> 1.0");

    // None relevant
    let r = recall_at_k(&[2, 4, 6], &[0, 8], 5);
    assert!((r - 0.0).abs() < 1e-6, "no relevant -> 0.0");

    // Empty relevant set → perfect recall by convention
    let r = recall_at_k(&[1, 2], &[], 5);
    assert!((r - 1.0).abs() < 1e-6, "empty relevant -> 1.0");
}

/// The frozen set is ≥100 judged queries (the scale requirement pinned since
/// v0.9.1; the 37-query starter set was the wiring fixture, not evidence).
#[test]
fn test_eval_frozen_set_meets_scale_floor() {
    assert!(
        QUERIES.len() >= 100,
        "frozen eval set must hold >=100 judged queries, has {}",
        QUERIES.len()
    );
}

/// Per-vertical gold sets exist and are non-empty: migration (docs 10-14),
/// legal (15-19), troubleshoot (20-24).
#[test]
fn test_eval_vertical_gold_sets_present() {
    let vertical = |lo: usize, hi: usize| {
        QUERIES
            .iter()
            .filter(|(_, rel)| rel.iter().any(|&d| d >= lo && d <= hi))
            .count()
    };
    for (name, lo, hi) in [
        ("migration", 10, 14),
        ("legal", 15, 19),
        ("troubleshoot", 20, 24),
    ] {
        let n = vertical(lo, hi);
        assert!(n >= 10, "{name} vertical gold set too thin ({n} queries)");
    }
}

#[test]
fn test_eval_query_coverage() {
    // Sanity: every query has at least one relevant doc, and all doc indices
    // are within the DOCS set bounds.
    for (q, relevant) in QUERIES {
        // Negation probes (deliberately empty relevant sets) are allowed —
        // they measure whether absent terms stay absent.
        if q.contains("Kubernetes") || *q == "SnapSync" {
            assert!(relevant.is_empty(), "negation probe '{q}' must stay empty");
            continue;
        }
        assert!(!relevant.is_empty(), "query '{q}' has no relevant docs");
        for &idx in *relevant {
            assert!(
                idx < DOCS.len(),
                "query '{q}' references out-of-bounds doc {idx}"
            );
        }
    }
}
