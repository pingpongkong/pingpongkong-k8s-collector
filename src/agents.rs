use crate::models::{AgentResult, AgentResultsResponse};
use anyhow::Context;
use reqwest::{Client, Url};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

#[derive(Clone)]
pub struct AgentScraper {
    client: Client,
}

impl AgentScraper {
    /// Args: none.
    /// Builds an HTTP client with a short timeout for scraping local agent result endpoints.
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .context("failed to build agent scraper HTTP client")?,
        })
    }

    /// Args: `urls` is the agent URL list, `max_concurrent` limits in-flight HTTP requests.
    /// Scrapes agents concurrently with bounded work and returns one result bucket per request.
    pub async fn scrape(
        &self,
        urls: Vec<Url>,
        max_concurrent: usize,
    ) -> Vec<anyhow::Result<Vec<AgentResult>>> {
        if urls.is_empty() {
            return Vec::new();
        }

        let mut tasks = JoinSet::new();
        let permits = Arc::new(Semaphore::new(max_concurrent.max(1)));

        for url in urls {
            let client = self.client.clone();
            let permits = Arc::clone(&permits);
            tasks.spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .context("agent scrape limiter closed")?;
                scrape_one(client, url).await
            });
        }

        let mut results = Vec::with_capacity(tasks.len());
        while let Some(result) = tasks.join_next().await {
            results.push(
                result.unwrap_or_else(|err| Err(anyhow::anyhow!("scrape task failed: {err}"))),
            );
        }

        results
    }
}

/// Args: `client` is the reusable HTTP client, `url` is one agent JSON endpoint.
/// Fetches and decodes probe results from one agent.
async fn scrape_one(client: Client, url: Url) -> anyhow::Result<Vec<AgentResult>> {
    let response = client
        .get(url.clone())
        .send()
        .await
        .with_context(|| format!("failed to scrape agent at {}", url))?;

    let status = response.status();
    anyhow::ensure!(status.is_success(), "agent {} returned {}", url, status);

    let payload = response
        .json::<AgentResultsResponse>()
        .await
        .with_context(|| format!("failed to decode agent results from {}", url))?;

    Ok(payload.into_results())
}
