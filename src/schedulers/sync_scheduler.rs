use crate::{
    configs::AppConfig,
    infra::{ConfigSource, K8sClient},
    services::{AppState, ConfigSyncService},
};
use tokio::time;

/// Args: `config` is runtime config, `state` is shared global state, `source` fetches Git data, `k8s` publishes it.
/// Periodically updates collector state from Git and Kubernetes.
pub async fn update_collector_state_loop(
    config: AppConfig,
    state: AppState,
    source: ConfigSource,
    k8s: Option<K8sClient>,
) -> anyhow::Result<()> {
    let mut interval = time::interval(config.collector_update_interval);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    let service = ConfigSyncService::new(config, state, source, k8s);

    loop {
        interval.tick().await;
        service.update_collector_state().await?;
    }
}
