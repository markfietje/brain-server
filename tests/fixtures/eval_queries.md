# Frozen Eval Query Fixture (v0.9.1 starter set)

This file is the **judged query taxonomy** for the eval harness. It documents the
query categories Brain Server must cover before any "parity with QMD" claim:

- exact IDs / codes
- semantic paraphrase
- ambiguous terms
- negation
- freshness
- source filters
- short notes
- long notes
- code

## Corpus

Relevant doc indices below are into the `DOCS` array in `tests/eval.rs` (0-based):

| idx | doc (truncated) |
|----:|---|
| 0 | Bignay is a tropical fruit and a good alternative to blueberry, rich in antioxidants. |
| 1 | The Rust programming language guarantees memory safety without a garbage collector. |
| 2 | Vitamin D3 supplementation improves immune function and bone density in deficient adults. |
| 3 | The GDPR is a European regulation protecting the personal data of EU residents. |
| 4 | Gut microbiome diversity affects inflammation markers and immune system regulation. |
| 5 | SQLite is an embedded relational database with FTS5 full-text search support. |
| 6 | ISO 9001 is the international standard for quality management systems. |
| 7 | Ownership and borrowing are Rust's core concepts for compile-time memory safety. |
| 8 | Antioxidants in tropical fruits like bignay help reduce oxidative stress. |
| 9 | The GDPR covers any organization processing EU residents' data, with fines up to four percent of global revenue. |

## Status / scale requirement

This is a **starter set of 32 judged queries** over the 10-doc smoke-corpus in
`eval.rs`. It is NOT sufficient for a parity claim.

> **REQUIRED before any "parity with QMD" claim:** expand to **≥ 100 judged queries**
> over a representative, versioned corpus. The 10-doc set above is only a wiring/CI
> smoke fixture; recall numbers on it are not evidence of quality.

> **Set hygiene:** maintain **separate dev / validation / final** query sets.
> Thresholds and PRF/RRF confidence constants MUST NOT be tuned on the final set. The
> final set is only for reporting. Re-judging or re-ordering relevance after seeing
> results invalidates the set.

## Format

Each query block:
```
### Q<n> — [category]
Query: "<the query string>"
Relevant: [<doc indices into DOCS>]
```

---

### Q1 — [exact_id_code]
Query: "GDPR"
Relevant: [3, 9]

### Q2 — [exact_id_code]
Query: "ISO 9001"
Relevant: [6]

### Q3 — [exact_id_code]
Query: "FTS5"
Relevant: [5]

### Q4 — [exact_id_code]
Query: "9001 quality management"
Relevant: [6]

### Q5 — [semantic_paraphrase]
Query: "fruit that beats blueberries for antioxidants"
Relevant: [0, 8]

### Q6 — [semantic_paraphrase]
Query: "language that keeps memory safe with no GC"
Relevant: [1, 7]

### Q7 — [semantic_paraphrase]
Query: "supplement for weak bones and low immunity"
Relevant: [2]

### Q8 — [semantic_paraphrase]
Query: "regulation protecting personal data of EU residents"
Relevant: [3, 9]

### Q9 — [semantic_paraphrase]
Query: "gut bacteria and inflammation"
Relevant: [4]

### Q10 — [semantic_paraphrase]
Query: "small embedded SQL database with full text search"
Relevant: [5]

### Q11 — [semantic_paraphrase]
Query: "how does Rust avoid use-after-free at compile time"
Relevant: [1, 7]

### Q12 — [semantic_paraphrase]
Query: "tropical berry that fights oxidative stress"
Relevant: [0, 8]

### Q13 — [ambiguous_terms]
Query: "regulation"
Relevant: [3, 9]

### Q14 — [ambiguous_terms]
Query: "immunity"
Relevant: [2, 4]

### Q15 — [ambiguous_terms]
Query: "database"
Relevant: [5]

### Q16 — [ambiguous_terms]
Query: "standard"
Relevant: [6]

### Q17 — [negation]
Query: "fruit that is not a blueberry but similar"
Relevant: [0, 8]

### Q18 — [negation]
Query: "memory safety without a garbage collector"
Relevant: [1, 7]

### Q19 — [negation]
Query: "not a relational database"
Relevant: [] 

### Q20 — [negation]
Query: "vitamin that does not help bones"
Relevant: []

### Q21 — [freshness]
Query: "current GDPR fine structure"
Relevant: [9]

### Q22 — [freshness]
Query: "latest ISO 9001 revision"
Relevant: [6]

### Q23 — [freshness]
Query: "most recent guidance on vitamin D3 for adults"
Relevant: [2]

### Q24 — [freshness]
Query: "newest note on gut microbiome and inflammation"
Relevant: [4]

### Q25 — [source_filter]
Query: "source: regulation notes — EU data protection"
Relevant: [3, 9]

### Q26 — [source_filter]
Query: "source: standards notes — quality management"
Relevant: [6]

### Q27 — [source_filter]
Query: "source: rust docs — ownership"
Relevant: [7]

### Q28 — [source_filter]
Query: "source: fruit notes — bignay"
Relevant: [0, 8]

### Q29 — [short_note]
Query: "blueberry alternative"
Relevant: [0, 8]

### Q30 — [short_note]
Query: "Rust borrowing"
Relevant: [1, 7]

### Q31 — [long_note]
Query: "I'm a deficient adult and want to improve both my immune system and bone density with a supplement, what should I take?"
Relevant: [2]

### Q32 — [long_note]
Query: "Our company processes EU residents' personal data and we want to know which regulation applies and how large the fines can be."
Relevant: [3, 9]

### Q33 — [code]
Query: "Rust borrow checker"
Relevant: [1, 7]

### Q34 — [code]
Query: "SQLite PRAGMA full text search"
Relevant: [5]

### Q35 — [code]
Query: "Rust ownership vs borrowing compile time"
Relevant: [1, 7]

### Q36 — [code]
Query: "embedded DB FTS5 index"
Relevant: [5]

---

## Coverage checklist

- [x] exact IDs/codes (Q1–Q4)
- [x] semantic paraphrase (Q5–Q12)
- [x] ambiguous terms (Q13–Q16)
- [x] negation (Q17–Q20)
- [x] freshness (Q21–Q24)
- [x] source filters (Q25–Q28)
- [x] short notes (Q29–Q30)
- [x] long notes (Q31–Q32)
- [x] code (Q33–Q36)

36 judged queries over the 10-doc smoke corpus. **Reach ≥ 100 on a versioned,
representative corpus (dev/validation/final kept separate) before claiming QMD parity.**
