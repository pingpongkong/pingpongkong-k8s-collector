use anyhow::Context;
use std::{env, net::SocketAddr, time::Duration};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub source: SourceConfig,
    pub namespace: String,
    pub config_map_name: String,
    pub sync_interval: Duration,
    pub scrape_interval: Duration,
    pub agent_selector: String,
    pub agent_results_path: String,
    pub agent_port: u16,
    pub max_concurrent_agent_scrapes: usize,
    pub http_addr: SocketAddr,
    pub dry_run: bool,
}

impl AppConfig {
    /// Args: none.
    /// Reads process environment and returns the complete collector runtime configuration.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            source: SourceConfig::from_env()?,
            namespace: env_or("K8S_NAMESPACE", "default"),
            config_map_name: env_or("CONFIG_MAP_NAME", "pingpongkong-current-matrix"),
            sync_interval: duration_from_env("SYNC_INTERVAL_SECONDS", 60)?,
            scrape_interval: duration_from_env("SCRAPE_INTERVAL_SECONDS", 15)?,
            agent_selector: env_or(
                "AGENT_LABEL_SELECTOR",
                "app.kubernetes.io/name=pingpongkong-agent",
            ),
            agent_results_path: env_or("AGENT_RESULTS_PATH", "/results"),
            agent_port: parse_env("AGENT_PORT", 8080)?,
            max_concurrent_agent_scrapes: parse_bounded_usize(
                "MAX_CONCURRENT_AGENT_SCRAPES",
                64,
                1,
                4096,
            )?,
            http_addr: env_or("HTTP_ADDR", "0.0.0.0:8080")
                .parse()
                .context("HTTP_ADDR must be a socket address, for example 0.0.0.0:8080")?,
            dry_run: bool_from_env("DRY_RUN", false)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SourceConfig {
    pub git_url: String,
    pub matrix_path: String,
    pub discord_path: String,
    pub token: Option<String>,
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
            matrix_path: format!("k8s/{cluster_name}.yaml"),
            discord_path: "notification/discord.yaml".to_string(),
            token: env::var("CONFIG_GIT_TOKEN").ok(),
        })
    }
}

/// Args: `name` is the environment variable, `default` is used when it is unset.
/// Returns the environment value or the provided default string.
fn env_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

/// Args: `name` is the environment variable, `default_seconds` is used when it is unset.
/// Parses a seconds value into a `Duration`.
fn duration_from_env(name: &str, default_seconds: u64) -> anyhow::Result<Duration> {
    Ok(Duration::from_secs(parse_env(name, default_seconds)?))
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
