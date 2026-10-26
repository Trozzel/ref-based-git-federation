use std::collections::{BTreeSet, HashMap};

use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use tracing::info;

use super::app_state::AppState;
use super::models::{
    ConflictResolutionChoice, FederationSitePreference, FederationSitePreferencesResponse,
    FetchRemoteResponse, ProposalActionRequest, ProposalActionResponse,
    ProposalConflictFileResponse, ProposalConflictResolutionRequest, ProposalResponse,
    SetAllowFastForwardMergeRequest, SiteFollowRequest, SiteInfoResponse,
};
use crate::federation::{
    FOLLOW_ALL_SITES, add_following_site, load_allow_fast_forward_merge_sites,
    load_following_sites, set_allow_fast_forward_merge_site,
};
use crate::git::proposal::{derive_proposal_state_from_git, maybe_auto_accept_proposal};
use crate::git::reconciliation::{
    fetch_and_import_proposals, import_proposals, set_missing_refs_to_rejected,
};
use crate::git::utils::{
    delete_ref, fetch_named_ref, fetch_remotes, get_commit_hash, get_merge_base, get_remotes,
    merge_proposal_into_main, move_ref, push_main, read_file_at_ref, ref_diff_from_base,
    resolve_conflicted_proposal_ref,
};
use crate::nats::proposal::publish_proposal_record_status;
use crate::proposal::persistence::persist_terminal_proposal_record;
use crate::proposal::state::ProposalState;
use crate::proposal::{
    Proposal, ProposalKind, ProposalRecord, ref_load_my_proposals, ref_load_source_proposals,
};

// TODO: update this to use the `ProposalRecord` commit hash

fn get_conflicted_files_for_proposal(
    cache: &crate::proposal::proposal_cache::ProposalCache,
    proposal: &Proposal,
) -> Result<BTreeSet<String>, (StatusCode, String)> {
    match cache.read().unwrap().get(proposal) {
        Some(record) => match &record.state {
            ProposalState::Conflicted(files) => Ok(files.clone()),
            _ => Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "proposal is not conflicted for source_site='{}' proposal_id='{}'",
                    proposal.source_site, proposal.proposal_id
                ),
            )),
        },
        None => Err((
            StatusCode::NOT_FOUND,
            format!(
                "proposal not found in cache for source_site='{}' proposal_id='{}'",
                proposal.source_site, proposal.proposal_id
            ),
        )),
    }
}

fn list_federation_site_preferences(
    repo_path: &str,
) -> Result<FederationSitePreferencesResponse, (StatusCode, String)> {
    let followed_sites = load_following_sites(repo_path).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to load following sites: {err}"),
        )
    })?;
    let allow_fast_forward_merge_sites =
        load_allow_fast_forward_merge_sites(repo_path).map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load allow-fast-forward-merge sites: {err}"),
            )
        })?;
    let remotes = get_remotes(repo_path).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to list remotes: {err}"),
        )
    })?;

    let follow_all = followed_sites.contains(FOLLOW_ALL_SITES);
    let mut known_sites = remotes.into_iter().collect::<BTreeSet<_>>();
    known_sites.extend(
        followed_sites
            .iter()
            .filter(|site| site.as_str() != FOLLOW_ALL_SITES)
            .cloned(),
    );
    known_sites.extend(allow_fast_forward_merge_sites.iter().cloned());

    let sites = known_sites
        .into_iter()
        .map(|site| FederationSitePreference {
            followed: follow_all || followed_sites.contains(&site),
            auto_accept_fast_forward_merge: allow_fast_forward_merge_sites.contains(&site),
            site,
        })
        .collect();

    Ok(FederationSitePreferencesResponse { follow_all, sites })
}

