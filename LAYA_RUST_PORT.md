# LAYA_RUST_PORT — local-only System-1 decision port for brain-server

> Status: plan — targets `brain-server 1.28.91`, `edition 2024`, `rust 1.98.1`.
> Scope: port **only** the open-source local Laya family (`NandhaKishorM/laya 0.3.4`,
> Apache-2.0). No Jev hosted path, no outbound network in prod, no Python in prod.
> Primary device: **MacBook Pro M1 Pro (arm64, 10 cores, 17GB unified)**.
> Jetson 4GB stays on the static path and is explicitly out of scope for Laya load.
>
> Companion sources (verified 2026-09-22):
> `laya/__init__.py`, `laya/agent.py`, `laya/common.py`, `laya/router.py`,
> `laya/lang.py`, `laya/presets.py`, `tests/test_router.py`, `pyproject.toml`.

## 0. Ground truth — versions and seams we must not break

### 0.1 Cargo graph (exact, from `Cargo.toml` + `Cargo.lock`)

* `brain-server 1.28.91` — `Cargo.toml:1-11`.
* `ort =2.0.0-rc.13` optional, `default-features=true` — `Cargo.toml:245`.
  Lock: `ort 2.0.0-rc.13` (`Cargo.lock:2728`). Unified with `fastembed 6.0.2`'s
  exact-pinned `ort ==2.0.0-rc.13` — `Cargo.toml:99-100`. Do NOT bump independently.
  Re-verify `Session::builder` API on any bump (comment at `Cargo.toml:239-244`).
* `tokenizers =0.23` optional — `Cargo.toml:246`. Lock has `0.23.2` (direct) and
  `0.21.4` (transitive via fastembed). Use `Tokenizer::from_file` + `encode`
  (single) — matches `src/screen.rs:379-393`.
* `fastembed =6.0.2` optional, `default-features=false` — `Cargo.toml:101`.
  Only for `neural-embed` / `rerank-tier`. Laya port must NOT depend on fastembed.
* `model2vec-rs =0.2` always-on default embedder — `Cargo.toml:96`.
* `rusqlite 0.40.1 bundled`, `r2d2 0.8.10`, `r2d2_sqlite 0.35.0`,
  `tokio 1.53 full`, `axum 0.8.9`, `serde 1.0.229`, `serde_json 1.0.150`,
  `anyhow 1.0.104`, `rayon 1.12` optional (loom).
* `brain-engine-sdk path crates/brain-engine-sdk, features=["harness-kernel"]` —
  `Cargo.toml:91`. Calibration types live here (`CalibrationRecord`, `NO_KAPPA`,
  `month_due`, `month_index`, `week_due`).
* Lints: `[lints.rust] unsafe_op_in_unsafe_fn=deny`, `[lints.clippy] missing_safety_doc=deny`,
  SDK forbids `unsafe_code`, `unwrap_used`, `expect_used`, `panic` — new code must
  follow (no unwrap/expect/panic outside tests).
* Release profile: `opt-level=2, lto=fat, codegen-units=1, strip=true, panic=abort,
  overflow-checks=true` — `Cargo.toml:297-309`. Integer math near i64 edge must abort.

New deps allowed under `laya-local` feature ONLY:

```toml
[features]
laya-local = ["dep:ort", "dep:tokenizers"]
# Do NOT add safetensors/ndarray/hf-hub/candle/burn to the server.
# Weights arrive as pre-exported ONNX (Phase 1). Tokenizer arrives as
# tokenizer.json. No runtime HF fetch.
```

No new direct dep beyond reusing `ort` + `tokenizers`. If a helper is needed
(e.g. small softmax/vec math), hand-roll with `f32` at the ONNX boundary only,
then convert to integer units immediately (rubric pin §1).

### 0.2 Reusable seams (do not reinvent)

* ONNX session: `src/screen.rs:322-436` (`onnx::OnnxScorer`) is the verbatim pattern:
  `Session::builder()?.with_optimization_level(Level3)?.with_intra_threads(1)?.
  commit_from_file(path)?`, `Tokenizer::from_file`, `TensorRef::from_array_view`,
  `guard.run(ort::inputs![ids, mask])`, `try_extract_array::<f32>()`,
  `Mutex<Session>` + `crate::concurrency::mutex_guard_measured`, poison → fail-open
  for screen (`0.0`), fail-closed-for-data for embed (`warn!` + empty vec).
  Laya inference copies this shape, with different fail policy (§5.4).
* Embedder factory: `src/embed.rs:423-438` `embedder_for_profile()` + `src/config.rs:1381-1411`
  `model_profile()/model_id_for_profile()`. Profiles are stringly-typed
  (`edge-default`, `compact`, `desktop`, `enterprise`, …). Laya adds a parallel
  decision-checkpoint selector, NOT a new embedder variant.
* Config vocabulary: `src/config.rs:602-657` (`injection_classifier_setting`,
  `injection_classifier_default_paths`, `validate_injection_classifier_env`,
  `injection_tokenizer_path`) is the template for `BRAIN_DECIDE_*` (§4).
  Fail-closed on typo'd PATH, silent `absent` when artifacts missing and not
  explicitly requested.
* Audit: `src/workflow/mod.rs:171-181` `audit_write_global(conn, target, status, detail)`
  via `record_tenant(..., "global")`. Calibration rides this —
  `src/workflow/calibration.rs:65-91,97-123`. Laya decisions log here, never raw text
  (hashes at rest, `detail` is hashed — test at `calibration.rs:164-187`).
* Tiers: `src/workflow/tiers.rs:14-30` `PROFILE_KEYS` + `TIER_PROFILE_PATHS`
  (`deploy/tiers/t1..t4.env`). Tests `guide_and_profiles_never_drift` and
  `tier_profiles_boot_and_pass_smoke` will fail the build if we add a key without
  updating `docs/deployment.md` and the checked-in profiles. Plan for this.
* Frontdoor: `src/workflow/frontdoor.rs:25-47` closed `IntentClass→Worktype`,
  `WORKTYPE_TABLE` closed rows, `route(input,envelope,conv,plan)->RouteDecision::{Resolved|Routed|Escalated}`
  at `frontdoor.rs:281-306`. Laya may only advise the `Routed` arm, never widen the
  closed table.
* Domain routing: `src/domain_router.rs:44-78` `route(query,centroids)->Option<String>`
  (cosine ≥0.30, ties alphabetical) + `route_domain_label`. L0 stays. Laya is L1.
* Concurrency telemetry: `src/concurrency.rs:43-80` lock-wait histogram. Any new
  `Mutex<Session>` must use `mutex_guard_measured` so `/metrics` sees queue depth.
* Rubric pin: integer ten-thousandths `u32 0..10000`, `score_to_units` grid reject,
  no `f32/f64` downstream of parse (steward `docs/rubric-pin-int-ten-thousandths.md`).
  Applies to all Laya probs/confidences stored or compared in Rust.
* Benchmark contract: `BENCHMARKS.md` same-corpus/same-hardware rules, p50/p95,
  cold-start, RSS, DB size, model-cache size, ingest throughput. Machine table has
  PLACEHOLDER desktop + 4GB ARM rows — M1 row must be filled, Jetson row untouched.

### 0.3 Laya Python semantics (exact — port must match)

From `pyproject.toml`: `laya 0.3.4`, `requires-python>=3.8`,
`torch>=2.0, transformers>=4.45, safetensors>=0.4, huggingface_hub>=0.20, numpy>=1.20`.

