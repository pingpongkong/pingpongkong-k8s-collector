use crate::{
    errors::STATE_POISONED,
    models::{ConnectivityReport, DesiredNotificationState, DesiredPingState, PublishedConfig},
};
use chrono::{DateTime, Utc};
use std::sync::{Arc, RwLock};
use tracing::debug;

#[derive(Clone, Default)]
pub struct AppState {
    current_config: Arc<RwLock<Option<PublishedConfig>>>,
    last_updated_cluster_ping_state_time: Arc<RwLock<Option<DateTime<Utc>>>>,
    last_config_hash: Arc<RwLock<Option<String>>>,
    report: Arc<RwLock<Option<ConnectivityReport>>>,
}

impl AppState {
    /// Args: `published` is the accepted config, `config_hash` is its stable fingerprint.
    /// Stores the active config and marks the sync as successful.
    pub fn accept_config(&self, published: PublishedConfig, config_hash: String) {
        debug!(
            cluster = %published.desired_ping_state.cluster,
            revision = %published.revision,
            config_hash = %config_hash,
            "accepting config into global state"
        );
        {
            let mut current = self.current_config.write().expect(STATE_POISONED);
            *current = Some(published);
        }
        {
            let mut last_updated = self
                .last_updated_cluster_ping_state_time
                .write()
                .expect(STATE_POISONED);
            *last_updated = Some(Utc::now());
        }
        {
            let mut last_config_hash = self.last_config_hash.write().expect(STATE_POISONED);
            *last_config_hash = Some(config_hash);
        }
    }

    /// Args: none.
    /// Returns the active notification state when one has been synced.
    pub fn current_notification_state(&self) -> Option<DesiredNotificationState> {
        self.current_config
            .read()
            .expect(STATE_POISONED)
            .as_ref()
            .map(|config| config.desired_notification_state.clone())
    }

    /// Args: none.
    /// Returns the active desired ping state when one has been synced.
    pub fn current_cluster_ping_state(&self) -> Option<DesiredPingState> {
        self.current_config
            .read()
            .expect(STATE_POISONED)
            .as_ref()
            .map(|config| config.desired_ping_state.clone())
    }

    /// Args: `report` is the latest observed connectivity report.
    /// Stores the report for controller responses.
    pub fn update_report(&self, report: ConnectivityReport) {
        debug!(
            cluster = %report.cluster_name,
            nodes = report.node_statuses.len(),
            health = ?report.health_status(),
            "updating report in global state"
        );
        let mut current = self.report.write().expect(STATE_POISONED);
        *current = Some(report);
    }

    /// Args: none.
    /// Returns the latest observed connectivity report.
    pub fn current_report(&self) -> Option<ConnectivityReport> {
        self.report.read().expect(STATE_POISONED).clone()
    }

    /// Args: `next_hash` is the fetched config fingerprint.
    /// Returns true when the fetched config differs from the last accepted config.
    pub fn config_changed(&self, next_hash: &str) -> bool {
        self.last_config_hash
            .read()
            .expect(STATE_POISONED)
            .as_deref()
            != Some(next_hash)
    }

    /// Args: none.
    /// Returns a health snapshot for controller responses.
    pub fn health_snapshot(&self) -> HealthSnapshot {
        let last_updated = *self
            .last_updated_cluster_ping_state_time
            .read()
            .expect(STATE_POISONED);
        let current = self.current_config.read().expect(STATE_POISONED);

        HealthSnapshot {
            ok: last_updated.is_some(),
            last_updated_cluster_ping_state_time: last_updated,
            revision: current.as_ref().map(|config| config.revision.clone()),
            config_hash: current.as_ref().map(|config| config.config_hash.clone()),
        }
    }
}

pub struct HealthSnapshot {
    pub ok: bool,
    pub last_updated_cluster_ping_state_time: Option<DateTime<Utc>>,
    pub revision: Option<String>,
    pub config_hash: Option<String>,
}
