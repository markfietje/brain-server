//! The delivery loop's attestation chain (R40).
//!
//! RED-FIRST STAGE: this file currently carries ONLY its test region. The
//! production surface it pins lands in the implementation commit.

#![deny(unsafe_code)]

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic test signing key. Generated, never literal (the
    /// `src/testkeys.rs` doctrine: CodeQL's hard-coded-cryptographic-value
    /// query taint-flags literal key bytes reaching a signing sink, and these
    /// ARE signing sinks).
    fn test_key(seed: u64) -> SigningKey {
        let bytes = crate::testkeys::unit_hmac_key(seed);
        let fixed: [u8; 32] = bytes.as_slice().try_into().expect("a 32-byte seed");
        SigningKey::from_bytes(&fixed)
    }

    // ── the envelope ────────────────────────────────────────────────────────

    /// The envelope is a pure function of its facts: the same link sealed twice
    /// is byte-identical, or nothing downstream can be reasoned about.
    #[test]
    fn attestation_envelope_is_deterministic() {
        let key = test_key(11);
        let did = test_did(&key);
        let first = seal_envelope(&root_link(), &did, &key).expect("first seal");
        let second = seal_envelope(&root_link(), &did, &key).expect("second seal");
        assert_eq!(
            first.envelope, second.envelope,
            "two seals of the same link must be byte-identical"
        );
        assert_eq!(
            first.chain_hash, second.chain_hash,
            "the record digest is a function of the link's facts"
        );
        assert_eq!(first.row_id, second.row_id, "the row id is content-addressed");
    }

    /// KNOWN ANSWER, not self-consistency. Three mutually incompatible signing
    /// formulas coexist in `ump_integrity` (`sign_hash` over the raw BLAKE3
    /// digest, `sign_manifest_bytes` over a hex-SHA256 string, `sign_hash_string`
    /// over BLAKE3 of the `blake3:…` STRING) and two canonicalizers coexist
    /// (`canonical_ump`, the RFC 8785 flavor the reference `contentHash`
    /// reproduces, and `canonical_jcs`, a bare `serde_json::to_vec`). A
    /// self-consistent test passes under ANY of the six pairings, so this one
    /// recomputes the signed message by hand from the stored envelope and
    /// pins the exact `content_hash` string and the exact signature bytes.
    #[test]
    fn attestation_signs_the_shipped_canonical_form() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");

        // The signed object is the envelope MINUS its integrity block.
        let mut signed = sealed.envelope.clone();
        let removed = signed
            .as_object_mut()
            .expect("an envelope is an object")
            .remove("integrity");
        assert_eq!(
            removed.as_ref().and_then(|v| v.get("content_hash")).is_some(),
            true,
            "the integrity block is the signed object's only exclusion"
        );

        let canonical =
            crate::ump_integrity::canonical_ump(&signed).expect("the shipped canonicalizer");
        let expected_hash = crate::ump_integrity::content_hash_string(&canonical);
        assert_eq!(
            sealed.content_hash, expected_hash,
            "the recorded content_hash IS blake3: + base32(blake3(canonical_ump(envelope minus integrity)))"
        );

        let emitted = sealed
            .envelope["integrity"]["signature"]
            .as_str()
            .expect("the integrity block carries a signature")
            .strip_prefix("ed25519:")
            .expect("the signature is scheme-prefixed");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(emitted)
                .expect("standard base64"),
            crate::ump_integrity::sign_hash_string(&expected_hash, &key),
            "the signature is sign_hash_string over the content-hash STRING"
        );
    }

    /// The formula is pinned against its SIBLINGS, not against itself: the
    /// recorded signature must verify under the chosen formula and must NOT
    /// verify under the raw-digest one.
    #[test]
    fn attestation_signs_with_the_pinned_formula() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let pk = key.verifying_key().to_bytes();

        assert!(
            crate::ump_integrity::verify_hash_string(&sealed.content_hash, &pk, &sealed.signature),
            "the pinned formula verifies its own signature"
        );
        let raw = crate::ump_integrity::record_hash(sealed.content_hash.as_bytes());
        assert!(
            !crate::ump_integrity::verify_hash(&pk, &raw, &sealed.signature),
            "the same bytes must NOT verify under sign_hash's raw-digest message — if this \
             ever passes, the two formulas have converged and the pin is no longer discriminating"
        );
    }

    /// The predicate TYPE and the PARENT are inside the signed object (A1).
    /// Changing either stops the ORIGINAL signature verifying — which is the
    /// point: a re-typed or re-parented link is not the link that was signed.
    #[test]
    fn attestation_signature_covers_predicate_type_and_parent() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let pk = key.verifying_key().to_bytes();
        assert!(
            crate::ump_integrity::verify_hash_string(&sealed.content_hash, &pk, &sealed.signature),
            "the link verifies as sealed"
        );

        for (field, value) in [
            ("predicate_type", json!("urn:brain:attest:delivery:v2")),
            ("parent_id", json!("att_somewhere_else")),
        ] {
            let mut tampered = sealed.envelope.clone();
            tampered[field] = value;
            let mut stripped = tampered.clone();
            stripped.as_object_mut().expect("object").remove("integrity");
            let canonical =
                crate::ump_integrity::canonical_ump(&stripped).expect("canonicalize the tamper");
            let rehashed = crate::ump_integrity::content_hash_string(&canonical);
            assert_ne!(
                rehashed, sealed.content_hash,
                "changing the signed `{field}` must change the hashed object at all"
            );
            assert!(
                !crate::ump_integrity::verify_hash_string(&rehashed, &pk, &sealed.signature),
                "changing the signed `{field}` must stop the ORIGINAL signature verifying"
            );
        }

        assert_eq!(
            sealed.envelope["predicate_type"],
            json!(brain_delivery_core::PREDICATE_TYPE),
            "the envelope's predicate_type is the crate's shipped constant, never a literal here"
        );
    }

    // ── the offline verifier ────────────────────────────────────────────────

    /// A real cryptographic failure: one flipped signature bit.
    #[test]
    fn attestation_verify_rejects_a_flipped_bit() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let mut row = sealed.row(&did);
        let mut signature = sealed.signature.clone();
        signature[0] ^= 0x01;
        row.envelope_json = replace_signature(&sealed.envelope, &signature);
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a flipped signature bit must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::SignatureInvalid.as_str()),
            "the refusal names the signature: {:?}",
            verdict.links[0].refusal
        );
    }

    /// A `did:key` that does not decode to a key at all.
    #[test]
    fn attestation_verify_rejects_an_unknown_signer() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let mut row = sealed.row(&did);
        row.signer_did = "did:key:zNotBase58!!".to_string();
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "an undecodable signer must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::UnknownSigner.as_str())
        );
    }

    /// A correctly-signed link whose `signer_did` names a DIFFERENT key. The
    /// bytes are fine and the attribution is not — and it refuses under a
    /// DIFFERENT code than an unknown signer, because the two are different
    /// failures and a caller must be able to tell them apart.
    #[test]
    fn attestation_verify_rejects_a_foreign_signer() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let foreign = test_key(99);
        let mut envelope = sealed.envelope.clone();
        // Re-sign the same signed object with a different key: the signature
        // verifies, but not under the signer the row claims.
        let signature = crate::ump_integrity::sign_hash_string(&sealed.content_hash, &foreign);
        let mut row = sealed.row(&did);
        row.envelope_json = replace_signature(&envelope, &signature);
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a foreign signer must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::SignerMismatch.as_str()),
            "the integrity block's signer and the signed signer_did must agree"
        );
    }

    /// A row with no signature is never a degraded mark: it is a refusal. The
    /// hash-only L2 posture is a DIFFERENT product, and this surface never
    /// silently downgrades into it.
    #[test]
    fn attestation_verify_rejects_an_unsigned_row() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let mut row = sealed.row(&did);
        let mut envelope = sealed.envelope.clone();
        let object = envelope.as_object_mut().expect("object");
        object.get_mut("integrity").and_then(|v| v.as_object_mut()).map(|i| {
            i.remove("signature");
            i.remove("signer");
        });
        row.envelope_json = serde_json::to_string(&envelope).expect("re-serialize");
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "an unsigned row must never read as verified");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::IntegrityIncomplete.as_str())
        );
    }

    /// The predicate inside the envelope must be the crate's CANONICAL form. A
    /// re-serialized, factually-different-key-order predicate is refused: the
    /// digest is over canonical bytes, and re-encoding asserts a byte string
    /// that was never the one digested.
    #[test]
    fn attestation_verify_rejects_a_non_canonical_predicate() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let mut row = sealed.row(&did);
        let mut envelope = sealed.envelope.clone();
        let predicate = envelope["predicate"].clone();
        let object = predicate.as_object().expect("the predicate is an object");
        // Same two facts, wrong key order and a dropped field: a value that is
        // semantically close and byte-wise different.
        envelope["predicate"] = json!({
            "tier": object["tier"].clone(),
            "run_ref": object["run_ref"].clone(),
        });
        row.envelope_json = serde_json::to_string(&envelope).expect("re-serialize");
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a non-canonical predicate must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::PredicateMalformed.as_str()),
            "parse_canonical_predicate's own refusal, carried verbatim: {:?}",
            verdict.links[0].refusal
        );
    }

    /// The stored `predicate_digest` COLUMN is cross-checked against the
    /// envelope's own predicate, so a column that disagrees with the signed
    /// bytes is caught even when the signature over those bytes is valid. This
    /// is the check that stops a digest from being asserted beside a
    /// predicate it does not describe.
    #[test]
    fn attestation_verify_rejects_a_predicate_digest_that_disagrees() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let mut row = sealed.row(&did);
        // Same length, one hex character different: a digest that is
        // well-formed and describes something else.
        let flipped = if sealed.predicate_digest.starts_with('0') { "1" } else { "0" };
        row.predicate_digest = format!(
            "{flipped}{}",
            &sealed.predicate_digest[1..]
        );
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a disagreeing predicate digest must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::PredicateDigestMismatch.as_str()),
            "the recomputed predicate digest must equal the stored column: {:?}",
            verdict.links[0].refusal
        );
    }

    /// The chain walk itself (DO invariant 2, offline): a link whose
    /// `parent_id` names a row that is not the previous link.
    #[test]
    fn attestation_verify_rejects_a_broken_chain() {
        let key = test_key(11);
        let did = test_did(&key);
        let first = seal_envelope(&root_link(), &did, &key).expect("seal the first");
        let mut child_link = root_link();
        child_link.step_id = 9818;
        child_link.subject_name = "delivery/phase/release".to_string();
        let child = seal_envelope(&child_link, &did, &key).expect("seal the child");
        let mut child_row = child.row(&did);
        // Sealed as a root, so its parent is empty; point it at a row that is
        // not in this chain.
        child_row.parent_id = "att_not_in_this_chain".to_string();
        let chain = vec![first.row(&did), child_row];
        let verdict = verify_chain(&chain, 1_790_000_100).expect("a two-link chain is readable");
        assert!(!verdict.verified, "a broken link must not verify");
        assert_eq!(
            verdict.links[1].refusal,
            Some(ChainRefusal::BrokenLink.as_str())
        );
    }

    /// A root that names a parent is a FRAGMENT of a longer chain, not a
    /// chain — the crate's own `RootHasParent` defect, adopted here.
    #[test]
    fn attestation_verify_rejects_a_root_that_names_a_parent() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let mut row = sealed.row(&did);
        row.parent_id = "att_something".to_string();
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a fragment is not a chain");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::RootHasParent.as_str())
        );
    }

    /// A link that claims a creation time in the future is refused. `now` is a
    /// PARAMETER precisely so the verifier never reads a clock (DO invariant
    /// 2) — and this is the check that parameter exists for.
    #[test]
    fn attestation_verify_rejects_a_link_dated_in_the_future() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let row = sealed.row(&did);
        let verdict = verify_chain(std::slice::from_ref(&row), row.created_at - 1)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a link dated after the read must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::DatedInFuture.as_str())
        );
    }

    /// `chain_hash` IS the shipped `Attestation::record_digest()` (A2) — the
    /// crate's own pipe-framed, domain-separated digest. Recomputing it from
    /// the seven fields must reproduce the stored value byte for byte.
    #[test]
    fn attestation_chain_hash_is_the_shipped_record_digest() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let row = sealed.row(&did);
        let rebuilt = Attestation {
            subject_name: root_link().subject_name.clone(),
            subject_digest: root_link().subject_digest.clone(),
            predicate_digest: row.predicate_digest.clone(),
            predicate_type: brain_delivery_core::PREDICATE_TYPE.to_string(),
            policy_digest: String::new(),
            signer_did: did,
            parent_digest: String::new(),
        };
        assert_eq!(
            row.chain_hash,
            rebuilt.record_digest(),
            "the stored chain_hash is the crate's record_digest over the same seven fields"
        );
    }

    /// The links are ORDERED (A3): the child's `parent_id` is the parent's ROW
    /// ID, and the child's `Attestation::parent_digest` is the parent's
    /// `chain_hash`. Both halves are checked, because either alone is forgeable
    /// — a row id is not a digest, and a digest alone does not order the rows.
    #[test]
    fn attestation_chain_links_are_ordered() {
        let mut conn = test_db();
        let operator = crate::test_support::operator_key_guard();
        let first_link = ChainLink {
            step_id: 1,
            ..root_link()
        };
        let first = append_in_own_tx(&mut conn, &first_link).expect("the first link");
        let second_link = ChainLink {
            step_id: 2,
            subject_name: "delivery/phase/release".to_string(),
            ..root_link()
        };
        let second = append_in_own_tx(&mut conn, &second_link).expect("the second link");

        let rows = read_chain(&conn, root_link().run_id).expect("read the chain");
        assert_eq!(rows.len(), 2, "two links were appended");
        assert_eq!(rows[0].id, first, "the chain reads in append order");
        assert_eq!(rows[1].id, second);
        assert_eq!(rows[0].parent_id, "", "the root names no parent");
        assert_eq!(rows[1].parent_id, first, "the child names the parent's ROW ID");
        assert_eq!(
            rows[1].chain_hash,
            Attestation {
                subject_name: second_link.subject_name.clone(),
                subject_digest: second_link.subject_digest.clone(),
                predicate_digest: rows[1].predicate_digest.clone(),
                predicate_type: brain_delivery_core::PREDICATE_TYPE.to_string(),
                policy_digest: String::new(),
                signer_did: operator.did.clone(),
                parent_digest: rows[0].chain_hash.clone(),
            }
            .record_digest(),
            "the child's record digest binds the PARENT'S CHAIN HASH, not its row id"
        );
        let verdict = verify_chain(&rows, 1_790_000_100).expect("a two-link chain is readable");
        assert!(verdict.verified, "a well-formed two-link chain verifies: {verdict:?}");
        assert_eq!(verdict.head.as_deref(), Some(second.as_str()), "the head is the last link");
        assert_eq!(verdict.signer_dids, vec![operator.did.clone()]);
    }

    /// A5: `record_digest` frames its seven fields with `|`, so a value that
    /// itself contains `|` — or a control byte — would let two different links
    /// frame to the same bytes. That is a typed refusal at the WRITE.
    /// `record_digest` itself is NOT modified: it is the frozen
    /// back-compatible form, and the fence lives at the boundary instead.
    #[test]
    fn attestation_refuses_a_pipe_in_a_record_digest_field() {
        let key = test_key(11);
        let did = test_did(&key);
        for (field, poisoned) in [
            ("subject_name", "delivery/phase|build"),
            ("subject_digest", "sha256:aa\u{1f}bb"),
            ("policy_digest", "sha256:cc\ndd"),
        ] {
            let mut link = root_link();
            match field {
                "subject_name" => link.subject_name = poisoned.to_string(),
                "subject_digest" => link.subject_digest = poisoned.to_string(),
                _ => link.policy_digest = Some(poisoned.to_string()),
            }
            let err = seal_envelope(&link, &did, &key)
                .expect_err("a framing-ambiguous value must refuse");
            assert!(
                matches!(err, AttestationError::NameRefused { .. }),
                "the `{field}` refusal is typed, not a silent pass: {err:?}"
            );
        }
    }

    /// A9: `signer_did` IS the key history. A rotation changes who signs NEXT;
    /// it does not retroactively invalidate a link, because the verifying key
    /// is recovered from the link's OWN `signer_did` and nothing else. This is
    /// authorship-≠-authority in its most testable form: there is no key file,
    /// no counter, and no epoch in the loop.
    #[test]
    fn attestation_rotation_does_not_invalidate_history() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_envelope(&root_link(), &did, &key).expect("seal");
        let row = sealed.row(&did);
        let rotated = test_key(12);
        let rotated_did = test_did(&rotated);
        assert_ne!(did, rotated_did, "the rotation really changed the key");

        let verdict =
            verify_chain(std::slice::from_ref(&row), 1_790_000_100).expect("a one-link chain");
        assert!(
            verdict.verified,
            "a link signed by a rotated-away key stays verifiable: {verdict:?}"
        );
        assert_eq!(verdict.signer_dids, vec![did]);
        assert!(
            !verdict.signer_dids.contains(&rotated_did),
            "the new key is not in the history"
        );
    }

    /// The chain walk is BOUNDED. An unbounded read of an append-only log is an
    /// availability hole, so the cap is a refusal, not a truncation.
    #[test]
    fn attestation_chain_walk_is_bounded() {
        let mut conn = test_db();
        let _operator = crate::test_support::operator_key_guard();
        for step in 1..=MAX_CHAIN_LINKS {
            let link = ChainLink {
                step_id: step,
                ..root_link()
            };
            append_in_own_tx(&mut conn, &link).expect("append inside the cap");
        }
        let rows = read_chain(&conn, root_link().run_id).expect("read the chain");
        assert_eq!(rows.len(), MAX_CHAIN_LINKS, "every link inside the cap was read");
        let verdict = verify_chain(&rows, 1_790_000_100).expect("a bounded chain is readable");
        assert!(verdict.verified, "a full chain inside the cap verifies");
        assert_eq!(verdict.link_count, MAX_CHAIN_LINKS);

        // One past the cap refuses rather than reading a partial chain.
        let over = ChainLink {
            step_id: MAX_CHAIN_LINKS as i64 + 1,
            ..root_link()
        };
        let err = append_in_own_tx(&mut conn, &over).expect_err("a chain past the cap refuses");
        assert!(
            matches!(err, AttestationError::ChainFull),
            "the refusal is the cap's own: {err:?}"
        );
    }

    // ── the module's own gates ──────────────────────────────────────────────

    /// DO invariant 2 as a named test: the chain verifies OFFLINE. The
    /// verifier is a pure function — no socket, no config, no key file, no
    /// AppState, no clock. `now` is a parameter precisely so it cannot read
    /// one. This is a source scan of the PRODUCTION region with the
    /// `#[cfg(test)]` boundary LOCATED, not assumed (the R39 lesson: four
    /// guards shipped vacuous because their scan region was wrong).
    #[test]
    fn attestation_offline_verification_touches_no_io() {
        let body = fn_body(include_str!("attestations.rs"), "verify_chain");
        for forbidden in [
            "std::fs",
            "std::net",
            "std::process",
            "chrono",
            "Utc",
            "env::",
            "AppState",
            "resolve_operator_key",
            "operator_key",
            "Pool",
            "read_chain",
        ] {
            assert!(
                !body.contains(forbidden),
                "the offline verifier must not reach `{forbidden}` — it is handed the rows, \
                 never the store"
            );
        }
        assert!(
            body.contains("now: i64"),
            "the clock is a parameter, never read: the signature takes `now`"
        );
    }

    /// A13: the verifier parses each `envelope_json` ONCE and hands the SAME
    /// parsed object onward. A second parse after verification would re-derive
    /// the payload from bytes that were never the ones checked — the parse-once
    /// property DSSE v1.0.2 states for its own envelope, adopted here for a
    /// format that is NOT DSSE.
    #[test]
    fn attestation_verifier_does_not_reparse_after_verification() {
        let body = fn_body(include_str!("attestations.rs"), "verify_chain");
        let parses = body.matches("from_str").count();
        assert_eq!(
            parses, 1,
            "the verifier parses each envelope exactly once (found {parses} parses in the \
             production body of verify_chain)"
        );
    }

    // ── helpers ─────────────────────────────────────────────────────────────

    fn test_did(key: &SigningKey) -> String {
        crate::ump_integrity::did_key_from_ed25519(&key.verifying_key().to_bytes())
    }

    use base64::Engine as _;
    use serde_json::json;
    use serde_json::Value;

    /// A root link over the shipped 13-field predicate.
    fn root_link() -> ChainLink<'static> {
        ChainLink {
            domain: "global",
            run_id: 4211,
            step_id: 9817,
            subject_name: "delivery/phase/build".to_string(),
            subject_digest: format!("sha256:{}", "a".repeat(64)),
            policy_digest: None,
            config_digest: None,
            model_ref: None,
            model_digest: None,
            tier: AutonomyTier::BoundedAuto,
        }
    }

    /// Swap a link's signature bytes into its stored envelope, leaving every
    /// other byte alone — the shape of a real tamper, not a re-seal.
    fn replace_signature(envelope: &Value, signature: &[u8]) -> String {
        let mut tampered = envelope.clone();
        tampered["integrity"]["signature"] =
            json!(format!(
                "ed25519:{}",
                base64::engine::general_purpose::STANDARD.encode(signature)
            ));
        serde_json::to_string(&tampered).expect("re-serialize the tampered envelope")
    }

    fn test_db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open in-memory DB");
        crate::migration::run_migration(&mut conn, 512).expect("migration");
        conn
    }

    /// The PRODUCTION region of a function: the file is split at its
    /// `#[cfg(test)]` boundary, the named function is located inside it, and
    /// its braces are matched. An empty or unlocatable extraction fails loudly
    /// rather than passing on nothing.
    fn fn_body(source: &str, symbol: &str) -> String {
        let boundary = source
            .find("#[cfg(test)]")
            .expect("the test boundary must be locatable — a scan of the whole file is vacuous");
        let production = &source[..boundary];
        let needle = format!("fn {symbol}(");
        let start = production
            .find(&needle)
            .unwrap_or_else(|| panic!("the production region must declare `fn {symbol}`"));
        let open = production[start..]
            .find('{')
            .unwrap_or_else(|| panic!("`fn {symbol}` must have a body"));
        let mut depth = 0_usize;
        let end = production[start + open..]
            .char_indices()
            .find_map(|(i, c)| {
                match c {
                    '{' => {
                        depth += 1;
                        None
                    }
                    '}' => {
                        depth -= 1;
                        if depth == 0 { Some(i + 1) } else { None }
                    }
                    _ => None,
                }
            })
            .unwrap_or_else(|| panic!("`fn {symbol}`'s braces must balance"));
        let body = &production[start + open..start + open + end];
        assert!(
            body.len() > 40,
            "`fn {symbol}`'s extracted body is implausibly short ({}) — the extractor, not the \
             function, is broken",
            body.len()
        );
        // Line comments are stripped so a doc comment naming a forbidden token
        // cannot fail the scan (the F7-07 comment-stripping precedent).
        body.lines()
            .map(|l| l.split_once("//").map_or(l, |(code, _)| code))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Append one link in its own transaction, mirroring how `advance()` calls
    /// it — inside a caller-owned `WorkflowTx`.
    fn append_in_own_tx(
        conn: &mut Connection,
        link: &ChainLink<'_>,
    ) -> Result<String, AttestationError> {
        let mut tx = crate::workflow::tx::WorkflowTx::begin(conn).expect("begin");
        let id = append_link(tx.tx(), link, ROOT_LINK_NOW)?;
        tx.commit().expect("commit");
        Ok(id)
    }

    const ROOT_LINK_NOW: i64 = 1_790_000_000;
}