async fn publish_current_proposal_status(state: &AppState, proposal: &Proposal) {
    let record = state.proposals.read().unwrap().get(proposal).cloned();
    let Some(record) = record else {
        return;
    };

    if let Err(err) = publish_proposal_record_status(&state.nats, proposal, &record).await {
        tracing::error!(
            "failed to publish proposal status for proposal {} from {} to {}: {}",
            proposal.proposal_id,
            proposal.source_site,
            proposal.target_site,
            err
        );
    }
}

pub async fn site_info_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(SiteInfoResponse {
            site_name: (*state.site_name).clone(),
        }),
    )
        .into_response()
}

pub async fn federation_site_preferences_endpoint(
    State(state): State<AppState>,
) -> impl IntoResponse {
    match list_federation_site_preferences(&state.repo_path) {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err((status, message)) => (status, message).into_response(),
    }
}

pub async fn add_following_site_endpoint(
    State(state): State<AppState>,
    Json(req): Json<SiteFollowRequest>,
) -> impl IntoResponse {
    if let Err(err) = add_following_site(&*state.repo_path, &req.site) {
        return (
            StatusCode::BAD_REQUEST,
            format!("failed to add followed site '{}': {err}", req.site.trim()),
        )
            .into_response();
    }

    match list_federation_site_preferences(&state.repo_path) {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err((status, message)) => (status, message).into_response(),
    }
}

pub async fn set_allow_fast_forward_merge_endpoint(
    State(state): State<AppState>,
    Json(req): Json<SetAllowFastForwardMergeRequest>,
) -> impl IntoResponse {
    if let Err(err) = set_allow_fast_forward_merge_site(&*state.repo_path, &req.site, req.enabled) {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "failed to update allow-fast-forward-merge for site '{}' : {err}",
                req.site.trim()
            ),
        )
            .into_response();
    }

    match list_federation_site_preferences(&state.repo_path) {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err((status, message)) => (status, message).into_response(),
    }
}

pub async fn get_proposal_diff_endpoint(
    State(state): State<AppState>,
    Path((source_site, proposal_id)): Path<(String, String)>,
) -> impl IntoResponse {
    // Create a `Proposal` from source_site and proposal_id
    let proposal = Proposal {
        target_site: (*state.site_name).clone(),
        source_site: source_site.clone(),
        proposal_id: proposal_id.clone(),
    };
    let (commit_hash, base_ref) = match state.proposals.read().unwrap().get(&proposal) {
        Some(record) => (
            if record.commit_hash.is_empty() {
                proposal.proposal_ref()
            } else {
                record.commit_hash.clone()
            },
            if record.base_commit_hash.is_empty() {
                "main".to_string()
            } else {
                record.base_commit_hash.clone()
            },
        ),
        None => {
            return (
                StatusCode::NOT_FOUND,
                format!(
                    "proposal not found in cache for source_site='{}' proposal_id='{}'",
                    source_site, proposal_id
                ),
            )
                .into_response();
        }
    };

    match ref_diff_from_base(&state.repo_path, &base_ref, &commit_hash) {
        Ok(diff) => (StatusCode::OK, diff).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to get proposal diff: {err}"),
        )
            .into_response(),
    }
}

pub async fn get_source_proposal_diff_endpoint(
    State(state): State<AppState>,
    Path((target_site, proposal_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let proposal = Proposal {
        target_site,
        source_site: (*state.site_name).clone(),
        proposal_id,
    };

    let cached_record = state.proposals.read().unwrap().get(&proposal).cloned();
    let source_proposals = match ref_load_source_proposals(&state.repo_path, &state.site_name) {
        Ok(proposals) => proposals,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to list source proposals: {err}"),
            )
                .into_response();
        }
    };
    if !source_proposals.contains(&proposal) && cached_record.is_none() {
        return (
            StatusCode::NOT_FOUND,
            format!(
                "source proposal not found for target_site='{}' proposal_id='{}'",
                proposal.target_site, proposal.proposal_id
            ),
        )
            .into_response();
    }

    let target_main_ref = cached_record
        .as_ref()
        .filter(|record| !record.base_commit_hash.is_empty())
        .map(|record| record.base_commit_hash.clone())
        .unwrap_or_else(|| format!("refs/remotes/{}/main", proposal.target_site));
    let proposal_ref = cached_record
        .as_ref()
        .filter(|record| !record.commit_hash.is_empty())
        .map(|record| record.commit_hash.clone())
        .unwrap_or_else(|| proposal.source_ref());
    match ref_diff_from_base(&state.repo_path, &target_main_ref, &proposal_ref) {
        Ok(diff) => (StatusCode::OK, diff).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to get source proposal diff: {err}"),
        )
            .into_response(),
    }
}