Checkpoints (`laya/router.py` docstring + `DEFAULT_MODELS`):

| key | repo / subfolder | params | backbone | tokens |
|---|---|---|---|---|
| `english` | `convaiinnovations/laya` root `(BUNDLE_REPO, None)` | 421M | ModernBERT-large | 512 |
| `multilingual` | `convaiinnovations/laya/multilingual` | 322M | mmBERT-base, 100+ langs | 1024 |
| `typed-decisions` | `convaiinnovations/laya/typed-decisions` | 421M | ModernBERT-large fine-tuned on 4 synthetic workflows | 1024 |

Standalone mirrors: `convaiinnovations/laya`, `laya-multilingual`, `laya-typed-decisions`.
All three resident ≈1.16B params. Aliases: `en/laya/default→english`,
`multi/ml/laya-multilingual→multilingual`, `typed/typed_decisions/decisions→typed-decisions`.
Unknown name raises `ValueError`.

`Router` (`router.py`):

* `__init__(models, device, token, max_loaded=1, default="english", auto_task_detection=False,
  standalone_repos=False, preload=False)`. `models` overrides merged after normalisation.
  `token` or `HF_TOKEN`. `max_loaded≥1`. `preload()` raises `max_loaded` to fit.
* LRU: `_agents: dict`, `_order: list` (LRU-first). `load()` builds `Agent(repo,device,token,subfolder)`,
  `_touch()` on hit, `_evict()` while `len>max_loaded`. `attach(name,agent)` registers
  prebuilt agent, bumps `max_loaded`. `unload(name?)` frees one/all. `loaded` property.
* `route(state,questions,model?,task?,lang?)` — pure, no load:
  `explicit model > explicit task > workflow match (only if auto_task_detection) >
  explicit lang (en/eng/english→english else multilingual) > analyse(state) >
  default`. `analyse` from `lang.py`. Empty/digits (`script==unknown`) → default.
  Non-latin script → multilingual. Latin non-English → multilingual. Else english.
  Returns `RouteDecision(model,repo,reason,detection,workflow)` (dict + `.model/.reason`).
* `predict(state,questions,…)` = `route` + `load(model)` + `agent.system_one` + `result["routing"]=dict(decision)`.
  Alias `system_one = predict`.
* `match_typed_decisions_workflow(questions)` — exact id-set match only:
  `agent_trace_observability{action,needs_review,outcome,risk,urgency}`,
  `customer_service{action,category,churn_risk,needs_human,urgency}`,
  `invoice_processing{discrepancy_severity,disposition,duplicate,matches_order,urgency}`,
  `security_incidents{credential_compromise,disposition,severity,true_positive,urgency}`.
  Partial/superset/empty → None.

`lang.py` — dependency-free, must be byte-faithful:

* `_SCRIPT_RANGES`: greek, cyrillic, hebrew, arabic (4 ranges), devanagari, bengali,
  gurmukhi, gujarati, oriya, tamil, telugu, kannada, malayalam, sinhala, thai, lao,
  tibetan, myanmar, georgian, ethiopic, khmer, hangul (3 ranges), kana (3), han (3).
  Latin = `cp<0x0250` or `0x1E00..0x1EFF`. `detect_script` = dominant count, `unknown`
  if no letters. `script_profile` = fractions. `state_text(state,max 4000)` flattens
  str/dict/list (depth≤6, values only, keys ignored), joined with space.
* `guess_latin_language` — stopword sets for en/fr/de/es/pt/it/nl + diacritic set,
  `WORD=[^\W\d_]+`. `<4` words → None. Scoring + margins: winner non-en needs
  `best≥max(2,en+2)`; diacritic rate path `≥0.04 and best≥en`. Else `en` if any en hit
  else None. Ordinary English never misrouted by design.
* `analyse(state)` → `{script, script_profile, language, is_english, non_latin_fraction}`.
  `unknown→{is_english True, non_latin 0}`. Non-latin → `is_english False`.
  Latin → `is_english = language in (None,"en")`.
* Benchmark note in docstring: English collapses off-English (Hindi 0.100, Korean 0.103,
  Swahili 0.103, Tamil 0.113 at 20 opts vs random 0.050, ECE 0.855 Hindi) while Latin-script
  French 0.487/Spanish 0.480 hold. Script is primary signal.

`common.py`:

* `QTYPES={choice:0,score:1,noul:2}`. `serialize_state`: str passthrough else compact JSON
  `ensure_ascii=False`. `render_criterion`: str passthrough else compact JSON
  (`", ", ": "`, `default=str`). `render_options`: choice `[k or "k: crit"]`
  (only None/"" = no desc; 0/False are values), score `["level i: c"]`,
  noul always `[false:… or "no, the statement does not hold", true:… or "yes, the statement holds"]`.
* `build_sequence(tok,state,q,max_len,head_max_len,option_order?,truncate_left=False)`:
  format `[CLS] <type> instructions [SEP] [MASK] opt0 [MASK] opt1 … [SEP] state [SEP]`.
  `ins` with mask-token replaced by space. `head_ids=tok("{t} question: {ins}")`.
  Each opt `[mask_id]+tok(" "+opt)[:48]` with mask replaced. `opt_budget=head_max_len-sum(opt_lens)`;
  if `<16`: `per=max(4,(head_max_len-16)//n)`, truncate each to `per`, recompute.
  `head_ids[:max(8,opt_budget)]`. `ids=[cls]+head+[sep]+markers…+[sep]+state[:room]+[sep]`,
  `room=max(0,max_len-len(ids)-1)`, `state=tok(serialize(state))[:room]` (or `[-room:]` if
  `truncate_left`). Return `ids[:max_len]`, markers `<max_len` only.
* `DecisionModel(encoder,head_layers=2,n_act=2,dropout=0.1)`: `d=hidden_size`,
  `nhead=max(1,d//64)`, `TransformerEncoderLayer(d,nhead,4*d,dropout,batch_first,norm_first)`,
  `head=TransformerEncoder(layer,head_layers)` or None, `type_emb=Embedding(3,d)`,
  `scorer=LN→Linear(d,d)→GELU→Linear(d,1)`, `act_head=Linear(d+4,256)→GELU→Linear(256,n_act)`,
  `temperature=ones(3)` buffer. Forward: `h=encoder().last_hidden_state`,
  `h+=type_emb(qtype)[:,None,:]`, head layers with `src_key_padding_mask=~attention_mask`,
  `m=gather(h,marker_pos)`, `logits=scorer(m).squeeze(-1).masked_fill(~marker_mask,-1e4)`,
  `p=softmax(logits.detach)`, `k=sum(mask).clamp(min2)`, `ent=-(p log p).sum/log(k)`,
  `top2=p.topk(2)`, `feats=[top1,top1-top2,ent,k/255]`, `pooled=h[:,0]`,
  `act=logits(act_head([pooled,feats]))`. `build_model(cfg,encoder_dir?)`: SDPA attn,
  `AutoModel.from_config` if local encoder else `from_pretrained(cfg["encoder"])`,
  `n_act=len(act_costs)+1`.
* `proper_reward`, `td_lambda_targets` — training-only, do NOT port to server.
  Port only `ece_score` (15 bins, `sel.mean()*|conf-correct|`) and
  `confidence_from_probs` (`1-H/log(k)`, `k<2→1.0`, clipped 0..1) and `temp_bucket`
  (`{name}:{2|3-5|6-10|11+}`) and `amp_dtype` mapping (server always fp32 — §3.3).
