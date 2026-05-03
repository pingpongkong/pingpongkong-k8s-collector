use crate::models::NodeHealthStatus;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{env, net::SocketAddr, time::Duration};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub log_level: LogLevel,
    pub source: SourceConfig,
    pub namespace: String,
    pub config_map_name: String,
    pub collector_update_interval: Duration,
    pub agent_check_interval: Duration,
    pub agent_api_port: u16,
    pub max_concurrent_agent_checks: usize,
    pub http_addr: SocketAddr,
    pub report_notification_mode: ReportNotificationMode,
    pub dry_run: bool,
}

impl AppConfig {
    /// Args: none.
    /// Reads process environment and returns the complete collector runtime configuration.
    pub fn from_env() -> anyhow::Result<Self> {
        let source = SourceConfig::from_env()?;

        Ok(Self {
            log_level: log_level_from_env()?,
            namespace: namespace_from_env(),
            config_map_name: config_map_name(&source.cluster_name),
            source,
            collector_update_interval: human_duration_from_env("COLLECTOR_UPDATE_INTERVAL", "5m")?,
            agent_check_interval: human_duration_from_env("AGENT_CHECK_INTERVAL", "5m")?,
            agent_api_port: parse_env("AGENT_API_PORT", 8080)?,
            max_concurrent_agent_checks: parse_bounded_usize(
                "MAX_CONCURRENT_AGENT_CHECKS",
                64,
                1,
                4096,
            )?,
            http_addr: collector_http_addr_from_env()?,
            report_notification_mode: report_notification_mode_from_env()?,
            dry_run: bool_from_env("DRY_RUN", false)?,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Args: none.
    /// Returns the lowercase tracing filter value for this log level.
    pub fn as_filter(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum ReportNotificationMode {
    Always,
    NonHealthy,
}

impl ReportNotificationMode {
    /// Args: `health_status` is the aggregate connectivity report health.
    /// Returns whether a report with this health should trigger notifications.
    pub fn should_notify(self, health_status: &NodeHealthStatus) -> bool {
        match self {
            Self::Always => true,
            Self::NonHealthy => !matches!(health_status, NodeHealthStatus::Healthy),
        }
    }

    /// Args: none.
    /// Returns the stable environment/config label for this mode.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Always => "ALWAYS",
            Self::NonHealthy => "NON_HEALTHY",
        }
    }
}

impl Default for ReportNotificationMode {
    /// Args: none.
    /// Returns the report notification mode used when REPORT_NOTIFICATION_MODE is unset.
    fn default() -> Self {
        Self::Always
    }
}

#[derive(Debug, Clone)]
pub struct SourceConfig {
    pub git_url: String,
    pub cluster_name: String,
    pub token: Option<String>,
    pub checkout_dir: String,
}

impl SourceConfig {
    /// Args: none.
    /// Reads source-related environment variables for the Git-backed config files.
    fn from_env() -> anyhow::Result<Self> {
        let git_url = env::var("CONFIG_GIT_URL").context("CONFIG_GIT_URL must be set")?;
        let cluster_name =
            env::var("CONFIG_GIT_CLUSTERNAME").context("CONFIG_GIT_CLUSTERNAME must be set")?;

        Ok(Self {
            git_url,
            cluster_name,
            token: env::var("CONFIG_GIT_TOKEN").ok(),
            checkout_dir: env_or(
                "CONFIG_GIT_CHECKOUT_DIR",
                "/tmp/pingpongkong-k8s-collector-config",
            ),
        })
    }
}

/// Args: `name` is the environment variable, `default` is used when it is unset.
/// Returns the environment value or the provided default string.
fn env_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

/// Args: none.
/// Reads the namespace where the collector and its ConfigMap live.
fn namespace_from_env() -> String {
    env::var("K8S_NAMESPACE").unwrap_or_else(|_| "default".to_string())
}

/// Args: `cluster_name` is the configured cluster identifier.
/// Returns the target ConfigMap name for the cluster ping state.
fn config_map_name(cluster_name: &str) -> String {
    format!("pingpongkong-{cluster_name}-ping-state")
}

/// Args: none.
/// Builds the collector HTTP listen address from HTTP_ADDR or COLLECTOR_API_PORT.
fn collector_http_addr_from_env() -> anyhow::Result<SocketAddr> {
    if let Ok(value) = env::var("HTTP_ADDR") {
        return value
            .parse()
            .context("HTTP_ADDR must be a socket address, for example 0.0.0.0:8081");
    }

    let port = parse_env("COLLECTOR_API_PORT", 8081)?;
    Ok(SocketAddr::from(([0, 0, 0, 0], port)))
}

/// Args: none.
/// Parses LOG_LEVEL from TRACE, DEBUG, INFO, WARN, or ERROR.
fn log_level_from_env() -> anyhow::Result<LogLevel> {
    match env_or("LOG_LEVEL", "INFO").to_ascii_uppercase().as_str() {
        "TRACE" => Ok(LogLevel::Trace),
        "DEBUG" => Ok(LogLevel::Debug),
        "INFO" => Ok(LogLevel::Info),
        "WARN" => Ok(LogLevel::Warn),
        "ERROR" => Ok(LogLevel::Error),
        value => anyhow::bail!(
            "LOG_LEVEL='{}' is invalid; expected TRACE, DEBUG, INFO, WARN, or ERROR",
            value
        ),
    }
}

/// Args: none.
/// Parses REPORT_NOTIFICATION_MODE from ALWAYS or NON_HEALTHY.
fn report_notification_mode_from_env() -> anyhow::Result<ReportNotificationMode> {
    let value = env_or("REPORT_NOTIFICATION_MODE", ReportNotificationMode::default().as_str());
    let normalized = value.trim().replace('-', "_").to_ascii_uppercase();

    match normalized.as_str() {
        "ALWAYS" => Ok(ReportNotificationMode::Always),
        "NON_HEALTHY" | "NOT_HEALTHY" | "UNHEALTHY" => Ok(ReportNotificationMode::NonHealthy),
        value => anyhow::bail!(
            "REPORT_NOTIFICATION_MODE='{}' is invalid; expected ALWAYS or NON_HEALTHY",
            value
        ),
    }
}

/// Args: `name` is the env var, `default` is used when unset.
/// Parses human duration values such as 30s, 5m, or 1h.
fn human_duration_from_env(name: &str, default: &str) -> anyhow::Result<Duration> {
    let raw = env_or(name, default);
    parse_human_duration(&raw)
        .with_context(|| format!("{name} must be a duration like 30s, 5m, or 1h"))
}

/// Args: `raw` is a duration string with an s, m, or h suffix.
/// Returns the parsed duration.
fn parse_human_duration(raw: &str) -> anyhow::Result<Duration> {
    let trimmed = raw.trim();
    anyhow::ensure!(!trimmed.is_empty(), "duration cannot be empty");

    let (number, unit) = trimmed.split_at(trimmed.len().saturating_sub(1));
    let value: u64 = number
        .parse()
        .with_context(|| format!("invalid duration value '{}'", raw))?;

    let seconds = match unit {
        "s" | "S" => value,
        "m" | "M" => value * 60,
        "h" | "H" => value * 60 * 60,
        _ => anyhow::bail!("duration '{}' must end with s, m, or h", raw),
    };

    anyhow::ensure!(seconds > 0, "duration must be greater than zero");
    Ok(Duration::from_secs(seconds))
}

/// Args: `name` is the environment variable, `default` is used when it is unset.
/// Parses a typed environment variable with a precise validation error.
fn parse_env<T>(name: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|err| anyhow::anyhow!("{}='{}' is invalid: {}", name, value, err)),
        Err(_) => Ok(default),
    }
}

/// Args: `name` is the env var, `default` is used when unset, `min` and `max` bound the value.
/// Parses and bounds a `usize` setting to avoid accidental unbounded runtime work.
fn parse_bounded_usize(
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> anyhow::Result<usize> {
    let value = parse_env(name, default)?;
    anyhow::ensure!(
        (min..=max).contains(&value),
        "{} must be between {} and {}",
        name,
        min,
        max
    );
    Ok(value)
}

/// Args: `name` is the environment variable, `default` is used when it is unset.
/// Parses common boolean spellings from environment variables.
fn bool_from_env(name: &str, default: bool) -> anyhow::Result<bool> {
    match env::var(name) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => anyhow::bail!("{}='{}' is invalid boolean", name, value),
        },
        Err(_) => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, ReportNotificationMode};
    use crate::models::NodeHealthStatus;
    use std::{env, net::SocketAddr, sync::Mutex};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn reads_helm_port_env_values() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");

