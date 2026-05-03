mod configs;
mod controllers;
mod errors;
mod infra;
mod models;
mod schedulers;
mod services;

use anyhow::Context;
use configs::AppConfig;
use controllers::http_server;
use infra::{ConfigSource, K8sClient};
use schedulers::{get_ping_state_loop, update_collector_state_loop};
use services::AppState;

/// Args: none.
/// Starts logging, loads environment config, launches collector/update loops, and serves health checks.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = AppConfig::from_env()?;
    let log_level = config.log_level.as_filter();
    let log_filter = format!("warn,pingpongkong_k8s_collector={log_level}");

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(log_filter))
        .init();

    tracing::info!(
        log_level = %log_level,
        namespace = %config.namespace,
        config_map = %config.config_map_name,
        collector_update_interval_secs = config.collector_update_interval.as_secs(),
        agent_check_interval_secs = config.agent_check_interval.as_secs(),
        agent_api_port = config.agent_api_port,
        report_notification_mode = %config.report_notification_mode.as_str(),
        dry_run = config.dry_run,
        "collector starting"
    );
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
    let ping_state_task = tokio::spawn(get_ping_state_loop(config.clone(), state.clone(), k8s));
    let http_task = tokio::spawn(http_server(config.http_addr, state));

    tokio::select! {
        result = sync_task => result.context("sync task join failed")??,
        result = ping_state_task => result.context("ping state task join failed")??,
        result = http_task => result.context("http task join failed")??,
    }

    Ok(())
}