* `collate_items(batch,pad_id)`: flatten groups, `L=max len`, `kmax=max markers`,
  `ids [n,L] pad`, `att [n,L]`, `mpos [n,kmax]`, `mmask [n,kmax] bool`,
  `qtype [n]`, `label [n]` (-1 default), `meta` minus ids/markers/target, optional `target`.

`agent.py`:

* `Agent(model_id_or_path,device?,token?,subfolder?)`: local path or HF `snapshot_download`
  (with `allow_patterns=[subfolder/*]` when subfolder). `subfolder` joined. `_fix_tokenizer_config`
  (tokenizer_class None/TokenizersBackend→PreTrainedTokenizerFast; `extra_special_tokens`
  list→dict). Require `rl_agent_config.json` + `model.safetensors` else `FileNotFoundError`.
  `_verify_compatibility`: require cfg `encoder,head_layers`; require weight prefixes
  `encoder.,type_emb.,scorer.,act_head.`; shape-match every param (first 5 shown); missing keys error.
* Device: explicit or auto `cuda>mps>cpu` with availability check + warn + CPU fallback.
  Tokenizer `AutoTokenizer.from_pretrained(tok_dir or cfg["encoder"])`.
  Model `build_model`, `load_state_dict(strict=True)`, `reference_compile=False` (eager;
  compile hurts small batches / hangs). `temperature=cfg[temperature,[1,1,1]]`,
  `temperature_by_options`, `dtype=amp_dtype` then override: CUDA sm<8→fp16, cpu/mps→fp32.
  `model.to(device).eval()` with OOM fallback to CPU + printed warning
  (10-15x slower, ~200-500ms vs ~35ms, Blackwell note).
* `_to_internal(qdef)`: choice list criteria→`{c:None}` dict; instructions non-str→JSON.
* `system_one(state,questions)`: for each qid `build_sequence` with
  `max_len=cfg[max_len,512]`, `head_max_len=cfg[head_max_len,192]`; raise if
  `len(markers)!=len(render_options(q))` (`options exceed head_max_len`).
  `collate([items],pad_id)`, autocast only on CUDA, forward, OOM→CPU retry once,
  `logits.float.cpu.numpy`, `act=softmax(act.float).cpu.numpy`,
  `n_tokens=sum(attention_mask)`. Per q: `t_scale=temperature_by_options[temp_bucket(qt,k)]
  or temperature[qt]`, `z=logits[:k]/max(1e-3,t)`, `p=softmax(z)`,
  `conf=round(confidence(p,k),4)`, `ext={act_probability: round(act[r,0],4)}`.
  choice `{type,choice:keys[argmax],probabilities:{k:round4},confidence,action:ext}`,
  score `{type,score:round4(sum(i*p)),legend:{i:c},probabilities:{i:round4},confidence,action}`,
  noul `{type,noul:round4(p[1]),confidence:round4(max(p1,1-p1)),action}`.
  Return `{model:"laya-rl-agent",answers,usage:{input_tokens:n,output_tokens:0}}`.

`presets.py` + `email.py` (`__init__` exports): triage/email/guard/moderation/router
question dicts + `clean_email_body/email_state/detect_language/detect_script/is_english`.
Port presets as data, `email_state/clean_email_body` only if frontdesk pilot needs it.

`tests/test_router.py`: pure `Router.route` tests, no weights. Full case list must be
ported 1:1 (§6.1). `tests/test_criteria.py`, `test_local_e2e.py` (model) become ONNX
parity tests (§6.3), not unit tests.

---

## 1. Target shape (M1, feature-gated, no Python in prod)

```
brain-server/
  Cargo.toml                    # + laya-local feature (ort+tokenizers, §0.1)
  deploy/tiers/t1.env..t4.env   # + BRAIN_DECIDE_* keys (§4.3)
  deploy/tiers/m1-local.env     # NEW — M1 single-operator profile
  docs/deployment.md            # + m1-local + BRAIN_DECIDE_* matrix rows
  docs/BENCHMARKS.md            # + M1 machine row + decide latency/ECE tables
  models/laya/                  # NOT in git — operator dir (§3.1)
    english/{model.onnx,tokenizer.json,rl_agent_config.json,SHA256SUMS}
    multilingual/{...}
    typed-decisions/{...}       # explicit-only
  src/
    config.rs                   # + BRAIN_DECIDE_* resolvers + validators
    concurrency.rs              # reuse mutex_guard_measured (no change)
    screen.rs                   # optional guard-questions advisor call (L1 only)
    domain_router.rs            # + route_hierarchical helper (pure)
    workflow/
      mod.rs                    # + pub mod decide; audit_write_global reuse
      decide/
        mod.rs                  # feature gate + public API + prelude
        lang.rs                 # §2.1 port of lang.py (pure, always compiled)
        router.rs               # §2.2 port of router.py route/LRU (pure core always; loader behind feature)
        sequence.rs             # §2.3 port of build_sequence/render (pure, always compiled)
        calibration.rs          # §2.4 temp_bucket/confidence/ECE + units (pure, always compiled)
        presets.rs              # §2.5 question presets (data, always compiled)
        config_ext.rs           # decide env resolvers live in config.rs; this is thin re-export + tests
        inference.rs            # §3 ort session + forward post (feature laya-local only)
        audit_ext.rs            # decide audit detail builder + schema_meta keys (uses calibration.rs pattern)
      calibration.rs            # + decide ECE/temp keys passthrough (no logic move)
      tiers.rs                  # + PROFILE_KEYS entries (5 keys)
      frontdoor.rs              # + Routed-arm advisor hook (§5.2, ~20 lines)
      frontdesk.rs              # + email-questions pilot mapping (optional Phase 3)
```

Design rules (non-negotiable, from repo precedent):

1. Default build byte-identical. `laya-local` off → no ort/tokenizers linkage, no new
   threads, no new tables. Pure modules (`lang/router-core/sequence/calibration/presets`)
   are dependency-free and always compiled so CI runs them ungated (the `connector-crm`
   precedent — `Cargo.toml:32-36`: contract + tests ungated, transport gated).
2. No new heavy runtime on edge. M1 is a desktop-class profile, not a default.
   Jetson/t1-t2 default profiles never set `BRAIN_DECIDE=on`.
3. Integer-only downstream. `f32` exists only inside `inference.rs` between
   `try_extract_array` and `score_to_units`. Everything stored/compared/audited is
   `u32/i32` units. Source-scan test `no_f32_in_decide_math` mirrors the rubric pin.
4. Closed vocabularies stay closed. Decide head cannot create a new worktype/intent.
   Unknown → `Routed` → human/run, exactly as today.
5. Tripwire, not boundary. Like `src/screen.rs:18-30`, the decide head advises;
   the HITL gate + `flagged/untrusted` segregation + audit chain remain the boundary.
6. Fail-closed on config, fail-closed-for-data on model errors (embed precedent),
   except the screen-advisor path which fails open to `score 0.0` (screen precedent —
   a dead classifier must not eat every ingest; §5.4 table).

## 2. Pure port (no model — Phase 0, ungated, M1 + CI)

All files dependency-free (`std` + `serde_json` + `serde` only). Total functions must
match Python names where feasible for review (`analyse`, `detect_script`, `route`,
`build_sequence`, `render_options`, `confidence_from_probs`, `temp_bucket`, `ece_score`).

