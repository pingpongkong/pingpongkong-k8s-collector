use crate::services::AppState;
use anyhow::Context;
use axum::{Json, Router, http::StatusCode, routing::get};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    last_updated_cluster_ping_state_time: Option<DateTime<Utc>>,
    revision: Option<String>,
    config_hash: Option<String>,
}

/// Args: `addr` is the bind address, `state` backs health and readiness responses.
/// Runs the collector HTTP server until the process is stopped.
pub async fn http_server(addr: std::net::SocketAddr, state: AppState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health", get(health))
        .route("/healthz", get(healthz))
        .route("/readyz", get(healthz))
        .route("/report", get(report))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind HTTP server on {}", addr))?;
    tracing::info!(%addr, "collector HTTP server listening");

    axum::serve(listener, app).await?;
    Ok(())
}

/// Args: `state` is the shared collector state injected by Axum.
/// Returns a compact health payload with sync status and the current revision.
async fn healthz(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Json<HealthResponse> {
    let snapshot = state.health_snapshot();

    Json(HealthResponse {
        ok: snapshot.ok,
        last_updated_cluster_ping_state_time: snapshot.last_updated_cluster_ping_state_time,
        revision: snapshot.revision,
        config_hash: snapshot.config_hash,
    })
}

/// Args: none.
/// Returns a plain OK body for simple load balancer health checks.
async fn health() -> &'static str {
    "OK"
}

/// Args: `state` is the shared collector state injected by Axum.
/// Returns the latest connectivity report when one has been created.
async fn report(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> (StatusCode, Json<Value>) {
    match state.current_report() {
        Some(report) => (StatusCode::OK, Json(json!(report))),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "report is not available yet" })),
        ),
    }
}
