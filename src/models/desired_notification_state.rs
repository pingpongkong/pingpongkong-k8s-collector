use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DesiredNotificationState {
    pub version: String,
    #[serde(default)]
    pub destinations: BTreeMap<String, NotificationDestination>,
}

impl DesiredNotificationState {
    /// Args: `files` contains notification files keyed by configured destination name.
    /// Builds the aggregate notification state used at runtime.
    pub fn from_files(files: BTreeMap<String, DesiredNotificationFile>) -> anyhow::Result<Self> {
        let mut version = None;
        let mut destinations = BTreeMap::new();

        for (name, file) in files {
            if let Some(existing) = &version {
                anyhow::ensure!(
                    existing == &file.version,
                    "notification file '{}' version '{}' does not match '{}'",
                    name,
                    file.version,
                    existing
                );
            } else {
                version = Some(file.version.clone());
            }

            destinations.insert(name, file.into_destination());
        }

        let state = Self {
            version: version.unwrap_or_default(),
            destinations,
        };
        state.validate()?;
        Ok(state)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DesiredNotificationFile {
    pub version: String,
    pub provider: String,
    #[serde(default)]
    pub webhook: Option<Webhook>,
    #[serde(default)]
    pub telegram: Option<Telegram>,
    #[serde(default)]
    pub display: DisplaySettings,
    #[serde(default)]
    pub rate_limit: NotificationRateLimit,
}

impl DesiredNotificationFile {
    /// Args: none.
    /// Converts one notification/{name}.yaml file into a named runtime destination.
    fn into_destination(self) -> NotificationDestination {
        NotificationDestination {
            provider: self.provider,
            webhook: self.webhook,
            telegram: self.telegram,
            display: self.display,
            rate_limit: self.rate_limit,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NotificationDestination {
    pub provider: String,
    #[serde(default)]
    pub webhook: Option<Webhook>,
    #[serde(default)]
    pub telegram: Option<Telegram>,
    #[serde(default)]
    pub display: DisplaySettings,
    #[serde(default)]
    pub rate_limit: NotificationRateLimit,
}

impl DesiredNotificationState {
    /// Args: none.
    /// Validates notification destinations and their throttling settings.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.version.trim().is_empty(),
            "desired notification state version cannot be empty"
        );
        anyhow::ensure!(
            !self.destinations.is_empty(),
            "desired notification state must contain at least one destination"
        );

        for (name, destination) in &self.destinations {
            destination.validate(name)?;
        }

        Ok(())
    }
}

impl NotificationDestination {
    /// Args: `name` identifies this destination in validation messages.
    /// Validates one configured notification destination.
    fn validate(&self, name: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            !name.trim().is_empty(),
            "desired notification destination name cannot be empty"
        );
        anyhow::ensure!(
            !self.provider.trim().is_empty(),
            "desired notification destination '{}' provider cannot be empty",
            name
        );

        if provider_uses_webhook(&self.provider) {
            let webhook = self.webhook.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "desired notification destination '{}' webhook is required for provider '{}'",
                    name,
                    self.provider
                )
            })?;

            anyhow::ensure!(
                !webhook.env_var.trim().is_empty(),
                "desired notification destination '{}' webhook.env_var cannot be empty",
                name
            );
        }