### 2.1 `src/workflow/decide/lang.rs` ← `laya/lang.py` (verbatim port)

* Constants: `_SCRIPT_RANGES` as `&[(&str, &[(u32,u32)])]` with identical codepoints
  (arabic 5 ranges, hangul 3, kana 3, han 3, devanagari 2 + A8E0-A8FF, etc.).
  Latin rule: `cp<0x0250 || 0x1E00<=cp<=0x1EFF`. `isalpha` via `char::is_alphabetic`.
* `_STOP: 7 languages` (en 30 words, fr/de/es/pt/it/nl as in source) + `_NON_EN_DIACRITICS`
  string + `_WORD` regex replaced by hand-rolled `char::is_alphanumeric` splitter
  (no `regex` dep — keep dependency-free; document equivalence + test).
* `state_text(state: &serde_json::Value, max_chars=4000) -> String`: recurse
  str/dict/list depth≤6, values only, join `" "`, truncate to 4000 chars on char boundary
  (mirror `src/embed.rs:492` multibyte-boundary rule).
* `detect_script(text:&str)->String`, `script_profile(text)->Vec<(String,f32)>`,
  `guess_latin_language(text)->Option<String>`, `analyse(state)->Detection{script,
  script_profile, language, is_english, non_latin_fraction: f32 (0..1, 4dp)}`,
  `is_english(state)->bool`.
* Precision: `non_latin_fraction = round4(1-latin_frac)`, `0.0` when empty.
  `unknown` when zero letters (empty + digits-only).
* Tests: port every `SCRIPTS` (14), `is_english` (7), `latin_lang` (5+1 long-English),
  `state_text` (4+keys-ignored) cases verbatim. Names: `script_english`, …,
  `latin_lang_long_english_stays_en`. Any deviation fails review.

### 2.2 `src/workflow/decide/router.rs` ← `laya/router.py` route + LRU (no torch)

```rust
pub const BUNDLE_REPO: &str = "convaiinnovations/laya";
pub const CHECKPOINTS: &[&str] = &["english","multilingual","typed-decisions"];
pub fn normalise_name(name:&str)->Result<String,String> // aliases + ValueError→Err
pub fn match_typed_decisions_workflow(ids:&std::collections::BTreeSet<String>)->Option<String>
pub struct Detection { script:String, language:Option<String>, is_english:bool, non_latin_fraction:f32 }
pub struct RouteDecision { model:String, repo:String, reason:String, detection:Option<Detection>, workflow:Option<String> }
pub fn route(state:&serde_json::Value, question_ids:&BTreeSet<String>,
  model:Option<&str>, task:Option<&str>, lang:Option<&str>,
  auto_task_detection:bool, default:&str, detection:Detection)->Result<RouteDecision,String>
pub struct LruState { max_loaded:usize, order:Vec<String>, resident:Vec<String> }
impl LruState { new(max_loaded)->Self, touch(), evict_to_cap(), preload(names)->(), unload(name?)->(), loaded()->&[String], attach()->() }
```

* Precedence identical to Python. `task` normalised `lower+replace("-","_")=="typed_decisions"`.
  `lang` split on `-`, first segment `en/eng/english→english` else multilingual.
  `repo` string: bundle `(repo,None)→repo`, `(repo,sub)→repo/sub`; plain path passthrough.
  Reasons verbatim: `"explicit model=%r"`, `"explicit task=%r"`, `"question ids match …"`,
  `"explicit lang=%r"`, `"no letters detected…; using default (…)"`,
  `"non-Latin script (…, …% of letters); the English checkpoint cannot read it"`,
  `"Latin script but language looks like …"`, `"English Latin text"`.
* Loader is a trait so pure tests run without ort:
  `trait CheckpointLoader { fn load(&mut LruState, name:&str)->Result<(),String>; }`
  Real impl in `inference.rs` (feature-gated). Tests use stub (mirrors `_Stub` in
  `test_router.py`).
* Tests: port all 14 `route/*` cases, `alias/*` (8+unknown-raises), `workflow/*` (4+partial/superset/empty),
  `decision/*` (repo/reason/detection/model/is-dict), `custom default`, `lru/*` (cap1/cap2/touch/unload),
  `bundle/*` (root/sub/plain/bundle-vs-standalone/override/standalone-map), `preload/*` (all-three/subset/touch-no-evict),
  `attach/*` (5). ~60 asserts. Names prefixed `route_`, `lru_`, `bundle_`, `preload_`, `attach_`.

### 2.3 `src/workflow/decide/sequence.rs` ← `build_sequence` + render

```rust
pub struct TypedQuestion { qtype:QType, instructions:String, criteria:Criteria }
pub enum QType { Choice=0, Score=1, Noul=2 }
pub enum Criteria { ChoiceMap(Vec<(String,Option<String>)>), ScoreList(Vec<String>), NoulPair{false_opt:Option<String>,true_opt:Option<String>} }
pub fn render_criterion(v:&serde_json::Value)->String // str→self else compact JSON ", ", ": "
pub fn render_options(q:&TypedQuestion)->Vec<String>
pub fn serialize_state(s:&serde_json::Value)->String
pub struct BuiltSequence { ids:Vec<i64>, markers:Vec<usize> }
pub fn build_sequence(tokenizer:&dyn TokenizerLike, state:&serde_json::Value, q:&TypedQuestion,
  max_len:usize, head_max_len:usize, option_order:Option<&[usize]>, truncate_left:bool)->Result<BuiltSequence,String>
pub trait TokenizerLike { fn cls_id(&self)->i64; fn sep_id(&self)->i64; fn mask_id(&self)->i64; fn mask_str(&self)->&str; fn encode_nospecial(&self, text:&str)->Vec<i64>; }
```

* Rules identical: mask→space sanitise, `"{t} question: {ins}"` head, opt `[mask]+encode(" "+opt)[:48]`,
  budget math, `head[:max(8,budget)]`, `[cls]+head+[sep]+markers+[sep]+state[:room]+[sep]`,
  `room=max(0,max_len-len-1)`, error when `markers.len()!=options.len()`.
* Real `TokenizerLike` impl for `tokenizers::Tokenizer` lives in `inference.rs`
  (feature-gated) to keep this module dependency-free; tests use a fake tokenizer
  with fixed ids (cls 101/sep 102/mask 103 style) + golden vectors generated from Python.
* Validation here: `options.len()>20 → Err("schema exceeds 20 options — split hierarchically")`.
  This is the hard ceiling from the videos (§B.5). No bypass flag.

### 2.4 `src/workflow/decide/calibration.rs` ← confidence/ECE/temp (pure)

```rust
pub fn confidence_from_probs(p:&[f32], k:usize)->f32 // 1-H/ln(k), k<2→1.0, clip 0..1
pub fn temp_bucket(qtype:QType, k:usize)->String      // "choice:2" etc.
pub fn ece_score(conf:&[f32], correct:&[bool], bins:usize=15)->f32
pub fn softmax_inplace(z:&mut [f32])
pub fn score_to_units(s:f32)->Option<u32> // reuse steward pin: 0..1, ≤4dp, else None
pub fn units_to_f32(u:u32)->f32 // u as f32 /10000.0 (display only)
```

* `ece_score` empty → NaN (match Python `float("nan")`; Rust `f32::NAN`, test `is_nan`).
* No `f32` escapes: public outputs are `u32` units. Internal `f32` confined to this file
  + `inference.rs` post-step. Add source-scan test `no_f32_in_decide_math_outside_boundary`
  (allowlist: `calibration.rs` convert fns + `inference.rs` post).
