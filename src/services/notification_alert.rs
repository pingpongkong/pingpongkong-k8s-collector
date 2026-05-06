use crate::models::{
    ConnectivityReport, DiscordWebhookPayload, NodeHealthStatus, NotificationDestination,
};
use anyhow::Context;
use reqwest::{Client, multipart};
use serde_json::json;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use tracing::{debug, warn};

#[derive(Clone)]
pub struct NotificationAlerter {
    client: Client,
}

impl NotificationAlerter {
    /// Args: none.
    /// Builds the reusable notification HTTP client.
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Args: `name` identifies the destination and `destination` describes the provider.
    /// Sends a collector-updated notification when the destination provider is supported.
    pub async fn send_collector_updated(
        &self,
        name: &str,
        destination: &NotificationDestination,
    ) -> anyhow::Result<bool> {
        self.send_text_notification(
            name,
            destination,
            "PingPongKong collector updated",
            "**PingPongKong collector updated**",
            "collector update",
        )
        .await
    }

    /// Args: `name` identifies the destination, `destination` describes the provider, and `error` explains the failure.
    /// Sends a collector sync failure notification when the destination provider is supported.
    pub async fn send_collector_sync_failed(
        &self,
        name: &str,
        destination: &NotificationDestination,
        error: &str,
    ) -> anyhow::Result<bool> {
        self.send_text_notification(
            name,
            destination,
            "PingPongKong collector sync failed",
            &format!("**PingPongKong collector sync failed**\nerror: `{error}`"),
            "collector sync failure",
        )
        .await
    }

    /// Args: `name` identifies the destination, `destination` describes the provider, and `report` is the latest report.
    /// Sends a connectivity report notification.
    pub async fn send_connectivity_report(
        &self,
        name: &str,
        destination: &NotificationDestination,
        report: &ConnectivityReport,
    ) -> anyhow::Result<bool> {
        if destination.provider.eq_ignore_ascii_case("discord") {
            debug!(
                destination = %name,
                cluster = %report.cluster_name,
                health = ?report.health_status(),
                "sending Discord connectivity report"
            );
            self.send_discord_report(destination, report).await?;
            return Ok(true);
        }

        if destination.provider.eq_ignore_ascii_case("teams") {
            debug!(
                destination = %name,
                cluster = %report.cluster_name,
                health = ?report.health_status(),
                "sending Teams connectivity report"
            );
            self.send_teams_message(destination, &report_message(report))
                .await?;
            return Ok(true);
        }

        if destination.provider.eq_ignore_ascii_case("email") {
            debug!(
                destination = %name,
                cluster = %report.cluster_name,
                health = ?report.health_status(),
                "sending email connectivity report"
            );
            self.send_email_report(destination, report).await?;
            return Ok(true);
        }

        if destination.provider.eq_ignore_ascii_case("telegram") {
            debug!(
                destination = %name,
                cluster = %report.cluster_name,
                health = ?report.health_status(),
                "sending Telegram connectivity report"
            );
            self.send_telegram_message(destination, &report_message(report))
                .await?;
            return Ok(true);
        }

        if destination.provider.eq_ignore_ascii_case("sms") {
            debug!(
                destination = %name,
                cluster = %report.cluster_name,
                health = ?report.health_status(),
                "sending SMS connectivity report"
            );
            self.send_sms_report(destination, report).await?;
            return Ok(true);
        }

        warn!(
            destination = %name,
            provider = %destination.provider,
            "connectivity report notification provider is configured but not implemented"
        );
        Ok(false)
    }

