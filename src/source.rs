use crate::{
    config::{SourceConfig, TokenHeader},
    models::{DiscordConfig, MatrixConfig, PublishedConfig},
};
use anyhow::Context;
use chrono::Utc;
use reqwest::{Client, Url};

#[derive(Clone)]
pub struct ConfigSource {
    client: Client,
    config: SourceConfig,
}

pub struct LoadedConfig {
    pub published: PublishedConfig,
    pub matrix_yaml: String,
    pub discord_yaml: String,
}

impl ConfigSource {
    /// Args: `config` contains the raw config base URL, file paths, and optional auth details.
    /// Builds a source client for fetching PingPongKong state files.
    pub fn new(config: SourceConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    /// Args: none.
    /// Fetches, parses, validates, and packages the current state config.
    pub async fn load(&self) -> anyhow::Result<LoadedConfig> {
        let matrix_yaml = self.fetch(&self.config.matrix_path).await?;
        let discord_yaml = self.fetch(&self.config.discord_path).await?;

        let matrix: MatrixConfig =
            serde_yaml::from_str(&matrix_yaml).context("failed to parse matrix yaml")?;
        matrix.validate()?;

        let discord: DiscordConfig =
            serde_yaml::from_str(&discord_yaml).context("failed to parse discord yaml")?;
        discord.validate()?;

        let synced_at = Utc::now();
        let config_hash = config_hash(&matrix_yaml, &discord_yaml);
        let revision = format!("config-{}", config_hash);

        Ok(LoadedConfig {
            published: PublishedConfig {
                matrix,
                discord,
                revision,
                config_hash,
                synced_at,
            },
            matrix_yaml,
            discord_yaml,
        })
    }

    /// Args: `path` is the config file path relative to the configured base URL.
    /// Downloads one raw config file and returns its body as text.
    async fn fetch(&self, path: &str) -> anyhow::Result<String> {
        let url = join_url(&self.config.base_url, path)?;
        let mut request = self.client.get(url);

        if let Some(token) = &self.config.token {
            request = match self.config.token_header {
                TokenHeader::Bearer => request.bearer_auth(token),
                TokenHeader::GitLabPrivateToken => request.header("PRIVATE-TOKEN", token),
            };
        }

        let response = request.send().await.context("failed to fetch config")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("failed to read config body")?;

        anyhow::ensure!(
            status.is_success(),
            "config fetch failed with {}: {}",
            status,
            body
        );

        Ok(body)
    }
}

/// Args: `base` is the raw source base URL, `path` is the relative file path.
/// Joins the URL parts without losing nested paths or accidentally duplicating slashes.
fn join_url(base: &str, path: &str) -> anyhow::Result<Url> {
    let mut base = base.trim_end_matches('/').to_string();
    base.push('/');
    Url::parse(&base)
        .and_then(|url| url.join(path.trim_start_matches('/')))
        .context("CONFIG_BASE_URL must be a valid URL")
}

/// Args: `matrix_yaml` and `discord_yaml` are the raw config file contents.
/// Returns a stable content fingerprint used to detect real config changes.
fn config_hash(matrix_yaml: &str, discord_yaml: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in matrix_yaml
        .as_bytes()
        .iter()
        .chain([0xff].iter())
        .chain(discord_yaml.as_bytes())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Args: none.
    /// Verifies that identical config content produces the same fingerprint.
    #[test]
    fn config_hash_is_stable_for_same_content() {
        assert_eq!(config_hash("a", "b"), config_hash("a", "b"));
    }

    /// Args: none.
    /// Verifies that changes in either config file change the fingerprint.
    #[test]
    fn config_hash_changes_when_content_changes() {
        assert_ne!(config_hash("a", "b"), config_hash("a", "c"));
        assert_ne!(config_hash("a", "b"), config_hash("x", "b"));
    }
}
