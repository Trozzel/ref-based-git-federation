use std::net::SocketAddr;
use std::time::Duration;

use clap::Parser;
use ref_based_fed::axum::app_state::AppState;
use ref_based_fed::federation::{
    ensure_allow_fast_forward_merge_file, ensure_following_file, reconcile_following_remotes_loop,
};
use ref_based_fed::git::reconciliation::{cleanup_rejected_proposal_refs, git_reconciliation_loop};
use ref_based_fed::git::utils;
use ref_based_fed::git::utils::get_commit_hash;
use ref_based_fed::http_endpoints::build_router;
use ref_based_fed::nats::advertisement::{SiteAdvertisement, publish_site_advertisement};
use ref_based_fed::nats::core::connect;
use ref_based_fed::nats::farewell::{SiteFarewell, publish_site_farewell};
use ref_based_fed::nats::listener::{
    repo_proposal_handler, repo_proposal_status_handler, repo_update_handler,
    site_advertisement_handler, site_farewell_handler,
};
use ref_based_fed::proposal::persistence::hydrate_persisted_terminal_proposal_states;
use tokio_stream::StreamExt;
use tracing::{error, info};

#[derive(clap::Parser, Debug)]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    url: String,

    #[arg(long, default_value_t = 8081)]
    port: u16,

    #[arg(long)]
    site_name: String,

    #[arg(long)]
    repo_path: String,

    #[arg(long, default_value = "nats://127.0.0.1:4222")]
    nats_url: String,

    // TODO: remove does not appear to be used
    #[arg(long, default_value = "mys3cr3t")]
    secret: String,

    #[arg(
        long,
        default_value = "http://${gitea_username}:${gitea_password}@localhost:$((30000 + ${site_id}))/${gitea_username}/site-${site_id}.git"
    )]
    repo_url: String,

    #[arg(long, default_value_t = 60)]
    publish_interval: u64,

    #[arg(long, default_value_t = false)]
    farewell: bool,

    /// Snoop on all NATS messages and print them to the console.
    #[arg(long)]
    snoop: bool,
}