    /// Args: `name` identifies the destination, `destination` describes the provider, `subject` and `content` are the message.
    /// Sends a concise text notification through providers that accept text messages.
    async fn send_text_notification(
        &self,
        name: &str,
        destination: &NotificationDestination,
        subject: &str,
        content: &str,
        event: &str,
    ) -> anyhow::Result<bool> {
        if destination.provider.eq_ignore_ascii_case("discord") {
            debug!(
                destination = %name,
                provider = %destination.provider,
                "sending Discord text notification"
            );
            self.send_discord_message(destination, content).await?;
            return Ok(true);
        }

        if destination.provider.eq_ignore_ascii_case("teams") {
            debug!(
                destination = %name,
                provider = %destination.provider,
                "sending Teams text notification"
            );
            self.send_teams_message(destination, content).await?;
            return Ok(true);
        }

        if destination.provider.eq_ignore_ascii_case("email") {
            debug!(
                destination = %name,
                provider = %destination.provider,
                "sending email text notification"
            );
            self.send_email_message(destination, subject, content)
                .await?;
            return Ok(true);
        }

        if destination.provider.eq_ignore_ascii_case("telegram") {
            debug!(
                destination = %name,
                provider = %destination.provider,
                "sending Telegram text notification"
            );
            self.send_telegram_message(destination, content).await?;
            return Ok(true);
        }

        if destination.provider.eq_ignore_ascii_case("sms") {
            debug!(
                destination = %name,
                provider = %destination.provider,
                "sending SMS text notification"
            );
            self.send_sms_message(destination, content).await?;
            return Ok(true);
        }

        warn!(
            destination = %name,
            provider = %destination.provider,
            event,
            "notification provider is configured but not implemented"
        );
        Ok(false)
    }

    /// Args: `destination` is one Discord notification destination and `content` is the message body.
    /// Sends a simple Discord webhook message.
    async fn send_discord_message(
        &self,
        destination: &NotificationDestination,
        content: &str,
    ) -> anyhow::Result<()> {
        let webhook = destination
            .webhook
            .as_ref()
            .context("discord notification destination requires webhook settings")?;
        let webhook_url = webhook_url(&webhook.env_var)?;

        let payload = DiscordWebhookPayload {
            content: content.to_string(),
            username: destination.display.username.clone(),
            avatar_url: destination.display.avatar_url.clone(),
        };

        debug!("posting Discord webhook message");
        let response = self.client.post(webhook_url).json(&payload).send().await?;
        ensure_success(response, "discord webhook").await
    }

    /// Args: `destination` is one Teams notification destination and `content` is the message body.
    /// Sends a simple Teams-compatible webhook message.
    async fn send_teams_message(
        &self,
        destination: &NotificationDestination,
        content: &str,
    ) -> anyhow::Result<()> {
        let webhook = destination
            .webhook
            .as_ref()
            .context("teams notification destination requires webhook settings")?;
        let webhook_url = webhook_url(&webhook.env_var)?;
        let response = self
            .client
            .post(webhook_url)
            .json(&json!({ "text": content }))
            .send()
            .await?;
        ensure_success(response, "teams webhook").await
    }

    /// Args: `destination` is one email webhook destination, `subject` and `content` are the email fields.
    /// Sends a generic email webhook payload.
    async fn send_email_message(
        &self,
        destination: &NotificationDestination,
        subject: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        let webhook = destination
            .webhook
            .as_ref()
            .context("email notification destination requires webhook settings")?;
        let webhook_url = webhook_url(&webhook.env_var)?;
        let response = self
            .client
            .post(webhook_url)
            .json(&json!({
                "subject": subject,
                "message": content,
            }))
            .send()
            .await?;
        ensure_success(response, "email webhook").await
    }

    /// Args: `destination` is one Telegram notification destination and `content` is the message body.
    /// Sends a Telegram Bot API sendMessage request.
    async fn send_telegram_message(
        &self,
        destination: &NotificationDestination,
        content: &str,
    ) -> anyhow::Result<()> {
        let telegram = destination
            .telegram
            .as_ref()
            .context("telegram notification destination requires telegram settings")?;
        let token = telegram_config_value(
            &telegram.bot_token,
            &telegram.bot_token_env_var,
            "Telegram bot token",
        )?;
        let chat_id = telegram_config_value(
            &telegram.chat_id,
            &telegram.chat_id_env_var,
            "Telegram chat id",
        )?;
        let response = self
            .client
            .post(format!("https://api.telegram.org/bot{token}/sendMessage"))
            .json(&json!({
                "chat_id": chat_id,
                "text": content,
            }))
            .send()
            .await?;
        ensure_success(response, "telegram api").await
    }