pub async fn get_proposal_conflict_file_endpoint(
    State(state): State<AppState>,
    Path((source_site, proposal_id, file_path)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let normalized_file_path = file_path.trim_start_matches('/').to_string();
    let proposal = Proposal {
        target_site: (*state.site_name).clone(),
        source_site: source_site.clone(),
        proposal_id: proposal_id.clone(),
    };

    let conflicted_files = match get_conflicted_files_for_proposal(&state.proposals, &proposal) {
        Ok(files) => files,
        Err((status, message)) => return (status, message).into_response(),
    };

    if !conflicted_files.contains(&normalized_file_path) {
        return (
            StatusCode::NOT_FOUND,
            format!(
                "file '{}' is not part of the conflicted file set for proposal '{}'",
                normalized_file_path, proposal_id
            ),
        )
            .into_response();
    }

    let proposal_ref = proposal.proposal_ref();
    let ours = match read_file_at_ref(&state.repo_path, "main", &normalized_file_path) {
        Ok(contents) => contents,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "failed to read main side for '{}': {err}",
                    normalized_file_path
                ),
            )
                .into_response();
        }
    };
    let theirs = match read_file_at_ref(&state.repo_path, &proposal_ref, &normalized_file_path) {
        Ok(contents) => contents,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "failed to read proposal side for '{}' from '{}': {err}",
                    normalized_file_path, proposal_ref
                ),
            )
                .into_response();
        }
    };
    let base = match get_merge_base(&state.repo_path, "main", &proposal_ref)
        .and_then(|base_ref| read_file_at_ref(&state.repo_path, &base_ref, &normalized_file_path))
    {
        Ok(contents) => contents,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "failed to read merge-base side for '{}' from '{}': {err}",
                    normalized_file_path, proposal_ref
                ),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        Json(ProposalConflictFileResponse::new(
            normalized_file_path,
            ours,
            theirs,
            base,
        )),
    )
        .into_response()
}

