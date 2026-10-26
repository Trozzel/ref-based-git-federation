use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tracing::{error, info};

use crate::axum::app_state::AppState;
use crate::git::proposal::update_state_via_git;
use crate::nats::proposal::{
    ProposalCreated, publish_proposal_created, publish_proposal_record_status,
};
use crate::nats::repo_update::{RepoUpdated, publish_repo_updated};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Deserialize)]
struct PushHookPayload {
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    before: String,
    #[serde(default)]
    after: String,
    repository: Repository,
}

#[derive(Debug, Deserialize)]
pub struct Repository {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    full_name: String,
    #[serde(default)]
    pub clone_url: String,
}

#[derive(Deserialize)]
enum RefKind {
    Main,
    Proposal {
        target_site: String,
        proposal_id: String,
    },
    Other,
}

impl RefKind {
    fn from_git_ref(git_ref: &str) -> Self {
        info!("processing git ref: {git_ref}");
        if git_ref == "refs/heads/main" {
            return RefKind::Main;
        }
        let prefix = "refs/heads/proposals/";
        if let Some(rest) = git_ref.strip_prefix(prefix) {
            let mut parts = rest.split('/');
            // "refs/heads/proposals/site-1/00001"
            if let (Some(target_site), Some(proposal_id), None) =
                (parts.next(), parts.next(), parts.next())
            {
                return RefKind::Proposal {
                    target_site: target_site.to_owned(),
                    proposal_id: proposal_id.to_owned(),
                };
            }
        }
        RefKind::Other
    }

    // Move `self` to ensure it's only called once
    async fn handle_webhook_payload(self, state: AppState, payload: PushHookPayload) -> Response {
        match self {
            // THIS SITE PUSHED TO MAIN - update state and broadcast to all sites
            RefKind::Main => {
                let msg = RepoUpdated {
                    event_type: "repo.updated".to_string(),
                    source_site: state.site_name.to_string(),
                    repo_name: payload.repository.name.clone(),
                    repo_full_name: payload.repository.full_name.clone(),
                    git_ref: payload.r#ref.clone(),
                    before: payload.before.clone(),
                    after: payload.after.clone(),
                };

                match update_state_via_git(
                    &state.repo_path,
                    &payload.after,
                    state.proposals.clone(),
                ) {
                    Ok(None) => (),
                    Ok(Some(proposal)) => {
                        info!(
                            "proposal {} from {} has been merged into main",
                            proposal.proposal_id, proposal.source_site
                        );
                        let record = state.proposals.read().unwrap().get(&proposal).cloned();
                        if let Some(record) = record {
                            if let Err(err) =
                                publish_proposal_record_status(&state.nats, &proposal, &record)
                                    .await
                            {
                                error!(
                                    "failed to publish accepted proposal status for proposal {} from {}: {err}",
                                    proposal.proposal_id, proposal.source_site
                                );
                            }
                        }
                    }
                    Err(err) => {
                        error!("failed to update state via git: {err}");
                        return (StatusCode::INTERNAL_SERVER_ERROR, "git state update failed")
                            .into_response();
                    }
                }

                // Broadcast push to other sites
                if let Err(err) = publish_repo_updated(&state.nats, &msg).await {
                    error!("failed to publish NATS message: {err}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "nats publish failed")
                        .into_response();
                }

                info!(
                    "repo={} ref={} before={} after={}",
                    payload.repository.full_name, payload.r#ref, payload.before, payload.after
                );

                (StatusCode::OK, "ok").into_response()
            }
            // THIS SITE PUSHED A PROPOSAL - broadcast to all sites
            RefKind::Proposal {
                target_site,
                proposal_id,
            } => {
                let msg = ProposalCreated {
                    event_type: "proposal.created".to_string(),
                    proposal_id,
                    source_site: state.site_name.to_string(),
                    target_site,
                    target_repo: payload.repository.full_name.clone(),
                    source_ref: payload.r#ref.clone(),
                    repo_full_name: payload.repository.full_name.clone(),
                    before: payload.before.clone(),
                    after: payload.after.clone(),
                };

                if let Err(err) = publish_proposal_created(&state.nats, &msg).await {
                    error!("failed to publish NATS message: {err}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "nats publish failed")
                        .into_response();
                }

                info!(
                    "proposal={} repo={} ref={} before={} after={}",
                    msg.proposal_id,
                    payload.repository.full_name,
                    payload.r#ref,
                    payload.before,
                    payload.after
                );

                (StatusCode::OK, "ok").into_response()
            }
            RefKind::Other => (StatusCode::OK, "ignored non-main push").into_response(),
        }
    }
}

/// From Gitea docs: https://docs.gitea.com/usage/webhooks
/// X-GitHub-Delivery: f6266f16-1bf3-46a5-9ea4-602e06ead473
/// X-GitHub-Event: push
/// X-Gogs-Delivery: f6266f16-1bf3-46a5-9ea4-602e06ead473
/// X-Gogs-Event: push
/// X-Gitea-Delivery: f6266f16-1bf3-46a5-9ea4-602e06ead473
/// X-Gitea-Event: push
pub async fn gitea_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let event = headers
        .get("X-Gitea-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let signature = headers
        .get("X-Gitea-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_signature(&state.secret, &body, signature) {
        return (StatusCode::UNAUTHORIZED, "bad signature").into_response();
    }

    if event != "push" {
        return (StatusCode::OK, "ignored non-push event").into_response();
    }

    let payload: PushHookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid json").into_response(),
    };

    let ref_kind = RefKind::from_git_ref(&payload.r#ref);

    // Handle the push event based on the ref kind (main, proposal, or other)
    ref_kind.handle_webhook_payload(state, payload).await
}

fn verify_signature(secret: &str, body: &[u8], header_sig: &str) -> bool {
    if header_sig.is_empty() {
        return false;
    }

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());

    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), header_sig.as_bytes()).into()
}