    /// Args: `destination` is one Discord notification destination and `report` is the latest connectivity report.
    /// Sends a short Discord message with the full report attached as JSON.
    async fn send_discord_report(
        &self,
        destination: &NotificationDestination,
        report: &ConnectivityReport,
    ) -> anyhow::Result<()> {
        let webhook = destination
            .webhook
            .as_ref()
            .context("discord notification destination requires webhook settings")?;
        let webhook_url = webhook_url(&webhook.env_var)?;
        let report_json = serde_json::to_string_pretty(report)?;
        let report_bytes = report_json.len();
        let health = report.health_status();
        let payload = json!({
            "content": format!(
                "**PingPongKong report**\ncluster: `{}`\nhealth: `{}`",
                report.cluster_name,
                health_label(&health)
            ),
            "username": destination.display.username.clone(),
            "avatar_url": destination.display.avatar_url.clone(),
        });

        let form = multipart::Form::new()
            .text("payload_json", payload.to_string())
            .part(
                "files[0]",
                multipart::Part::bytes(report_json.into_bytes())
                    .file_name("connectivity-report.json")
                    .mime_str("application/json")?,
            );

        debug!(
            cluster = %report.cluster_name,
            report_bytes,
            "posting Discord webhook report attachment"
        );
        let response = self.client.post(webhook_url).multipart(form).send().await?;
        ensure_success(response, "discord webhook").await
    }

    /// Args: `destination` is one email webhook destination and `report` is the latest connectivity report.
    /// Sends a generic email webhook payload with the report embedded as JSON.
    async fn send_email_report(
        &self,
        destination: &NotificationDestination,
        report: &ConnectivityReport,
    ) -> anyhow::Result<()> {
        let webhook = destination
            .webhook
            .as_ref()
            .context("email notification destination requires webhook settings")?;
        let webhook_url = webhook_url(&webhook.env_var)?;
        let response = self
            .client
            .post(webhook_url)
            .json(&json!({
                "subject": format!("PingPongKong {} {}", report.cluster_name, health_label(&report.health_status())),
                "message": report_message(report),
                "report": report,
            }))
            .send()
            .await?;
        ensure_success(response, "email webhook").await
    }

    /// Args: `destination` is an SMS-like webhook destination and `report` is the latest connectivity report.
    /// Sends a concise cluster health message plus report URL.
    async fn send_sms_report(
        &self,
        destination: &NotificationDestination,
        report: &ConnectivityReport,
    ) -> anyhow::Result<()> {
        let webhook = destination
            .webhook
            .as_ref()
            .context("sms notification destination requires webhook settings")?;
        let webhook_url = webhook_url(&webhook.env_var)?;
        let report_url = std::env::var("REPORT_URL").unwrap_or_else(|_| "/report".to_string());
        let message = format!(
            "PingPongKong {} {} report: {}",
            report.cluster_name,
            health_label(&report.health_status()),
            report_url
        );

        let response = self
            .client
            .post(webhook_url)
            .json(&json!({ "message": message }))
            .send()
            .await?;
        debug!(
            cluster = %report.cluster_name,
            "posted SMS webhook report"
        );
        ensure_success(response, "sms webhook").await
    }

    /// Args: `destination` is an SMS-like webhook destination and `message` is the text body.
    /// Sends a concise text message through a generic SMS webhook payload.
    async fn send_sms_message(
        &self,
        destination: &NotificationDestination,
        message: &str,
    ) -> anyhow::Result<()> {
        let webhook = destination
            .webhook
            .as_ref()
            .context("sms notification destination requires webhook settings")?;
        let webhook_url = webhook_url(&webhook.env_var)?;
        let response = self
            .client
            .post(webhook_url)
            .json(&json!({ "message": message }))
            .send()
            .await?;
        ensure_success(response, "sms webhook").await
    }
}

/// Args: `value` is either a webhook URL or the name of an environment variable containing one.
/// Resolves the configured webhook value into the URL used for the provider request.
fn webhook_url(value: &str) -> anyhow::Result<String> {
    if value.starts_with("http://") || value.starts_with("https://") {
        return Ok(value.to_string());
    }

    std::env::var(value).with_context(|| format!("{value} must contain the webhook URL"))
}