pub async fn resolve_proposal_conflicts_endpoint(
    State(state): State<AppState>,
    Json(req): Json<ProposalConflictResolutionRequest>,
) -> impl IntoResponse {
    let proposal = Proposal {
        target_site: (*state.site_name).clone(),
        source_site: req.source_site.clone(),
        proposal_id: req.proposal_id.clone(),
    };
    let conflicted_files = match get_conflicted_files_for_proposal(&state.proposals, &proposal) {
        Ok(files) => files,
        Err((status, message)) => return (status, message).into_response(),
    };

    let mut seen = BTreeSet::new();
    let mut choice_by_file = HashMap::<String, ConflictResolutionChoice>::new();
    for resolution in req.resolutions {
        let normalized_file = resolution.file.trim_start_matches('/').to_string();
        if !seen.insert(normalized_file.clone()) {
            return (
                StatusCode::BAD_REQUEST,
                format!(
                    "duplicate resolution provided for file '{}'",
                    normalized_file
                ),
            )
                .into_response();
        }
        if !conflicted_files.contains(&normalized_file) {
            return (
                StatusCode::BAD_REQUEST,
                format!(
                    "file '{}' is not part of the conflicted file set for proposal '{}'",
                    normalized_file, proposal.proposal_id
                ),
            )
                .into_response();
        }
        choice_by_file.insert(normalized_file, resolution.choice);
    }

    let missing_files = conflicted_files
        .iter()
        .filter(|file| !choice_by_file.contains_key(*file))
        .cloned()
        .collect::<Vec<_>>();
    if !missing_files.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "missing conflict resolution choices for files: {}",
                missing_files.join(", ")
            ),
        )
            .into_response();
    }

    let proposal_ref = proposal.proposal_ref();
    let mut ordered_resolutions = choice_by_file.into_iter().collect::<Vec<_>>();
    ordered_resolutions.sort_by(|(left, _), (right, _)| left.cmp(right));

    let new_commit = match resolve_conflicted_proposal_ref(
        &state.repo_path,
        &proposal_ref,
        &ordered_resolutions,
    ) {
        Ok(commit) => commit,
        Err(err) => {
            let e_string = format!("failed to apply conflict resolutions: {err}");
            state
                .proposals
                .set_proposal_state(&proposal, ProposalState::Error(e_string.clone()));
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ProposalActionResponse::new(
                    false,
                    e_string,
                    Some(proposal_ref),
                )),
            )
                .into_response();
        }
    };

    state
        .proposals
        .set_proposal_commit_hash(&proposal, new_commit);
    let next_state = match derive_proposal_state_from_git(&state.repo_path, &proposal_ref) {
        Ok(next_state) => next_state,
        Err(err) => {
            let e_string =
                format!("failed to derive proposal state after resolving conflicts: {err}");
            state
                .proposals
                .set_proposal_state(&proposal, ProposalState::Error(e_string.clone()));
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ProposalActionResponse::new(
                    false,
                    e_string,
                    Some(proposal_ref),
                )),
            )
                .into_response();
        }
    };
    state.proposals.set_proposal_state(&proposal, next_state);
    publish_current_proposal_status(&state, &proposal).await;

    (
        StatusCode::OK,
        Json(ProposalActionResponse::new(
            true,
            "proposal conflict selections applied".to_string(),
            Some(proposal_ref),
        )),
    )
        .into_response()
}

pub async fn import_proposal_endpoint(
    State(state): State<AppState>,
    Json(proposal): Json<Proposal>,
) -> impl IntoResponse {
    if proposal.target_site != *state.site_name {
        let e_string = format!(
            "target_site '{}' does not match this site '{}'",
            proposal.target_site, state.site_name
        );
        state
            .proposals
            .set_proposal_state(&proposal, ProposalState::Error(e_string.clone()));
        return (
            StatusCode::BAD_REQUEST,
            Json(ProposalResponse::new(false, e_string, None)),
        )
            .into_response();
    }

    if proposal.source_site == *state.site_name {
        let e_string = "cannot import proposal from self".to_string();
        state
            .proposals
            .set_proposal_state(&proposal, ProposalState::Error(e_string.clone()));
        return (
            StatusCode::BAD_REQUEST,
            Json(ProposalResponse::new(false, e_string, None)),
        )
            .into_response();
    }

    let target_ref = format!(
        "refs/fed/proposals/{}/{}/{}",
        proposal.target_site, proposal.source_site, proposal.proposal_id
    );

    match fetch_named_ref(
        &state.repo_path,
        &proposal.source_site,
        &proposal.source_ref(),
        &target_ref,
    ) {
        Ok(commit_hash) => {
            let base_commit_hash =
                get_commit_hash(&state.repo_path, "main").unwrap_or_else(|err| {
                    tracing::error!("failed to capture proposal base commit: {err}");
                    String::new()
                });
            let proposal_state = match derive_proposal_state_from_git(&state.repo_path, &target_ref)
            {
                Ok(state) => state,
                Err(err) => {
                    let e_string = format!("failed to derive proposal state: {err}");
                    state
                        .proposals
                        .set_proposal_state(&proposal, ProposalState::Error(e_string.clone()));
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ProposalResponse::new(false, e_string, None)),
                    )
                        .into_response();
                }
            };

            let proposal_record = ProposalRecord {
                state: proposal_state,
                commit_hash,
                base_commit_hash,
            };
            state
                .proposals
                .set_proposal_record(&proposal, proposal_record);
            if let Err(err) =
                maybe_auto_accept_proposal(&state.repo_path, &proposal, state.proposals.clone())
            {
                let e_string = format!("failed to auto-accept imported proposal: {err}");
                state
                    .proposals
                    .set_proposal_state(&proposal, ProposalState::Error(e_string.clone()));
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ProposalResponse::new(false, e_string, Some(target_ref))),
                )
                    .into_response();
            }
            publish_current_proposal_status(&state, &proposal).await;
            (
                StatusCode::OK,
                Json(ProposalResponse::new(
                    true,
                    "proposal imported".to_string(),
                    Some(target_ref),
                )),
            )
                .into_response()
        }

        Err(err) => {
            let e_string = format!("failed to import proposal: {err}");
            state
                .proposals
                .set_proposal_state(&proposal, ProposalState::Error(e_string.clone()));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ProposalResponse::new(false, e_string, None)),
            )
                .into_response()
        }
    }
}

