mod configs;
mod errors;
mod infra;
mod models;
mod schedulers;
mod services;

use anyhow::Context;
use configs::AppConfig;
use infra::{ConfigSource, K8sClient};
use schedulers::{get_ping_state_loop, http_server, update_collector_state_loop};
use services::AppState;

/// Args: none.
/// Starts logging, loads environment config, launches sync/scrape loops, and serves health checks.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pingpongkong_k8s_collector=debug,debug".into()),
        )
        .init();

    let config = AppConfig::from_env()?;
    let state = AppState::default();
    let source = ConfigSource::new(config.source.clone());

    let k8s = if config.dry_run {
        None
    } else {
        Some(K8sClient::new(config.namespace.clone()).await?)
    };

    let sync_task = tokio::spawn(update_collector_state_loop(
        config.clone(),
        state.clone(),
        source,
        k8s.clone(),
    ));
    let scrape_task = tokio::spawn(get_ping_state_loop(config.clone(), state.clone(), k8s));
    let http_task = tokio::spawn(http_server(config.http_addr, state));

    tokio::select! {
        result = sync_task => result.context("sync task join failed")??,
        result = scrape_task => result.context("scrape task join failed")??,
        result = http_task => result.context("http task join failed")??,
    }

    Ok(())
}
