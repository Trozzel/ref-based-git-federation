use async_nats::Client;

use crate::git::proposal::{
    derive_proposal_state_from_git, maybe_auto_accept_proposal, ref_load_proposals,
};
use crate::git::utils::{
    fetch_named_ref, fetch_remotes, get_commit_hash, get_remotes, is_ancestor,
};
use crate::nats::proposal::publish_proposal_record_status;
use crate::proposal::persistence::{
    apply_persisted_terminal_proposal_state, clear_persisted_terminal_proposal_state,
    persist_terminal_proposal_record,
};
use crate::proposal::proposal_cache::ProposalCache;
use crate::proposal::state::ProposalState;
use crate::proposal::{Proposal, ProposalKind, ref_load_my_proposals};

const RECONCILIATION_DELAY: u64 = 30;

pub(crate) fn import_proposal(
    repo_path: &str,
    proposal: &Proposal,
    cache: ProposalCache,
) -> std::io::Result<()> {
    use ProposalState::*;
    if apply_persisted_terminal_proposal_state(repo_path, proposal, cache.clone())?.is_some() {
        return Ok(());
    }

    // No need to "import" to "refs/fed" if in terminal state or already Ready
    if matches!(
        cache
            .read()
            .unwrap()
            .get(proposal)
            .map(|record| record.state.clone()),
        Some(Accepted) | Some(Resolved) | Some(Rejected) | Some(Conflicted(_)) | Some(Ready)
    ) {
        return Ok(());
    }
    let source_ref = proposal.source_ref();
    let target_ref = proposal.proposal_ref();
    // Fetch the proposal ref from the remote to local "refs/fed" namespace
    if let Err(err) = fetch_named_ref(repo_path, &proposal.source_site, &source_ref, &target_ref) {
        cache.set_proposal_state(proposal, ProposalState::Error(err.to_string()));
        return Err(err);
    }

    // Decide whether we need to populate the cached commit hash without holding a read
    // lock across the later write in `set_proposal_commit_hash`.
    let needs_commit_hash = cache
        .read()
        .unwrap()
        .get(proposal)
        .map(|prop| prop.commit_hash.is_empty())
        .unwrap_or(false);

    if needs_commit_hash {
        let commit_hash = get_commit_hash(repo_path, &proposal.proposal_ref())?;
        cache.set_proposal_commit_hash(proposal, commit_hash);
    }
    let needs_base_commit_hash = cache
        .read()
        .unwrap()
        .get(proposal)
        .map(|prop| prop.base_commit_hash.is_empty())
        .unwrap_or(false);
    if needs_base_commit_hash {
        let base_commit_hash = get_commit_hash(repo_path, "main")?;
        cache.set_proposal_base_commit_hash(proposal, base_commit_hash);
    }

    let state = match derive_proposal_state_from_git(repo_path, &proposal.proposal_ref()) {
        Ok(state) => state,
        Err(err) => {
            cache.set_proposal_state(proposal, ProposalState::Error(err.to_string()));
            return Err(err);
        }
    };
    cache.set_proposal_state(proposal, state.clone());
    if matches!(state, ProposalState::Ready)
        && let Err(err) = maybe_auto_accept_proposal(repo_path, proposal, cache.clone())
    {
        cache.set_proposal_state(proposal, ProposalState::Error(err.to_string()));
        return Err(err);
    }
    Ok(())
}

pub(crate) fn import_proposals(
    repo_path: &str,
    proposals: &[Proposal],
    cache: ProposalCache,
) -> anyhow::Result<()> {
    for proposal in proposals {
        if let Err(err) = import_proposal(repo_path, proposal, cache.clone()) {
            tracing::error!(
                "failed to import proposal {} from {} to {}: {}",
                proposal.proposal_id,
                proposal.source_site,
                proposal.target_site,
                err
            );
        }
    }
    Ok(())
}

pub(crate) fn fetch_and_import_proposals(
    repo_path: &str,
    remotes: &[impl AsRef<str>],
    cache: ProposalCache,
    my_site: &str,
) -> anyhow::Result<()> {
    use ProposalState::*;
    fetch_remotes(repo_path, remotes)?; // fetch all  all remotes to "refs/remotes"
    let proposals = ref_load_my_proposals(repo_path, ProposalKind::Remote, my_site)?;

    // Any proposal in "refs/remotes" that isn't in cache or in `Queued` state -> set to `Backlog`
    for proposal in &proposals {
        if apply_persisted_terminal_proposal_state(repo_path, proposal, cache.clone())?.is_some() {
            continue;
        }

        let prop_record_op = cache.read().unwrap().get(proposal).cloned();

        match prop_record_op.map(|record| record.state) {
            None | Some(Queued) => cache.set_proposal_state(proposal, Backlog),
            Some(Rejected) | Some(Accepted) | Some(Resolved) => continue,
            _ => (),
        }
    }
    import_proposals(repo_path, &proposals, cache.clone())?;
    Ok(())
}

pub(crate) fn reconcile_once(
    repo_path: &str,
    cache: ProposalCache,
    my_site: &str,
) -> anyhow::Result<()> {
    let remotes = get_remotes(repo_path)?;
    fetch_and_import_proposals(repo_path, &remotes, cache.clone(), my_site)
}

pub async fn git_reconciliation_loop(repo_path: String, cache: ProposalCache, my_site: String) {
    let interval = std::time::Duration::from_secs(RECONCILIATION_DELAY);
    loop {
        if let Err(err) = reconcile_once(&repo_path, cache.clone(), &my_site) {
            tracing::error!("Reconciliation failed; Error: {err}");
        }
        tokio::time::sleep(interval).await;
    }
}

