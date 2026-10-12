# Test census — what the suite is made of

Machine-derived by `scripts/test-census.py`. **Re-derive, never edit:**

```
scripts/test-census.py --check    # exits non-zero on drift
```

No commit hash is recorded here on purpose: a hash is the one field
the script cannot derive from the tree, so recording one would make
`--check` depend on an environment variable and stop the table from
re-deriving byte-for-byte. Git already records which commit shipped it.

## The classes, and why they are ordered

First match wins; every test gets exactly one class. Full rule text,
including the stdlib allowlist that decides rule 4, is the module
docstring in `scripts/test-census.py` — the rules live with the script
that applies them, not in prose that can drift from it.

1. `residual_documentation` — asserts a stated limitation ON PURPOSE.
2. `control` — the anti-vacuity arm that makes a pin's failure readable.
3. `structural` — reads the tree (source text, wiring, manifest).
4. `behavioral_pin` — POSITIVE evidence of a drive: the body invokes at
   least one non-stdlib callable.
5. `unclassified` — no evidence of driving a subject. Reported, never
   folded into 4 by default: "no read found" is precisely the shape of
   the four vacuous census pins R123 shipped on first draft.

## The count

| Class | Tests | Share |
|---|---:|---:|
| `behavioral_pin` | 2849 | 83.8% |
| `structural` | 175 | 5.1% |
| `control` | 58 | 1.7% |
| `residual_documentation` | 53 | 1.6% |
| `unclassified` | 266 | 7.8% |
| **total** | **3401** | |

Scope: `src/` + `tests/`, the two trees the badge's count comes from.
The census reads HIGHER than the README badge for two reasons, both
expected. The badge counts tests that
*passed* a real run, after twelve `--skip` families remove the
platform-only sandbox tests (`scripts/badges.sh`); the census counts
tests as written. And a raw `rg` count of `#[test]` over-reads this
tree by ~64, because tests that pin a text-scanner carry `#[test]`
inside their own string literals (tests/rbac_evaluation_pins.rs has
eleven such cases). The census counts what the compiler does.

## The top ten files by size

| File | Tests |
|---|---:|
| `tests/main_suite.rs` | 355 |
| `src/workflow/gdl.rs` | 122 |
| `src/workflow/decide/router.rs` | 59 |
| `src/workflow/delivery.rs` | 54 |
| `src/agentloop/run_loop.rs` | 51 |
| `src/bin/mcp.rs` | 48 |
| `src/bin/brain.rs` | 43 |
| `src/screen.rs` | 43 |
| `src/search/tests.rs` | 40 |
| `tests/authz_matrix.rs` | 39 |

## `unclassified` — the honesty check on the rule set

266 tests invoke nothing outside the assert vocabulary.
They are listed so a reader can move them by adding evidence, not
by re-deciding.

