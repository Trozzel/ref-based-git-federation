use crate::git::utils::is_ancestor;
use crate::proposal::Proposal;
use crate::proposal::proposal_cache::ProposalCache;
use crate::proposal::state::ProposalState;

impl ProposalCache {
    /// If the hash of a `Proposal` commit is an ancestor of main, then update to `Accepted`
    /// Return the accepted proposal if any
    pub fn update_state_via_git(
        &self,
        repo_path: &str,
        main_hash: &str,
    ) -> anyhow::Result<Option<Proposal>> {
        let ready_statuses = self.db_load_statuses(Some(ProposalState::Ready));

        for (proposal, record) in &ready_statuses {
            if is_ancestor(repo_path, &record.commit_hash, main_hash)? {
                self.set_proposal_state(proposal, ProposalState::Accepted);
                return Ok(Some(proposal.clone()));
            }
        }
        Ok(None)
    }
}
