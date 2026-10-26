use async_nats::Client;
use tokio_stream::StreamExt;
use tracing::{debug, error, info};

use crate::federation::follows_site;
use crate::git::proposal::{derive_proposal_state_from_git, maybe_auto_accept_proposal};
use crate::git::utils::{
    add_remote, fetch_named_ref, fetch_remote, get_commit_hash, remote_exists,
    remove_remote_if_exists,
};
use crate::nats::advertisement::{SITE_ADVERTISEMENT_SUBJECT, SiteAdvertisement};
use crate::nats::farewell::{SITE_FAREWELL_SUBJECT, SiteFarewell};
use crate::nats::proposal::{
    PROPOSAL_CREATED_SUBJECT, PROPOSAL_STATUS_UPDATED_SUBJECT, ProposalCreated,
    ProposalStatusUpdated, publish_proposal_status_updated,
};
use crate::nats::repo_update::{REPO_UPDATED_SUBJECT, RepoUpdated};
use crate::proposal::persistence::apply_persisted_terminal_proposal_state;
use crate::proposal::proposal_cache::ProposalCache;
use crate::proposal::state::ProposalState;
use crate::proposal::{Proposal, ProposalRecord};

fn should_ignore_proposal_created(state: Option<&ProposalState>) -> bool {
    matches!(
        state,
        Some(
            ProposalState::Accepted
                | ProposalState::Resolved
                | ProposalState::Rejected
                | ProposalState::Ready
        ) | Some(ProposalState::Conflicted(_))
    )
}

/// Another site updated their main branch. Fetch updates. This is best effort, if fetch fails, we
/// will try again on next update.
pub async fn site_advertisement_handler(
    site_name: String,
    local_repo_path: String,
    client: Client,
) {
    let mut sub = match client
        .subscribe(SITE_ADVERTISEMENT_SUBJECT.to_string())
        .await
    {
        Ok(s) => s,
        Err(err) => {
            error!("failed to subscribe to NATS: {err}");
            return;
        }
    };

    info!(
        "nats listener started: site={} subject={}",
        site_name, SITE_ADVERTISEMENT_SUBJECT
    );

    while let Some(message) = sub.next().await {
        let event: SiteAdvertisement = match serde_json::from_slice(&message.payload) {
            Ok(v) => v,
            Err(err) => {
                error!("failed to parse NATS message: {err}");
                continue;
            }
        };

        if event.site == site_name {
            debug!(
                "ignoring self message from site={} repo_url={} commit_hash={}",
                event.site, event.repo_url, event.commit_hash
            );
            continue;
        }

        debug!(
            "received site advertisement from site={} repo_url={} commit_hash={}",
            event.site, event.repo_url, event.commit_hash
        );

        let site_is_followed = match follows_site(&local_repo_path, &event.site) {
            Ok(is_followed) => is_followed,
            Err(err) => {
                error!(
                    "failed to read following file for repo {}: {}",
                    local_repo_path, err
                );
                continue;
            }
        };

        if !site_is_followed {
            debug!(
                "ignoring advertisement from unfollowed site '{}' for repo {}",
                event.site, local_repo_path
            );
            continue;
        }

        // Check if the remote already exists.
        match remote_exists(&local_repo_path, &event.site) {
            Ok(true) => {
                debug!("remote '{}' already exists, skipping add.", event.site);
                continue;
            }
            Ok(false) => {}
            Err(err) => {
                error!("failed to check for remote existence: {}", err);
                continue;
            }
        }

        // add site/branch as remote if it doesn't exist
        if let Err(err) = add_remote(&local_repo_path, &event.site, &event.repo_url) {
            error!(
                "git add remote failed for: remote={}, url={}, error={}",
                event.site, event.repo_url, err
            );
        }

        // now that we have it registered as a remote let's go fetch it
        if let Err(err) = fetch_remote(&local_repo_path, &event.site) {
            error!("git fetch failed for remote={}: {err}", event.site);
        }
    }
}

