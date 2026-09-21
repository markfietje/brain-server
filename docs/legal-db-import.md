# The curated legal DB — the DPO import procedure

The `/legal/rules` surface reads a **curated** SQLite file at `BRAIN_LEGAL_DB_PATH`. The
server opens it **READ-ONLY per request** — a fresh import is live on the next request, no
restart — and the server **never writes it**. Population is the DPO's quarterly review, done
by hand with the `sqlite3` CLI. There is no auto-pull from the EU Official Journal (published
every EU working day) or the PH NPC (advisories issued ad hoc, year-based numbering): law
evolves, code does not pre-implement it. **The human DPO is the source of truth.**

## 1. The quarterly review (operator steps)

1. **Diff the law, outside this repo.** Review the sources your deployment answers to —
   e.g. EUR-Lex for EU instruments, the NPC site for PH advisories, IMDA for the (voluntary)
   MGF — against the DB's current rows. The operator's own tooling does this; nothing in this
   repo pulls for you.
2. **Back up the file.** `cp "$BRAIN_LEGAL_DB_PATH" "$BRAIN_LEGAL_DB_PATH.bak-$(date +%F)"`.
3. **Make your edits with plain SQL** (the schema is below). An import that changes rows
   typically:
   - adds a new version row: `INSERT INTO law_version(jurisdiction, version, effective_at,
     source_ref, reviewed_by, reviewed_at) VALUES ('ph', 'npc-advisory-2026-01', 1790000000,
     'NPC advisory 2026-01 URL', 'your-dpo-id', strftime('%s','now'));`
   - adds or revises rules: `INSERT INTO jurisdiction_rules(jurisdiction, subject, rule_key,
     body, source_ref, law_version, deadline_days, rights, effective_at, reviewed_at,
     revision) VALUES (...)` — keep `rights` a JSON array of strings
     (e.g. `'["access","erasure"]'`); mark superseded rules via `superseded_by`.
   - attests a transfer mechanism's posture (only the operator knows which safeguard they
     signed): `UPDATE surveillance_postures SET status='attested', law_version='…',
     jurisdictions='["eu"]', reviewed_by='your-dpo-id', reviewed_at=strftime('%s','now')
     WHERE mechanism='scc-eu-2021';`
4. **Re-pin the head.** The file carries a single `schema_meta['law_version']` pin — counts
   plus max rule id — which must describe the rows exactly (the same law as the audit chain's
   head pin). Compute and write it in one statement:

   ```sql
   INSERT INTO schema_meta(key, value)
   SELECT 'law_version',
          json_object('rules', (SELECT COUNT(*) FROM jurisdiction_rules),
                      'versions', (SELECT COUNT(*) FROM law_version),
                      'postures', (SELECT COUNT(*) FROM surveillance_postures),
                      'max_rules_id', (SELECT COALESCE(MAX(id),0) FROM jurisdiction_rules))
   ON CONFLICT(key) DO UPDATE SET value = excluded.value;
   ```

   Forgetting this step is detectable: the crate's consistency test fails on drift, and the
   counts the diff route echoes will not match the rows.
5. **Record the sign-off in the rows themselves** — every row you touched carries your
   `reviewed_by` + `reviewed_at`. That record IS the DPO sign-off; the DB has no other auth.
6. **Verify read-only access still works**: with the server running, `GET /legal/rules` (Admin
   + DPO role) should return the updated diff. If you moved the file, update
   `BRAIN_LEGAL_DB_PATH` — the route refuses NAMED (`legal_db_unconfigured` / 
   `legal_db_unavailable`) rather than guessing.

## 2. First-time initialization

Ship the file either by seeding from the crate's own curated seed (the SDK law-version table
+ the transfers register's DSAR rules + the mechanism vocabulary), or by creating the schema
and inserting your rows directly:

- schema + seed (Rust): `legal_rules_db::db::create_schema(&conn)` then
  `legal_rules_db::db::seed(&conn, "your-dpo-id", now)`.
- schema only (SQL): the five `CREATE` statements live in
  `crates/legal-rules-db/src/db.rs` (`create_schema`) — copy them verbatim; the FTS5 index
  (`rules_fts`) must be kept in step with `jurisdiction_rules` (the seed paths do this; if
  you insert rows via raw SQL, also
  `INSERT INTO rules_fts(rowid, body, source_ref) SELECT id, body, source_ref FROM
  jurisdiction_rules` and afterwards `INSERT INTO rules_fts(rules_fts) VALUES('rebuild');`).

Then set `BRAIN_LEGAL_DB_PATH` (see `docs/configuration.md`) and restart is NOT required —
the route opens the file per request. Unset, the route refuses NAMED and everything else is
byte-unaffected.

## 3. What this file does NOT claim

- No auto-pull: nothing in the server fetches law text from anywhere.
- No auto-block: a stale `law_version` pin on a run report yields the advisory
  `law_version_mismatch` field — advisory only, never a refusal.
- No HK surveillance-posture content: the mechanism vocabulary ships; jurisdiction-specific
  posture text is curated here, by the DPO, or it stays empty.
- The DB's contents are curation, not legal advice.