        if self.provider.eq_ignore_ascii_case("telegram") {
            let telegram = self.telegram.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "desired notification destination '{}' telegram is required for provider '{}'",
                    name,
                    self.provider
                )
            })?;

            anyhow::ensure!(
                !telegram.bot_token_env_var.trim().is_empty(),
                "desired notification destination '{}' telegram.bot_token_env_var cannot be empty",
                name
            );
            anyhow::ensure!(
                !telegram.chat_id_env_var.trim().is_empty(),
                "desired notification destination '{}' telegram.chat_id_env_var cannot be empty",
                name
            );
        }

        anyhow::ensure!(
            self.rate_limit.max_notifications_per_minute > 0,
            "desired notification destination '{}' rate_limit.max_notifications_per_minute must be greater than zero",
            name
        );
        anyhow::ensure!(
            self.rate_limit.repeat_interval_minutes > 0,
            "desired notification destination '{}' rate_limit.repeat_interval_minutes must be greater than zero",
            name
        );

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Webhook {
    pub env_var: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Telegram {
    pub bot_token_env_var: String,
    pub chat_id_env_var: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DisplaySettings {
    #[serde(default = "default_username")]
    pub username: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

impl Default for DisplaySettings {
    /// Args: none.
    /// Creates the default notification display settings.
    fn default() -> Self {
        Self {
            username: default_username(),
            avatar_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NotificationRateLimit {
    #[serde(default = "default_max_notifications_per_minute")]
    pub max_notifications_per_minute: usize,
    #[serde(default = "default_repeat_interval_minutes")]
    pub repeat_interval_minutes: u64,
}

impl Default for NotificationRateLimit {
    /// Args: none.
    /// Creates the default notification throttling settings.
    fn default() -> Self {
        Self {
            max_notifications_per_minute: default_max_notifications_per_minute(),
            repeat_interval_minutes: default_repeat_interval_minutes(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscordWebhookPayload {
    pub content: String,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// Args: none.
/// Returns the default username shown on notification messages.
fn default_username() -> String {
    "PingPongKong".to_string()
}

/// Args: none.
/// Returns the default maximum notifications per minute.
fn default_max_notifications_per_minute() -> usize {
    5
}

/// Args: none.
/// Returns the default repeat interval for identical notification streams.
fn default_repeat_interval_minutes() -> u64 {
    30
}

/// Args: `provider` is the configured notification provider name.
/// Returns true when the destination sends through a generic webhook URL.
fn provider_uses_webhook(provider: &str) -> bool {
    provider.eq_ignore_ascii_case("discord")
        || provider.eq_ignore_ascii_case("teams")
        || provider.eq_ignore_ascii_case("email")
        || provider.eq_ignore_ascii_case("sms")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Args: none.
    /// Verifies that one notification file parses the provider-level shape.
    #[test]
    fn parses_desired_notification_file() {
        let file: DesiredNotificationFile = serde_yaml::from_str(
            r#"
version: "1.0"
provider: "discord"
webhook:
  env_var: "https://discord.com/api/webhooks/example"
display:
  username: "PingPongKong"
  avatar_url: "https://example.com/kong-avatar.png"
rate_limit:
  max_notifications_per_minute: 5
  repeat_interval_minutes: 30
"#,
        )
        .unwrap();

        assert_eq!(file.version, "1.0");
        assert_eq!(file.provider, "discord");
        assert_eq!(
            file.webhook.as_ref().unwrap().env_var,
            "https://discord.com/api/webhooks/example"
        );
        assert_eq!(file.display.username, "PingPongKong");
        assert_eq!(file.rate_limit.max_notifications_per_minute, 5);
        assert_eq!(file.rate_limit.repeat_interval_minutes, 30);
    }

    /// Args: none.
    /// Verifies that multiple notification files aggregate into named destinations.
    #[test]
    fn builds_desired_notification_state_from_files() {
        let files = BTreeMap::from([
            (
                "discord1".to_string(),
                DesiredNotificationFile {
                    version: "1.0".to_string(),
                    provider: "discord".to_string(),
                    webhook: Some(Webhook {
                        env_var: "DISCORD_WEBHOOK_URL_1".to_string(),
                    }),
                    telegram: None,
                    display: DisplaySettings::default(),
                    rate_limit: NotificationRateLimit::default(),
                },
            ),
            (
                "email".to_string(),
                DesiredNotificationFile {
                    version: "1.0".to_string(),
                    provider: "email".to_string(),
                    webhook: Some(Webhook {
                        env_var: "EMAIL_WEBHOOK_URL".to_string(),
                    }),
                    telegram: None,
                    display: DisplaySettings::default(),
                    rate_limit: NotificationRateLimit::default(),
                },
            ),
        ]);

        let state = DesiredNotificationState::from_files(files).unwrap();

        assert_eq!(state.version, "1.0");
        assert_eq!(state.destinations["discord1"].provider, "discord");
        assert_eq!(state.destinations["email"].provider, "email");
    }

    /// Args: none.
    /// Verifies default display and rate-limit values when optional sections are omitted.
    #[test]
    fn defaults_notification_settings() {
        let file: DesiredNotificationFile = serde_yaml::from_str(
            r#"
version: "1.0"
provider: "discord"
webhook:
  env_var: "DISCORD_WEBHOOK_URL"
"#,
        )
        .unwrap();

        assert_eq!(file.display.username, "PingPongKong");
        assert_eq!(file.display.avatar_url, None);
        assert_eq!(file.rate_limit.max_notifications_per_minute, 5);
        assert_eq!(file.rate_limit.repeat_interval_minutes, 30);
    }

    /// Args: none.
    /// Verifies that Telegram notification files validate their provider-specific settings.
    #[test]
    fn parses_telegram_notification_file() {
        let file: DesiredNotificationFile = serde_yaml::from_str(
            r#"
version: "1.0"
provider: "telegram"
telegram:
  bot_token_env_var: "TELEGRAM_BOT_TOKEN"
  chat_id_env_var: "TELEGRAM_CHAT_ID"
"#,
        )
        .unwrap();
        let state = DesiredNotificationState::from_files(BTreeMap::from([(
            "telegram".to_string(),
            file,
        )]))
        .unwrap();

        let destination = &state.destinations["telegram"];
        assert_eq!(destination.provider, "telegram");
        assert_eq!(
            destination.telegram.as_ref().unwrap().bot_token_env_var,
            "TELEGRAM_BOT_TOKEN"
        );
    }
}
