#!/usr/bin/env bash
# Rename the round-numbered test files to subject names.
#
# A test file name is an API: it tells a reader what invariant is enforced and
# what to grep when it fires. `r45_0_claim_pins.rs` names a ROUND, which is a
# point in the programme's history — a fact about when a pin was written, not
# about what it enforces. The round lives in the file's header comment, where a
# reader can act on it; the filename should carry the SUBJECT.
#
# Pre-checked: no CI job, script, or [[test]] entry names any of these targets,
# and src/spire_inventory.rs mentions them only in prose comments.
set -euo pipefail

cd "$(dirname "$0")/.."

# old:new  — subject, not round.
map=(
  "r46_evidence_pins.rs:evidence_byte_range_pins.rs"
  "r47_rbac_pins.rs:rbac_evaluation_pins.rs"
  "r48_clean_cycle.rs:clean_cycle_pins.rs"
  "r50_create.rs:create_loop_pins.rs"
  "r51_gate_law_pins.rs:gate_law_pins.rs"
  "r51_taxonomy_pins.rs:loop_taxonomy_pins.rs"
  "r51_version_axis_db.rs:version_axis_db_pins.rs"
  "r51_version_axis_pins.rs:version_axis_pins.rs"
  "r53a_decision_class_pins.rs:decision_class_pins.rs"
  "r55p_agreement_pins.rs:agreement_path_pins.rs"
  "r57_census_pins.rs:drift_census_pins.rs"
  "r57b_citation_pins.rs:trace_citation_pins.rs"
  "r57b_lineage_pins.rs:decision_lineage_pins.rs"
  "r61p_per_domain_axis.rs:per_domain_axis_pins.rs"
  "r63_azp_pins.rs:azp_enforcement_pins.rs"
  "r63a_determinism_pins.rs:determinism_contract_pins.rs"
  "r65_confidence_pins.rs:confidence_input_pins.rs"
  "r65b_ladder_pins.rs:approve_ladder_pins.rs"
  "r66a_verdict_carry_pins.rs:verdict_carry_pins.rs"
  "r66b_join_refusal_pins.rs:join_refusal_pins.rs"
  "r7_scanner_pins.rs:scanner_order_pins.rs"
  "r45_0_claim_pins.rs:external_claim_pins.rs"
)

for pair in "${map[@]}"; do
  old="${pair%%:*}"
  new="${pair##*:}"
  src="tests/$old"
  dst="tests/$new"
  [[ -f "$src" ]] || { echo "MISSING: $src" >&2; exit 1; }
  if [[ -e "$dst" ]]; then echo "COLLISION: $dst already exists" >&2; exit 1; fi
  git mv "$src" "$dst"
  echo "  $old -> $new"
done

echo "renamed ${#map[@]} test files"