/// Args: `plain_text` is an optional literal secret value, `env_var` is an optional environment variable name, and `label` describes the secret.
/// Resolves a Telegram setting from plaintext first, then an environment variable.
fn telegram_config_value(
    plain_text: &Option<String>,
    env_var: &Option<String>,
    label: &str,
) -> anyhow::Result<String> {
    if let Some(value) = plain_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(value.to_string());
    }

    let env_var = env_var
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{label} must be configured with a plaintext value or env var"))?;

    std::env::var(env_var).with_context(|| format!("{env_var} must contain the {label}"))
}

#[derive(Debug, Default)]
pub struct AlertRateLimiter {
    sent: HashMap<String, VecDeque<Instant>>,
    sends_since_prune: usize,
}

impl AlertRateLimiter {
    /// Args: `key` identifies the alert bucket, `max` and `window` define the allowed burst.
    /// Returns true when the alert may be sent and records the send time in O(1) amortized time.
    pub fn allow(&mut self, key: &str, max: usize, window: Duration) -> bool {
        let now = Instant::now();
        let entries = self.sent.entry(key.to_string()).or_default();

        while entries
            .front()
            .is_some_and(|sent_at| now.duration_since(*sent_at) > window)
        {
            entries.pop_front();
        }

        if entries.len() >= max {
            return false;
        }

        entries.push_back(now);
        self.sends_since_prune += 1;
        if self.sends_since_prune >= 1_024 {
            self.prune_idle(now, window);
            self.sends_since_prune = 0;
        }
        true
    }

    /// Args: `window` is the active rate-limit window from the alert config.
    /// Performs scheduled cleanup so stale alert keys are released even during quiet periods.
    pub fn prune_expired(&mut self, window: Duration) {
        self.prune_idle(Instant::now(), window);
        self.sends_since_prune = 0;
    }

    /// Args: `now` is the current monotonic time, `window` is the active rate-limit window.
    /// Removes stale alert buckets in a batched O(number_of_keys) maintenance pass.
    fn prune_idle(&mut self, now: Instant, window: Duration) {
        self.sent.retain(|_, entries| {
            while entries
                .front()
                .is_some_and(|sent_at| now.duration_since(*sent_at) > window)
            {
                entries.pop_front();
            }

            !entries.is_empty()
        });
    }
}

/// Args: `health` is an aggregate report health.
/// Returns a compact stable label for notifications.
fn health_label(health: &NodeHealthStatus) -> &'static str {
    match health {
        NodeHealthStatus::Healthy => "Healthy",
        NodeHealthStatus::Unreachable => "Unreachable",
        NodeHealthStatus::Degraded => "Degraded",
    }
}

/// Args: `report` is the latest connectivity report.
/// Returns a compact human-readable report summary.
fn report_message(report: &ConnectivityReport) -> String {
    format!(
        "PingPongKong report\ncluster: {}\nhealth: {}",
        report.cluster_name,
        health_label(&report.health_status())
    )
}

/// Args: `response` is a provider response and `provider` labels the error.
/// Converts non-success webhook responses into compact errors.
async fn ensure_success(response: reqwest::Response, provider: &str) -> anyhow::Result<()> {
    let status = response.status();
    let body = truncate(response.text().await.unwrap_or_default(), 512);

    anyhow::ensure!(
        status.is_success(),
        "{} returned {}: {}",
        provider,
        status,
        body
    );

    Ok(())
}

/// Args: `value` is an arbitrary response body, `max_chars` is the maximum retained length.
/// Truncates large webhook error bodies before placing them into logs or errors.
fn truncate(value: String, max_chars: usize) -> String {
    if value.len() <= max_chars {
        return value;
    }

    let mut truncated = value;
    truncated.truncate(max_chars);
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Args: none.
    /// Verifies that alert streams are rate-limited independently by key.
    #[test]
    fn rate_limiter_blocks_after_limit() {
        let mut limiter = AlertRateLimiter::default();
        let window = Duration::from_secs(60);

        assert!(limiter.allow("redis", 2, window));
        assert!(limiter.allow("redis", 2, window));
        assert!(!limiter.allow("redis", 2, window));
        assert!(limiter.allow("postgres", 2, window));
    }
}