pub async fn accept_proposal_endpoint(
    State(state): State<AppState>,
    Json(req): Json<ProposalActionRequest>,
) -> impl IntoResponse {
    let proposal_ref = format!(
        "refs/fed/proposals/{}/{}/{}",
        state.site_name, req.source_site, req.proposal_id
    );

    let proposal = Proposal {
        target_site: (*state.site_name).clone(),
        source_site: req.source_site.clone(),
        proposal_id: req.proposal_id.clone(),
    };
    let decision_base_commit_hash =
        get_commit_hash(&state.repo_path, "main").unwrap_or_else(|err| {
            tracing::error!("failed to capture accept decision base commit: {err}");
            String::new()
        });
    if let Err(err) = merge_proposal_into_main(&state.repo_path, &proposal_ref) {
        let e_string = format!("failed to merge proposal: {err}");
        state
            .proposals
            .set_proposal_state(&proposal, ProposalState::Error(e_string.clone()));
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ProposalActionResponse::new(
                false,
                e_string,
                Some(proposal_ref),
            )),
        )
            .into_response();
    }

    if let Err(err) = push_main(&state.repo_path) {
        let e_string = format!("proposal merged locally, but failed to push to origin: {err}");
        state
            .proposals
            .set_proposal_state(&proposal, ProposalState::Error(e_string.clone()));
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ProposalActionResponse::new(
                false,
                e_string,
                Some(proposal_ref),
            )),
        )
            .into_response();
    }

    // optional cleanup
    if !decision_base_commit_hash.is_empty() {
        state
            .proposals
            .set_proposal_base_commit_hash(&proposal, decision_base_commit_hash);
    }
    state
        .proposals
        .set_proposal_state(&proposal, ProposalState::Accepted);
    let _ = move_ref(&state.repo_path, &proposal_ref, &proposal.accepted_ref());
    publish_current_proposal_status(&state, &proposal).await;

    (
        StatusCode::OK,
        Json(ProposalActionResponse::new(
            true,
            "proposal accepted and pushed".to_string(),
            Some(proposal.accepted_ref()),
        )),
    )
        .into_response()
}

