use crate::models::{AgentResult, DiscordConfig, DiscordWebhookPayload};
use anyhow::Context;
use reqwest::Client;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct DiscordAlerter {
    client: Client,
}

impl DiscordAlerter {
    /// Args: none.
    /// Builds the reusable Discord webhook HTTP client.
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Args: `config` is the active Discord alert config, `result` is a failed probe result.
    /// Formats and sends one Discord failure notification when alerting is enabled.
    pub async fn send_failure(
        &self,
        config: &DiscordConfig,
        result: &AgentResult,
    ) -> anyhow::Result<()> {
        if !config.enabled {
            return Ok(());
        }

        let webhook_url = config
            .webhook_url
            .as_deref()
            .context("discord webhook_url is required when alerting is enabled")?;

        let payload = DiscordWebhookPayload {
            content: format_failure(result),
        };

        let response = self.client.post(webhook_url).json(&payload).send().await?;
        let status = response.status();
        let body = truncate(response.text().await.unwrap_or_default(), 512);

        anyhow::ensure!(
            status.is_success(),
            "discord webhook returned {}: {}",
            status,
            body
        );

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct AlertRateLimiter {
    sent: HashMap<String, VecDeque<Instant>>,
    sends_since_prune: usize,
}

impl AlertRateLimiter {
    /// Args: `key` identifies the alert stream, `max` and `window` define the allowed burst.
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

/// Args: `result` is a failed probe result from an agent.
/// Builds the Discord message body for a failed probe.
fn format_failure(result: &AgentResult) -> String {
    let target = result.target.as_deref().unwrap_or("unknown target");
    let message = result.message.as_deref().unwrap_or("probe failed");
    let latency = result
        .latency_ms
        .map(|value| format!("{value:.1}ms"))
        .unwrap_or_else(|| "n/a".to_string());

    format!(
        "**PingPongKong probe failed**\nagent: `{}`\ncheck: `{}`\ntarget: `{}`\nlatency: `{}`\nmessage: {}",
        result.agent, result.check_id, target, latency, message
    )
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
