//! The CID: a multihash over exact bytes, encoded in the kernel's own base32.
//!
//! Deliberately tiny, and deliberately the crate's only hash surface. E4
//! declined adding a hashing crate on the grounds that the primitive is already
//! in the tree; the honest version of that argument for a *workspace-local,
//! zero-external-edge* crate is that `sha2` is already locked by
//! `crates/Cargo.lock` for four sibling cores, so naming it adds no new package.
//! What this module therefore does NOT contain is a hand-rolled digest: a
//! provenance verifier whose trust anchor is a bespoke hash implementation is
//! the wrong place to save a dependency line.

use sha2::{Digest, Sha256};

/// The algorithm tag, mirroring the kernel's `blake3:` prefix convention at
/// `src/ump_integrity.rs:202`.
///
/// **An R46 CID is `sha256:`-prefixed and is NOT interchangeable with the
/// kernel's `blake3:`-prefixed `content_hash_string`.** R50 must not treat the
/// two as the same identifier; the plan's E1 named BLAKE3 and §0.3 named
/// SHA-256, and the round's zero-external-edge law forced §0.3's answer.
pub const CID_ALGORITHM_PREFIX: &str = "sha256:";

/// RFC 4648 base32, no padding, LOWERCASE — the kernel's exact alphabet
/// (`src/ump_integrity.rs:23`), so a CID encoded here is comparable with every
/// content hash the tree already stores.
const CID_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Multihash code for SHA-2 with a 256-bit digest (0x12), a single-byte varint.
const CID_MULTIHASH_CODE: u8 = 0x12;

/// Digest length in bytes (32), also a single-byte varint.
const CID_DIGEST_LEN: usize = 32;

/// The two multihash prefix bytes: `<code><length>`, then the raw digest.
pub const CID_MULTIHASH_PREFIX: [u8; 2] = [CID_MULTIHASH_CODE, CID_DIGEST_LEN as u8];

/// The byte length of the multihash body: 2 prefix bytes plus a 32-byte digest.
const CID_MULTIHASH_LEN: usize = CID_MULTIHASH_PREFIX.len() + CID_DIGEST_LEN;

/// The base32 length of an encoded CID: `ceil(34 * 8 / 5)`, unpadded.
pub const CID_ENCODED_LEN: usize = (CID_MULTIHASH_LEN * 8).div_ceil(5);

/// RFC 4648 base32, no padding, lowercase. 5 bits per char.
///
/// Copied from the kernel's `base32_encode` (`src/ump_integrity.rs:22-39`) so
/// the two agree byte-for-byte. The crate cannot *call* the kernel's copy
/// without a path-dependency on the server, which would defeat the entire
/// point of a zero-edge crate — so the twenty lines are reproduced here and
/// pinned to the same known answers by the round's battery.
pub fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(CID_ALPHABET[((acc >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(CID_ALPHABET[((acc << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// A content identifier over `bytes`: `"sha256:"` + base32(`0x12 0x20` + digest).
///
/// The prefix is a hand-rolled multihash header (E4), not a CIDv1 multicodec
/// string: the plan asked for the multihash prefix, and a full CIDv1 would add
/// a version byte, a multicodec code, and a multibase prefix this round has no
/// reader for.
///
/// Changes with **every single byte** of the input and with nothing else. That
/// avalanche is the whole anti-laundering mechanism: a consolidation pass that
/// rewrites a source produces different bytes, therefore a different CID,
/// therefore a visible `CidMismatch` rather than a silent drift.
pub fn cid_v1(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut buf = [0u8; CID_MULTIHASH_LEN];
    buf[..CID_MULTIHASH_PREFIX.len()].copy_from_slice(&CID_MULTIHASH_PREFIX);
    buf[CID_MULTIHASH_PREFIX.len()..].copy_from_slice(&digest);
    let mut out = String::with_capacity(CID_ALGORITHM_PREFIX.len() + CID_ENCODED_LEN);
    out.push_str(CID_ALGORITHM_PREFIX);
    out.push_str(&base32_encode(&buf));
    out
}

/// Whether `cid` is a syntactically well-formed CID of this algorithm.
///
/// A FORMAT check, not a decode: the shape, the length, and the alphabet. It
/// exists so that a missing or malformed `source_cid` is caught as
/// `UnresolvedSource` rather than falling through to a comparison that might
/// pass — there is deliberately **no** default that could turn an absent CID
/// into a support answer. A well-formed CID that does not match the bytes is a
/// `CidMismatch`, which is a different verdict with a different meaning.
pub fn is_well_formed_cid(cid: &str) -> bool {
    let Some(encoded) = cid.strip_prefix(CID_ALGORITHM_PREFIX) else {
        return false;
    };
    encoded.len() == CID_ENCODED_LEN && encoded.bytes().all(|c| CID_ALPHABET.contains(&c))
}
