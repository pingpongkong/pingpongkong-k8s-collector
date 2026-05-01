use crate::{
    configs::AppConfig,
    infra::K8sClient,
    services::{AppState, PingStateService},
};
use tokio::time;
use tracing::{debug, info};

/// Args: `config` is runtime config, `state` stores global collector state, `k8s` discovers nodes.
/// Periodically gets node agent data and updates the connectivity report.
pub async fn get_ping_state_loop(
    config: AppConfig,
    state: AppState,
    k8s: Option<K8sClient>,
) -> anyhow::Result<()> {
    let Some(k8s) = k8s else {
        info!("dry-run enabled; agent scraping is disabled");
        return futures_pending().await;
    };

    debug!(
        interval_secs = config.agent_check_interval.as_secs(),
        "starting agent check scheduler"
    );
    let mut interval = time::interval(config.agent_check_interval);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    let mut service = PingStateService::new(config, state, k8s)?;

    loop {
        interval.tick().await;
        debug!("agent check scheduler tick");
        service.get_ping_state().await?;
    }
}

/// Args: none.
/// Awaits forever for disabled background tasks such as dry-run scraping.
async fn futures_pending() -> anyhow::Result<()> {
    std::future::pending::<()>().await;
    Ok(())
}
