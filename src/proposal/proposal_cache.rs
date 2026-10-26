use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, RwLock};

use crate::proposal::{Proposal, ProposalRecord, ProposalState};

#[derive(Clone)]
pub struct ProposalCache {
    cache: Arc<RwLock<HashMap<Proposal, ProposalRecord>>>,
}

impl Default for ProposalCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for ProposalCache {
    type Target = Arc<RwLock<HashMap<Proposal, ProposalRecord>>>;

    fn deref(&self) -> &Self::Target {
        &self.cache
    }
}

impl DerefMut for ProposalCache {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cache
    }
}

impl ProposalCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn set_proposal_state(&self, proposal: &Proposal, state: ProposalState) {
        let mut cache = self.write().unwrap();

        match cache.get_mut(proposal) {
            Some(record) => {
                record.state = state;
            }
            None => {
                cache.insert(
                    proposal.clone(),
                    ProposalRecord {
                        state,
                        commit_hash: String::new(),
                        base_commit_hash: String::new(),
                    },
                );
            }
        }
    }

    pub fn set_proposal_commit_hash(&self, proposal: &Proposal, commit_hash: String) {
        let mut proposal_record =
            self.read()
                .unwrap()
                .get(proposal)
                .cloned()
                .unwrap_or_else(|| ProposalRecord {
                    state: ProposalState::Queued,
                    commit_hash: commit_hash.clone(),
                    base_commit_hash: String::new(),
                });
        proposal_record.commit_hash = commit_hash;
        self.write()
            .unwrap()
            .insert(proposal.clone(), proposal_record);
    }

    pub fn set_proposal_base_commit_hash(&self, proposal: &Proposal, base_commit_hash: String) {
        let mut proposal_record =
            self.read()
                .unwrap()
                .get(proposal)
                .cloned()
                .unwrap_or_else(|| ProposalRecord {
                    state: ProposalState::Queued,
                    commit_hash: String::new(),
                    base_commit_hash: base_commit_hash.clone(),
                });
        proposal_record.base_commit_hash = base_commit_hash;
        self.write()
            .unwrap()
            .insert(proposal.clone(), proposal_record);
    }

    pub fn db_load_statuses(
        &self,
        state: Option<ProposalState>,
    ) -> Vec<(Proposal, ProposalRecord)> {
        if let Some(filter_state) = state {
            self.read()
                .unwrap()
                .iter()
                .filter(|(_p, record)| record.state == filter_state)
                .map(|(p, record)| (p.clone(), record.clone()))
                .collect()
        } else {
            self.read()
                .unwrap()
                .iter()
                .map(|(p, record)| (p.clone(), record.clone()))
                .collect()
        }
    }

    pub fn db_load_my_statuses(
        &self,
        site_name: &str,
        state: Option<ProposalState>,
    ) -> Vec<(Proposal, ProposalRecord)> {
        self.db_load_statuses(state)
            .iter()
            .filter(|(p, _r)| p.target_site == site_name)
            .cloned()
            .collect()
    }

    pub fn db_load_source_statuses(
        &self,
        site_name: &str,
        state: Option<ProposalState>,
    ) -> Vec<(Proposal, ProposalRecord)> {
        self.db_load_statuses(state)
            .iter()
            .filter(|(p, _r)| p.source_site == site_name)
            .cloned()
            .collect()
    }

    pub fn set_proposal_record(&self, proposal: &Proposal, record: ProposalRecord) {
        self.write().unwrap().insert(proposal.clone(), record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn set_proposal_commit_hash_updates_existing_record() {
        let cache = ProposalCache::new();
        let proposal = Proposal {
            target_site: "site-1".to_string(),
            source_site: "site-2".to_string(),
            proposal_id: "00001".to_string(),
        };

        cache.set_proposal_record(
            &proposal,
            ProposalRecord {
                state: ProposalState::Ready,
                commit_hash: String::new(),
                base_commit_hash: String::new(),
            },
        );

        cache.set_proposal_commit_hash(&proposal, "abc123".to_string());

        let stored = cache.read().unwrap().get(&proposal).cloned().unwrap();
        assert_eq!(stored.commit_hash, "abc123");
        assert_eq!(stored.state, ProposalState::Ready);
    }

    #[test]
    fn set_proposal_base_commit_hash_updates_existing_record() {
        let cache = ProposalCache::new();
        let proposal = Proposal {
            target_site: "site-1".to_string(),
            source_site: "site-2".to_string(),
            proposal_id: "00001".to_string(),
        };

        cache.set_proposal_state(&proposal, ProposalState::Ready);
        cache.set_proposal_base_commit_hash(&proposal, "base123".to_string());

        let stored = cache.read().unwrap().get(&proposal).cloned().unwrap();
        assert_eq!(stored.base_commit_hash, "base123");
        assert_eq!(stored.state, ProposalState::Ready);
    }

    #[test]
    fn db_load_source_statuses_filters_by_source_site() {
        let cache = ProposalCache::new();
        let authored = Proposal {
            target_site: "site-2".to_string(),
            source_site: "site-1".to_string(),
            proposal_id: "00001".to_string(),
        };
        let inbound = Proposal {
            target_site: "site-1".to_string(),
            source_site: "site-2".to_string(),
            proposal_id: "00002".to_string(),
        };

        cache.set_proposal_state(&authored, ProposalState::Accepted);
        cache.set_proposal_state(&inbound, ProposalState::Ready);

        let statuses = cache.db_load_source_statuses("site-1", None);

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].0, authored);
        assert_eq!(statuses[0].1.state, ProposalState::Accepted);
    }
}