/// Another site updated their main branch. Fetch updates. This is best effort, if fetch fails, we
/// will try again on next update.
pub async fn repo_update_handler(site_name: String, repo_path: String, client: Client) {
    let mut sub = match client.subscribe(REPO_UPDATED_SUBJECT.to_string()).await {
        Ok(s) => s,
        Err(err) => {
            error!("failed to subscribe to NATS: {err}");
            return;
        }
    };

    info!(
        "nats listener started: site={} subject={}",
        site_name, REPO_UPDATED_SUBJECT
    );

    while let Some(message) = sub.next().await {
        let event: RepoUpdated = match serde_json::from_slice(&message.payload) {
            Ok(v) => v,
            Err(err) => {
                error!("failed to parse NATS message: {err}");
                continue;
            }
        };

        if event.source_site == site_name {
            debug!(
                "ignoring self message from {} repo={} ref={}",
                event.source_site, event.repo_full_name, event.git_ref
            );
            continue;
        }

        info!(
            "received update from {} repo={} ref={} before={} after={}",
            event.source_site, event.repo_full_name, event.git_ref, event.before, event.after
        );

        let site_is_followed = match follows_site(&repo_path, &event.source_site) {
            Ok(is_followed) => is_followed,
            Err(err) => {
                error!(
                    "failed to read following file for repo {}: {}",
                    repo_path, err
                );
                continue;
            }
        };

        if !site_is_followed {
            debug!(
                "ignoring repo update from unfollowed site '{}' for repo {}",
                event.source_site, repo_path
            );
            continue;
        }

        match remote_exists(&repo_path, &event.source_site) {
            Ok(true) => {}
            Ok(false) => {
                debug!(
                    "remote '{}' is not configured yet for repo {}; skipping update fetch until site advertisement is processed",
                    event.source_site, repo_path
                );
                continue;
            }
            Err(err) => {
                error!("failed to check for remote existence: {}", err);
                continue;
            }
        }

        if let Err(err) = fetch_remote(&repo_path, &event.source_site) {
            error!("git fetch failed for remote={}: {err}", event.source_site);
        }
    }
}

