use crate::{
    configs::SourceConfig,
    models::{
        DesiredNotificationFile, DesiredNotificationState, DesiredPingState, PublishedConfig,
    },
};
use anyhow::Context;
use chrono::Utc;
use reqwest::Url;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tokio::{fs, process::Command};
use tracing::{debug, info};

#[derive(Clone)]
pub struct ConfigSource {
    config: SourceConfig,
}

pub struct LoadedConfig {
    pub published: PublishedConfig,
    pub desired_ping_state_yaml: String,
    pub notification_yamls: BTreeMap<String, String>,
}

impl ConfigSource {
    /// Args: `config` contains the config repository URL, cluster name, and optional auth details.
    /// Builds a source client for fetching PingPongKong state files.
    pub fn new(config: SourceConfig) -> Self {
        Self { config }
    }

    /// Args: none.
    /// Updates the local Git checkout and returns the validated runtime config bundle.
    pub async fn load(&self) -> anyhow::Result<LoadedConfig> {
        debug!(
            cluster = %self.config.cluster_name,
            checkout_dir = %self.config.checkout_dir,
            "loading config source"
        );
        self.retrieve_git_data().await?;

        let checkout = PathBuf::from(&self.config.checkout_dir);
        let (desired_ping_state_yaml, desired_ping_state) =
            self.get_cluster_ping_state_dto(&checkout).await?;
        let (notification_yamls, desired_notification_state) =
            self.get_notification_state_dto_list(&checkout).await?;

        let synced_at = Utc::now();
        let config_hash = config_hash(&desired_ping_state_yaml, &notification_yamls);
        let revision = format!("config-{}", config_hash);
        debug!(
            revision = %revision,
            config_hash = %config_hash,
            notification_files = notification_yamls.len(),
            "loaded config source"
        );

        Ok(LoadedConfig {
            published: PublishedConfig {
                desired_ping_state,
                desired_notification_state,
                revision,
                config_hash,
                synced_at,
            },
            desired_ping_state_yaml,
            notification_yamls,
        })
    }

    /// Args: none.
    /// Clones the config repository once, then force-updates the checkout from origin on later runs.
    async fn retrieve_git_data(&self) -> anyhow::Result<()> {
        let checkout = PathBuf::from(&self.config.checkout_dir);
        let git_dir = checkout.join(".git");

        if !git_dir.exists() {
            info!(
                checkout_dir = %checkout.display(),
                "config repository checkout missing; cloning"
            );
            if let Some(parent) = checkout.parent() {
                fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("failed to create '{}'", parent.display()))?;
            }
            run_git(
                Command::new("git")
                    .arg("clone")
                    .arg(authenticated_git_url(
                        &self.config.git_url,
                        self.config.token.as_deref(),
                    )?)
                    .arg(&checkout),
            )
            .await
            .context("failed to clone config repository")?;
        } else {
            debug!(
                checkout_dir = %checkout.display(),
                "config repository checkout exists"
            );
        }

        debug!(checkout_dir = %checkout.display(), "fetching config repository");
        run_git(
            Command::new("git")
                .arg("-C")
                .arg(&checkout)
                .arg("fetch")
                .arg("--prune")
                .arg("origin"),
        )
        .await
        .context("failed to fetch config repository")?;
        debug!(checkout_dir = %checkout.display(), "resetting config repository to origin/HEAD");
        run_git(
            Command::new("git")
                .arg("-C")
                .arg(&checkout)
                .arg("reset")
                .arg("--hard")
                .arg("origin/HEAD"),
        )
        .await
        .context("failed to reset config repository to origin/HEAD")?;
        debug!(checkout_dir = %checkout.display(), "cleaning config repository checkout");
        run_git(
            Command::new("git")
                .arg("-C")
                .arg(&checkout)
                .arg("clean")
                .arg("-fd"),
        )
        .await
        .context("failed to clean config repository")?;