* Tests: entropy vectors (uniform→0, one-hot→1, k=1→1), bucket edges (2/5/10/11),
  ECE hand-computed 2-bin case, grid reject (`0.21000001→None`).

### 2.5 `src/workflow/decide/presets.rs` ← `presets.py` (data only)

`serde_json::Value` constructors: `triage_questions()`, `email_questions(categories?)`,
`guard_questions()`, `moderation_questions()`, `router_questions()`. Keys/strings verbatim
from source (e.g. guard `jailbreak/prompt_injection/sensitive_data/harm_severity/topic`).
Used by pilots (§5.2). Test: JSON snapshot + `validate_schema` passes + `match_typed_decisions_workflow`
returns None for presets (they are NOT the 4 fine-tuned workflows — must not auto-trigger
typed-decisions).

## 3. Model artifacts + ONNX export (one-time offline, Python only — never in server)

### 3.1 Artifact layout (operator dir, NOT git)

```
~/.config/brain-server/models/laya/
  english/{model.onnx,tokenizer.json,rl_agent_config.json,SHA256SUMS}
  multilingual/{...}
  typed-decisions/{...}
```

* Source: `convaiinnovations/laya` bundle subfolders OR standalone repos
  (`laya`, `laya-multilingual`, `laya-typed-decisions`). Only requested subfolder downloaded
  (`allow_patterns=[subfolder/*]` precedent).
* `rl_agent_config.json` required keys: `encoder, head_layers, max_len, head_max_len,
  temperature[3], temperature_by_options, act_costs, amp_dtype`. Missing → boot refuse.
* `tokenizer/` must contain `tokenizer.json` directly usable by `tokenizers::Tokenizer::from_file`.
  Apply `_fix_tokenizer_config` equivalent at export time (tokenizer_class→PreTrainedTokenizerFast,
  `extra_special_tokens` list→dict), then freeze. Server never mutates tokenizer dir.
* `model.safetensors` never ships to prod. Exported `model.onnx` + SHA is the artifact.
  Record SHA256 in `SHA256SUMS` + in `BENCHMARKS.md` + audit `schema_meta`.

### 3.2 Export script (run on build machine with python3.12, torch, transformers)

`tools/export_laya_onnx.py` (not shipped in binary, checked in for reproducibility):

1. Load cfg + `build_model(cfg, encoder_dir)` with `attn_implementation="sdpa"`.
2. `load_file(model.safetensors)` + `_verify_compatibility` (prefixes + shapes) + `load_state_dict(strict=True)`.
3. `model.encoder.config.reference_compile=False`, `.eval()`, dtype fp32.
4. Dummy inputs: `input_ids [2,32] i64, attention_mask [2,32] i64, marker_pos [2,4] i64,
   marker_mask [2,4] bool, qtype [2] i64`.
5. `torch.onnx.export(model, args, "model.onnx", input_names=[…5], output_names=["logits","act_logits"],
   dynamic_axes={each dim 0:"batch",1:"seq"/"markers"}, opset_version=17,
   do_constant_folding=True, dynamo=False)`.
6. Verify: run 20 fixtures (triage/email/guard × english/multilingual) through torch vs
   `onnxruntime` Python; assert max `|logits|` diff `<1e-4`, argmax equal, `act` diff `<1e-4`.
7. Emit `tokenizer.json` (from fixed `AutoTokenizer`), `rl_agent_config.json` (pruned to
   needed keys + `onnx_opset:17, export_torch, export_date, sha`), `SHA256SUMS`.

Constraints: opset 17 (ort rc.13 stable; avoid 18+ CoreML gaps), fp32 (M1 MPS/CPU exact;
no fp16/bf16 in ONNX — server sets fp32 always, matching `agent.py` cpu/mps branch),
eager only (no `torch.compile`), SDPA decomposed to standard ops (verify with
`onnx.checker` + `ort` load on M1 before sign-off).

### 3.3 Runtime dtype/device policy (M1)

* Server dtype always fp32. `amp_dtype` from cfg ignored except to warn if `bf16/fp16`
  (log once: `laya dtype {v} normalised to fp32 on M1`).
* Providers: try CoreML EP then CPU EP via `ort` execution providers; if unavailable,
  CPU with `with_intra_threads(cores-1 capped 4)` on M1 (loom precedent caps at 4;
  decide inference is latency-sensitive, not throughput — 4 is ceiling, 1 is Jetson value
  which we do NOT use here). OOM or EP failure → CPU retry once, then fail-closed-for-data
  (drop decision + audit `decide/oom-fallback`, escalate caller). Message mirrors `agent.py`
  warning (10-15x slower expectation) but as `tracing::warn!`, not stdout.
* `reference_compile` N/A in ONNX. Graph opt Level3 (screen precedent).

## 4. Rust wiring (exact diffs)

### 4.1 `Cargo.toml`

```diff
 # v1.20.3 Classify precedent…
 injection-classifier = ["dep:ort", "dep:tokenizers"]
+# v1.28.9x Laya-local System-1 (M1): full DecisionModel as pre-exported ONNX +
+# tokenizer.json. OFF by default — default build unchanged. Reuses ort rc.13 +
+# tokenizers 0.23 (no new native surface beyond what injection-classifier already pulls).
+# No safetensors/torch/transformers/hf-hub in the server.
+laya-local = ["dep:ort", "dep:tokenizers"]
```

No other dependency change. If `cargo tree -i ort` shows duplicate ort (fastembed vs direct),
resolve before merge (comment at `Cargo.toml:57` requires re-verify).

### 4.2 `src/config.rs` — `BRAIN_DECIDE_*` (mirror injection-classifier vocabulary)

| env | values | default | boot behaviour |
|---|---|---|---|
| `BRAIN_DECIDE` | `on/off` (also `1/0`, case-insensitive) | `off` | unknown → refuse boot |
| `BRAIN_DECIDE_MODELS_DIR` | dir path | `~/.config/brain-server/models/laya` | when `on`: must exist + contain requested checkpoint subdir with 3 files, else refuse |
| `BRAIN_DECIDE_CHECKPOINTS` | csv subset of `english,multilingual,typed-decisions` | `english,multilingual` | unknown name → refuse; `typed-decisions` allowed only explicit (log warn) |
| `BRAIN_DECIDE_DEFAULT` | `english/multilingual` | `english` | unknown → refuse |
| `BRAIN_DECIDE_AUTO_TASK` | `on/off` | `off` | unknown → refuse (opt-in only, router.py default) |
| `BRAIN_DECIDE_MAX_LOADED` | `1..3` | `2` on M1 | `0/>3`/unparsable → refuse |
| `BRAIN_DECIDE_THRESHOLD_CHOICE` etc. | `0..1` string (4dp grid) | choice 0.75, noul 0.75, score 0.80, safety 0.85 | reuse `parse_threshold_env` fail-closed + `high<low` refuse (`config.rs:697-…` pattern) |

Functions: `decide_enabled()->bool`, `decide_models_dir()->PathBuf`,
`decide_checkpoints()->Result<Vec<String>,String>`, `decide_default()->String`,
`decide_auto_task()->bool`, `decide_max_loaded()->Result<usize,String>`,
`decide_thresholds()->Result<Thresholds,String>`, `validate_decide_env()->Result<(),String>`.
Call `validate_decide_env()` in the same boot sequence as
`validate_injection_classifier_env` + `validate_injection_thresholds`.
When feature `laya-local` off, `BRAIN_DECIDE=on` refuses boot with
`built without laya-local — rebuild with --features laya-local or set off`.

