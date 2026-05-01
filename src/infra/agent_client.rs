use crate::{
    infra::K8sNode,
    models::{NodeHealthStatus, NodeStatus},
};
use anyhow::Context;
use reqwest::{Client, Url};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, warn};

#[derive(Clone)]
pub struct AgentClient {
    client: Client,
}

impl AgentClient {
    /// Args: none.
    /// Builds an HTTP client with a short timeout for checking node-local agents.
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .context("failed to build agent HTTP client")?,
        })
    }

    /// Args: `nodes` is the Kubernetes node list, `port` is the agent API port, and `max_concurrent` limits work.
    /// Gets /node-status from every node and returns one NodeStatus per Kubernetes node.
    pub async fn get_agent_data(
        &self,
        nodes: Vec<K8sNode>,
        port: u16,
        max_concurrent: usize,
    ) -> Vec<NodeStatus> {
        if nodes.is_empty() {
            debug!("no Kubernetes nodes found for agent checks");
            return Vec::new();
        }

        debug!(
            nodes = nodes.len(),
            port, max_concurrent, "starting agent checks"
        );
        let mut tasks = JoinSet::new();
        let permits = Arc::new(Semaphore::new(max_concurrent.max(1)));

        for node in nodes {
            let client = self.client.clone();
            let permits = Arc::clone(&permits);
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await.map_err(|err| {
                    NodeStatus::unreachable(
                        node.name.clone(),
                        node.labels.clone(),
                        node.ip_address.clone(),
                        format!("agent check limiter closed: {err}"),
                    )
                })?;
                get_node_status(client, node, port).await
            });
        }

        let mut results = Vec::with_capacity(tasks.len());
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok(status)) => {
                    debug!(
                        node = %status.node_name,
                        health = ?status.health_status,
                        targets = status.targets.len(),
                        "agent check succeeded"
                    );
                    results.push(status);
                }
                Ok(Err(status)) => {
                    warn!(
                        node = %status.node_name,
                        ip = %status.ip_address,
                        "agent check failed; marking node unreachable"
                    );
                    results.push(status);
                }
                Err(err) => warn!(error = %err, "agent check task failed"),
            }
        }

        debug!(statuses = results.len(), "agent checks completed");
        results
    }
}

/// Args: `client` is the reusable HTTP client, `node` is one Kubernetes node, and `port` is the agent API port.
/// Fetches one node status, or returns an Unreachable node status if the agent does not respond.
async fn get_node_status(
    client: Client,
    node: K8sNode,
    port: u16,
) -> Result<NodeStatus, NodeStatus> {
    let url = match build_agent_url(&node.ip_address, port) {
        Ok(url) => url,
        Err(err) => {
            return Err(NodeStatus::unreachable(
                node.name,
                node.labels,
                node.ip_address,
                err.to_string(),
            ));
        }
    };
    debug!(
        node = %node.name,
        ip = %node.ip_address,
        url = %url,
        "requesting node status from agent"
    );

    let response = match client.get(url.clone()).send().await {
        Ok(response) => response,
        Err(err) => {
            return Err(NodeStatus::unreachable(
                node.name,
                node.labels,
                node.ip_address,
                format!("agent request failed: {err}"),
            ));
        }
    };

    if !response.status().is_success() {
        return Err(NodeStatus::unreachable(
            node.name,
            node.labels,
            node.ip_address,
            format!("agent returned {}", response.status()),
        ));
    }

    match response.json::<NodeStatus>().await {
        Ok(mut status) => {
            status.node_name = node.name;
            status.labels = node.labels;
            status.ip_address = node.ip_address;
            status.health_status = derive_node_health_status(status.health_status, &status.targets);
            Ok(status)
        }
        Err(err) => Err(NodeStatus::unreachable(
            node.name,
            node.labels,
            node.ip_address,
            format!("agent response decode failed: {err}"),
        )),
    }
}

/// Args: `ip` is the node IP and `port` identifies the node-local agent API.
/// Builds the /node-status URL, including bracket handling for IPv6 addresses.
fn build_agent_url(ip: &str, port: u16) -> anyhow::Result<Url> {
    let host = if ip.contains(':') {
        format!("[{}]", ip)
    } else {
        ip.to_string()
    };

    Url::parse(&format!("http://{}:{}/node-status", host, port))
        .with_context(|| format!("failed to build agent URL for node IP '{}'", ip))
}

/// Args: `reported` is the agent-level status and `targets` are target check statuses.
/// Keeps explicit Unreachable, otherwise marks any failed target as Degraded.
fn derive_node_health_status(
    reported: NodeHealthStatus,
    targets: &[crate::models::TargetStatus],
) -> NodeHealthStatus {
    if matches!(reported, NodeHealthStatus::Unreachable) {
        return NodeHealthStatus::Unreachable;
    }

    if targets.iter().any(|target| {
        !matches!(
            target.health_status,
            crate::models::TargetHealthStatus::Healthy
        )
    }) {
        return NodeHealthStatus::Degraded;
    }

    NodeHealthStatus::Healthy
}
