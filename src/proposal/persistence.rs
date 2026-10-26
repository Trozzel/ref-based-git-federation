use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

use serde::{Deserialize, Serialize};

use crate::federation::FEDERATION_DIR;
use crate::proposal::proposal_cache::ProposalCache;
use crate::proposal::state::ProposalState;
use crate::proposal::{Proposal, ProposalRecord};

const TERMINAL_PROPOSAL_STATES_FILE: &str = "terminal-proposal-states.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTerminalProposalState {
    proposal: Proposal,
    state: ProposalState,
    #[serde(default)]
    commit_hash: String,
    #[serde(default)]
    base_commit_hash: String,
}

fn is_persisted_terminal_state(state: &ProposalState) -> bool {
    matches!(state, ProposalState::Rejected | ProposalState::Resolved)
}

pub fn terminal_proposal_states_path(repo_path: impl AsRef<Path>) -> PathBuf {
    repo_path
        .as_ref()
        .join(FEDERATION_DIR)
        .join(TERMINAL_PROPOSAL_STATES_FILE)
}

fn ensure_terminal_state_dir(repo_path: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(repo_path.as_ref().join(FEDERATION_DIR))
}

fn write_persisted_terminal_proposal_records(
    repo_path: impl AsRef<Path>,
    records_by_proposal: &HashMap<Proposal, ProposalRecord>,
) -> io::Result<()> {
    ensure_terminal_state_dir(&repo_path)?;
    let path = terminal_proposal_states_path(repo_path);

    let mut records = records_by_proposal
        .iter()
        .filter(|(_proposal, record)| is_persisted_terminal_state(&record.state))
        .map(|(proposal, record)| PersistedTerminalProposalState {
            proposal: proposal.clone(),
            state: record.state.clone(),
            commit_hash: record.commit_hash.clone(),
            base_commit_hash: record.base_commit_hash.clone(),
        })
        .collect::<Vec<_>>();

    records.sort_by(|a, b| {
        a.proposal
            .target_site
            .cmp(&b.proposal.target_site)
            .then(a.proposal.source_site.cmp(&b.proposal.source_site))
            .then(a.proposal.proposal_id.cmp(&b.proposal.proposal_id))
    });

    let json = serde_json::to_string_pretty(&records).map_err(|err| {
        io::Error::other(format!(
            "failed to serialize terminal proposal states: {err}"
        ))
    })?;
    fs::write(path, json)
}

pub fn load_persisted_terminal_proposal_records(
    repo_path: impl AsRef<Path>,
) -> io::Result<HashMap<Proposal, ProposalRecord>> {
    let path = terminal_proposal_states_path(repo_path);
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let contents = fs::read_to_string(path)?;
    if contents.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let records: Vec<PersistedTerminalProposalState> =
        serde_json::from_str(&contents).map_err(|err| {
            io::Error::other(format!("failed to parse terminal proposal states: {err}"))
        })?;

    Ok(records
        .into_iter()
        .filter(|record| is_persisted_terminal_state(&record.state))
        .map(|record| {
            (
                record.proposal,
                ProposalRecord {
                    state: record.state,
                    commit_hash: record.commit_hash,
                    base_commit_hash: record.base_commit_hash,
                },
            )
        })
        .collect())
}

pub fn load_persisted_terminal_proposal_states(
    repo_path: impl AsRef<Path>,
) -> io::Result<HashMap<Proposal, ProposalState>> {
    Ok(load_persisted_terminal_proposal_records(repo_path)?
        .into_iter()
        .map(|(proposal, record)| (proposal, record.state))
        .collect())
}

pub fn load_persisted_terminal_proposal_state(
    repo_path: impl AsRef<Path>,
    proposal: &Proposal,
) -> io::Result<Option<ProposalState>> {
    Ok(load_persisted_terminal_proposal_states(repo_path)?.remove(proposal))
}

pub fn persist_terminal_proposal_state(
    repo_path: impl AsRef<Path>,
    proposal: &Proposal,
    state: ProposalState,
) -> io::Result<()> {
    if !is_persisted_terminal_state(&state) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("state {state:?} is not a persisted terminal proposal state"),
        ));
    }

    let mut records = load_persisted_terminal_proposal_records(&repo_path)?;
    let mut record = records.remove(proposal).unwrap_or_else(|| ProposalRecord {
        state: state.clone(),
        commit_hash: String::new(),
        base_commit_hash: String::new(),
    });
    record.state = state;
    records.insert(proposal.clone(), record);
    write_persisted_terminal_proposal_records(repo_path, &records)
}