Default-paths helper mirrors `injection_classifier_default_paths`:
probe `<models_dir>/<checkpoint>/{model.onnx,tokenizer.json,rl_agent_config.json}`;
missing → `None` = `absent` posture (layer off, deterministic paths remain) unless
explicitly requested → refuse.

### 4.3 Tiers + docs (build will fail if skipped)

* `src/workflow/tiers.rs:14` add 5 keys: `BRAIN_DECIDE`, `BRAIN_DECIDE_MODELS_DIR`,
  `BRAIN_DECIDE_CHECKPOINTS`, `BRAIN_DECIDE_DEFAULT`, `BRAIN_DECIDE_MAX_LOADED`.
  (Thresholds are per-domain DB values, not tier keys — do not add.)
* New `deploy/tiers/m1-local.env`:
  ```
  # M1 single-operator — loopback, open posture, Laya preloaded (docs/deployment.md § M1)
  BRAIN_WRITE_POSTURE=open
  BIND_PUBLIC=0
  BRAIN_DECIDE=on
  BRAIN_DECIDE_CHECKPOINTS=english,multilingual
  BRAIN_DECIDE_DEFAULT=english
  BRAIN_DECIDE_MAX_LOADED=2
  ```
  Add path to `TIER_PROFILE_PATHS`. `t1..t4.env` unchanged (decide off).
* `docs/deployment.md`: add `m1-local` section + matrix rows for each new key,
  else `guide_and_profiles_never_drift` fails. `docs/BENCHMARKS.md`: add M1 machine row
  (M1 Pro/17GB/arm64/macos/rustc 1.98.1) + decide tables (§7).

### 4.4 `src/workflow/decide/mod.rs` — public API

```rust
#![allow(dead_code)] // follow workflow/mod.rs:23 precedent until frontdoor consumes it
pub mod lang; pub mod router; pub mod sequence; pub mod calibration; pub mod presets;
#[cfg(feature="laya-local")] pub mod inference;
#[cfg(feature="laya-local")] pub mod audit_ext;
pub use router::{RouteDecision, route};
pub use sequence::{TypedQuestion, QType, build_sequence, validate_schema};
// Pure core usable without feature; inference::DecideEngine only with feature.
```

Register `pub mod decide;` in `src/workflow/mod.rs:25-…` alongside `calibration`.
Keep `pub(crate)` visibility until Phase 3 flips to `pub` with callers (dead-code allow
pattern at `mod.rs:23`).

### 4.5 `src/workflow/decide/inference.rs` (feature `laya-local` only)

```rust
pub struct CheckpointMeta { name:String, dir:PathBuf, max_len:usize, head_max_len:usize,
  temperature:[f32;3], temperature_by_options:HashMap<String,f32>, encoder:String, sha:String }
pub struct LoadedCheckpoint { meta:CheckpointMeta, session:Mutex<Session>, tokenizer:tokenizers::Tokenizer,
  cls_id:i64, sep_id:i64, mask_id:i64, pad_id:i64 }
pub struct DecideEngine { checkpoints:HashMap<String,LoadedCheckpoint>, lru:LruState,
  default:String, auto_task:bool, thresholds:Thresholds }
impl DecideEngine {
  pub fn boot()->anyhow::Result<Option<Self>> // None = absent posture; Some = preloaded
  pub fn route_only(&self, state:&Value, qids:&BTreeSet<String>, model:Option<&str>, task:Option<&str>, lang:Option<&str>)->Result<RouteDecision,String>
  pub fn predict(&self, state:&Value, questions:&HashMap<String,TypedQuestion>, model:Option<&str>, task:Option<&str>, lang:Option<&str>)->anyhow::Result<DecideResult>
}
pub struct DecideResult { answers:HashMap<String,AnswerUnits>, routing:RouteDecision, usage:Usage }
pub struct AnswerUnits { qtype:QType, label:String, prob_units:u32, confidence_units:u32, act_units:u32, probs_units:HashMap<String,u32> }
```

* Boot: resolve env, read each checkpoint dir, parse `rl_agent_config.json`
  (fail-closed on missing keys/shapes), `Tokenizer::from_file`, cache ids
  (`cls/sep/mask/pad`), `Session::builder()?.with_optimization_level(Level3)?.
  with_intra_threads(min(cores-1,4))?.commit_from_file(model.onnx)?`,
  verify weight-prefix invariant indirectly (ONNX graph has 2 outputs `logits,act_logits`;
  input names exactly 5; else refuse — ONNX-era equivalent of `_verify_compatibility`).
  `preload` all requested checkpoints now (cold cost at boot, not request).
  Wrap in `Arc<DecideEngine>` in `AppState` (alongside embedder), `Mutex` per session.
* `predict` steps (mirror `agent.py:system_one` + `router.predict`):
  1. `validate_schema` (≤20, non-empty, known qtype) → Err refuses.
  2. `route()` pure → checkpoint name. If not resident (should not happen after preload),
     fail-closed escalate (do NOT lazy-load on request path — log + return Err).
  3. For each qid: `build_sequence` via `TokenizerLike` over real tokenizer →
     `collate` (pad to `L`, `kmax`) → input tensors `i64` (mask as `i64` 0/1; ort bool
     input avoided for CoreML compat) → `session.run` under `mutex_guard_measured`
     → `try_extract_array::<f32>` for both outputs.
  4. Post: `t=temperature_by_options[temp_bucket] or temperature[qt]`, `z=logits/t`,
     softmax, `conf=confidence_from_probs`, `act=softmax(act_logits)[0]`,
     `score_to_units` each (off-grid/NaN → Err → escalate, never clamp silently).
  5. Threshold: `prob_units < threshold(qtype,safety?) → abstained=true`, caller must
     `Escalated`. Separate `prob_units` vs `confidence_units` vs `act_units` in output
     (the sales 40.79%/0.0577 lesson).
  6. `usage{input_tokens: sum(attention_mask), output_tokens:0}`.
* Lock bounds doc-comment required (Headroom style): critical section = session.run only,
  tokenization before acquire, no I/O/SQL/nesting inside, poison → fail-closed-for-data
  (`warn!` + Err → escalate). Request-path holder, wait-measured.
* `ort::Error` is `!Send/!Sync` — map to string before `?` (screen.rs:369 comment).
* No autocast (ort fp32 graph). No `torch.compile`. No HF fetch. No `token` env.

`audit_ext.rs`: `fn decide_audit_detail(result:&DecideResult, schema_hash:&str, latency_ms:u64,
ece_units:Option<u32>)->String` → `"decide/<checkpoint> schema:{hash} label:{…} prob:{units}
conf:{units} thr:{units} route:{reason} lat:{ms}ms"` + `schema_meta` keys
`decide_temperature_{bucket}`, `decide_ece_units`, `decide_checkpoint`, `decide_schema_hash`
via `meta_set` pattern (`calibration.rs:26-32`). Writes through `audit_write_global`
with target `decide/predict` (Ok) or `decide/escalate` (Ok + reason) / `decide/refuse` (Denied).

## 5. Integration points (minimal diffs, exact call sites)

### 5.1 Hierarchical split — `src/domain_router.rs`

Add pure helper (no model):

```rust
pub fn split_for_decide(options:Vec<String>)->Vec<Vec<String>> // chunks of ≤20, coarse-first
```

