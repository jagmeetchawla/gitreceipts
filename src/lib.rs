//! gitreceipts — reconcile a coding-agent session log against the git
//! history it claims to explain.
//!
//! The pipeline: [`ingest`] (tolerant JSONL) → [`causal`] (parentUuid
//! order) → [`extract`] (claims with receipts) → [`reconcile`] (the
//! interval equation against the repo's reflog spine) → [`report`]
//! (console ledger).

pub mod causal;
pub mod discover;
pub mod extract;
pub mod gitio;
pub mod ingest;
pub mod reconcile;
pub mod report;
pub mod schema;
