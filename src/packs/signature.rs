//! The pack key: minisign-format verification against a KERNEL-SIDE pin.
//!
//! **TWO KEYS, NEVER ONE** (C2 READ FIRST ③). The pack key is a SECOND
//! minisign keypair, distinct from the updater's in generation, custody, and
//! pin location:
//!
//! | | updater key | pack key |
//! |---|---|---|
//! | private half | operator custody (`~/.minisign/brain-server.key`) | operator custody (`minisign -G`, a different key) |
//! | public pin | `BRAIN_RELEASE_PUBKEY` (install-service.sh, install time) | `BRAIN_PACK_PUBKEY` (default `pack.pub` beside the data root, 0644) |
//! | verifies | release binaries | plugin packs |
//!
//! A pack signed with the UPDATER key is a REFUSAL naming both keynums —
//! red-proved, and the pin holds the refusal, not a warning.
//!
//! The byte format is the one the III-8 integration drill MEASURED against
//! minisign's own verdicts (register, III-8 row), not guessed:
//!
//! * signature file, four lines — `untrusted comment: …` / base64 of
//!   `"Ed"` + u64-BE length (64) + keynum (8) + signature (64) /
//!   `untrusted comment: <trusted comment>` / base64 of the raw 64-byte
//!   secondary signature (no algorithm marker, no key id);
//! * the primary signature signs the BLAKE2b-512 prehash of the content;
//! * the secondary signs the primary signature followed by the trusted
//!   comment;
//! * public-key file, two lines — `untrusted comment: …` / base64 of `"Ed"`
//!   + u64-BE length (32) + keynum (8) + public key (32).
//!
//! The key PAIR is minted by the operator with the minisign CLI (the
//! updater chain's own tooling); the kernel only ever reads the pinned
//! public half. Nothing here writes keys.

use blake2::Blake2b512;
use blake2::Digest;
use ed25519_dalek::{SIGNATURE_LENGTH, Signature, VerifyingKey};

/// A signature-chain refusal. Every arm names what it saw — a refusal that
/// cannot say whether the chain was absent, malformed, foreign, or broken
/// is not an operator-usable refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigError {
    /// No signature file beside the pack: unsigned, refused.
    Unsigned,
    /// The signature or key file does not decode to the measured minisign
    /// shape.
    Malformed(&'static str),
    /// The signature's keynum is not the pinned keynum — the UPDATER-KEY
    /// arm. Names both, as the III-8 drill does.
    KeynumMismatch { sig: String, pinned: String },
    /// The chain decodes, the keynum matches, the crypto refuses: the bytes
    /// under the signature are not the bytes signed (the tamper arm), or
    /// the secondary (comment) signature does not bind.
    Tampered(&'static str),
}

impl std::fmt::Display for SigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SigError::Unsigned => {
                write!(
                    f,
                    "the pack carries no signature beside it — unsigned packs are refused"
                )
            }
            SigError::Malformed(what) => {
                write!(f, "the minisign-format file is malformed: {what}")
            }
            SigError::KeynumMismatch { sig, pinned } => write!(
                f,
                "the pack is not signed by the pinned pack key — not by the app's pinned \
                 key (signature keynum {sig}, pinned keynum {pinned})"
            ),
            SigError::Tampered(what) => {
                write!(f, "the pack's signature does not verify: {what}")
            }
        }
    }
}

/// The pinned keynum as the drills print it: 16 uppercase hex chars.
fn keynum_hex(keynum: &[u8]) -> String {
    hex::encode_upper(keynum)
}

/// The public half of the pack key, parsed from the two-line minisign
/// public-key file: (keynum, verifying key).
pub fn parse_pubkey(file: &str) -> Result<(String, VerifyingKey), SigError> {
    let line = file
        .lines()
        .nth(1)
        .ok_or(SigError::Malformed("public key file has no key line"))?;
    let blob = decode_b64(line.trim())?;
    // The MEASURED layout (minisign 0.12, cross-checked 2026-10-27): the
    // blob is exactly `Ed` + keynum (8) + key (32) = 42 bytes — no length
    // field. A synthetic chain that imagined an 8-byte length field passed
    // every self-consistent pin and failed only against the real CLI; the
    // probe test is what caught it.
    if blob.len() != 2 + 8 + 32 {
        return Err(SigError::Malformed(
            "public key blob is not Ed|keynum|key (42 bytes)",
        ));
    }
    if &blob[..2] != b"Ed" {
        return Err(SigError::Malformed("public key algorithm is not Ed"));
    }
    let keynum = keynum_hex(&blob[2..10]);
    let key = VerifyingKey::from_bytes(
        blob[10..42]
            .try_into()
            .map_err(|_| SigError::Malformed("public key slice"))?,
    )
    .map_err(|_| SigError::Malformed("public key is not a valid Ed25519 key"))?;
    Ok((keynum, key))
}

