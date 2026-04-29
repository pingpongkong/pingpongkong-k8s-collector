mod agents;
mod config;
mod discord;
mod k8s;
mod models;
mod source;

use agents::AgentScraper;
use anyhow::Context;
use axum::{Json, Router, routing::get};
use chrono::{DateTime, Utc};
use config::AppConfig;
use discord::{AlertRateLimiter, DiscordAlerter};
use models::{DiscordConfig, PublishedConfig};
use serde::Serialize;
use source::ConfigSource;
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::time;
use tracing::{info, warn};

#[derive(Clone, Default)]
struct AppState {
    current_config: Arc<RwLock<Option<PublishedConfig>>>,
    last_sync: Arc<RwLock<Option<DateTime<Utc>>>>,
    last_config_hash: Arc<RwLock<Option<String>>>,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    last_sync: Option<DateTime<Utc>>,
    revision: Option<String>,
    config_hash: Option<String>,
}

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
        Some(k8s::K8sClient::new(config.namespace.clone()).await?)
    };

    let sync_task = tokio::spawn(sync_loop(
        config.clone(),
        state.clone(),
        source,
        k8s.clone(),
    ));
    let scrape_task = tokio::spawn(scrape_loop(config.clone(), state.clone(), k8s));
    let http_task = tokio::spawn(http_server(config.http_addr, state));

    tokio::select! {
        result = sync_task => result.context("sync task join failed")??,
        result = scrape_task => result.context("scrape task join failed")??,
        result = http_task => result.context("http task join failed")??,
    }

    Ok(())
}

/// Args: `config` is runtime config, `state` is shared health state, `source` fetches config, `k8s` publishes it.
/// Periodically fetches state files, validates them, and writes the active ConfigMap.
async fn sync_loop(
    config: AppConfig,
    state: AppState,
    source: ConfigSource,
    k8s: Option<k8s::K8sClient>,
) -> anyhow::Result<()> {
    let mut interval = time::interval(config.sync_interval);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        interval.tick().await;

        match source.load().await {
            Ok(loaded) => {
                info!(revision = %loaded.published.revision, "loaded state config");
                let next_hash = loaded.published.config_hash.clone();
                let config_changed = config_changed(&state, &next_hash);
                let mut accepted = false;

                if !config_changed {
                    info!(
                        config_hash = %loaded.published.config_hash,
                        "config unchanged; skipping ConfigMap publish"
                    );
                    accepted = true;
                } else if config.dry_run {
                    let normalized = serde_json::to_string_pretty(&loaded.published)?;
                    info!(%normalized, "dry-run normalized config");
                    accepted = true;
                } else if let Some(k8s) = &k8s {
                    match k8s.publish_config(&config.config_map_name, &loaded).await {
                        Ok(()) => {
                            info!(
                                config_map = %config.config_map_name,
                                "published config to Kubernetes"
                            );
                            accepted = true;
                        }
                        Err(err) => {
                            warn!(
                                error = %err,
                                config_map = %config.config_map_name,
                                "ConfigMap publish failed; will retry on next sync"
                            );
                        }
                    }
                }

                if !accepted {
                    continue;
                }

                {
                    let mut current = state.current_config.write().expect("state poisoned");
                    *current = Some(loaded.published);
                }
                {
                    let mut last_sync = state.last_sync.write().expect("state poisoned");
                    *last_sync = Some(Utc::now());
                }
                {
                    let mut last_config_hash =
                        state.last_config_hash.write().expect("state poisoned");
                    *last_config_hash = Some(next_hash);
                }
            }
            Err(err) => {
                warn!(error = %err, "config sync failed");
            }
        }
    }
}

/// Args: `config` is runtime config, `state` stores active alert settings, `k8s` discovers agents.
/// Periodically scrapes agent results and sends rate-limited Discord alerts for failures.
async fn scrape_loop(
    config: AppConfig,
    state: AppState,
    k8s: Option<k8s::K8sClient>,
) -> anyhow::Result<()> {
    let Some(k8s) = k8s else {
        info!("dry-run enabled; agent scraping is disabled");
        return futures_pending().await;
    };

    let scraper = AgentScraper::new()?;
    let alerter = DiscordAlerter::new();
    let mut limiter = AlertRateLimiter::default();
    let mut interval = time::interval(config.scrape_interval);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        interval.tick().await;

        let Some(discord_config) = current_discord_config(&state) else {
            continue;
        };

        let urls = match k8s
            .list_agent_urls(
                &config.agent_selector,
                config.agent_port,
                &config.agent_results_path,
            )
            .await
        {
            Ok(urls) => urls,
            Err(err) => {
                warn!(error = %err, "failed to discover agents");
                continue;
            }
        };

        limiter.prune_expired(Duration::from_secs(
            discord_config.rate_limit.window_seconds,
        ));

        for scrape_result in scraper
            .scrape(urls, config.max_concurrent_agent_scrapes)
            .await
        {
            match scrape_result {
                Ok(results) => {
                    for result in results.into_iter().filter(|result| !result.success) {
                        let key = format!("{}:{}", result.agent, result.check_id);
                        let allowed = limiter.allow(
                            &key,
                            discord_config.rate_limit.max_per_window,
                            Duration::from_secs(discord_config.rate_limit.window_seconds),
                        );

                        if !allowed {
                            warn!(alert_key = %key, "discord alert rate limited");
                            continue;
                        }

                        if let Err(err) = alerter.send_failure(&discord_config, &result).await {
                            warn!(error = %err, alert_key = %key, "failed to send discord alert");
                        }
                    }
                }
                Err(err) => warn!(error = %err, "agent scrape failed"),
            }
        }
    }
}

/// Args: `addr` is the bind address, `state` backs health and readiness responses.
/// Runs the collector HTTP server until the process is stopped.
async fn http_server(addr: std::net::SocketAddr, state: AppState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(healthz))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind HTTP server on {}", addr))?;
    info!(%addr, "collector HTTP server listening");

    axum::serve(listener, app).await?;
    Ok(())
}

/// Args: `state` is the shared collector state injected by Axum.
/// Returns a compact health payload with sync status and the current revision.
async fn healthz(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Json<HealthResponse> {
    let last_sync = *state.last_sync.read().expect("state poisoned");
    let current = state.current_config.read().expect("state poisoned");
    let revision = current.as_ref().map(|config| config.revision.clone());
    let config_hash = current.as_ref().map(|config| config.config_hash.clone());

    Json(HealthResponse {
        ok: last_sync.is_some(),
        last_sync,
        revision,
        config_hash,
    })
}

/// Args: `state` is the shared collector state.
/// Clones the current Discord config when a synced config is available.
fn current_discord_config(state: &AppState) -> Option<DiscordConfig> {
    state
        .current_config
        .read()
        .expect("state poisoned")
        .as_ref()
        .map(|config| config.discord.clone())
}

/// Args: `state` is the shared collector state, `next_hash` is the fetched config fingerprint.
/// Returns true when the fetched config differs from the last accepted config.
fn config_changed(state: &AppState, next_hash: &str) -> bool {
    state
        .last_config_hash
        .read()
        .expect("state poisoned")
        .as_deref()
        != Some(next_hash)
}

/// Args: none.
/// Awaits forever for disabled background tasks such as dry-run scraping.
async fn futures_pending() -> anyhow::Result<()> {
    std::future::pending::<()>().await;
    Ok(())
}