/// Site receives proposal from another site. Set state to Queued. Fetch proposal. If fetch fails,
/// set state to Error. If fetch succeeds, set state to Ready.
pub async fn repo_proposal_handler(
    site_name: String,
    repo_path: String,
    client: Client,
    cache: ProposalCache,
) {
    let mut sub = client
        .subscribe(PROPOSAL_CREATED_SUBJECT.to_string())
        .await
        .expect("failed to subscribe to proposal subject");

    info!(
        "proposal handler started: site={} subject={}",
        site_name, PROPOSAL_CREATED_SUBJECT
    );

    while let Some(message) = sub.next().await {
        let proposal_created: ProposalCreated = match serde_json::from_slice(&message.payload) {
            Ok(v) => v,
            Err(err) => {
                error!("failed to parse proposal message: {err}");
                continue;
            }
        };

        let proposal = Proposal {
            proposal_id: proposal_created.proposal_id.clone(),
            source_site: proposal_created.source_site.clone(),
            target_site: proposal_created.target_site.clone(),
        };

        // Ignore if not for me
        if proposal_created.target_site != site_name {
            debug!(
                "ignoring proposal {} from {} to {} (not my target)",
                proposal_created.proposal_id,
                proposal_created.source_site,
                proposal_created.target_site
            );
            continue;
        }

        // Ignore if from myself
        if proposal_created.source_site == site_name {
            debug!(
                "ignoring self proposal {} from {}",
                proposal_created.proposal_id, proposal_created.source_site
            );
            continue;
        }

        match apply_persisted_terminal_proposal_state(&repo_path, &proposal, cache.clone()) {
            Ok(Some(state)) => {
                debug!(
                    "ignoring proposal {} from {} because persisted local state is {:?}",
                    proposal_created.proposal_id, proposal_created.source_site, state
                );
                continue;
            }
            Ok(None) => {}
            Err(err) => {
                error!(
                    "failed to load persisted terminal state for proposal {} from {}: {}",
                    proposal_created.proposal_id, proposal_created.source_site, err
                );
                continue;
            }
        }

        let existing_record = cache.read().unwrap().get(&proposal).cloned();
        if should_ignore_proposal_created(existing_record.as_ref().map(|record| &record.state)) {
            debug!(
                "ignoring proposal {} from {} because local state is already terminal or imported",
                proposal_created.proposal_id, proposal_created.source_site
            );
            continue;
        }

        let commit_hash = proposal_created.after;
        let base_commit_hash = get_commit_hash(&repo_path, "main").unwrap_or_else(|err| {
            error!(
                "failed to capture base commit for proposal {} from {}: {}",
                proposal_created.proposal_id, proposal_created.source_site, err
            );
            String::new()
        });
        match existing_record {
            None => {
                let proposal_record = ProposalRecord {
                    state: ProposalState::Queued,
                    commit_hash: commit_hash.clone(),
                    base_commit_hash: base_commit_hash.clone(),
                };
                cache.set_proposal_record(&proposal, proposal_record);
            }
            Some(record) if record.commit_hash.is_empty() => {
                cache.set_proposal_commit_hash(&proposal, commit_hash.clone());
                if record.base_commit_hash.is_empty() && !base_commit_hash.is_empty() {
                    cache.set_proposal_base_commit_hash(&proposal, base_commit_hash.clone());
                }
            }
            Some(record) if record.base_commit_hash.is_empty() && !base_commit_hash.is_empty() => {
                cache.set_proposal_base_commit_hash(&proposal, base_commit_hash.clone());
            }
            Some(_) => {}
        }

        let target_ref = format!(
            "refs/fed/proposals/{}/{}/{}",
            proposal_created.target_site,
            proposal_created.source_site,
            proposal_created.proposal_id
        );

        if let Err(err) = fetch_named_ref(
            &repo_path,
            &proposal_created.source_site,
            &proposal_created.source_ref,
            &target_ref,
        ) {
            error!(
                "failed to fetch proposal {} from {}: {}",
                proposal_created.proposal_id, proposal_created.source_site, err
            );
            continue;
        }
        let state = match derive_proposal_state_from_git(&repo_path, &target_ref) {
            Ok(state) => state,
            Err(err) => {
                error!(
                    "failed to derive proposal state for proposal {} from {}: {}",
                    proposal_created.proposal_id, proposal_created.source_site, err
                );
                continue;
            }
        };

        if let ProposalState::Conflicted(conflicted_files) = &state {
            info!(
                "proposal {} from {} has conflicts: {:?}",
                proposal_created.proposal_id, proposal_created.source_site, conflicted_files
            );
        }

        cache.set_proposal_state(&proposal, state);

        match maybe_auto_accept_proposal(&repo_path, &proposal, cache.clone()) {
            Ok(true) => {
                let record = cache.read().unwrap().get(&proposal).cloned();
                let status = ProposalStatusUpdated {
                    event_type: "proposal.status".to_string(),
                    proposal_id: proposal.proposal_id.clone(),
                    source_site: proposal.source_site.clone(),
                    target_site: proposal.target_site.clone(),
                    state: record
                        .as_ref()
                        .map(|record| record.state.clone())
                        .unwrap_or(ProposalState::Accepted),
                    commit_hash: record
                        .as_ref()
                        .map(|record| record.commit_hash.clone())
                        .unwrap_or_else(String::new),
                    base_commit_hash: record
                        .map(|record| record.base_commit_hash)
                        .unwrap_or_else(String::new),
                };
                if let Err(err) = publish_proposal_status_updated(&client, &status).await {
                    error!(
                        "failed to publish status for auto-accepted proposal {} from {}: {}",
                        proposal_created.proposal_id, proposal_created.source_site, err
                    );
                }
                info!(
                    "proposal auto-accepted via ff-only: id={} source={} target={}",
                    proposal_created.proposal_id,
                    proposal_created.source_site,
                    proposal_created.target_site
                );
                continue;
            }
            Ok(false) => {}
            Err(err) => {
                error!(
                    "failed to auto-accept proposal {} from {}: {}",
                    proposal_created.proposal_id, proposal_created.source_site, err
                );
                cache.set_proposal_state(&proposal, ProposalState::Error(err.to_string()));
                continue;
            }
        }

        let record = cache.read().unwrap().get(&proposal).cloned();
        let status = ProposalStatusUpdated {
            event_type: "proposal.status".to_string(),
            proposal_id: proposal.proposal_id.clone(),
            source_site: proposal.source_site.clone(),
            target_site: proposal.target_site.clone(),
            state: record
                .as_ref()
                .map(|record| record.state.clone())
                .unwrap_or(ProposalState::Ready),
            commit_hash: record
                .as_ref()
                .map(|record| record.commit_hash.clone())
                .unwrap_or_else(String::new),
            base_commit_hash: record
                .map(|record| record.base_commit_hash)
                .unwrap_or_else(String::new),
        };
        if let Err(err) = publish_proposal_status_updated(&client, &status).await {
            error!(
                "failed to publish status for proposal {} from {}: {}",
                proposal_created.proposal_id, proposal_created.source_site, err
            );
        }

        info!(
            "proposal ready: id={} source={} target={} ref={}",
            proposal_created.proposal_id,
            proposal_created.source_site,
            proposal_created.target_site,
            target_ref
        );
    }
}