/// Verify one pack's signature chain against the pinned public half.
/// Returns the pinned keynum on success (the receipt names the key that
/// spoke). The signature's presence is checked FIRST — an unsigned pack is
/// the operator's most common mistake and gets the clearest refusal.
pub fn verify_signature(
    content: &[u8],
    sig_file: &str,
    pubkey_file: &str,
) -> Result<String, SigError> {
    // Line 2: the primary signature blob; line 3+4: the trusted comment and
    // its signature. Anything else is the unsigned or malformed arm.
    let mut lines = sig_file.lines();
    let _comment = lines.next().ok_or(SigError::Unsigned)?;
    let sig_line = lines.next().ok_or(SigError::Unsigned)?;
    let comment_line = lines
        .next()
        .ok_or(SigError::Malformed("signature file has no trusted comment"))?;
    let trusted_comment = comment_line
        .strip_prefix("trusted comment: ")
        // minisign's own file (0.12, measured 2026-10-27) writes
        // `trusted comment:`; some tooling writes `untrusted comment:`.
        // The prefix is unauthenticated framing — the TEXT is what the
        // secondary signature binds, so either prefix is accepted and the
        // remainder is the authenticated comment.
        .or_else(|| comment_line.strip_prefix("untrusted comment: "))
        .ok_or(SigError::Malformed("line 3 is not a trusted comment line"))?;
    let secondary_line = lines.next().ok_or(SigError::Malformed(
        "signature file has no secondary signature",
    ))?;

    let (pinned_keynum, pinned_key) = parse_pubkey(pubkey_file)?;
    let blob = decode_b64(sig_line.trim())?;
    // Same measured layout: `Ed` + keynum (8) + signature (64) = 74 bytes.
    if blob.len() != 2 + 8 + SIGNATURE_LENGTH {
        return Err(SigError::Malformed(
            "signature blob is not Ed|keynum|sig (74 bytes)",
        ));
    }
    if &blob[..2] != b"ED" {
        // MEASURED QUIRK (minisign 0.12): the SIGNATURE blob's algorithm
        // marker is "ED" — both uppercase — while the PUBLIC KEY blob
        // carries "Ed". Verified against a real chain 2026-10-27; a
        // verifier demanding "Ed" here refuses every real signature.
        return Err(SigError::Malformed("signature algorithm is not ED"));
    }
    let sig_keynum = keynum_hex(&blob[2..10]);
    if sig_keynum != pinned_keynum {
        // THE two-key law, enforced here: an updater-key signature (or any
        // foreign key's) is a refusal naming both keynums, before any
        // crypto runs.
        return Err(SigError::KeynumMismatch {
            sig: sig_keynum,
            pinned: pinned_keynum,
        });
    }
    let primary = Signature::from_slice(&blob[10..10 + SIGNATURE_LENGTH])
        .map_err(|_| SigError::Malformed("primary signature bytes"))?;

    // The primary signs the BLAKE2b-512 prehash (the measured scheme).
    // BLAKE2b-512 is Blake2b512's fixed output — the prehash minisign signs.
    let digest = Blake2b512::digest(content);
    pinned_key
        .verify_strict(&digest, &primary)
        .map_err(|_| SigError::Tampered("the primary signature is not over these bytes"))?;

    // The secondary signs primary-sig || trusted-comment (the measured
    // scheme) — this is what binds the trusted comment, and the comment is
    // where a binding claim (file name, version) would live.
    let secondary = decode_b64(secondary_line.trim())?;
    if secondary.len() != SIGNATURE_LENGTH {
        return Err(SigError::Malformed(
            "secondary signature is not a raw 64-byte signature",
        ));
    }
    let secondary = Signature::from_slice(&secondary)
        .map_err(|_| SigError::Malformed("secondary signature bytes"))?;
    let mut signed = Vec::with_capacity(SIGNATURE_LENGTH + trusted_comment.len());
    signed.extend_from_slice(&blob[10..10 + SIGNATURE_LENGTH]);
    signed.extend_from_slice(trusted_comment.as_bytes());
    pinned_key
        .verify_strict(&signed, &secondary)
        .map_err(|_| SigError::Tampered("the trusted comment's signature does not bind"))?;

    Ok(pinned_keynum)
}