/// Connects to NATS and snoops on all message channels, dumping them to the console.
async fn snoop_nats_messages() -> Result<(), Box<dyn std::error::Error>> {
    // You might want to make the NATS URL configurable.
    let nats_url = "nats://localhost:4222";
    let client = async_nats::connect(nats_url).await?;
    info!("Connected to NATS at {}", nats_url);

    // Subscribe to all subjects using the '>' wildcard.
    let mut subscriber = client.subscribe(">").await?;
    info!("Snooping on all NATS subjects...");

    // Process messages from the subscription.
    while let Some(message) = subscriber.next().await {
        let payload_str = String::from_utf8_lossy(&message.payload);
        info!(
            "[SNOOP] subject: {:<30} | payload: {}",
            message.subject, payload_str
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    env_logger::init(); // reads RUST_LOG env var
    let args = Args::parse();

    info!("args:\n{:#?}", args); // pretty-printed Debug

    if args.snoop {
        info!("Snoop mode enabled. Listening for all NATS messages.");
        // Spawn the snooping task.
        tokio::spawn(async move {
            if let Err(e) = snoop_nats_messages().await {
                error!("NATS snooping task failed: {}", e);
            }
        });

        // Keep the main thread alive to allow the snooping to continue.
        // If your app has other long-running tasks, this might not be necessary.
        std::future::pending::<()>().await;
        std::process::exit(0);
    }

    if args.farewell {
        let nats = connect(&args.nats_url).await.unwrap();
        let msg = SiteFarewell {
            site: args.site_name.clone(),
            repo_url: args.repo_url.clone(),
        };

        info!("Publishing farewell message for site: {}", args.site_name);
        if let Err(err) = publish_site_farewell(&nats, &msg).await {
            error!("failed to publish NATS farewell message: {err}");
            std::process::exit(1);
        }

        info!("Removing all remotes except 'origin' from the local repository...");
        let repo = match git2::Repository::open(&args.repo_path) {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to open repository at {}: {}", args.repo_path, e);
                std::process::exit(1);
            }
        };

        let remotes = match repo.remotes() {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to list remotes for repository: {}", e);
                std::process::exit(1);
            }
        };

        for remote_name in remotes.iter().flatten() {
            if remote_name != "origin" {
                info!("Attempting to remove remote: {}", remote_name);
                if let Err(err) = utils::remove_remote(&args.repo_path, remote_name) {
                    error!("Failed to remove remote '{}': {}", remote_name, err);
                } else {
                    info!("Successfully removed remote: {}", remote_name);
                }
            } else {
                info!("Keeping 'origin' remote.");
            }
        }

        info!("Farewell message published. Exiting.");
        std::process::exit(0);
    }

    let nats = connect(&args.nats_url).await.unwrap();
    if let Err(err) = ensure_following_file(&args.repo_path) {
        error!(
            "failed to initialize federation following file in repo {}: {}",
            args.repo_path, err
        );
        std::process::exit(1);
    }
    if let Err(err) = ensure_allow_fast_forward_merge_file(&args.repo_path) {
        error!(
            "failed to initialize federation allow-fast-forward-merge file in repo {}: {}",
            args.repo_path, err
        );
        std::process::exit(1);
    }

    let state = AppState::new(
        "mys3cr3t".to_string(),
        args.site_name.clone(),
        args.repo_path.clone(),
        nats.clone(),
    );
    if let Err(err) = hydrate_persisted_terminal_proposal_states(
        &args.repo_path,
        state.proposals.clone(),
        &args.site_name,
    ) {
        error!(
            "failed to hydrate persisted terminal proposal states for repo {}: {}",
            args.repo_path, err
        );
    }

    // NATS handler - other site advertisements
    tokio::spawn(site_advertisement_handler(
        args.site_name.clone(),
        args.repo_path.clone(),
        nats.clone(),
    ));
    // Reconcile remotes when following preferences change.
    tokio::spawn(reconcile_following_remotes_loop(args.repo_path.clone()));
    // NATS handler - other site farewells
    tokio::spawn(site_farewell_handler(
        args.site_name.clone(),
        args.repo_path.clone(),
        nats.clone(),
    ));
    // NATS handler - other site updated "main"
    tokio::spawn(repo_update_handler(
        args.site_name.clone(),
        args.repo_path.clone(),
        nats.clone(),
    ));
    // NATS handler - other site makes proposal for _this_ site
    tokio::spawn(repo_proposal_handler(
        args.site_name.clone(),
        args.repo_path.clone(),
        nats.clone(),
        state.proposals.clone(),
    ));
    // NATS handler - target site reports status for proposals created by this site
    tokio::spawn(repo_proposal_status_handler(
        args.site_name.clone(),
        nats.clone(),
        state.proposals.clone(),
    ));
    // Git fetch remotes
    tokio::spawn(git_reconciliation_loop(
        args.repo_path.clone(),
        state.proposals.clone(),
        args.site_name.clone(),
    ));
    // Cleanup old proposals every hour
    tokio::spawn(cleanup_rejected_proposal_refs(
        args.repo_path.clone(),
        state.proposals.clone(),
        args.site_name.clone(),
        nats.clone(),
    ));

    // Periodically publish this site's status
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(args.publish_interval));
        loop {
            interval.tick().await;
            let main_git_ref = match get_commit_hash(&args.repo_path, "main") {
                Ok(hash) => hash.trim().to_string(),
                Err(err) => {
                    error!("failed to get main branch hash: {err}");
                    "unknown".to_string()
                }
            };

            let msg = SiteAdvertisement {
                site: args.site_name.clone(),
                repo_url: args.repo_url.clone(),
                commit_hash: main_git_ref,
            };

            if let Err(err) = publish_site_advertisement(&nats, &msg).await {
                error!("failed to publish NATS message: {err}");
            }
        }
    });

    let app = build_router(state);

    let addr: SocketAddr = format!("{}:{}", args.url, args.port).parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    info!("listening on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
