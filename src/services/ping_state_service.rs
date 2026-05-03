use crate::{
    configs::AppConfig,
    infra::{AgentClient, K8sClient},
    models::ConnectivityReport,
    services::{AlertRateLimiter, AppState, NotificationAlerter},
};
use std::time::Duration;
use tracing::{debug, warn};

pub struct PingStateService {
    config: AppConfig,
    state: AppState,
    k8s: K8sClient,
    agent_client: AgentClient,
    alerter: NotificationAlerter,
    limiter: AlertRateLimiter,
}

impl PingStateService {
    /// Args: `config` is runtime config, `state` holds global collector state, `k8s` discovers nodes.
    /// Builds the service that checks agents and sends report notifications.
    pub fn new(config: AppConfig, state: AppState, k8s: K8sClient) -> anyhow::Result<Self> {
        Ok(Self {
            config,
            state,
            k8s,
            agent_client: AgentClient::new()?,
            alerter: NotificationAlerter::new(),
            limiter: AlertRateLimiter::default(),
        })
    }

    /// Args: none.
    /// Gets current ping state, checks all node agents, stores a ConnectivityReport, and notifies destinations.
    pub async fn get_ping_state(&mut self) -> anyhow::Result<()> {
        debug!("agent ping state cycle started");
        let Some(cluster_ping_state) = self.state.current_cluster_ping_state() else {
            debug!("cluster ping state not available yet; skipping agent checks");
            return Ok(());
        };
        debug!(
            cluster = %cluster_ping_state.cluster,
            environment = ?cluster_ping_state.environment,
            "loaded cluster ping state from global state"
        );

        let k8s_nodes = match self.k8s.list_nodes().await {
            Ok(nodes) => nodes,
            Err(err) => {
                warn!(error = %err, "failed to list Kubernetes nodes");
                return Ok(());
            }
        };
        debug!(
            nodes = k8s_nodes.len(),
            "Kubernetes nodes discovered for agent checks"
        );

        let node_statuses = self
            .agent_client
            .get_agent_data(
                k8s_nodes,
                self.config.agent_api_port,
                self.config.max_concurrent_agent_checks,
            )
            .await;
        debug!(node_statuses = node_statuses.len(), "agent data collected");

        let report = ConnectivityReport {
            cluster_name: cluster_ping_state.cluster,
            environment: cluster_ping_state.environment.unwrap_or_default(),
            node_statuses,
        };

        debug!(
            cluster = %report.cluster_name,
            environment = %report.environment,
            nodes = report.node_statuses.len(),
            health = ?report.health_status(),
            "connectivity report built"
        );
        self.state.update_report(report.clone());
        self.send_report_notifications(&report).await;

        Ok(())
    }

    /// Args: `report` is the latest connectivity report.
    /// Sends the report to configured notification destinations with simple rate limiting.
    async fn send_report_notifications(&mut self, report: &ConnectivityReport) {
        let health_status = report.health_status();
        if !self
            .config
            .report_notification_mode
            .should_notify(&health_status)
        {
            debug!(
                cluster = %report.cluster_name,
                health = ?health_status,
                mode = %self.config.report_notification_mode.as_str(),
                "skipping connectivity report notification by mode"
            );
            return;
        }

        let Some(notification_state) = self.state.current_notification_state() else {
            debug!("notification state not available; skipping report notifications");
            return;
        };
        debug!(
            destinations = notification_state.destinations.len(),
            mode = %self.config.report_notification_mode.as_str(),
            "sending connectivity report notifications"
        );

        for (destination_name, destination) in &notification_state.destinations {
            let minute_window = Duration::from_secs(60);
            self.limiter.prune_expired(minute_window);
            let rate_key = format!("{destination_name}:report");
            let allowed = self.limiter.allow(
                &rate_key,
                destination.rate_limit.max_notifications_per_minute,
                minute_window,
            );

            if !allowed {
                warn!(
                    destination = %destination_name,
                    "connectivity report notification rate limited"
                );
                continue;
            }

            debug!(
                destination = %destination_name,
                provider = %destination.provider,
                "sending connectivity report notification"
            );
            if let Err(err) = self
                .alerter
                .send_connectivity_report(destination_name, destination, report)
                .await
            {
                warn!(
                    error = %err,
                    destination = %destination_name,
                    "failed to send connectivity report notification"
                );
            }
        }
    }
}
