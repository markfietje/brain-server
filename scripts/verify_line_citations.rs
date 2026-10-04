// Verifies every line citation in IMPL_R57B_LINEAGE_AND_CITATION_2026-09-29.md
// against the live tree. A plan that cites a line which has moved is worse than
// one that cites nothing: it transfers a claim the reader will not re-check.
use std::fs;
use std::path::Path;

fn main() {
    let root = Path::new("/Users/mark/Sites/brain-server");
    let checks: &[(&str, usize, &str)] = &[
        ("src/workflow/registry.rs", 21, "citation travels on the run's trace"),
        ("src/workflow/registry.rs", 460, "fn register_model"),
        ("src/workflow/registry.rs", 659, "fn apply_lifecycle"),
        ("src/workflow/registry.rs", 808, "fn resolve_for_execution"),
        ("src/handlers/decision_runs.rs", 199, "fn load_bound_model"),
        ("src/handlers/decision_runs.rs", 236, "resolve_for_execution"),
        // NOT a writer of decision_run_traces — measured 0 references, and
        // the citation it resolves already lands on `delivery_traces`
        // (model_ref + config_digest). Kept as an anchor BECAUSE it is the
        // seam the plan wrongly claimed as a second trace writer.
        ("src/workflow/delivery.rs", 904, "resolve_for_execution"),
        ("src/migration.rs", 2400, "decision_run_traces"),
        ("src/migration.rs", 2473, "ALTER TABLE proposals ADD COLUMN decision_run_ref"),
        ("src/migration.rs", 2481, "decision_model_registry"),
        ("src/migration.rs", 2512, "model_registry_id"),
        ("src/storage_layout.rs", 272, "LATEST_KNOWN_SCHEMA"),
        // The refuse-newer probe, ABOVE the 1.32.21 ceiling by construction.
        ("src/storage_layout.rs", 729, "1.32.22"),
        ("src/workflow/outbox.rs", 49, "RESERVED_OUTBOX_TOPICS"),
        ("src/workflow/outbox.rs", 290, "fn append_lineage"),
        ("src/workflow/gdl.rs", 3898, "fn write_handoff_transition"),
        ("src/workflow/gdl.rs", 4044, "fn write_back_referral_return"),
        ("src/workflow/pipeline.rs", 139, "fn advance_pipeline"),
        // The two global `COUNT(*) FROM outbox = 0` assertions the round
        // walks into. The plan named ONE; the tree has TWO. Both sit on
        // drawbridge_state() databases, i.e. per-test isolated, which is
        // why neither moved — but a reader deserves the whole risk class.
        ("tests/main_suite.rs", 8588, "SELECT COUNT(*) FROM outbox"),
        ("tests/main_suite.rs", 15139, "SELECT COUNT(*) FROM outbox"),
    ];
    let mut bad = 0;
    for (rel, line, needle) in checks {
        let path = root.join(rel);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                bad += 1;
                println!("DRIFT cannot read {rel}: {e}");
                continue;
            }
        };
        let actual = text.lines().nth(line - 1).unwrap_or("<past EOF>");
        if !actual.contains(needle) {
            bad += 1;
            println!("DRIFT {rel}:{line} expected `{needle}`");
            println!("            actual  `{}`", actual.trim());
        }
    }
    // Each cited site must be the ONLY definition of its fn.
    for (rel, fn_name) in [
        ("src/workflow/gdl.rs", "fn write_handoff_transition"),
        ("src/workflow/gdl.rs", "fn write_back_referral_return"),
        ("src/workflow/pipeline.rs", "fn advance_pipeline"),
    ] {
        let text = fs::read_to_string(root.join(rel)).unwrap();
        let n = text.matches(fn_name).count();
        if n != 1 {
            bad += 1;
            println!("DRIFT {rel}: `{fn_name}` appears {n} times; the citation must be unambiguous");
        }
    }
    if bad == 0 {
        println!("OK all {} citations current", checks.len());
    } else {
        println!("\n{bad} citation(s) DRIFTED — fix the plan, not the tree.");
        std::process::exit(1);
    }
}
