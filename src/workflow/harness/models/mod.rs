//! The kernel-side models — this round: the deterministic reference rules
//! model (a declared, serializable rule table whose canonical
//! serialization hashes to the config digest) and the decide adapter (the
//! shipped pure decide line consumed, never rebuilt). No learned models,
//! no inference runtime — the learned-model/Laya gate is untouched.

pub(crate) mod decide_adapter;
pub(crate) mod rules;