fn decode_b64(line: &str) -> Result<Vec<u8>, SigError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(line)
        .map_err(|_| SigError::Malformed("base64 line does not decode"))
}

/// The pack pubkey's pin location: `BRAIN_PACK_PUBKEY` when set, otherwise
/// `pack.pub` beside the data root — the audit-chain key's own "beside the
/// DB" law, and deliberately NOT the updater's `BRAIN_RELEASE_PUBKEY`
/// (different var, different default, different custody trail).
pub fn pack_pubkey_path(
    env_value: Option<&str>,
    data_root: &std::path::Path,
) -> std::path::PathBuf {
    env_value
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| data_root.join("pack.pub"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn b64(data: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(data)
    }

    /// Mint a minisign-shaped keypair and format the public-key file.
    fn mint(id: u64) -> (SigningKey, String) {
        let sk = SigningKey::from_bytes(&{
            let mut seed = [0u8; 32];
            seed[..8].copy_from_slice(&id.to_be_bytes());
            seed
        });
        let keynum = id.to_be_bytes();
        // the measured blob: Ed + keynum + pk (42 bytes, no length field)
        let mut blob = Vec::new();
        blob.extend_from_slice(b"Ed");
        blob.extend_from_slice(&keynum);
        blob.extend_from_slice(&sk.verifying_key().to_bytes());
        let keyfile = format!(
            "untrusted comment: minisign public key {}\n{}\n",
            keynum_hex(&keynum),
            b64(&blob)
        );
        (sk, keyfile)
    }

    /// Sign `content` the measured minisign way and format the signature
    /// file: primary over the BLAKE2b-512 prehash, secondary over
    /// primary||trusted-comment, raw 64-byte secondary line.
    fn sign(sk: &SigningKey, keynum: &[u8; 8], content: &[u8], trusted: &str) -> String {
        let digest = Blake2b512::digest(content);
        let primary = sk.sign(&digest);
        let mut signed = Vec::new();
        signed.extend_from_slice(&primary.to_bytes());
        signed.extend_from_slice(trusted.as_bytes());
        let secondary = sk.sign(&signed);

        let mut blob = Vec::new();
        // the measured marker: the SIGNATURE blob carries "ED" (uppercase)
        blob.extend_from_slice(b"ED");
        blob.extend_from_slice(keynum);
        blob.extend_from_slice(&primary.to_bytes());
        format!(
            "untrusted comment: signature from minisign secret key\n{}\nuntrusted comment: {}\n{}\n",
            b64(&blob),
            trusted,
            b64(&secondary.to_bytes())
        )
    }

    const CONTENT: &[u8] = b"the archive bytes a pack arrives as";

    #[test]
    fn a_true_chain_verifies_and_receipts_the_pinned_keynum() {
        let (sk, keyfile) = mint(0xD562_6019_111E_7CC2);
        let sig = sign(
            &sk,
            &0xD562_6019_111E_7CC2u64.to_be_bytes(),
            CONTENT,
            "file:pack.tar.gz",
        );
        let receipt = verify_signature(CONTENT, &sig, &keyfile).expect("a true chain verifies");
        assert_eq!(receipt, "D5626019111E7CC2");
    }

    #[test]
    fn a_pack_signed_by_another_key_is_refused_naming_both_keynums() {
        // THE two-key law, red-provable: the UPDATER key signs, the PACK
        // key's pin verifies. Keynum mismatch, both named, before crypto.
        let (_pack_sk, pack_keyfile) = mint(0xAAAA_0000_0000_0001);
        let (updater_sk, _updater_keyfile) = mint(0xD562_6019_111E_7CC2);
        let sig = sign(
            &updater_sk,
            &0xD562_6019_111E_7CC2u64.to_be_bytes(),
            CONTENT,
            "the updater key's chain over a pack",
        );
        let err = verify_signature(CONTENT, &sig, &pack_keyfile).unwrap_err();
        assert!(
            matches!(err, SigError::KeynumMismatch { .. }),
            "a foreign-key signature is a keynum refusal: {err}"
        );
        assert!(err.to_string().contains("D5626019111E7CC2"));
        assert!(err.to_string().contains("AAAA000000000001"));
    }

    #[test]
    fn an_unsigned_pack_is_refused() {
        let err = verify_signature(CONTENT, "", "not even a key file").unwrap_err();
        assert_eq!(err, SigError::Unsigned);
    }

    #[test]
    fn a_mutated_archive_is_refused_after_signature() {
        let (sk, keyfile) = mint(0xBBBB_0000_0000_0002);
        let sig = sign(&sk, &0xBBBB_0000_0000_0002u64.to_be_bytes(), CONTENT, "c");
        let mut tampered = CONTENT.to_vec();
        tampered[0] ^= 0x01;
        let err = verify_signature(&tampered, &sig, &keyfile).unwrap_err();
        assert!(
            matches!(err, SigError::Tampered(_)),
            "the tamper arm refuses by name: {err}"
        );
    }

    #[test]
    fn a_secondary_that_binds_another_comment_is_refused() {
        let (sk, keyfile) = mint(0xCCCC_0000_0000_0003);
        let mut sig = sign(
            &sk,
            &0xCCCC_0000_0000_0003u64.to_be_bytes(),
            CONTENT,
            "file:pack.tar.gz",
        );
        // swap the trusted comment after signing: the primary still
        // verifies, the SECONDARY must refuse
        sig = sig.replacen(
            "untrusted comment: file:pack.tar.gz",
            "untrusted comment: file:OTHER.tar.gz",
            1,
        );
        let err = verify_signature(CONTENT, &sig, &keyfile).unwrap_err();
        assert!(matches!(err, SigError::Tampered(_)), "{err}");
    }

    #[test]
    fn malformed_files_refuse_without_panicking() {
        let (sk, keyfile) = mint(1);
        let sig = sign(&sk, &1u64.to_be_bytes(), CONTENT, "c");

        // truncated signature file
        let truncated: String = sig.lines().take(2).collect::<Vec<_>>().join("\n");
        assert!(verify_signature(CONTENT, &truncated, &keyfile).is_err());

        // a key file whose blob is the wrong length
        let bad_key = format!("untrusted comment: x\n{}\n", b64(b"short"));
        assert!(matches!(
            verify_signature(CONTENT, &sig, &bad_key),
            Err(SigError::Malformed(_))
        ));

        // non-base64 body
        let bad_sig = "untrusted comment: x\nNOT BASE64 !!\nuntrusted comment: c\nAAAA\n";
        assert!(verify_signature(CONTENT, &bad_sig, &keyfile).is_err());
    }

    #[test]
    fn the_pack_pin_is_not_the_updater_pin() {
        // distinct env var, distinct default: the pack pubkey defaults
        // beside the data root, never at BRAIN_RELEASE_PUBKEY's post
        let root = std::path::Path::new("/data");
        assert_eq!(
            pack_pubkey_path(None, root),
            std::path::PathBuf::from("/data/pack.pub")
        );
        assert_eq!(
            pack_pubkey_path(Some("/etc/pack.pub"), root),
            std::path::PathBuf::from("/etc/pack.pub")
        );
    }

    /// The cross-check against the REAL minisign CLI — the III-8 drill's
    /// own honesty rule: the synthetic chains above prove the arms, this
    /// probe proves the FORMAT against the tool that mints operator keys.
    /// Skipped unless a probe dir is supplied (CI carries no minisign):
    ///
    /// ```sh
    /// d=$(mktemp -d) && cd $d && printf 'payload' > pack.tar.gz
    /// minisign -G -p pack.pub -s pack.key -W </dev/null
    /// minisign -Sm pack.tar.gz -s pack.key -c "file:pack.tar.gz" -t "t" -W
    /// BRAIN_PACKKEY_PROBE_DIR=$d cargo test --lib packs::signature
    /// ```
    #[test]
    fn a_real_minisign_chain_verifies_when_a_probe_dir_is_supplied() {
        let Ok(dir) = std::env::var("BRAIN_PACKKEY_PROBE_DIR") else {
            eprintln!(
                "skipped: set BRAIN_PACKKEY_PROBE_DIR to a dir holding pack.tar.gz, \
                 pack.tar.gz.minisig and pack.pub minted by the real minisign CLI"
            );
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let content = std::fs::read(dir.join("pack.tar.gz")).expect("probe pack.tar.gz");
        let sig = std::fs::read_to_string(dir.join("pack.tar.gz.minisig")).expect("probe sig");
        let key = std::fs::read_to_string(dir.join("pack.pub")).expect("probe pub");
        let receipt = verify_signature(&content, &sig, &key).expect("the real chain verifies");
        let pinned = parse_pubkey(&key).unwrap().0;
        assert_eq!(receipt, pinned);
    }
}
