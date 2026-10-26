use std::sync::Arc;

use async_nats::Client;

use crate::proposal::proposal_cache::ProposalCache;

#[derive(Clone)]
pub struct AppState {
    pub secret: Arc<String>,
    pub site_name: Arc<String>,
    pub repo_path: Arc<String>,
    pub proposals: ProposalCache,
    pub nats: Client,
}

impl AppState {
    pub fn new(secret: String, site_name: String, repo_path: String, nats: Client) -> Self {
        Self {
            secret: Arc::new(secret),
            site_name: Arc::new(site_name),
            repo_path: Arc::new(repo_path),
            proposals: ProposalCache::new(),
            nats,
        }
    }
}
