#![no_main]
use std::sync::LazyLock;

use async_nats::Client;
use axum::{
    body::Body,
    http::{self, Request},
};
use libfuzzer_sys::fuzz_target;
use ref_based_fed::{
    axum::app_state::AppState,
    http_endpoints::build_router,
};
use tower::ServiceExt;
use tokio::runtime::Runtime;

use ref_based_fed::nats::core::connect;

const ENDPOINTS: [(http::Method, &str); 12] = [
    (http::Method::POST, "/webhook/gitea"),
    (http::Method::GET, "/api/proposals/a/b/diff"),
    (http::Method::POST, "/api/proposals/accept"),
    (http::Method::POST, "/api/proposals/reject"),
    (http::Method::GET, "/api/proposals/all"),
    (http::Method::GET, "/api/proposals/remote"),
    (http::Method::GET, "/api/proposals/active"),
    (http::Method::POST, "/api/proposals/import_all"),
    (http::Method::GET, "/api/proposals/statuses"),
    (http::Method::GET, "/api/proposals/statuses/any"),
    (http::Method::POST, "/api/proposals/reconcile"),
    (http::Method::POST, "/api/remotes/fetch"),
];

static RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new().expect("Failed to create Tokio runtime for fuzzing"));
static NATS: LazyLock<Client> = LazyLock::new(connect_nats);

fn connect_nats() -> Client {
    let nats_url = "nats://127.0.0.1:4222";
    RUNTIME
        .block_on(connect(nats_url))
        .expect("Failed to connect to NATS for fuzzing")
}

fn app() -> axum::Router {
    // Rebuild state per input to avoid sharing mutable state across fuzz cases.
    let state = AppState::new(
        "fuzz_secret".to_string(),
        "fuzz_site".to_string(),
        "/tmp/fuzz_repo".to_string(),
        NATS.clone(),
    );

    build_router(state)
}

fuzz_target!(|data: &[u8]| {
    // Ensure the lazy NATS client is initialized before we enter the fuzz runtime.
    let _ = LazyLock::force(&NATS);

    RUNTIME.block_on(async {
        if data.len() < 2 {
            return;
        }

        let endpoint_selector = usize::from(data[0]) % ENDPOINTS.len();
        let body_data = &data[1..];
        let (method, uri) = &ENDPOINTS[endpoint_selector];

        let request = Request::builder()
            .method(method.clone())
            .uri(*uri)
            .header("Content-Type", "application/json")
            .body(Body::from(body_data.to_vec()))
            .unwrap();

        let response = app().oneshot(request).await.unwrap();
        let _ = response.status();
    });
});
