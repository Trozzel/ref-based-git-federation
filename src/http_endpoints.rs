use axum::Router;
use axum::routing::{get, post};

use crate::axum::app_state::AppState;
use crate::axum::git_webhook::gitea_webhook;
use crate::axum::web_serve::{
    accept_proposal_endpoint, add_following_site_endpoint, all_my_statuses_endpoint,
    all_source_statuses_endpoint, federation_site_preferences_endpoint,
    fetch_and_import_proposals_endpoint, fetch_remotes_endpoint,
    get_proposal_conflict_file_endpoint, get_proposal_diff_endpoint,
    get_source_proposal_diff_endpoint, import_proposals_endpoint, list_all_my_proposals_endpoint,
    list_local_proposals_endpoint, list_my_remote_proposals_endpoint,
    list_source_proposals_endpoint, my_statuses_by_state_endpoint, reject_proposal_endpoint,
    resolve_proposal_conflicts_endpoint, resolve_proposal_endpoint,
    set_allow_fast_forward_merge_endpoint, site_info_endpoint,
};

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/webhook/gitea", post(gitea_webhook))
        .route(
            "/api/proposals/{source_site}/{proposal_id}/diff",
            get(get_proposal_diff_endpoint),
        )
        .route(
            "/api/proposals/source/{target_site}/{proposal_id}/diff",
            get(get_source_proposal_diff_endpoint),
        )
        .route(
            "/api/proposals/conflicts/{source_site}/{proposal_id}/{*file_path}",
            get(get_proposal_conflict_file_endpoint),
        )
        .route(
            "/api/proposals/conflicts/resolve",
            post(resolve_proposal_conflicts_endpoint),
        )
        .route("/api/proposals/accept", post(accept_proposal_endpoint))
        .route("/api/proposals/reject", post(reject_proposal_endpoint))
        .route("/api/proposals/resolve", post(resolve_proposal_endpoint))
        .route("/api/proposals/all", get(list_all_my_proposals_endpoint))
        .route(
            "/api/proposals/remote",
            get(list_my_remote_proposals_endpoint),
        )
        .route("/api/proposals/active", get(list_local_proposals_endpoint))
        .route("/api/proposals/source", get(list_source_proposals_endpoint))
        .route("/api/proposals/import_all", post(import_proposals_endpoint))
        .route("/api/proposals/statuses", get(all_my_statuses_endpoint))
        .route(
            "/api/proposals/source/statuses",
            get(all_source_statuses_endpoint),
        )
        .route(
            "/api/proposals/statuses/{status}",
            get(my_statuses_by_state_endpoint),
        )
        .route(
            "/api/federation/sites",
            get(federation_site_preferences_endpoint),
        )
        .route("/api/site", get(site_info_endpoint))
        .route(
            "/api/federation/following",
            post(add_following_site_endpoint),
        )
        .route(
            "/api/federation/allow-fast-forward-merge",
            post(set_allow_fast_forward_merge_endpoint),
        )
        .route(
            "/api/proposals/reconcile",
            post(fetch_and_import_proposals_endpoint),
        )
        .route("/api/remotes/fetch", post(fetch_remotes_endpoint))
        .with_state(state)
}
