use crate::{
    config::SourceConfig,
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
    /// Args: `config` contains the config repository URL, file paths, and optional auth details.
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
        let url = raw_file_url(&self.config.git_url, path)?;
        let mut request = self.client.get(url);

        if let Some(token) = &self.config.token {
            request = apply_git_token(request, &self.config.git_url, token);
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

/// Args: `request` is the outbound request, `git_url` identifies the provider, and `token` is the Git token.
/// Applies the provider-specific token header for private repositories.
fn apply_git_token(
    request: reqwest::RequestBuilder,
    git_url: &str,
    token: &str,
) -> reqwest::RequestBuilder {
    if git_url_host(git_url).is_some_and(|host| host.eq_ignore_ascii_case("github.com")) {
        request.bearer_auth(token)
    } else {
        request.header("PRIVATE-TOKEN", token)
    }
}

/// Args: `git_url` is the repository URL, `path` is the config file path inside the repository.
/// Builds a raw-file URL using the repository default branch via `HEAD`.
fn raw_file_url(git_url: &str, path: &str) -> anyhow::Result<Url> {
    let repo = normalized_repo_url(git_url)?;
    let path = path.trim_start_matches('/');

    if repo
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
    {
        github_raw_url(repo, path)
    } else {
        gitlab_raw_url(repo, path)
    }
}

/// Args: `git_url` is the configured repository URL.
/// Parses and normalizes common copy-pasted repository URLs.
fn normalized_repo_url(git_url: &str) -> anyhow::Result<Url> {
    let mut repo = Url::parse(git_url.trim()).context("CONFIG_GIT_URL must be a valid URL")?;
    repo.set_query(None);
    repo.set_fragment(None);

    let normalized_path = repo
        .path()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string();
    repo.set_path(&normalized_path);

    Ok(repo)
}

/// Args: `repo` is a github.com repository URL, `path` is the file path inside the repository.
/// Returns the raw.githubusercontent.com URL for the repository default branch.
fn github_raw_url(repo: Url, path: &str) -> anyhow::Result<Url> {
    let segments: Vec<_> = repo
        .path_segments()
        .context("CONFIG_GIT_URL must include a GitHub owner and repository")?
        .filter(|segment| !segment.is_empty())
        .collect();

    anyhow::ensure!(
        segments.len() >= 2,
        "CONFIG_GIT_URL must include a GitHub owner and repository"
    );

    Url::parse(&format!(
        "https://raw.githubusercontent.com/{}/{}/HEAD/{}",
        segments[0], segments[1], path
    ))
    .context("failed to build GitHub raw URL")
}

/// Args: `repo` is a GitLab-compatible repository URL, `path` is the file path inside the repository.
/// Returns the GitLab raw URL for the repository default branch.
fn gitlab_raw_url(mut repo: Url, path: &str) -> anyhow::Result<Url> {
    let mut repo_path = repo.path().trim_end_matches('/').to_string();
    repo_path.push_str("/-/raw/HEAD/");
    repo_path.push_str(path);
    repo.set_path(&repo_path);
    Ok(repo)
}

/// Args: `git_url` is the configured repository URL.
/// Returns the lowercase host when the URL parses successfully.
fn git_url_host(git_url: &str) -> Option<String> {
    Url::parse(git_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
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

    /// Args: none.
    /// Verifies that GitHub repository URLs are converted to raw file URLs on the default branch.
    #[test]
    fn raw_file_url_supports_github_repo_urls() {
        let url = raw_file_url(
            "https://github.com/acme/pingpongkong-state.git",
            "k8s/prod.yaml",
        )
        .unwrap();

        assert_eq!(
            url.as_str(),
            "https://raw.githubusercontent.com/acme/pingpongkong-state/HEAD/k8s/prod.yaml"
        );
    }

    /// Args: none.
    /// Verifies that GitLab-compatible repository URLs are converted to raw file URLs on the default branch.
    #[test]
    fn raw_file_url_supports_gitlab_repo_urls() {
        let url = raw_file_url(
            "https://gitlab.company.com/group/subgroup/pingpongkong-state",
            "notification/discord.yaml",
        )
        .unwrap();

        assert_eq!(
            url.as_str(),
            "https://gitlab.company.com/group/subgroup/pingpongkong-state/-/raw/HEAD/notification/discord.yaml"
        );
    }
}
