use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DesiredPingState {
    pub version: String,
    pub cluster: String,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub topology: Topology,
    pub matrix: PingMatrix,
}

impl DesiredPingState {
    /// Args: none.
    /// Validates that the desired ping state contains usable connectivity rules.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.version.trim().is_empty(),
            "desired ping state version cannot be empty"
        );
        anyhow::ensure!(
            !self.cluster.trim().is_empty(),
            "desired ping state cluster cannot be empty"
        );
        self.matrix.validate(&self.topology)?;

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Topology {
    #[serde(default, rename = "node-labels")]
    pub node_labels: BTreeMap<String, String>,
}

impl Default for Topology {
    /// Args: none.
    /// Creates an empty topology section for desired states that omit node labels.
    fn default() -> Self {
        Self {
            node_labels: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PingMatrix {
    #[serde(default)]
    pub internal: Vec<InternalRule>,
    #[serde(default)]
    pub external: Vec<ExternalRule>,
}

impl PingMatrix {
    /// Args: `topology` contains known node label aliases referenced by rules.
    /// Validates the internal and external rule sets.
    fn validate(&self, topology: &Topology) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.internal.is_empty() || !self.external.is_empty(),
            "desired ping state must contain at least one internal or external rule"
        );

        for rule in &self.internal {
            rule.validate(topology)?;
        }
        for rule in &self.external {
            rule.validate(topology)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct InternalRule {
    pub from: String,
    pub to: String,
    pub ports: Vec<u16>,
    #[serde(default)]
    pub protocol: PingRuleProtocol,
    pub action: PingRuleAction,
    #[serde(default)]
    pub description: Option<String>,
}

impl InternalRule {
    /// Args: `topology` contains known node label aliases referenced by this rule.
    /// Validates one node-to-node connectivity rule.
    fn validate(&self, topology: &Topology) -> anyhow::Result<()> {
        validate_topology_ref(topology, &self.from, "internal rule from")?;
        validate_topology_ref(topology, &self.to, "internal rule to")?;
        validate_ports(&self.ports, "internal rule")?;

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ExternalRule {
    #[serde(default)]
    pub name: Option<String>,
    pub from: String,
    pub endpoint: String,
    #[serde(default)]
    pub protocol: PingRuleProtocol,
    pub action: PingRuleAction,
    #[serde(default)]
    pub description: Option<String>,
}

impl ExternalRule {
    /// Args: `topology` contains known node label aliases referenced by this rule.
    /// Validates one outside-to-cluster connectivity rule.
    fn validate(&self, topology: &Topology) -> anyhow::Result<()> {
        validate_topology_ref(topology, &self.from, "external rule from")?;
        anyhow::ensure!(
            !self.endpoint.trim().is_empty(),
            "external rule endpoint cannot be empty"
        );
        anyhow::ensure!(
            endpoint_port(&self.endpoint).is_some(),
            "external rule endpoint '{}' must include a valid port",
            self.endpoint
        );

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PingRuleProtocol {
    #[default]
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PingRuleAction {
    Allow,
    Deny,
}

/// Args: `topology` contains known aliases, `value` is a rule endpoint alias, `field` names the field.
/// Ensures topology aliases are present and known when a topology map is configured.
fn validate_topology_ref(topology: &Topology, value: &str, field: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "{} cannot be empty", field);

    if !topology.node_labels.is_empty() {
        anyhow::ensure!(
            topology.node_labels.contains_key(value),
            "{} '{}' is not defined in topology.node-labels",
            field,
            value
        );
    }

    Ok(())
}

/// Args: `ports` is a rule port list, `label` names the rule in validation errors.
/// Ensures a rule has at least one non-zero TCP/UDP port.
fn validate_ports(ports: &[u16], label: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!ports.is_empty(), "{} ports cannot be empty", label);
    anyhow::ensure!(
        ports.iter().all(|port| *port > 0),
        "{} ports must be between 1 and 65535",
        label
    );

    Ok(())
}

/// Args: `endpoint` is an external endpoint in `host:port` form.
/// Returns the parsed port when the endpoint contains a valid non-zero port.
fn endpoint_port(endpoint: &str) -> Option<u16> {
    endpoint
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .filter(|port| *port > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Args: none.
    /// Verifies that the desired ping state parses the current topology and matrix shape.
    #[test]
    fn parses_desired_ping_state() {
        let state: DesiredPingState = serde_yaml::from_str(
            r#"
version: "1.0"
cluster: "sample-k8s-cluster"
environment: "prod"
description: "Production Kubernetes cluster using Cilium networking"
topology:
  node-labels:
    controlplane: "node-role.kubernetes.io/control-plane"
    etcd: "node-role.kubernetes.io/etcd"
    worker: "node-role.kubernetes.io/worker"
    monitoring: "monitoring"
    admin: "admin-cidr"
matrix:
  internal:
    - from: "worker"
      to: "controlplane"
      ports: [6443]
      protocol: "tcp"
      action: "allow"
      description: "Allow worker nodes to access the Kubernetes API server"
  external:
    - name: "Admin Kubernetes API"
      from: "admin"
      endpoint: "10.0.0.10:6443"
      protocol: "tcp"
      action: "allow"
      description: "Allow administrators to access the Kubernetes API using kubectl"
"#,
        )
        .unwrap();

        assert_eq!(state.version, "1.0");
        assert_eq!(state.cluster, "sample-k8s-cluster");
        assert_eq!(
            state.topology.node_labels["controlplane"],
            "node-role.kubernetes.io/control-plane"
        );
        assert_eq!(state.matrix.internal[0].ports, vec![6443]);
        assert!(matches!(
            state.matrix.external[0].protocol,
            PingRuleProtocol::Tcp
        ));
        state.validate().unwrap();
    }

    /// Args: none.
    /// Verifies that an empty matrix is rejected during validation.
    #[test]
    fn validates_empty_matrix() {
        let state: DesiredPingState = serde_yaml::from_str(
            r#"
version: "1.0"
cluster: "sample-k8s-cluster"
matrix:
  internal: []
  external: []
"#,
        )
        .unwrap();
        assert!(state.validate().is_err());
    }

    /// Args: none.
    /// Verifies that rules cannot reference missing topology aliases.
    #[test]
    fn validates_topology_refs() {
        let state: DesiredPingState = serde_yaml::from_str(
            r#"
version: "1.0"
cluster: "sample-k8s-cluster"
topology:
  node-labels:
    worker: "node-role.kubernetes.io/worker"
matrix:
  internal:
    - from: "worker"
      to: "controlplane"
      ports: [6443]
      action: "allow"
"#,
        )
        .unwrap();

        let err = state.validate().unwrap_err().to_string();
        assert!(err.contains("controlplane"));
    }

    /// Args: none.
    /// Verifies that external endpoints must include a parseable port.
    #[test]
    fn validates_external_endpoint_port() {
        let state: DesiredPingState = serde_yaml::from_str(
            r#"
version: "1.0"
cluster: "sample-k8s-cluster"
matrix:
  external:
    - from: "admin"
      endpoint: "10.0.0.10"
      action: "allow"
"#,
        )
        .unwrap();

        let err = state.validate().unwrap_err().to_string();
        assert!(err.contains("valid port"));
    }
}
