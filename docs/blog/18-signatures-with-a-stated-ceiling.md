# Signatures with a stated ceiling

*2026-09-11. Catalog-pin acknowledgments are now Ed25519-signed. This post
states exactly what that signature proves, and what it does not.*

Tool-identity drift is the MCP supply-chain attack that survives install
time. A server behaves, gets approved, then changes a tool description or
schema after trust is granted. The industry term is rug pull, and static
analysis at install cannot catch it because the malicious behavior did
not exist at install. The defense is per-run re-hashing against
acknowledged pins, which this stack has done since the Pin line, with
fingerprint-moved tools hard-blocked until re-acknowledged.

Signatures close the next hole: someone with filesystem write access
re-pinning the pins file by hand. The acknowledgment now carries a
detached signature over the exact file bytes. A forged file fails
verification and the drift machinery rebuilds loudly, every tool
re-notifying, nothing silenced.

Here is the ceiling, stated plainly because most vendors would not. The
signing key is trust-on-first-use, generated beside the pins. A
filesystem attacker can regenerate the keypair and re-sign. What the
signature proves is ack-path authorship: the file passed through the
acknowledgment flow, not around it. Operator-bound keys, where the
signature proves *who* acknowledged, are future work with a named owner.

A signature with a stated ceiling beats an unsigned file with an implied
promise. The promise was never in the code. Now the ceiling is in the
docs, which is where a buyer can price it.
