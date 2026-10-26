use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(crate) struct ProposalResponse {
    ok: bool,
    message: String,
    proposal_ref: Option<String>,
}

impl ProposalResponse {
    pub fn new(ok: bool, message: String, proposal_ref: Option<String>) -> Self {
        ProposalResponse {
            ok,
            message,
            proposal_ref,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ProposalActionRequest {
    pub source_site: String,
    pub proposal_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SiteFollowRequest {
    pub site: String,
}

#[derive(Debug, Deserialize)]
pub struct SetAllowFastForwardMergeRequest {
    pub site: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictResolutionChoice {
    Ours,
    Theirs,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProposalConflictResolution {
    pub file: String,
    pub choice: ConflictResolutionChoice,
}

#[derive(Debug, Deserialize)]
pub struct ProposalConflictResolutionRequest {
    pub source_site: String,
    pub proposal_id: String,
    pub resolutions: Vec<ProposalConflictResolution>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProposalActionResponse {
    ok: bool,
    message: String,
    proposal_ref: Option<String>,
}

impl ProposalActionResponse {
    pub fn new(ok: bool, message: String, proposal_ref: Option<String>) -> Self {
        ProposalActionResponse {
            ok,
            message,
            proposal_ref,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct FetchRemoteResponse {
    ok: bool,
    remotes: Vec<String>,
    message: String,
}

impl FetchRemoteResponse {
    pub fn new(ok: bool, remotes: Vec<String>, message: String) -> Self {
        FetchRemoteResponse {
            ok,
            remotes,
            message,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FederationSitePreference {
    pub site: String,
    pub followed: bool,
    pub auto_accept_fast_forward_merge: bool,
}

#[derive(Debug, Serialize)]
pub struct FederationSitePreferencesResponse {
    pub follow_all: bool,
    pub sites: Vec<FederationSitePreference>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProposalConflictFileResponse {
    pub file: String,
    pub ours: Option<String>,
    pub theirs: Option<String>,
    pub base: Option<String>,
}

impl ProposalConflictFileResponse {
    pub fn new(
        file: String,
        ours: Option<String>,
        theirs: Option<String>,
        base: Option<String>,
    ) -> Self {
        ProposalConflictFileResponse {
            file,
            ours,
            theirs,
            base,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SiteInfoResponse {
    pub site_name: String,
}
