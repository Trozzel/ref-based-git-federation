use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub enum ProposalState {
    Queued,                       // INITIAL: Received notification via NATS but not yet fetched
    Backlog,                      // INITIAL | INTERMEDIATE: Fetched into "refs/remotes"
    Ready,                        // INTERMEDIATE: Fetched into "refs/fed" and ready for review
    Accepted,                     // FINAL
    Resolved,                     // FINAL: manually set by user
    Rejected,                     // FINAL
    Conflicted(BTreeSet<String>), // INTERMEDIATE: holds list of conflicted files
    Error(String),
}