        // SAFETY: This test serializes env mutation and no other tests read these variables.
        unsafe {
            env::set_var("CONFIG_GIT_URL", "https://github.com/example/config.git");
            env::set_var("CONFIG_GIT_CLUSTERNAME", "test-cluster");
            env::set_var("AGENT_API_PORT", "9090");
            env::set_var("COLLECTOR_API_PORT", "9091");
            env::remove_var("HTTP_ADDR");
        }

        let config = AppConfig::from_env().expect("config should load from helm env values");

        assert_eq!(config.agent_api_port, 9090);
        assert_eq!(config.http_addr, SocketAddr::from(([0, 0, 0, 0], 9091)));
    }

    #[test]
    fn reads_report_notification_mode_env_value() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");

        // SAFETY: This test serializes env mutation and no other tests read these variables.
        unsafe {
            env::set_var("CONFIG_GIT_URL", "https://github.com/example/config.git");
            env::set_var("CONFIG_GIT_CLUSTERNAME", "test-cluster");
            env::set_var("REPORT_NOTIFICATION_MODE", "NON_HEALTHY");
            env::remove_var("HTTP_ADDR");
        }

        let config = AppConfig::from_env().expect("config should load notification mode");

        assert_eq!(
            config.report_notification_mode,
            ReportNotificationMode::NonHealthy
        );
    }

    #[test]
    fn defaults_report_notification_mode_to_always() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");

        // SAFETY: This test serializes env mutation and no other tests read these variables.
        unsafe {
            env::set_var("CONFIG_GIT_URL", "https://github.com/example/config.git");
            env::set_var("CONFIG_GIT_CLUSTERNAME", "test-cluster");
            env::remove_var("REPORT_NOTIFICATION_MODE");
            env::remove_var("HTTP_ADDR");
        }

        let config = AppConfig::from_env().expect("config should load default notification mode");

        assert_eq!(config.report_notification_mode, ReportNotificationMode::Always);
    }

    #[test]
    fn non_healthy_report_notification_mode_skips_healthy_reports() {
        assert!(!ReportNotificationMode::NonHealthy.should_notify(&NodeHealthStatus::Healthy));
        assert!(ReportNotificationMode::NonHealthy.should_notify(&NodeHealthStatus::Degraded));
        assert!(ReportNotificationMode::NonHealthy.should_notify(&NodeHealthStatus::Unreachable));
    }
}
