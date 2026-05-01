use crate::models::{DesiredNotificationState, DesiredPingState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PublishedConfig {
    pub desired_ping_state: DesiredPingState,
    pub desired_notification_state: DesiredNotificationState,
    pub revision: String,
    pub config_hash: String,
    pub synced_at: DateTime<Utc>,
}
