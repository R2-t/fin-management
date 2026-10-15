use std::sync::Arc;

use financial_management::application::analytics::projector::FactProjector;
use financial_management::repository::analytics::PgAnalyticsRepo;
use financial_management::{build_router, build_token_verifier, config};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let config = config::Config::new();

    let pg_pool = config.init();

    // Analytics maintenance modes, run before the server starts and then exit.
    let args: Vec<String> = std::env::args().collect();
    let analytics_repo = PgAnalyticsRepo::new(pg_pool.clone());
    if args.iter().any(|a| a == "--backfill-facts") {
        run_backfill(&analytics_repo).await;
        return;
    }
    if args.iter().any(|a| a == "--reconcile-facts") {
        run_reconcile(&analytics_repo).await;
        return;
    }

    // Project OLTP mutations into purchase_fact in the background. The listener
    // wakes the projector on commit; the 1s poll is the fallback.
    let wake = Arc::new(tokio::sync::Notify::new());
    tokio::spawn(financial_management::repository::analytics::listen_forever(
        config.postgresconfig.get_config(),
        wake.clone(),
    ));
    tokio::spawn(
        FactProjector::new(analytics_repo).run_forever(std::time::Duration::from_secs(1), wake),
    );

    let app = build_router(&pg_pool, build_token_verifier());

    let listener = match tokio::net::TcpListener::bind("0.0.0.0:8080").await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "failed to bind 0.0.0.0:8080");
            std::process::exit(1);
        }
    };

    tracing::info!("listening on 0.0.0.0:8080");
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(error = %e, "server terminated with error");
        std::process::exit(1);
    }
}

/// `--backfill-facts`: enqueue every live purchase and drain the projector.
async fn run_backfill(repo: &PgAnalyticsRepo) {
    match repo.enqueue_all_purchases().await {
        Ok(n) => tracing::info!(enqueued = n, "backfill: purchases enqueued"),
        Err(e) => {
            tracing::error!(error = %e, "backfill: enqueue failed");
            std::process::exit(1);
        }
    }
    let projector = FactProjector::new(repo.clone());
    match projector.drain().await {
        Ok(n) => tracing::info!(projected = n, "backfill: complete"),
        Err(e) => {
            tracing::error!(error = %e, "backfill: projection failed");
            std::process::exit(1);
        }
    }
}

/// `--reconcile-facts`: compare projection totals to OLTP and exit non-zero on
/// any mismatch.
async fn run_reconcile(repo: &PgAnalyticsRepo) {
    match repo.reconcile().await {
        Ok(mismatches) if mismatches.is_empty() => {
            tracing::info!("reconcile: purchase_fact matches OLTP");
        }
        Ok(mismatches) => {
            for m in &mismatches {
                tracing::error!(
                    user_id = %m.user_id,
                    base_currency = %m.base_currency,
                    day = %m.day,
                    fact_total = m.fact_total,
                    oltp_total = m.oltp_total,
                    "reconcile: mismatch"
                );
            }
            std::process::exit(1);
        }
        Err(e) => {
            tracing::error!(error = %e, "reconcile: query failed");
            std::process::exit(1);
        }
    }
}