pub async fn repo_proposal_status_handler(site_name: String, client: Client, cache: ProposalCache) {
    let mut sub = match client
        .subscribe(PROPOSAL_STATUS_UPDATED_SUBJECT.to_string())
        .await
    {
        Ok(s) => s,
        Err(err) => {
            error!("failed to subscribe to proposal status subject: {err}");
            return;
        }
    };

    info!(
        "proposal status handler started: site={} subject={}",
        site_name, PROPOSAL_STATUS_UPDATED_SUBJECT
    );

    while let Some(message) = sub.next().await {
        let status: ProposalStatusUpdated = match serde_json::from_slice(&message.payload) {
            Ok(v) => v,
            Err(err) => {
                error!("failed to parse proposal status message: {err}");
                continue;
            }
        };

        if status.source_site != site_name {
            debug!(
                "ignoring proposal status {} from {} to {} (not my source)",
                status.proposal_id, status.source_site, status.target_site
            );
            continue;
        }

        let proposal = Proposal {
            proposal_id: status.proposal_id,
            source_site: status.source_site,
            target_site: status.target_site,
        };
        cache.set_proposal_record(
            &proposal,
            ProposalRecord {
                state: status.state,
                commit_hash: status.commit_hash,
                base_commit_hash: status.base_commit_hash,
            },
        );
    }
}

/// A site has announced it is leaving the federation. Remove its remote.
pub async fn site_farewell_handler(site_name: String, local_repo_path: String, client: Client) {
    let mut sub = match client.subscribe(SITE_FAREWELL_SUBJECT.to_string()).await {
        Ok(s) => s,
        Err(err) => {
            error!("failed to subscribe to NATS farewell subject: {err}");
            return;
        }
    };

    info!(
        "nats farewell listener started: site={} subject={}",
        site_name, SITE_FAREWELL_SUBJECT
    );

    while let Some(message) = sub.next().await {
        let event: SiteFarewell = match serde_json::from_slice(&message.payload) {
            Ok(v) => v,
            Err(err) => {
                error!("failed to parse NATS farewell message: {err}");
                continue;
            }
        };

        if event.site == site_name {
            debug!("ignoring self farewell message from site={}", event.site);
            continue;
        }

        info!(
            "received site farewell from site={} repo_url={}",
            event.site, event.repo_url
        );

        info!("removing remote for site '{}'", event.site);
        match remove_remote_if_exists(&local_repo_path, &event.site) {
            Ok(true) => {}
            Ok(false) => {
                debug!(
                    "remote for site '{}' does not exist, nothing to do.",
                    event.site
                );
            }
            Err(err) => {
                error!("failed to remove remote for site '{}': {}", event.site, err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::should_ignore_proposal_created;
    use crate::proposal::Proposal;
    use crate::proposal::persistence::{
        apply_persisted_terminal_proposal_state, persist_terminal_proposal_state,
    };
    use crate::proposal::proposal_cache::ProposalCache;
    use crate::proposal::state::ProposalState;

    #[test]
    fn ignores_terminal_and_imported_proposal_states() {
        assert!(should_ignore_proposal_created(Some(
            &ProposalState::Accepted
        )));
        assert!(should_ignore_proposal_created(Some(
            &ProposalState::Resolved
        )));
        assert!(should_ignore_proposal_created(Some(
            &ProposalState::Rejected
        )));
        assert!(should_ignore_proposal_created(Some(&ProposalState::Ready)));
        assert!(should_ignore_proposal_created(Some(
            &ProposalState::Conflicted(Default::default(),)
        )));
    }

    #[test]
    fn does_not_ignore_pending_states() {
        assert!(!should_ignore_proposal_created(None));
        assert!(!should_ignore_proposal_created(Some(
            &ProposalState::Queued
        )));
        assert!(!should_ignore_proposal_created(Some(
            &ProposalState::Backlog
        )));
        assert!(!should_ignore_proposal_created(Some(
            &ProposalState::Error("fetch failed".to_string(),)
        )));
    }

    #[test]
    fn persisted_resolved_state_is_ignored_by_proposal_created() {
        let temp_dir = tempdir().unwrap();
        let proposal = Proposal {
            target_site: "site-1".to_string(),
            source_site: "site-2".to_string(),
            proposal_id: "00001".to_string(),
        };
        let cache = ProposalCache::new();

        persist_terminal_proposal_state(temp_dir.path(), &proposal, ProposalState::Resolved)
            .unwrap();
        let applied =
            apply_persisted_terminal_proposal_state(temp_dir.path(), &proposal, cache.clone())
                .unwrap();

        assert_eq!(applied, Some(ProposalState::Resolved));
        let cached_state = cache
            .read()
            .unwrap()
            .get(&proposal)
            .map(|record| record.state.clone());
        assert!(should_ignore_proposal_created(cached_state.as_ref()));
    }
}
