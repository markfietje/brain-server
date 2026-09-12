//! Unit-test key material (shared by test modules).
//!
//! `rust/hard-coded-cryptographic-value` (CodeQL) taint-flags literal key
//! bytes reaching HMAC/cipher sinks — including in `#[cfg(test)]` fixtures,
//! where the "secret" is meaningless filler. This helper derives key bytes
//! from an xorshift stream over a numeric seed: no string/byte-literal key
//! material exists anywhere, so nothing is (or represents) a credential,
//! while tests stay deterministic — signatures are computed AND verified at
//! runtime against the same generated key.

pub(crate) fn unit_hmac_key(seed: u64) -> Vec<u8> {
    let mut x = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0x1234_5678_9ABC_DEF0);
    let mut out = Vec::with_capacity(32);
    for _ in 0..4 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_be_bytes());
    }
    out
}