pub fn persist_terminal_proposal_record(
    repo_path: impl AsRef<Path>,
    proposal: &Proposal,
    record: ProposalRecord,
) -> io::Result<()> {
    if !is_persisted_terminal_state(&record.state) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "state {:?} is not a persisted terminal proposal state",
                record.state
            ),
        ));
    }

    let mut records = load_persisted_terminal_proposal_records(&repo_path)?;
    records.insert(proposal.clone(), record);
    write_persisted_terminal_proposal_records(repo_path, &records)
}

pub fn clear_persisted_terminal_proposal_state(
    repo_path: impl AsRef<Path>,
    proposal: &Proposal,
) -> io::Result<bool> {
    let mut records = load_persisted_terminal_proposal_records(&repo_path)?;
    let removed = records.remove(proposal).is_some();
    if removed {
        write_persisted_terminal_proposal_records(repo_path, &records)?;
    }
    Ok(removed)
}

pub fn hydrate_persisted_terminal_proposal_states(
    repo_path: impl AsRef<Path>,
    cache: ProposalCache,
    site_name: &str,
) -> io::Result<()> {
    for (proposal, record) in load_persisted_terminal_proposal_records(repo_path)? {
        if proposal.target_site != site_name {
            continue;
        }
        cache.set_proposal_record(&proposal, record);
    }
    Ok(())
}

pub fn apply_persisted_terminal_proposal_state(
    repo_path: impl AsRef<Path>,
    proposal: &Proposal,
    cache: ProposalCache,
) -> io::Result<Option<ProposalState>> {
    let record = load_persisted_terminal_proposal_records(repo_path)?.remove(proposal);
    let state = record.as_ref().map(|record| record.state.clone());
    if let Some(record) = record {
        cache.set_proposal_record(proposal, record);
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn sample_proposal() -> Proposal {
        Proposal {
            target_site: "site-1".to_string(),
            source_site: "site-2".to_string(),
            proposal_id: "00001".to_string(),
        }
    }

    #[test]
    fn persist_load_and_clear_terminal_states() {
        let temp_dir = tempdir().unwrap();
        let proposal = sample_proposal();

        persist_terminal_proposal_state(temp_dir.path(), &proposal, ProposalState::Rejected)
            .unwrap();

        let loaded = load_persisted_terminal_proposal_state(temp_dir.path(), &proposal).unwrap();
        assert_eq!(loaded, Some(ProposalState::Rejected));

        let removed = clear_persisted_terminal_proposal_state(temp_dir.path(), &proposal).unwrap();
        assert!(removed);
        assert_eq!(
            load_persisted_terminal_proposal_state(temp_dir.path(), &proposal).unwrap(),
            None
        );
    }

    #[test]
    fn hydrate_and_apply_persisted_terminal_states_to_cache() {
        let temp_dir = tempdir().unwrap();
        let proposal = sample_proposal();
        let cache = ProposalCache::new();

        persist_terminal_proposal_state(temp_dir.path(), &proposal, ProposalState::Resolved)
            .unwrap();

        let applied =
            apply_persisted_terminal_proposal_state(temp_dir.path(), &proposal, cache.clone())
                .unwrap();
        assert_eq!(applied, Some(ProposalState::Resolved));
        assert_eq!(
            cache
                .read()
                .unwrap()
                .get(&proposal)
                .map(|record| record.state.clone()),
            Some(ProposalState::Resolved)
        );

        let other_cache = ProposalCache::new();
        hydrate_persisted_terminal_proposal_states(temp_dir.path(), other_cache.clone(), "site-1")
            .unwrap();
        assert_eq!(
            other_cache
                .read()
                .unwrap()
                .get(&proposal)
                .map(|record| record.state.clone()),
            Some(ProposalState::Resolved)
        );
    }

    #[test]
    fn hydrate_ignores_terminal_states_for_other_target_sites() {
        let temp_dir = tempdir().unwrap();
        let proposal = Proposal {
            target_site: "site-2".to_string(),
            source_site: "site-1".to_string(),
            proposal_id: "00001".to_string(),
        };
        let cache = ProposalCache::new();

        persist_terminal_proposal_state(temp_dir.path(), &proposal, ProposalState::Rejected)
            .unwrap();
        hydrate_persisted_terminal_proposal_states(temp_dir.path(), cache.clone(), "site-1")
            .unwrap();

        assert!(cache.read().unwrap().get(&proposal).is_none());
    }
}
