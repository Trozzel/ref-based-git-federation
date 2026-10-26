pub mod persistence;
pub mod proposal_cache;
pub mod state;

use std::collections::HashSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use state::ProposalState;

use crate::git::proposal::ref_load_proposals;

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Hash, Clone)]
pub struct Proposal {
    pub target_site: String,
    pub source_site: String,
    pub proposal_id: String,
}

impl Proposal {
    pub fn source_ref(&self) -> String {
        format!(
            "refs/heads/proposals/{}/{}",
            self.target_site, self.proposal_id
        )
    }

    pub fn proposal_ref(&self) -> String {
        format!(
            "refs/fed/proposals/{}/{}/{}",
            self.target_site, self.source_site, self.proposal_id
        )
    }

    pub fn accepted_ref(&self) -> String {
        format!(
            "refs/fed/accepted/{}/{}/{}",
            self.target_site, self.source_site, self.proposal_id
        )
    }
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ProposalRecord {
    pub state: ProposalState,
    pub commit_hash: String,
    #[serde(default)]
    pub base_commit_hash: String,
}

pub enum ProposalKind {
    All,
    Local,
    Remote,
}

impl ProposalKind {
    pub fn to_git_ref(&self) -> &'static str {
        match self {
            ProposalKind::All => "refs",
            ProposalKind::Local => "refs/fed/proposals",
            ProposalKind::Remote => "refs/remotes",
        }
    }

    pub fn to_regex(&self) -> regex::Regex {
        match self {
            ProposalKind::All => regex::Regex::new(
                r"^(?:refs/remotes/(?P<source>[^/]+)/proposals/(?P<target>[^/]+)/(?P<proposal_id>[^/]+)|refs/fed/proposals/(?P<fed_target>[^/]+)/(?P<fed_source>[^/]+)/(?P<fed_proposal_id>[^/]+))$",
            )
            .unwrap(),
            ProposalKind::Local => regex::Regex::new(
                r"^refs/fed/proposals/(?P<target>[^/]+)/(?P<source>[^/]+)/(?P<proposal_id>[^/]+)$",
            )
            .unwrap(),
            ProposalKind::Remote => regex::Regex::new(
                r"^refs/remotes/(?P<source>[^/]+)/proposals/(?P<target>[^/]+)/(?P<proposal_id>[^/]+)$",
            )
            .unwrap(),
        }
    }
}

/// Load proposals from git refs and filter to only those targeting `my_site`
pub(crate) fn ref_load_my_proposals(
    repo_path: &str,
    kind: ProposalKind,
    my_site: &str,
) -> anyhow::Result<Vec<Proposal>> {
    let mut proposals = ref_load_proposals(repo_path, kind)?;
    proposals.retain(|p| p.target_site == my_site);
    Ok(proposals)
}

/// Load proposal branches created by `my_site`.
pub(crate) fn ref_load_source_proposals(
    repo_path: &str,
    my_site: &str,
) -> anyhow::Result<Vec<Proposal>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("for-each-ref")
        .arg("--format=%(refname)")
        .arg("refs/heads/proposals")
        .output()?;

    if !output.status.success() {
        anyhow::bail!("git for-each-ref failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    map_source_refs_to_proposals(stdout.lines(), my_site)
}

pub(crate) fn map_source_refs_to_proposals<'a>(
    refs: impl IntoIterator<Item = &'a str>,
    my_site: &str,
) -> Result<Vec<Proposal>> {
    let re = regex::Regex::new(r"^refs/heads/proposals/(?P<target>[^/]+)/(?P<proposal_id>[^/]+)$")
        .unwrap();
    let mut proposals = Vec::new();
    let mut seen = HashSet::new();

    for git_ref in refs {
        let Some(caps) = re.captures(git_ref) else {
            continue;
        };
        let proposal = Proposal {
            target_site: caps.name("target").unwrap().as_str().to_owned(),
            source_site: my_site.to_owned(),
            proposal_id: caps.name("proposal_id").unwrap().as_str().to_owned(),
        };
        if seen.insert(proposal.clone()) {
            proposals.push(proposal);
        }
    }

    Ok(proposals)
}

pub(crate) fn map_ref_to_proposals(
    refs: &[impl AsRef<str>],
    kind: ProposalKind,
) -> Result<Vec<Proposal>> {
    let re = kind.to_regex();

    let mut proposals = Vec::new();
    let mut seen = HashSet::new();

    for git_ref in refs {
        let git_ref = git_ref.as_ref();
        let Some(caps) = re.captures(git_ref) else {
            //error!("ignoring non-proposal ref: {git_ref}");
            continue;
        };
        let source_site = caps
            .name("source")
            .or_else(|| caps.name("fed_source"))
            .unwrap()
            .as_str();
        let target_site = caps
            .name("target")
            .or_else(|| caps.name("fed_target"))
            .unwrap()
            .as_str();
        let proposal_id = caps
            .name("proposal_id")
            .or_else(|| caps.name("fed_proposal_id"))
            .unwrap()
            .as_str();
        let proposal = Proposal {
            proposal_id: proposal_id.to_owned(),
            source_site: source_site.to_owned(),
            target_site: target_site.to_owned(),
        };
        if seen.insert(proposal.clone()) {
            proposals.push(proposal);
        }
    }

    Ok(proposals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_kind_parses_local_and_remote_refs() {
        let refs = vec![
            "refs/remotes/site-2/proposals/site-1/00001",
            "refs/fed/proposals/site-1/site-3/00002",
        ];

        let proposals = map_ref_to_proposals(&refs, ProposalKind::All).unwrap();

        assert_eq!(
            proposals,
            vec![
                Proposal {
                    target_site: "site-1".to_string(),
                    source_site: "site-2".to_string(),
                    proposal_id: "00001".to_string(),
                },
                Proposal {
                    target_site: "site-1".to_string(),
                    source_site: "site-3".to_string(),
                    proposal_id: "00002".to_string(),
                },
            ]
        );
    }

    #[test]
    fn all_kind_deduplicates_local_and_remote_versions_of_same_proposal() {
        let refs = vec![
            "refs/remotes/site-2/proposals/site-1/00001",
            "refs/fed/proposals/site-1/site-2/00001",
        ];

        let proposals = map_ref_to_proposals(&refs, ProposalKind::All).unwrap();

        assert_eq!(
            proposals,
            vec![Proposal {
                target_site: "site-1".to_string(),
                source_site: "site-2".to_string(),
                proposal_id: "00001".to_string(),
            }]
        );
    }

    #[test]
    fn source_refs_parse_with_local_site_as_source() {
        let refs = vec![
            "refs/heads/proposals/site-2/00001",
            "refs/heads/main",
            "refs/heads/proposals/site-3/00002",
        ];

        let proposals = map_source_refs_to_proposals(refs, "site-1").unwrap();

        assert_eq!(
            proposals,
            vec![
                Proposal {
                    target_site: "site-2".to_string(),
                    source_site: "site-1".to_string(),
                    proposal_id: "00001".to_string(),
                },
                Proposal {
                    target_site: "site-3".to_string(),
                    source_site: "site-1".to_string(),
                    proposal_id: "00002".to_string(),
                },
            ]
        );
    }
}
