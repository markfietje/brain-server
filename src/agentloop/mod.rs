//! The agent-loop driver: the kernel-side async runtime driving the SDK
//! harness — provider/stream seam, session store wiring, run loop,
//! compaction, subagents — without editing the dependency-free SDK.
//! Kernel law inherited, never re-invented: audit-per-write inside the
//! caller's tx; fail-closed (`let _ =` on writes forbidden); the provider
//! is a PLUGGABLE SEAM (no provider code in-tree; the loopback fixture
//! carries the tests); no routes — a loop-serving route ships its wire
//! tables in its own release. `#![allow(dead_code)]` is the truthful
//! connector precedent: substrates land before the run loop consumes
//! them, and every item is test-covered so removals re-flag.

#![allow(dead_code)]

pub(crate) mod compaction;
pub(crate) mod exec;
pub(crate) mod provider;
pub(crate) mod run_loop;
pub(crate) mod subagents;
