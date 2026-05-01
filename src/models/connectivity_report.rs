use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectivityReport {
    pub cluster_name: String,
    pub environment: String,
    pub node_statuses: Vec<NodeStatus>,
}

impl ConnectivityReport {
    /// Args: none.
    /// Returns the aggregate health for concise notifications.
    pub fn health_status(&self) -> NodeHealthStatus {
        if self
            .node_statuses
            .iter()
            .any(|node| matches!(node.health_status, NodeHealthStatus::Unreachable))
        {
            return NodeHealthStatus::Unreachable;
        }

        if self
            .node_statuses
            .iter()
            .any(|node| matches!(node.health_status, NodeHealthStatus::Degraded))
        {
            return NodeHealthStatus::Degraded;
        }

        NodeHealthStatus::Healthy
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeStatus {
    pub node_name: String,
    pub labels: BTreeMap<String, String>,
    pub ip_address: String,
    pub health_status: NodeHealthStatus,
    pub targets: Vec<TargetStatus>,
}

impl NodeStatus {
    /// Args: `node_name`, `labels`, and `ip_address` identify a Kubernetes node.
    /// Creates a timeout/no-response node status.
    pub fn unreachable(
        node_name: String,
        labels: BTreeMap<String, String>,
        ip_address: String,
        error_message: String,
    ) -> Self {
        Self {
            node_name,
            labels,
            ip_address,
            health_status: NodeHealthStatus::Unreachable,
            targets: vec![TargetStatus {
                target_name: "agent".to_string(),
                target_ip_address: String::new(),
                health_status: TargetHealthStatus::Unreachable,
                latency_ms: None,
                error_message: Some(error_message),
            }],
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum NodeHealthStatus {
    Healthy,
    Unreachable,
    Degraded,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TargetStatus {
    pub target_name: String,
    pub target_ip_address: String,
    pub health_status: TargetHealthStatus,
    pub latency_ms: Option<u64>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum TargetHealthStatus {
    Healthy,
    Unreachable,
    Failed,
}