pub async fn cleanup_rejected_proposal_refs(
    repo_path: String,
    cache: ProposalCache,
    my_site: String,
    client: Client,
) {
    let interval = std::time::Duration::from_secs(RECONCILIATION_DELAY / 2);
    loop {
        if let Err(err) =
            set_missing_refs_to_rejected(&repo_path, cache.clone(), &my_site, Some(&client)).await
        {
            tracing::error!("Cleanup of missing refs failed; Error: {err}");
        }
        tokio::time::sleep(interval).await;
    }
}

/// Set state to `Rejected` if a target-site proposal ref doesn't exist yet but the state is
/// `Ready` or `Conflicted`.
pub(crate) async fn set_missing_refs_to_rejected(
    repo_path: &str,
    cache: ProposalCache,
    my_site: &str,
    client: Option<&Client>,
) -> anyhow::Result<()> {
    use ProposalState::*;
    let ready_or_conflicted_statuses = cache
        .db_load_statuses(None)
        .into_iter()
        .filter(|(proposal, record)| {
            proposal.target_site == my_site && matches!(record.state, Ready | Conflicted(_))
        })
        .collect::<Vec<_>>();
    let proposals_from_local_refs = ref_load_proposals(repo_path, ProposalKind::Local)?;
    for (proposal, record) in ready_or_conflicted_statuses {
        // If Ready | Conflicted Proposal exists in "refs/fed/proposals/", do nothing
        if proposals_from_local_refs.contains(&proposal) {
            continue;
        }

        // If the proposal is an ancestor of main, set to Accepted
        if !record.commit_hash.is_empty() {
            let main_hash = get_commit_hash(repo_path, "main")?;
            if is_ancestor(repo_path, &record.commit_hash, &main_hash)? {
                cache.set_proposal_base_commit_hash(&proposal, main_hash);
                cache.set_proposal_state(&proposal, Accepted);
                if let Err(err) = clear_persisted_terminal_proposal_state(repo_path, &proposal) {
                    tracing::error!(
                        "failed to clear persisted terminal state for accepted proposal {} from {}: {}",
                        proposal.proposal_id,
                        proposal.source_site,
                        err
                    );
                }
                let published_record = cache.read().unwrap().get(&proposal).cloned();
                if let Some(client) = client
                    && let Some(record) = published_record
                    && let Err(err) =
                        publish_proposal_record_status(client, &proposal, &record).await
                {
                    tracing::error!(
                        "failed to publish accepted state for proposal {} from {} during cleanup: {}",
                        proposal.proposal_id,
                        proposal.source_site,
                        err
                    );
                }
                continue;
            }
        }

        // Otherwise, set to Rejected since the ref is missing and it's not already Accepted
        if let Ok(main_hash) = get_commit_hash(repo_path, "main") {
            cache.set_proposal_base_commit_hash(&proposal, main_hash);
        }
        cache.set_proposal_state(&proposal, Rejected);
        if let Some(record) = cache.read().unwrap().get(&proposal).cloned()
            && let Err(err) = persist_terminal_proposal_record(repo_path, &proposal, record)
        {
            tracing::error!(
                "failed to persist rejected state for proposal {} from {}: {}",
                proposal.proposal_id,
                proposal.source_site,
                err
            );
        }
        let published_record = cache.read().unwrap().get(&proposal).cloned();
        if let Some(client) = client
            && let Some(record) = published_record
            && let Err(err) = publish_proposal_record_status(client, &proposal, &record).await
        {
            tracing::error!(
                "failed to publish rejected state for proposal {} from {} during cleanup: {}",
                proposal.proposal_id,
                proposal.source_site,
                err
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::proposal::persistence::persist_terminal_proposal_state;

    #[test]
    fn import_proposal_skips_reimport_for_persisted_resolved_state() {
        let temp_dir = tempdir().unwrap();
        let proposal = Proposal {
            target_site: "site-1".to_string(),
            source_site: "site-2".to_string(),
            proposal_id: "00001".to_string(),
        };
        let cache = ProposalCache::new();

        persist_terminal_proposal_state(temp_dir.path(), &proposal, ProposalState::Resolved)
            .unwrap();

        import_proposal(temp_dir.path().to_str().unwrap(), &proposal, cache.clone()).unwrap();

        assert_eq!(
            cache
                .read()
                .unwrap()
                .get(&proposal)
                .map(|record| record.state.clone()),
            Some(ProposalState::Resolved)
        );
    }

    #[test]
    fn missing_ref_cleanup_ignores_source_site_statuses() {
        let temp_dir = tempdir().unwrap();
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(temp_dir.path())
            .arg("init")
            .output()
            .unwrap();
        assert!(init.status.success());

        let proposal = Proposal {
            target_site: "site-2".to_string(),
            source_site: "site-1".to_string(),
            proposal_id: "00001".to_string(),
        };
        let cache = ProposalCache::new();
        cache.set_proposal_state(&proposal, ProposalState::Ready);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(set_missing_refs_to_rejected(
                temp_dir.path().to_str().unwrap(),
                cache.clone(),
                "site-1",
                None,
            ))
            .unwrap();

        assert_eq!(
            cache
                .read()
                .unwrap()
                .get(&proposal)
                .map(|record| record.state.clone()),
            Some(ProposalState::Ready)
        );
    }
}
