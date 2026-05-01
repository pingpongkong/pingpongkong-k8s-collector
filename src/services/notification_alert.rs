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
        if destination.provider.eq_ignore_ascii_case("discord") {
            debug!(
                destination = %name,
                provider = %destination.provider,
                "sending collector-updated Discord notification"
            );
            self.send_discord_message(destination, "**PingPongKong collector updated**")
                .await?;
            return Ok(true);
        }

        warn!(
            destination = %name,
            provider = %destination.provider,
            "collector update notification provider is configured but not implemented"
        );
        Ok(false)
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
}

/// Args: `value` is either a webhook URL or the name of an environment variable containing one.
/// Resolves the configured webhook value into the URL used for the provider request.
fn webhook_url(value: &str) -> anyhow::Result<String> {
    if value.starts_with("http://") || value.starts_with("https://") {
        return Ok(value.to_string());
    }

    std::env::var(value).with_context(|| format!("{value} must contain the Discord webhook URL"))
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