        Ok(())
    }

    /// Args: `checkout` points at the updated config repository checkout.
    /// Reads k8s/{cluster}.yaml, k8s/{cluster}.yml, default.yaml, or default.yml in that order.
    async fn get_cluster_ping_state_dto(
        &self,
        checkout: &Path,
    ) -> anyhow::Result<(String, DesiredPingState)> {
        let k8s_dir = checkout.join("k8s");
        let candidates = [
            k8s_dir.join(format!("{}.yaml", self.config.cluster_name)),
            k8s_dir.join(format!("{}.yml", self.config.cluster_name)),
            k8s_dir.join("default.yaml"),
            k8s_dir.join("default.yml"),
        ];

        let path = candidates
            .iter()
            .find(|path| path.exists())
            .with_context(|| {
                format!(
                    "no desired ping state found for cluster '{}' or default in '{}'",
                    self.config.cluster_name,
                    k8s_dir.display()
                )
            })?;
        debug!(
            path = %path.display(),
            cluster = %self.config.cluster_name,
            "reading desired ping state"
        );

        let yaml = fs::read_to_string(path)
            .await
            .with_context(|| format!("failed to read '{}'", path.display()))?;
        let state: DesiredPingState = serde_yaml::from_str(&yaml)
            .with_context(|| format!("failed to parse '{}'", path.display()))?;
        state.validate()?;
        debug!(
            path = %path.display(),
            cluster = %state.cluster,
            environment = ?state.environment,
            internal_rules = state.matrix.internal.len(),
            external_rules = state.matrix.external.len(),
            "desired ping state parsed"
        );

        Ok((yaml, state))
    }

    /// Args: `checkout` points at the updated config repository checkout.
    /// Reads all YAML files from notification/ and returns the aggregate notification state.
    async fn get_notification_state_dto_list(
        &self,
        checkout: &Path,
    ) -> anyhow::Result<(BTreeMap<String, String>, DesiredNotificationState)> {
        let notification_dir = checkout.join("notification");
        let mut entries = fs::read_dir(&notification_dir)
            .await
            .with_context(|| format!("failed to read '{}'", notification_dir.display()))?;
        let mut notification_yamls = BTreeMap::new();
        let mut notification_files = BTreeMap::new();

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if !is_yaml_file(&path) {
                debug!(path = %path.display(), "skipping non-yaml notification file");
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .context("notification file name must be valid UTF-8")?
                .to_string();
            let yaml = fs::read_to_string(&path)
                .await
                .with_context(|| format!("failed to read '{}'", path.display()))?;
            let file: DesiredNotificationFile = serde_yaml::from_str(&yaml)
                .with_context(|| format!("failed to parse '{}'", path.display()))?;
            debug!(
                name = %name,
                path = %path.display(),
                provider = %file.provider,
                "notification file parsed"
            );

            notification_yamls.insert(name.clone(), yaml);
            notification_files.insert(name, file);
        }

        let state = DesiredNotificationState::from_files(notification_files)?;
        debug!(
            destinations = state.destinations.len(),
            "notification state aggregated"
        );
        Ok((notification_yamls, state))
    }
}

/// Args: `command` is a fully built Git command.
/// Runs Git and returns a compact error when the command fails.
async fn run_git(command: &mut Command) -> anyhow::Result<()> {
    let output = command.output().await.context("failed to start git")?;

    anyhow::ensure!(
        output.status.success(),
        "git exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

/// Args: `git_url` is the repository URL and `token` is the optional access token.
/// Embeds a token in HTTPS clone URLs so Git can authenticate private repositories.
fn authenticated_git_url(git_url: &str, token: Option<&str>) -> anyhow::Result<String> {
    let Some(token) = token else {
        return Ok(git_url.to_string());
    };
    let mut url = Url::parse(git_url).context("CONFIG_GIT_URL must be a valid URL")?;
    if url.scheme() != "https" {
        return Ok(git_url.to_string());
    }

    url.set_username("oauth2")
        .map_err(|_| anyhow::anyhow!("failed to set Git username"))?;
    url.set_password(Some(token))
        .map_err(|_| anyhow::anyhow!("failed to set Git token"))?;
    Ok(url.to_string())
}

/// Args: `path` is a candidate notification file path.
/// Returns true for .yaml and .yml files.
fn is_yaml_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
        })
}

/// Args: `desired_ping_state_yaml` and `notification_yamls` are the raw config file contents.
/// Returns a stable content fingerprint used to detect real config changes.
fn config_hash(
    desired_ping_state_yaml: &str,
    notification_yamls: &BTreeMap<String, String>,
) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;

    let mut hash = FNV_OFFSET;
    append_hash_bytes(&mut hash, desired_ping_state_yaml.as_bytes());
    append_hash_bytes(&mut hash, &[0xff]);

    for (name, yaml) in notification_yamls {
        append_hash_bytes(&mut hash, name.as_bytes());
        append_hash_bytes(&mut hash, &[0x00]);
        append_hash_bytes(&mut hash, yaml.as_bytes());
        append_hash_bytes(&mut hash, &[0xff]);
    }

    format!("{hash:016x}")
}

/// Args: `hash` is the rolling FNV-1a state, `bytes` are appended to it.
/// Updates a stable content fingerprint with one byte slice.
fn append_hash_bytes(hash: &mut u64, bytes: &[u8]) {
    const FNV_PRIME: u64 = 0x100000001b3;

    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}