pub async fn reject_proposal_endpoint(
    State(state): State<AppState>,
    Json(req): Json<ProposalActionRequest>,
) -> impl IntoResponse {
    let proposal_ref = format!(
        "refs/fed/proposals/{}/{}/{}",
        state.site_name, req.source_site, req.proposal_id
    );
    let proposal = Proposal {
        target_site: (*state.site_name).clone(),
        source_site: req.source_site.clone(),
        proposal_id: req.proposal_id.clone(),
    };
    let decision_base_commit_hash =
        get_commit_hash(&state.repo_path, "main").unwrap_or_else(|err| {
            tracing::error!("failed to capture reject decision base commit: {err}");
            String::new()
        });

    match delete_ref(&state.repo_path, &proposal_ref) {
        Ok(()) => {
            if !decision_base_commit_hash.is_empty() {
                state
                    .proposals
                    .set_proposal_base_commit_hash(&proposal, decision_base_commit_hash);
            }
            state
                .proposals
                .set_proposal_state(&proposal, ProposalState::Rejected);
            let record = state.proposals.read().unwrap().get(&proposal).cloned();
            if let Some(record) = record
                && let Err(err) =
                    persist_terminal_proposal_record(&*state.repo_path, &proposal, record)
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ProposalActionResponse::new(
                        false,
                        format!("proposal rejected locally, but failed to persist decision: {err}"),
                        Some(proposal_ref),
                    )),
                )
                    .into_response();
            }
            publish_current_proposal_status(&state, &proposal).await;

            (
                StatusCode::OK,
                Json(ProposalActionResponse::new(
                    true,
                    "proposal rejected and local proposal ref removed".to_string(),
                    Some(proposal_ref),
                )),
            )
                .into_response()
        }

        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ProposalActionResponse::new(
                false,
                format!("failed to reject proposal: {err}"),
                Some(proposal_ref),
            )),
        )
            .into_response(),
    }
}

pub async fn resolve_proposal_endpoint(
    State(state): State<AppState>,
    Json(req): Json<ProposalActionRequest>,
) -> impl IntoResponse {
    let proposal_ref = format!(
        "refs/fed/proposals/{}/{}/{}",
        state.site_name, req.source_site, req.proposal_id
    );
    let proposal = Proposal {
        target_site: (*state.site_name).clone(),
        source_site: req.source_site.clone(),
        proposal_id: req.proposal_id.clone(),
    };
    let decision_base_commit_hash =
        get_commit_hash(&state.repo_path, "main").unwrap_or_else(|err| {
            tracing::error!("failed to capture resolve decision base commit: {err}");
            String::new()
        });

    match delete_ref(&state.repo_path, &proposal_ref) {
        Ok(()) => {
            if !decision_base_commit_hash.is_empty() {
                state
                    .proposals
                    .set_proposal_base_commit_hash(&proposal, decision_base_commit_hash);
            }
            state
                .proposals
                .set_proposal_state(&proposal, ProposalState::Resolved);
            let record = state.proposals.read().unwrap().get(&proposal).cloned();
            if let Some(record) = record
                && let Err(err) =
                    persist_terminal_proposal_record(&*state.repo_path, &proposal, record)
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ProposalActionResponse::new(
                        false,
                        format!("proposal resolved locally, but failed to persist decision: {err}"),
                        Some(proposal_ref),
                    )),
                )
                    .into_response();
            }
            publish_current_proposal_status(&state, &proposal).await;

            (
                StatusCode::OK,
                Json(ProposalActionResponse::new(
                    true,
                    "proposal marked resolved and local proposal ref removed".to_string(),
                    Some(proposal_ref),
                )),
            )
                .into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ProposalActionResponse::new(
                false,
                format!("failed to resolve proposal: {err}"),
                Some(proposal_ref),
            )),
        )
            .into_response(),
    }
}

pub async fn fetch_remotes_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    // NOTE: `remotes` excludes "origin"
    let remotes = match get_remotes(&state.repo_path) {
        Ok(remotes) => remotes,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(FetchRemoteResponse::new(
                    false,
                    vec![],
                    format!("failed to get remotes: {err}"),
                )),
            )
                .into_response();
        }
    };

    info!("fetching following remotes: {remotes:?}");
    match fetch_remotes(&state.repo_path, &remotes) {
        Ok(()) => (
            StatusCode::OK,
            Json(FetchRemoteResponse::new(
                true,
                remotes,
                "fetched all remotes".to_string(),
            )),
        )
            .into_response(),

        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(FetchRemoteResponse::new(
                false,
                remotes,
                format!("failed to fetch remotes: {err}"),
            )),
        )
            .into_response(),
    }
}

