use crate::{
    configs::AppConfig,
    infra::K8sClient,
    services::{AppState, ScrapeAlertService},
};
use tokio::time;
use tracing::info;

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

    let mut interval = time::interval(config.agent_check_interval);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    let mut service = ScrapeAlertService::new(config, state, k8s)?;

    loop {
        interval.tick().await;
        service.get_ping_state().await?;
    }
}

/// Args: none.
/// Awaits forever for disabled background tasks such as dry-run scraping.
async fn futures_pending() -> anyhow::Result<()> {
    std::future::pending::<()>().await;
    Ok(())
}