Policy: `≤20` → single schema. `>20` → caller must build 2-step tree (coarse choice
≤20, then fine choice ≤20 within winner). `sequence::validate_schema` enforces;
`split_for_decide` is the authorised splitter. Test with 77-label banking fixture
(assert 4 chunks + coarse router ≤20). No centroid logic changed.

### 5.2 Frontdoor advisor — `src/workflow/frontdoor.rs:281`

```rust
pub fn route(input:&str, envelope:&Envelope, conversation:&str, plan_so_far:&str)->RouteDecision {
  if is_escape(input) { return Escalated{…} }           // unchanged
  if let Some((class,reason)) = classify_rules(input) { return Resolved{…} } // unchanged
  // NEW: optional L1 advisor, only when compiled + enabled + preloaded.
  #[cfg(feature="laya-local")]
  if let Some(advised) = crate::workflow::decide::advise_routed(input, envelope) {
    return advised; // Resolved only if prob≥thr AND conf≥thr AND checkpoint matches lang route; else Routed (unchanged string)
  }
  Routed{ reason:"outside closed vocabulary — routed to run".into() }
}
```

`advise_routed` builds a `choice` schema over eligible `IntentClass` (≤20, from
`WORKTYPE_TABLE` subset relevant to input length — default to the 9 universal classes
if unsure, never synthetic labels), `state={prompt:input}`, explicit `lang` from
`decide::lang::is_english` fast path when known, calls `DecideEngine::predict`,
maps winner string → `IntentClass` via closed map, unknown string → `Routed`.
Thresholds from `BRAIN_DECIDE_THRESHOLD_*` via `Thresholds`. Any Err/OOM/absent →
`None` (fall through to `Routed`, audit `decide/escalate` if engine present).
Adds ~20 lines + 5 tests (`closed_vocab_resolves` unchanged, `outside_vocab_routes`
still passes with feature off; new `decide_advisor_*` tests with stub engine).

### 5.3 Screen / frontdesk pilots (Phase 3, advisor-only)

* `src/screen.rs`: after Layer-1 blocklist, optional `guard_questions()["jailbreak"]`
  as `noul` via `DecideEngine` when `BRAIN_DECIDE=on`. Result only adjusts
  `Quarantine` suspense (never sole `Reject`); `flagged/untrusted` + HITL remain boundary.
  Poison/off → `0.0` path unchanged (fail-open precedent).
* `frontdesk` billing/technical/sales: `email_questions()` `category choice(6)` +
  `is_urgent noul` via engine; low `confidence_units` (e.g. sales 4079 units + 577 conf)
  → escalate to human queue, not auto-file. Record both fields in audit (never collapse).

### 5.4 Failure policy matrix (must be in code comments + tests)

| failure | screen-advisor | frontdoor-advisor | batch/ingest |
|---|---|---|---|
| feature off / `BRAIN_DECIDE=off` | skip (0.0) | `Routed` unchanged | skip |
| artifacts absent + not requested | skip | `Routed` | skip |
| artifacts absent + requested/on | boot refuse | boot refuse | boot refuse |
| tokenizer/ONNX load fail | `warn!` + off | boot refuse if preloaded set, else `Routed` + `decide/refuse` audit | Err + drop row (never corrupt) |
| session poison | fail-open 0.0 | fail-closed escalate | fail-closed drop |
| OOM / EP fail | CPU retry once → 0.0 | CPU retry once → `Escalated` | CPU retry → Err |
| off-grid prob / NaN | 0.0 | `Escalated` | Err |
| non-Latin → english requested | route multilingual anyway + audit note (never let english see non-Latin) |
| `>20` options | N/A | refuse + `decide/refuse` | refuse |

## 6. Tests (must land with code, names fixed)

### 6.1 Ported `test_router.py` → `decide::router::tests` + `decide::lang::tests` (~60 asserts)

Port every case (§0.3): scripts 14, is_english 7, latin guess 6, state_text 4,
workflow 7, alias 9, route 14, auto-ON 3, decision-shape 5, custom-default 1,
lru 6, bundle/standalone 8, preload 4, attach 5. Use stub loader (no ort).
Failure message format `"got {got:?}, want {want:?}"` to match Python report.

### 6.2 Sequence goldens → `decide::sequence::tests`

Generate `tests/fixtures/decide_sequence_golden.json` from Python
(`build_sequence` ids/markers for triage/email/guard × short/long state ×
`max_len 512/1024`, `head_max_len 192`). Rust fake-tokenizer test asserts exact
`ids/markers`. Real-tokenizer test (feature `laya-local`, `#[ignore]` operator-run
like `static_embedder_matches_model2vec_golden` at `embed.rs:626`) asserts parity
with `tokenizer.json`.

### 6.3 ONNX parity → `decide::inference::tests` (`#[ignore]`, needs artifacts)

For each of 20 fixtures: Python torch logits vs Rust ONNX logits
`max Abs diff <1e-4`, argmax equal, `act` diff `<1e-4`, `probabilities` round4 equal,
`usage.input_tokens` equal. Cold vs warm separated. Mismatched arch (wrong ONNX
input names/count) → boot refuse test.

### 6.4 Calibration + units → `decide::calibration::tests`

Entropy/confidence vectors, bucket edges, ECE hand calc, `score_to_units` grid reject
(`0.21000001/1.00001/-0.1/1.1/NaN→None`), idempotence, `no_f32_in_decide_math_outside_boundary`
source scan (allowlist: `calibration.rs` converters + `inference.rs` post block).

### 6.5 Integration → `frontdoor/decide_*`, `tiers::*`, `config::decide_*`

`decide_advisor_resolves_when_confident`, `decide_advisor_escalates_when_weak`,
`decide_never_widens_closed_table` (unknown model string → Routed),
`decide_refuses_over_20`, `decide_english_never_sees_non_latin`,
`tier_m1_local_boots_and_passes_smoke`, `decide_env_typo_refuses_boot`
(`BRAIN_DECIDE=maybe`, `BRAIN_DECIDE_MAX_LOADED=0`, bad checkpoint name).

### 6.6 Fuzz (follow `workflow/mod.rs:34-95` precedent)

Add `fuzz_decide_sequence_parser(question_json:&str)->String` — total, never panics,
drives real `TypedQuestion` parser. Wire to fuzz target list when harness adds one.

## 7. Benchmarks + M1 acceptance gates (update `docs/BENCHMARKS.md`)

Add machine row: `M1 Pro / 17GB unified / macOS / arm64 / rustc 1.98.1 / ort rc.13 /
tokenizers 0.23.2 / model {english 421M512, multilingual 322M1024} + SHA`.
Keep Jetson row untouched (still static-only).

Measure, same-fixture, warm vs cold separated:

* `decide_p50/p95_ms` per checkpoint (1Q + 10Q batch), `cold_load_s`,
  `RSS idle/under-load`, `model disk`, `route_only_us` (must be <500µs like Python),
  `ECE` pre/post temperature fit, `macro_acc` on pilot set.
* Reference expectations (not gates, for context): T4 32.8ms/1Q, Jev 236-276ms cited,
  CPU warm 3.23s/4Q reference, M1 target warm 100s ms (measure, do not claim).
* Gates to ship Phase 3: `route_only_us p95 <500`, `warm p95 <1000ms` on M1 CPU,
  `ECE` drops after fit without macro collapse, `preload` resident (no per-request load),
  `guide_and_profiles_never_drift` + `tier_profiles_boot_and_pass_smoke` green,
  `verify_chain` green after decide audit rows.