pub async fn list_all_my_proposals_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    match ref_load_my_proposals(&state.repo_path, ProposalKind::All, &state.site_name) {
        Ok(proposals) => (StatusCode::OK, Json(proposals)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(vec![Proposal {
                target_site: "error".to_string(),
                source_site: "error".to_string(),
                proposal_id: format!("failed to load proposals: {err}"),
            }]),
        )
            .into_response(),
    }
}

pub async fn list_my_remote_proposals_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    match ref_load_my_proposals(&state.repo_path, ProposalKind::Remote, &state.site_name) {
        Ok(proposals) => (StatusCode::OK, Json(proposals)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to list proposals: {err}"),
        )
            .into_response(),
    }
}

pub async fn list_local_proposals_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    match ref_load_my_proposals(&state.repo_path, ProposalKind::Local, &state.site_name) {
        Ok(proposals) => (StatusCode::OK, Json(proposals)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to get active proposals: {err}"),
        )
            .into_response(),
    }
}

pub async fn list_source_proposals_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    match ref_load_source_proposals(&state.repo_path, &state.site_name) {
        Ok(proposals) => (StatusCode::OK, Json(proposals)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to get source proposals: {err}"),
        )
            .into_response(),
    }
}

pub async fn all_my_statuses_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    if let Err(err) = set_missing_refs_to_rejected(
        &state.repo_path,
        state.proposals.clone(),
        &state.site_name,
        None,
    )
    .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to reconcile missing local proposal refs: {err}"),
        )
            .into_response();
    }

    let statuses = state.proposals.db_load_my_statuses(&state.site_name, None);
    (StatusCode::OK, Json(statuses)).into_response()
}

pub async fn all_source_statuses_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    let statuses = state
        .proposals
        .db_load_source_statuses(&state.site_name, None);
    (StatusCode::OK, Json(statuses)).into_response()
}

pub async fn my_statuses_by_state_endpoint(
    State(state): State<AppState>,
    Path(proposal_state): Path<ProposalState>,
) -> impl IntoResponse {
    if let Err(err) = set_missing_refs_to_rejected(
        &state.repo_path,
        state.proposals.clone(),
        &state.site_name,
        None,
    )
    .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to reconcile missing local proposal refs: {err}"),
        )
            .into_response();
    }

    let statuses = state
        .proposals
        .db_load_my_statuses(&state.site_name, Some(proposal_state));
    (StatusCode::OK, Json(statuses)).into_response()
}

pub async fn import_proposals_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    let Ok(proposals) =
        ref_load_my_proposals(&state.repo_path, ProposalKind::Remote, &state.site_name)
    else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to load proposals".to_string(),
        )
            .into_response();
    };
    info!("IMPORTING {:?} PROPOSALS", proposals);
    if let Err(err) = import_proposals(&state.repo_path, &proposals, state.proposals.clone()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to import proposals: {err}"),
        )
            .into_response();
    }
    for proposal in &proposals {
        publish_current_proposal_status(&state, proposal).await;
    }

    (StatusCode::OK, "proposals imported".to_string()).into_response()
}

pub async fn fetch_and_import_proposals_endpoint(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let remotes = match get_remotes(&state.repo_path) {
        Ok(remotes) => remotes,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to get remotes: {err}"),
            )
                .into_response();
        }
    };

    info!("fetching and importing proposals from remotes: {remotes:?}");
    if let Err(err) = fetch_and_import_proposals(
        &state.repo_path,
        &remotes,
        state.proposals.clone(),
        &state.site_name,
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to fetch and import proposals: {err}"),
        )
            .into_response();
    }
    if let Ok(proposals) =
        ref_load_my_proposals(&state.repo_path, ProposalKind::Remote, &state.site_name)
    {
        for proposal in &proposals {
            publish_current_proposal_status(&state, proposal).await;
        }
    }

    (StatusCode::OK, "proposals fetched and imported".to_string()).into_response()
}
