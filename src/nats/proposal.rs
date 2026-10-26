use anyhow::Result;
use async_nats::Client;

use crate::proposal::state::ProposalState;
use crate::proposal::{Proposal, ProposalRecord};

pub const PROPOSAL_CREATED_SUBJECT: &str = "fed.proposal.created";
pub const PROPOSAL_STATUS_UPDATED_SUBJECT: &str = "fed.proposal.status";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProposalCreated {
    pub event_type: String,
    pub proposal_id: String,
    pub source_site: String,
    pub target_site: String,
    pub target_repo: String,
    pub source_ref: String,
    pub repo_full_name: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProposalStatusUpdated {
    pub event_type: String,
    pub proposal_id: String,
    pub source_site: String,
    pub target_site: String,
    pub state: ProposalState,
    pub commit_hash: String,
    #[serde(default)]
    pub base_commit_hash: String,
}

pub fn proposal_status_from_record(
    proposal: &Proposal,
    record: &ProposalRecord,
) -> ProposalStatusUpdated {
    ProposalStatusUpdated {
        event_type: "proposal.status".to_string(),
        proposal_id: proposal.proposal_id.clone(),
        source_site: proposal.source_site.clone(),
        target_site: proposal.target_site.clone(),
        state: record.state.clone(),
        commit_hash: record.commit_hash.clone(),
        base_commit_hash: record.base_commit_hash.clone(),
    }
}

/// Publish a proposal created event to NATS
pub async fn publish_proposal_created(client: &Client, msg: &ProposalCreated) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    client
        .publish(PROPOSAL_CREATED_SUBJECT.to_string(), payload.into())
        .await?;
    Ok(())
}

pub async fn publish_proposal_status_updated(
    client: &Client,
    msg: &ProposalStatusUpdated,
) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    client
        .publish(PROPOSAL_STATUS_UPDATED_SUBJECT.to_string(), payload.into())
        .await?;
    Ok(())
}

pub async fn publish_proposal_record_status(
    client: &Client,
    proposal: &Proposal,
    record: &ProposalRecord,
) -> Result<()> {
    let status = proposal_status_from_record(proposal, record);
    publish_proposal_status_updated(client, &status).await
}