Temperature fitting: per `temp_bucket` grid search on held-out company slice
(dev/val/final split per BENCHMARKS rule 7 — never tune on final). Store
`temperature_by_options` override in `schema_meta` via calibration module;
weekly REPORT carries `decide_ece_units`, monthly sign re-anchors baseline
(`calibration.rs:65-123` pattern extended, not forked).

## 8. Rollout (4 phases, Jetson untouched)

* **Phase 0 (this PR): pure modules + tests, no feature.** `lang/router/sequence/
  calibration/presets` + ported tests + `split_for_decide` + docs. Zero Cargo change.
  CI green ungated. Reviewable without weights.
* **Phase 1: export + `laya-local` skeleton.** `tools/export_laya_onnx.py`,
  `inference.rs` + `audit_ext.rs` + `config.rs` resolvers + tier keys + `m1-local.env`
  + `docs/deployment.md` rows + parity tests `#[ignore]`. Default build unchanged.
  `cargo check --features laya-local` green on M1 + linux.
* **Phase 2: boot + preload on M1.** `AppState` holds `Option<Arc<DecideEngine>>`,
  boot preloads `english,multilingual` (`max_loaded=2`), `/healthz` reports
  `decide:{loaded, модели, sha, warm_ms}`. `frontdoor` advisor behind thresholds,
  default thresholds conservative (0.85) so behaviour is escalate-heavy initially.
  Bench on M1, fill BENCHMARKS table.
* **Phase 3: pilots + fit.** Screen guard advisor + frontdesk email pilot on M1 T1/T2
  only. Collect ECE slice, fit temps, lower thresholds per-domain with human sign
  (`calibration/sign` + complaints extract pattern). `typed-decisions` enabled only
  for exact-workflow schemas with explicit task + fine-tuned checkpoint SHA pinned.
  Score primitive quarantined until its own eval passes (videos: weakest on math scaling).

Non-goals: training/RLCD in server (`proper_reward/td_lambda` stay in notebooks),
Python sidecar in prod, Jev adapter, auto `typed-decisions` default, `>20` single schema,
CoreML GPU promises (measure CPU first), any change to `t1..t4.env` defaults.

## 9. Risks + honest ceilings (disclose like the videos do)

* ModernBERT-large in ONNX on ort CoreML EP may fall back to CPU ops (SDPA/̄GELU/̄LN
  coverage). Mitigation: fp32 CPU baseline first, CoreML as optimisation with parity gate.
  If CoreML diverges >1e-4, ship CPU-only on M1 and note it (no silent precision loss).
* `mmBERT` tokenizer `extra_special_tokens` list→dict fix must happen at export;
  server load of unfixed `tokenizer.json` fails loudly (do not auto-patch at runtime).
* Base checkpoints are weak zero-shot (36% business vs Jev 73% in videos). Ship message:
  fast base to specialise, not magic judgment. Require fine-tuned checkpoint + ECE proof
  for any auto-act path; else escalate.
* `score` primitive weak on strict scaling — gate separately, prefer `choice` buckets.
* All-three-resident (1.16B) exceeds M1 comfort with other tiers (bge-m3/GTE) co-loaded.
  Default `max_loaded=2`; three only with explicit operator opt-in + RSS gate in bench.
* Supply chain: ONNX + tokenizer.json are binary blobs. Require `SHA256SUMS` + pinned
  HF revision + `cargo audit` green + SBOM note (follow `sbom.sh --spec-version 1.5` precedent).
  No runtime HF download (air-gapped profile must work).

## 10. File-by-file checklist (reviewer ticks)

- [ ] `Cargo.toml`: `laya-local` feature only (§4.1), no other dep change, comments dated.
- [x] `src/workflow/mod.rs`: `pub mod decide;` + allow comment. *(ticked 2026-09-22, Phase 0: landed as `pub(crate) mod decide;` — the file-level dead-code allow covers the posture; the visibility flip to `pub` is Phase 3 with callers)*
- [x] `src/workflow/decide/{mod,lang,router,sequence,calibration,presets}.rs` + tests (§2). *(ticked 2026-09-22, Phase 0)*
- [ ] `src/workflow/decide/{inference,audit_ext}.rs` behind `#[cfg(feature="laya-local")]` (§4.5).
- [ ] `tools/export_laya_onnx.py` + `SHA256SUMS` procedure (§3.2).
- [ ] `src/config.rs`: 7 resolvers + `validate_decide_env` + boot call-site (§4.2).
- [ ] `src/workflow/tiers.rs`: 5 keys + `m1-local.env` + `TIER_PROFILE_PATHS` (§4.3).
- [ ] `deploy/tiers/m1-local.env` new; `t1..t4.env` untouched.
- [ ] `docs/deployment.md`: m1-local + key rows (else drift test fails).
- [x] `src/domain_router.rs`: `split_for_decide` + test. *(ticked 2026-09-22, Phase 0; 77-label fixture splits 20/20/20/17)*
- [ ] `src/workflow/frontdoor.rs`: 20-line advisor + 5 tests (§5.2).
- [ ] `src/workflow/calibration.rs`: decide keys passthrough (no logic fork).
- [ ] `docs/BENCHMARKS.md`: M1 row + decide tables (§7).
- [x] `cargo test` (default) green; `cargo test --features laya-local` green (M1); *(first clause ticked 2026-09-22, Phase 0; the laya-local clause is Phase 1)*
        `cargo test --features laya-local decide_` parity `#[ignore]` run by operator with artifacts.
- [x] `cargo audit` green; `cargo tree -i ort` single version. *(first clause ticked 2026-09-22, Phase 0: audit clean, zero new dependency edges; the ort tree check is Phase 1)*

## 11. Commands (M1)

```bash
# Phase 0 (no weights)
cargo test decide_ -- --nocapture
cargo clippy -- -D warnings

# Export (build machine, python3.12)
python3.12 -m venv ~/.venvs/laya-export && source ~/.venvs/laya-export/bin/activate
pip install "laya==0.3.4" torch transformers safetensors huggingface_hub numpy onnx onnxruntime
python tools/export_laya_onnx.py --checkpoint english --out ~/.config/brain-server/models/laya/english
python tools/export_laya_onnx.py --checkpoint multilingual --out ~/.config/brain-server/models/laya/multilingual
# typed-decisions only when piloting that workflow:
python tools/export_laya_onnx.py --checkpoint typed-decisions --out ~/.config/brain-server/models/laya/typed-decisions
shasum -a 256 ~/.config/brain-server/models/laya/*/* > SHA256SUMS.txt  # record in BENCHMARKS.md

# Phase 1-2 (M1 server)
cargo check --features laya-local
BRAIN_DECIDE=on BRAIN_DECIDE_MODELS_DIR=~/.config/brain-server/models/laya \
  cargo run --features laya-local -- --bind 127.0.0.1:3210
MODEL_PROFILE=desktop cargo test --features laya-local -- --ignored decide_parity_

# Tiers
cargo test tier_profiles_boot_and_pass_smoke guide_and_profiles_never_drift
```

---

*Integration contract: deterministic, audited, closed-vocabulary System-1 advice on M1,
with route-mode mandatory, ≤20 options, temperature-fitted ECE, preload-locked checkpoints,
and escalation on any doubt. Anything weaker stays a demo, not a shipment.*
