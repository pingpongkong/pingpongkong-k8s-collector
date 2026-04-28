use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MatrixConfig {
    #[serde(default, alias = "probes", alias = "targets")]
    pub checks: Vec<ProbeTarget>,
}

impl MatrixConfig {
    /// Args: none.
    /// Validates that the matrix contains at least one usable probe target.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.checks.is_empty(),
            "matrix config must contain at least one check, probe, or target"
        );

        for check in &self.checks {
            check.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProbeTarget {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub protocol: ProbeProtocol,
    #[serde(default = "default_interval_seconds")]
    pub interval_seconds: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

impl ProbeTarget {
    /// Args: none.
    /// Validates one probe target's identity, address, and timing settings.
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.id.trim().is_empty(), "probe id cannot be empty");
        anyhow::ensure!(
            !self.host.trim().is_empty(),
            "probe '{}' host cannot be empty",
            self.id
        );
        anyhow::ensure!(
            self.interval_seconds > 0,
            "probe '{}' interval_seconds must be greater than zero",
            self.id
        );
        anyhow::ensure!(
            self.timeout_ms > 0,
            "probe '{}' timeout_ms must be greater than zero",
            self.id
        );

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProbeProtocol {
    #[default]
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscordConfig {
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl DiscordConfig {
    /// Args: none.
    /// Validates webhook and rate-limit settings for Discord alert delivery.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.enabled {
            anyhow::ensure!(
                self.webhook_url
                    .as_ref()
                    .is_some_and(|url| !url.trim().is_empty()),
                "discord config is enabled but webhook_url is empty"
            );
        }

        anyhow::ensure!(
            self.rate_limit.max_per_window > 0,
            "discord rate_limit.max_per_window must be greater than zero"
        );
        anyhow::ensure!(
            self.rate_limit.window_seconds > 0,
            "discord rate_limit.window_seconds must be greater than zero"
        );

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RateLimitConfig {
    #[serde(default = "default_rate_limit_max")]
    pub max_per_window: usize,
    #[serde(default = "default_rate_limit_window")]
    pub window_seconds: u64,
}

impl Default for RateLimitConfig {
    /// Args: none.
    /// Creates the default Discord alert rate limit.
    fn default() -> Self {
        Self {
            max_per_window: default_rate_limit_max(),
            window_seconds: default_rate_limit_window(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishedConfig {
    pub matrix: MatrixConfig,
    pub discord: DiscordConfig,
    pub revision: String,
    pub config_hash: String,
    pub synced_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentResult {
    pub agent: String,
    pub check_id: String,
    #[serde(default)]
    pub target: Option<String>,
    pub success: bool,
    #[serde(default)]
    pub latency_ms: Option<f64>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default = "Utc::now")]
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AgentResultsResponse {
    List(Vec<AgentResult>),
    Wrapped { results: Vec<AgentResult> },
}

impl AgentResultsResponse {
    /// Args: none.
    /// Normalizes either supported agent response shape into a flat result list.
    pub fn into_results(self) -> Vec<AgentResult> {
        match self {
            Self::List(results) => results,
            Self::Wrapped { results } => results,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscordWebhookPayload {
    pub content: String,
}

/// Args: none.
/// Returns the default enabled flag for config fields that opt in by default.
fn default_enabled() -> bool {
    true
}

/// Args: none.
/// Returns the default probe interval in seconds.
fn default_interval_seconds() -> u64 {
    30
}

/// Args: none.
/// Returns the default per-probe timeout in milliseconds.
fn default_timeout_ms() -> u64 {
    1_000
}

/// Args: none.
/// Returns the default maximum Discord alerts per window.
fn default_rate_limit_max() -> usize {
    5
}

/// Args: none.
/// Returns the default Discord alert rate-limit window in seconds.
fn default_rate_limit_window() -> u64 {
    60
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Args: none.
    /// Verifies that matrix configs accept `targets` as an alias for checks.
    #[test]
    fn parses_matrix_aliases() {
        let config: MatrixConfig = serde_yaml::from_str(
            r#"
targets:
  - id: redis
    host: redis.default.svc
    port: 6379
    protocol: tcp
"#,
        )
        .unwrap();

        assert_eq!(config.checks[0].id, "redis");
        assert!(matches!(config.checks[0].protocol, ProbeProtocol::Tcp));
    }

    /// Args: none.
    /// Verifies that an empty matrix is rejected during validation.
    #[test]
    fn validates_empty_matrix() {
        let config: MatrixConfig = serde_yaml::from_str("checks: []").unwrap();
        assert!(config.validate().is_err());
    }

    /// Args: none.
    /// Verifies default Discord rate-limit values when they are omitted.
    #[test]
    fn defaults_discord_rate_limit() {
        let config: DiscordConfig = serde_yaml::from_str(
            r#"
enabled: false
"#,
        )
        .unwrap();

        assert_eq!(config.rate_limit.max_per_window, 5);
        assert_eq!(config.rate_limit.window_seconds, 60);
    }
}
