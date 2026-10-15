use std::sync::Arc;

use financial_management::application::analytics::projector::FactProjector;
use financial_management::repository::analytics::PgAnalyticsRepo;
use financial_management::{analytics_maintenance, build_router, build_token_verifier, config};

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
    if args.iter().any(|a| a == "--report-stuck-outbox") {
        run_report_stuck(&analytics_repo).await;
        return;
    }
    if args.iter().any(|a| a == "--purge-outbox") {
        run_purge_outbox(&analytics_repo).await;
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

/// `--report-stuck-outbox`: list outbox rows that exhausted their retries.
/// Exits non-zero when any exist, so it can be used as a health check.
async fn run_report_stuck(repo: &PgAnalyticsRepo) {
    match analytics_maintenance::stuck_outbox_report(repo).await {
        Ok(stuck) if stuck.is_empty() => tracing::info!("stuck outbox: none"),
        Ok(stuck) => {
            for item in &stuck {
                tracing::error!(
                    outbox_id = item.outbox_id,
                    user_id = %item.user_id,
                    purchase_id = %item.purchase_id,
                    attempts = item.attempts,
                    last_error = item.last_error.as_deref().unwrap_or(""),
                    "stuck outbox row"
                );
            }
            tracing::error!(count = stuck.len(), "stuck outbox rows found");
            std::process::exit(1);
        }
        Err(e) => {
            tracing::error!(error = %e, "stuck outbox: query failed");
            std::process::exit(1);
        }
    }
}

/// `--purge-outbox`: delete processed outbox rows older than the retention
/// window (default 7 days, override with `PURGE_KEEP_DAYS`). Pending rows are
/// never removed.
async fn run_purge_outbox(repo: &PgAnalyticsRepo) {
    let days: u64 = std::env::var("PURGE_KEEP_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let keep_for = std::time::Duration::from_secs(days * 24 * 3600);
    match analytics_maintenance::purge_processed_outbox(repo, keep_for).await {
        Ok(n) => tracing::info!(removed = n, keep_days = days, "outbox purged"),
        Err(e) => {
            tracing::error!(error = %e, "outbox purge failed");
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
