use crate::federation::allows_ffm_site;
use crate::git::utils::{
    fast_forward_proposal_into_main, get_commit_hash, get_conflicted_files, is_ancestor, move_ref,
    push_main,
};
use crate::proposal::persistence::clear_persisted_terminal_proposal_state;
use crate::proposal::proposal_cache::ProposalCache;
use crate::proposal::state::ProposalState;
use crate::proposal::{Proposal, ProposalKind, map_ref_to_proposals};

impl Proposal {
    pub fn commit_hash_from_git(&self, repo_path: &str) -> Option<String> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("rev-parse")
            .arg(self.proposal_ref())
            .output()
            .ok()?;

        if !output.status.success() {
            tracing::error!(
                "git rev-parse failed for proposal {}: {}",
                self.proposal_ref(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Load proposals from git refs. If `kind` is `Remote`, only load from "refs/remotes". If `Local`,
/// only load from "refs/fed". If `All`, load from both and combine.
pub(crate) fn ref_load_proposals(
    repo_path: &str,
    kind: ProposalKind,
) -> anyhow::Result<Vec<Proposal>> {
    let ref_root = kind.to_git_ref();
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("for-each-ref")
        .arg("--format=%(refname)")
        .arg(ref_root)
        .output()?;

    if !output.status.success() {
        anyhow::bail!("git for-each-ref failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let refs: Vec<&str> = stdout
        .lines()
        .filter(|refname| !refname.starts_with("refs/remotes/origin/"))
        .collect();

    map_ref_to_proposals(&refs, kind)
}

pub(crate) fn derive_proposal_state_from_git(
    repo_path: &str,
    proposal_ref: &str,
) -> std::io::Result<ProposalState> {
    let conflicted_files = get_conflicted_files(repo_path, proposal_ref)?;
    Ok(if conflicted_files.is_empty() {
        ProposalState::Ready
    } else {
        ProposalState::Conflicted(conflicted_files)
    })
}

/// If the hash of a `Proposal` commit is an ancestor of main, then update to `Accepted`
/// Return the accepted proposal if any
pub(crate) fn update_state_via_git(
    repo_path: &str,
    main_hash: &str,
    cache: ProposalCache,
) -> anyhow::Result<Option<Proposal>> {
    let ready_statuses = cache.db_load_statuses(Some(ProposalState::Ready));

    for (proposal, record) in &ready_statuses {
        if is_ancestor(repo_path, &record.commit_hash, main_hash)? {
            cache.set_proposal_base_commit_hash(proposal, main_hash.to_string());
            cache.set_proposal_state(proposal, ProposalState::Accepted);
            if let Err(err) = clear_persisted_terminal_proposal_state(repo_path, proposal) {
                tracing::error!(
                    "failed to clear persisted terminal state for accepted proposal {} from {}: {}",
                    proposal.proposal_id,
                    proposal.source_site,
                    err
                );
            }
            if let Err(err) = delete_ref(repo_path, &proposal.proposal_ref()) {
                tracing::error!(
                    "failed to delete local proposal ref after git-detected accept for proposal {} from {}: {}",
                    proposal.proposal_id,
                    proposal.source_site,
                    err
                );
            }
            return Ok(Some(proposal.clone()));
        }
    }
    Ok(None)
}

pub(crate) fn maybe_auto_accept_proposal(
    repo_path: &str,
    proposal: &Proposal,
    cache: ProposalCache,
) -> std::io::Result<bool> {
    if !allows_ffm_site(repo_path, &proposal.source_site)? {
        return Ok(false);
    }

    let proposal_ref = proposal.proposal_ref();
    let proposal_hash = get_commit_hash(repo_path, &proposal_ref)?;
    let main_hash = get_commit_hash(repo_path, "main")?;
    if !is_ancestor(repo_path, &main_hash, &proposal_hash)? {
        return Ok(false);
    }

    fast_forward_proposal_into_main(repo_path, &proposal_ref)?;
    push_main(repo_path)?;

    cache.set_proposal_commit_hash(proposal, proposal_hash);
    cache.set_proposal_base_commit_hash(proposal, main_hash);
    cache.set_proposal_state(proposal, ProposalState::Accepted);
    if let Err(err) = clear_persisted_terminal_proposal_state(repo_path, proposal) {
        tracing::error!(
            "failed to clear persisted terminal state for auto-accepted proposal {} from {}: {}",
            proposal.proposal_id,
            proposal.source_site,
            err
        );
    }
    if let Err(err) = move_ref(repo_path, &proposal_ref, &proposal.accepted_ref()) {
        tracing::error!(
            "failed to move local proposal ref after auto-accept for proposal {} from {}: {}",
            proposal.proposal_id,
            proposal.source_site,
            err
        );
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn git(repo_path: &str, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(args)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn update_state_via_git_marks_accepted_and_deletes_local_proposal_ref() {
        let temp_dir = tempdir().unwrap();
        let repo_path = temp_dir.path().to_str().unwrap();

        git(repo_path, &["init", "-b", "main"]);
        git(repo_path, &["config", "user.name", "Test User"]);
        git(repo_path, &["config", "user.email", "test@example.com"]);

        let file_path = temp_dir.path().join("stats.txt");
        fs::write(&file_path, "base\n").unwrap();
        git(repo_path, &["add", "stats.txt"]);
        git(repo_path, &["commit", "-m", "base"]);

        git(repo_path, &["checkout", "-b", "proposal"]);
        fs::write(&file_path, "base\nproposal change\n").unwrap();
        git(repo_path, &["commit", "-am", "proposal change"]);
        let proposal_hash = get_commit_hash(repo_path, "HEAD").unwrap();

        let proposal_ref = "refs/fed/proposals/site-1/site-2/00001";
        git(repo_path, &["update-ref", proposal_ref, &proposal_hash]);

        git(repo_path, &["checkout", "main"]);
        git(
            repo_path,
            &["merge", "--no-ff", "proposal", "-m", "merge proposal"],
        );
        let main_hash = get_commit_hash(repo_path, "main").unwrap();

        let proposal = Proposal {
            target_site: "site-1".to_string(),
            source_site: "site-2".to_string(),
            proposal_id: "00001".to_string(),
        };
        let cache = ProposalCache::new();
        cache.set_proposal_record(
            &proposal,
            crate::proposal::ProposalRecord {
                state: ProposalState::Ready,
                commit_hash: proposal_hash,
                base_commit_hash: String::new(),
            },
        );

        let accepted = update_state_via_git(repo_path, &main_hash, cache.clone()).unwrap();

        assert_eq!(accepted, Some(proposal.clone()));
        let record = cache.read().unwrap().get(&proposal).cloned().unwrap();
        assert_eq!(record.state, ProposalState::Accepted);
        assert_eq!(record.base_commit_hash, main_hash);
        assert!(get_commit_hash(repo_path, proposal_ref).is_err());
    }
}
