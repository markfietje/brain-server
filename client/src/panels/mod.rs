//! Panel modules. Recall + Health are fully wired to live endpoints here in
//! the scaffold; the other four flesh out as their v1.14/v1.15 APIs ship
//! (proposals → v1.14.0, DSAR/tombstones → v1.15.0, quarantine/audit-verify
//! already exist, recall-trace → v1.15.0).

pub mod audit;
pub mod health;
pub mod recall;
pub mod review;
pub mod security;
pub mod subjects;
