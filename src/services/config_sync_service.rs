use crate::{
    configs::AppConfig,
    infra::{ConfigSource, K8sClient},
    models::{DesiredNotificationState, DesiredPingState},
    services::{AppState, NotificationAlerter},
};
use tracing::{debug, info, warn};

pub struct ConfigSyncService {
    config: AppConfig,
    state: AppState,
    source: ConfigSource,
    k8s: Option<K8sClient>,
    alerter: NotificationAlerter,
}

impl ConfigSyncService {
    /// Args: `config` is runtime config, `state` stores accepted config, `source` fetches Git state, `k8s` publishes it.
    /// Builds the service that performs one config sync unit of work.
    pub fn new(
        config: AppConfig,
        state: AppState,
        source: ConfigSource,
        k8s: Option<K8sClient>,
    ) -> Self {
        Self {
            config,
            state,
            source,
            k8s,
            alerter: NotificationAlerter::new(),
        }
    }

    /// Args: none.
    /// Runs one collector state update cycle from Kubernetes state, Git state, and global state.
    pub async fn update_collector_state(&self) -> anyhow::Result<()> {
        debug!("collector state update cycle started");
        let prev_cluster_ping_state = self.get_k8s_configmap().await?;
        debug!(
            has_previous_cluster_state = prev_cluster_ping_state.is_some(),
            "previous cluster state read"
        );

        match self.source.load().await {
            Ok(loaded) => {
                info!(revision = %loaded.published.revision, "loaded state config");
                let next_hash = loaded.published.config_hash.clone();
                let cluster_ping_state = loaded.published.desired_ping_state.clone();
                let config_changed = self.state.config_changed(&next_hash);
                let cluster_state_changed = self.is_updated_cluster_state(
                    prev_cluster_ping_state.as_ref(),
                    &cluster_ping_state,
                );
                debug!(
                    config_changed,
                    cluster_state_changed,
                    cluster = %cluster_ping_state.cluster,
                    config_hash = %next_hash,
                    "collector state comparison completed"
                );
                let mut accepted = false;

                if !cluster_state_changed && !config_changed {
                    info!(
                        config_hash = %loaded.published.config_hash,
                        "config unchanged; skipping ConfigMap publish"
                    );
                    accepted = true;
                } else if self.config.dry_run {
                    let normalized = serde_json::to_string_pretty(&loaded.published)?;
                    info!(%normalized, "dry-run normalized config");
                    accepted = true;
                } else if cluster_state_changed {
                    match self.update_k8s_configmap(&loaded).await {
                        Ok(()) => {
                            info!(
                                config_map = %self.config.config_map_name,
                                "published config to Kubernetes"
                            );
                            accepted = true;
                        }
                        Err(err) => {
                            warn!(
                                error = %err,
                                config_map = %self.config.config_map_name,
                                "ConfigMap publish failed; will retry on next sync"
                            );
                        }
                    }
                } else {
                    accepted = true;
                }

                if accepted {
                    if cluster_state_changed {
                        debug!("cluster state changed; sending collector update notifications");
                        self.notify_collector_updated(&loaded.published.desired_notification_state)
                            .await;
                    }
                    self.state.accept_config(loaded.published, next_hash);
                    debug!("global collector state updated");
                }
            }
            Err(err) => {
                warn!(error = %err, "config sync failed");
            }
        }

        Ok(())
    }

    /// Args: none.
    /// Reads the current desired ping state from the Kubernetes ConfigMap when it exists.
    async fn get_k8s_configmap(&self) -> anyhow::Result<Option<DesiredPingState>> {
        let Some(k8s) = &self.k8s else {
            debug!("dry-run enabled; skipping previous ConfigMap read");
            return Ok(None);
        };

        k8s.get_configmap(&self.config.config_map_name).await
    }

    /// Args: `prev` is the ConfigMap state and `next` is the Git state.
    /// Returns true when Kubernetes needs the desired cluster state updated.
    fn is_updated_cluster_state(
        &self,
        prev: Option<&DesiredPingState>,
        next: &DesiredPingState,
    ) -> bool {
        prev != Some(next)
    }

    /// Args: `loaded` is the validated Git state.
    /// Updates the Kubernetes ConfigMap with the new cluster state.
    async fn update_k8s_configmap(
        &self,
        loaded: &crate::infra::config_source::LoadedConfig,
    ) -> anyhow::Result<()> {
        let Some(k8s) = &self.k8s else {
            debug!("dry-run enabled; skipping ConfigMap update");
            return Ok(());
        };

        k8s.update_configmap(&self.config.config_map_name, loaded)
            .await
    }

    /// Args: none.
    /// Sends an update notification to configured destinations when notification state exists.
    async fn notify_collector_updated(&self, notification_state: &DesiredNotificationState) {
        for (destination_name, destination) in &notification_state.destinations {
            debug!(
                destination = %destination_name,
                provider = %destination.provider,
                "sending collector update notification"
            );
            if let Err(err) = self
                .alerter
                .send_collector_updated(destination_name, destination)
                .await
            {
                warn!(
                    error = %err,
                    destination = %destination_name,
                    "failed to send collector update notification"
                );
            }
        }
    }
}
