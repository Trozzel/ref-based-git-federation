use anyhow::Result;
use async_nats::Client;
use serde::{Deserialize, Serialize};

pub const REPO_UPDATED_SUBJECT: &str = "fed.repo.updated";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoUpdated {
    pub event_type: String,
    pub source_site: String,
    pub repo_name: String,
    pub repo_full_name: String,
    pub git_ref: String,
    pub before: String,
    pub after: String,
}

/// Publish that I updated my "main" branch
pub async fn publish_repo_updated(client: &Client, msg: &RepoUpdated) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    client
        .publish(REPO_UPDATED_SUBJECT.to_string(), payload.into())
        .await?;
    Ok(())
}