| Test |
|---|
| `src/agentloop/compaction_probes.rs:139` `short_or_repeated_tokens_never_become_probes` |
| `src/audit/decision.rs:766` `nul_in_engine_field_refuses_the_record` |
| `src/auth/policy.rs:553` `client_authorized_domains_is_restricted_only_for_client_auditor` |
| `src/auth/revocation.rs:398` `revocation_cache_is_stateless_unit_struct` |
| `src/authz/gates.rs:210` `r47_an_unknown_route_has_no_gate` |
| `src/authz/policy.rs:300` `r47_oracle_is_total_over_its_closed_vocabulary` |
| `src/bin/bench.rs:998` `ln_gamma_matches_known_values` |
| `src/bin/bench.rs:1027` `regularized_incomplete_beta_endpoints_and_symmetry` |
| `src/bin/bench.rs:1045` `cp_zero_of_500_matches_the_closed_form` |
| `src/bin/bench.rs:1059` `cp_is_monotone_in_the_success_count` |
| `src/bin/bench.rs:1101` `cp_handles_degenerate_inputs` |
| `src/bin/bench.rs:1569` `percentile_single_sample` |
| `src/bin/bench.rs:1577` `percentile_empty_is_zero` |
| `src/bin/bench.rs:1582` `percentile_five_samples` |
| `src/bin/brain.rs:5981` `doc_indices_parse_recall_hits_and_search_results` |
| `src/bin/brain.rs:6015` `boolean_flag_rejects_explicit_value` |
| `src/bin/brain.rs:6096` `floors_parse_and_reject_typos` |
| `src/bin/brain.rs:6468` `ump_requires_a_subcommand` |
| `src/bin/brain.rs:6575` `dsar_requires_explicit_action` |
| `src/bin/brain.rs:6587` `dsar_unknown_action_names_choices` |
| `src/bin/brain.rs:6611` `dsar_purge_prompts_without_yes` |
| `src/bin/brain.rs:6625` `dsar_yes_skips_prompt` |
| `src/bin/brain.rs:6805` `restore_prompts_even_with_force` |
| `src/bin/brain.rs:6815` `restore_yes_flag_is_the_automation_seam` |
| `src/bin/brain_migrate_rehearse.rs:1185` `schema_ge_handles_versions` |
| `src/bin/mcp.rs:1656` `ump_route_table_pins_methods_and_paths` |
| `src/bin/mcp.rs:2198` `non_loopback_bind_without_token_refused_fail_closed` |
| `src/bin_common/http.rs:363` `first_token_takes_one_slot_and_drops_rotation_remainder` |
| `src/breach.rs:393` `validate_open_bounds_fields` |
| `src/chunker.rs:787` `empty_input_yields_no_chunks` |
| `src/config.rs:2169` `require_webhook_signing_polarity` |
| `src/config.rs:2488` `health_db_echo_follows_injection_policy_env` |
| `src/config.rs:2532` `write_posture_resolves_and_validates` |
| `src/config.rs:2598` `injection_thresholds_refuse_bad_values` |
| `src/connector/auth/github_app.rs:514` `test_parse_github_timestamp_handles_canonical_format` |
| `src/connector/delivery/mod.rs:446` `the_digest_is_length_prefixed_so_triples_cannot_collide` |
| `src/connector/github/client.rs:234` `test_parse_link_rel_returns_none_when_no_link_header` |
| `src/connector/github/mod.rs:318` `test_get_cursor_returns_none_when_unset` |
| `src/connector/github/mod.rs:324` `test_upsert_then_get_roundtrips` |
| `src/connector/github/mod.rs:351` `test_cursors_are_isolated_per_connector_and_kind` |
| `src/connector/github/mod.rs:371` `test_extract_title_finds_h1` |
| `src/connector/github/mod.rs:377` `test_extract_title_returns_none_when_no_h1` |
| `src/connector/kind.rs:48` `vocabulary_includes_all_shipped_kinds` |
| `src/connector/kind.rs:66` `unknown_kind_is_rejected` |
| `src/connector/kind.rs:73` `family_splits_sub_kind_and_passes_flat_kind_through` |
| `src/connector/pipeline.rs:125` `connector_source_kind_uses_family_for_prefix_kinds` |
| `src/connector/pipeline.rs:185` `live_uris_collects_every_doc_uri` |
| `src/connector/supervisor.rs:84` `test_next_backoff_is_exponential_and_capped` |
| `src/domain_router.rs:519` `mean_averages_elementwise` |
| `src/domain_router.rs:526` `mean_of_empty_is_empty` |
| `src/domain_router.rs:609` `route_ignores_empty_centroids` |
| `src/domain_router.rs:725` `route_multi_returns_none_when_nothing_clears_the_threshold` |
| `src/domain_router.rs:752` `route_multi_skips_empty_prototype_vectors` |
| `src/domain_router.rs:763` `route_multi_on_empty_input_is_none` |
| `src/domain_router.rs:832` `route_multi_global_shadow_never_hides_a_near_topic` |
| `src/domain_router.rs:971` `route_domain_label_forced_wins_over_centroids` |
| `src/domain_router.rs:996` `route_domain_label_defaults_to_global_without_centroids` |
| `src/domain_router.rs:1003` `route_domain_label_is_deterministic` |
| `src/eval/mod.rs:507` `precision_at_k_hand_computed` |
| `src/eval/mod.rs:514` `recall_at_k_hand_computed` |
| `src/eval/mod.rs:521` `mrr_first_position` |
| `src/eval/mod.rs:526` `mrr_third_position` |
| `src/eval/mod.rs:531` `mrr_no_relevant` |
| `src/eval/mod.rs:536` `ndcg_perfect_ranking_is_one` |
| `src/eval/mod.rs:543` `ndcg_worst_ranking_is_zero` |
| `src/eval/mod.rs:550` `empty_relevant_returns_zero` |
| `src/eval/mod.rs:589` `eval_metrics_compute_correctly` |
| `src/fence.rs:324` `strip_markdown_refs_neutralizes_image_and_link` |
| `src/gate.rs:1308` `origin_for_source_maps_kinds` |
| `src/gate.rs:1329` `pii_scan_is_conservative_on_plain_text` |
| `src/gate.rs:1356` `confidence_factors_are_stored_rules` |
| `src/gate.rs:1365` `relevance_tiers_follow_bands` |
| `src/gate.rs:1372` `decay_is_nullable_and_instant_based` |
| `src/gate.rs:1568` `effective_expiry_own_expires_at_wins` |
| `src/gate.rs:2010` `prose_angle_brackets_survive` |
| `src/handlers/accounts.rs:460` `account_decision_ref_bounds_and_screening` |
| `src/handlers/auth.rs:656` `logout_row_outlives_long_lived_idp_token` |
| `src/handlers/auth.rs:685` `server_minted_logout_unchanged` |
| `src/handlers/compliance.rs:467` `wire_input_caps_reject_absurd_sizes` |
| `src/handlers/compliance.rs:474` `dataset_hash_predicate_is_strict` |
| `src/handlers/gate.rs:2916` `sha256_hex_is_deterministic_hex_of_content` |
| `src/handlers/gate.rs:2973` `review_digest_matches_gates_stale_approval` |
| `src/handlers/kappa.rs:306` `kappa_slot_derivation_is_pure_and_in_roster` |
| `src/handlers/mod.rs:1296` `audit_scope_none_principal_passes_requested_tenant_through` |
| `src/handlers/recall.rs:999` `global_rescue_flag_marks_cross_domain_mixing` |
| `src/handlers/recall.rs:1009` `shim_routing_targets_routed_domain_plus_global_rescue` |
| `src/handlers/recall.rs:1017` `shim_routing_targets_routed_to_global_scopes_to_global` |
| `src/handlers/recall.rs:1324` `results_to_hits_empty` |
| `src/handlers/reflection.rs:121` `corpus_limit_bounds_are_pinned` |
| `src/handlers/reflection.rs:135` `corpus_partition_vocabulary_is_closed` |
| `src/handlers/relay.rs:486` `decline_reason_validation_bounds_hold` |
| `src/handlers/suggest.rs:584` `validate_suggest_rejects_empty_context` |
| `src/handlers/suggest.rs:590` `validate_suggest_rejects_oversized_context` |
| `src/handlers/suggest.rs:609` `validate_suggest_accepts_valid_request_and_trims_context` |
| `src/handlers/ump.rs:829` `ump_round_trip_is_identity_on_row_fields` |
| `src/handlers/ump.rs:866` `ump_kind_mapping_keeps_raw_kind` |
| `src/handlers/ump.rs:881` `ump_rejects_malformed_records` |
| `src/handlers/ump.rs:1084` `emit_record_meta_overlay_round_trips` |
| `src/handlers/ump.rs:1189` `record_from_markdown_rejects_bad_headers` |
| `src/handlers/ump.rs:1196` `hex_helpers_round_trip` |
| `src/handlers/ump_ops.rs:1191` `filter_kinds_map_to_brain_memory_kinds` |
| `src/handlers/ump_ops.rs:1201` `feedback_followed_is_the_only_accept` |
| `src/handlers/ump_ops.rs:1209` `deep_merge_patches_nested_fields_and_ignores_id_integrity` |
| `src/handlers/verify.rs:190` `verify_claim_finds_case_insensitive_matches` |
| `src/handlers/verify.rs:209` `verify_claim_is_non_overlapping` |
| `src/handlers/verify.rs:216` `verify_claim_empty_claim_returns_empty` |
| `src/handlers/verify.rs:221` `verify_claim_no_match_returns_empty` |
| `src/handlers/workflow_decisions.rs:266` `decision_ref_bounds_and_screening` |
| `src/handlers/workflow_decisions.rs:286` `contract_key_required_nonempty_and_bounded` |
| `src/hygiene.rs:126` `strips_paired_thinking_block_with_content` |
| `src/hygiene.rs:132` `strips_is_think_tag` |
| `src/hygiene.rs:137` `strips_case_insensitive_and_attributes` |
| `src/hygiene.rs:154` `no_tags_passthrough_unchanged` |
| `src/hygiene.rs:169` `multiple_blocks_all_stripped` |
| `src/hygiene.rs:177` `should_skip_matches_configured_prefix` |
| `src/hygiene.rs:187` `should_skip_ignores_leading_whitespace_and_empty_patterns` |
| `src/kb.rs:1221` `slug_vocabulary_is_strict` |
| `src/legal_hold.rs:299` `validate_rejects_empty_and_oversized` |
| `src/linker.rs:1023` `strip_heading_number_removes_section_prefix` |
| `src/linker.rs:1348` `verb_suffix_accepts_verb_patterns` |
| `src/loom.rs:232` `brain_loom_parse_is_fail_closed` |
| `src/migration.rs:3473` `same_dim_rerun_is_idempotent` |
| `src/pii_mask.rs:175` `luhn_rejects_bad_checksum` |
| `src/procedural.rs:793` `classify_is_case_insensitive` |
| `src/procedural.rs:807` `classify_vendor_lexicon_matches_real_vendors` |
| `src/procedural.rs:881` `parse_condition_handles_all_operators` |
| `src/procedural.rs:891` `parse_condition_rejects_garbage` |
| `src/procedural.rs:906` `matches_op_all_six_comparisons` |
| `src/qa.rs:28` `scope_violation_flags_cross_domain_when_restricted` |
| `src/qa.rs:35` `scorecard_is_deterministic_and_bounded` |
| `src/qa.rs:49` `scorecard_penalizes_out_of_scope_sharply` |
| `src/qa.rs:61` `score_for_absent_trace_is_cited_neutral_not_nan` |
| `src/screen.rs:990` `suspicious_pattern_allows_benign_content` |
| `src/screen.rs:1328` `strip_invisible_preserves_visible_unicode` |
| `src/screen.rs:1629` `hex_wrapped_instruction_caught` |
| `src/screen.rs:1680` `no_false_positive_drift_on_clean_corpus` |
| `src/search/graph_ppr.rs:577` `type_base_weight_downgrades_taxonomy_noise` |
| `src/search/mod.rs:244` `rerank_top_n_absent_or_garbage_reads_the_default` |
| `src/search/mod.rs:253` `rerank_top_n_parses_and_caps_at_the_ceiling` |
| `src/search/packing.rs:390` `token_estimate_is_chars_div_four` |
| `src/search/query.rs:435` `lex_phrases_are_quoted` |
| `src/search/query.rs:440` `lex_exclusions_get_minus_prefix` |
| `src/search/query.rs:450` `lex_strips_embedded_quotes` |
| `src/search/query.rs:455` `lex_combines_all_kinds_with_implicit_and` |
| `src/search/tests.rs:42` `rrf_formula_correctness` |
| `src/search/tests.rs:70` `rrf_empty_inputs` |
| `src/search/tests.rs:260` `prf_empty_hits` |
| `src/search/tests.rs:373` `normalize_since_accepts_rfc3339_and_naive` |
| `src/search/tests.rs:386` `normalize_since_rejects_garbage` |
| `src/search/tests.rs:395` `highlight_ranges_finds_term_offsets_within_window` |
| `src/server/bootstrap.rs:1625` `bind_public_deprecation_is_limited_to_the_public_opt_in_path` |
| `src/server/router/memory.rs:3713` `tab_newline_behavior_pinned` |
| `src/service/ingest.rs:714` `screen_structured_rejects_fail_closed_and_quarantines_scrapes` |
| `src/service/register.rs:547` `validate_new_client_rejects_bad_name_and_jurisdiction` |
| `src/service/reindex.rs:36` `reindex_candidates_exclude_quarantined` |
| `src/service/review.rs:772` `decision_run_ref_mode_extraction_is_pure` |
| `src/service/review.rs:799` `approval_quorum_parses_fail_closed` |
| `src/service/write_deadline.rs:95` `a_write_with_budget_proceeds` |
| `src/service/write_deadline.rs:127` `the_margin_holds_back_from_the_router_deadline` |
| `src/sources.rs:376` `revision_is_full_content_hash` |
| `src/standby.rs:649` `wal_frame_count_arithmetic` |
| `src/storage_layout.rs:859` `is_valid_domain_accepts_safe_names` |
| `src/strip_invisible.rs:80` `arabic_letter_mark_stripped` |
| `src/strip_invisible.rs:86` `supplementary_variation_selectors_stripped` |
| `src/strip_invisible.rs:93` `existing_invisible_classes_still_stripped` |
| `src/strip_invisible.rs:115` `strip_fns_idempotent` |
| `src/temporal.rs:369` `determinism_same_input_same_output` |
| `src/trace.rs:35` `caps_are_finite` |
| `src/trace.rs:47` `trace_traversal_caps_are_bounded` |
| `src/ump_integrity.rs:463` `base32_encodes_rfc4648_vectors` |
| `src/ump_integrity.rs:474` `did_key_vector_is_stable` |
| `src/ump_integrity.rs:608` `content_id_is_deterministic_and_content_bound` |
| `src/ump_integrity.rs:618` `sign_verify_round_trip_and_tamper_detection` |
| `src/vault.rs:279` `parses_plain_wikilink` |
| `src/vault.rs:285` `parses_alias_and_heading_forms` |
| `src/vault.rs:297` `dedups_repeated_targets_case_insensitively` |
| `src/workflow/accounts.rs:571` `account_name_screening_refuses_control_invisible` |
| `src/workflow/calibration.rs:139` `report_due_then_stamped_for_a_week` |
| `src/workflow/case_status.rs:497` `base32_matches_rfc4648_vectors` |
| `src/workflow/create/replay_gate.rs:311` `the_gate_never_writes_and_never_repairs` |
| `src/workflow/decide/calibration.rs:128` `confidence_uniform_is_zero` |
| `src/workflow/decide/calibration.rs:137` `confidence_one_hot_is_one` |
| `src/workflow/decide/calibration.rs:142` `confidence_k_of_one_is_one` |
| `src/workflow/decide/calibration.rs:146` `confidence_k_below_two_is_one` |
| `src/workflow/decide/calibration.rs:150` `confidence_peaked_beats_flat` |
| `src/workflow/decide/calibration.rs:170` `ece_empty_is_nan_never_zero` |
| `src/workflow/decide/calibration.rs:175` `ece_length_mismatch_is_nan` |
| `src/workflow/decide/calibration.rs:179` `ece_hand_computed_two_bin_case` |
| `src/workflow/decide/calibration.rs:188` `ece_perfect_calibration_is_near_zero` |
| `src/workflow/decide/calibration.rs:222` `score_to_units_accepts_the_grid` |
| `src/workflow/decide/calibration.rs:231` `units_round_trip` |
| `src/workflow/decide/lang.rs:386` `script_english` |
| `src/workflow/decide/lang.rs:390` `script_unknown_for_empty` |
| `src/workflow/decide/lang.rs:394` `script_unknown_for_digits` |
| `src/workflow/decide/lang.rs:398` `script_greek` |
| `src/workflow/decide/lang.rs:402` `script_cyrillic` |
| `src/workflow/decide/lang.rs:406` `script_hebrew` |
| `src/workflow/decide/lang.rs:410` `script_arabic` |
| `src/workflow/decide/lang.rs:414` `script_devanagari` |
| `src/workflow/decide/lang.rs:418` `script_bengali` |
| `src/workflow/decide/lang.rs:422` `script_tamil` |
| `src/workflow/decide/lang.rs:426` `script_thai` |
| `src/workflow/decide/lang.rs:430` `script_hangul` |
| `src/workflow/decide/lang.rs:434` `script_kana` |
| `src/workflow/decide/lang.rs:438` `script_han` |
| `src/workflow/decide/lang.rs:444` `is_english_ordinary_text` |
| `src/workflow/decide/lang.rs:450` `is_english_empty_state` |
| `src/workflow/decide/lang.rs:454` `is_english_digits_only` |
| `src/workflow/decide/lang.rs:458` `is_english_french_is_not` |
| `src/workflow/decide/lang.rs:464` `is_english_cyrillic_is_not` |
| `src/workflow/decide/lang.rs:468` `is_english_nested_state_flattens` |
| `src/workflow/decide/lang.rs:479` `latin_lang_french` |
| `src/workflow/decide/lang.rs:486` `latin_lang_spanish` |
| `src/workflow/decide/lang.rs:493` `latin_lang_german` |
| `src/workflow/decide/lang.rs:500` `latin_lang_english_stays_en` |
| `src/workflow/decide/lang.rs:507` `latin_lang_too_short_is_none` |
| `src/workflow/decide/lang.rs:511` `latin_lang_long_english_stays_en` |
| `src/workflow/decide/lang.rs:521` `state_text_string_passthrough` |
| `src/workflow/decide/lang.rs:525` `state_text_flattens_lists_and_dicts` |
| `src/workflow/decide/presets.rs:193` `presets_validate_schema_passes` |
| `src/workflow/decide/presets.rs:224` `guard_keys_are_the_source_vocabulary` |
| `src/workflow/decide/router.rs:316` `alias_en_laya_default_resolve_to_english` |
| `src/workflow/decide/router.rs:322` `alias_multi_ml_resolve_to_multilingual` |
| `src/workflow/decide/router.rs:328` `alias_typed_family_resolves` |
| `src/workflow/decide/router.rs:334` `alias_unknown_is_a_named_refusal` |
| `src/workflow/decide/router.rs:339` `alias_case_and_whitespace_normalise` |
| `src/workflow/decide/router.rs:347` `alias_empty_refuses` |
| `src/workflow/decide/router.rs:351` `alias_checkpoints_table_is_the_three_names` |
| `src/workflow/decide/router.rs:355` `alias_every_table_name_normalises_to_itself` |
| `src/workflow/decide/router.rs:361` `alias_bundle_repo_is_the_english_root` |
| `src/workflow/decide/router.rs:367` `workflow_agent_trace_exact_match` |
| `src/workflow/decide/router.rs:380` `workflow_customer_service_exact_match` |
| `src/workflow/decide/router.rs:393` `workflow_invoice_processing_exact_match` |
| `src/workflow/decide/router.rs:406` `workflow_security_incidents_exact_match` |
| `src/workflow/decide/router.rs:419` `workflow_partial_set_matches_nothing` |
| `src/workflow/decide/router.rs:426` `workflow_superset_matches_nothing` |
| `src/workflow/decide/router.rs:440` `workflow_empty_matches_nothing` |
| `src/workflow/decide/router.rs:487` `route_explicit_model_unknown_refuses` |
| `src/workflow/decide/router.rs:741` `bundle_english_is_the_repo_root` |
| `src/workflow/decide/router.rs:745` `bundle_multilingual_is_a_subfolder` |
| `src/workflow/decide/router.rs:752` `bundle_typed_is_a_subfolder` |
| `src/workflow/decide/router.rs:759` `bundle_standalone_english_stays_the_root` |
| `src/workflow/decide/router.rs:763` `bundle_standalone_mirrors_differ` |
| `src/workflow/decide/sequence.rs:607` `build_sequence_option_order_must_be_a_permutation` |
| `src/workflow/decide/sequence.rs:646` `validate_schema_refuses_empty` |
| `src/workflow/decision_eval.rs:1754` `absent_and_unauthorized_evaluation_are_probe_blind` |
| `src/workflow/delivery.rs:3211` `delivery_absent_row_is_probe_blind_and_unaudited` |
| `src/workflow/gdl.rs:5772` `sla_clock_table_pins_the_p_class_literals` |
| `src/workflow/sandbox.rs:691` `profile_render_refuses_metacharacter_paths` |
| `src/workflow/session_log.rs:1013` `scoped_replay_refuses_malformed_modern_child_but_never_adopts_legacy_child` |
| `src/workflow/session_log.rs:1556` `r1_changed_key_refuses_and_controls_do_not_consume_replay` |
| `src/workflow/session_log.rs:1669` `bounds_are_pinned_and_refuse_loud` |
| `src/workflow/valet.rs:624` `consent_registry_gates_signal_and_is_single_subject` |
| `tests/approve_ladder_pins.rs:461` `b2_no_governed_branch_kind_is_also_a_promote_kind` |
| `tests/approve_ladder_pins.rs:751` `governed_kinds_are_always_promotable` |
| `tests/azp_enforcement_pins.rs:187` `blank_azp_is_treated_as_absent` |
| `tests/azp_enforcement_pins.rs:231` `blank_expectation_is_unbound_not_permissive` |
| `tests/eval_joint_objective_pins.rs:169` `eval_joint_r53_1_the_pre_r53_objective_has_no_safety_or_cost_input` |
| `tests/metrics.rs:110` `test_recall_at_k_metric` |
| `tests/metrics.rs:129` `test_precision_at_k_metric` |
| `tests/metrics.rs:148` `test_ndcg_at_k_metric` |
| `tests/metrics.rs:178` `test_mrr_metric` |
| `tests/metrics.rs:270` `test_stale_result_rate` |
| `tests/metrics.rs:290` `test_current_evidence_recall` |
| `tests/metrics.rs:327` `test_consolidation_false_positive_rate` |
| `tests/per_domain_axis_pins.rs:100` `r61p_publish_bumps_the_axis` |

## What this census does NOT police

It is not a gate on class counts. There is no floor on any class in
this round: a floor written before its population is measured is a
number nobody checked. The only thing that can fail is **drift** — the
committed table disagreeing with a fresh derivation (`--check`).

