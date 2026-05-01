use crate::infra::config_source::LoadedConfig;
use crate::models::DesiredPingState;
use anyhow::Context;
use k8s_openapi::api::core::v1::{ConfigMap, Node};
use kube::{
    Api, Client,
    api::{ObjectMeta, Patch, PatchParams},
};
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct K8sClient {
    namespace: String,
    config_maps: Api<ConfigMap>,
    nodes: Api<Node>,
}

#[derive(Debug, Clone)]
pub struct K8sNode {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub ip_address: String,
}

impl K8sClient {
    /// Args: `namespace` is the Kubernetes namespace where collector resources live.
    /// Creates typed Kubernetes API clients for ConfigMaps and agent Pods.
    pub async fn new(namespace: String) -> anyhow::Result<Self> {
        let client = Client::try_default()
            .await
            .context("failed to create Kubernetes client")?;

        Ok(Self {
            config_maps: Api::namespaced(client.clone(), &namespace),
            nodes: Api::all(client),
            namespace,
        })
    }

    /// Args: `name` is the ConfigMap name.
    /// Returns the previous desired ping state when the ConfigMap exists and contains normalized config.
    pub async fn get_configmap(&self, name: &str) -> anyhow::Result<Option<DesiredPingState>> {
        let config_map = match self.config_maps.get_opt(name).await {
            Ok(config_map) => config_map,
            Err(err) => {
                return Err(err).with_context(|| format!("failed to get ConfigMap '{}'", name));
            }
        };

        let Some(config_map) = config_map else {
            return Ok(None);
        };
        let Some(data) = config_map.data else {
            return Ok(None);
        };

        if let Some(normalized) = data.get("normalized.json") {
            let published: crate::models::PublishedConfig = serde_json::from_str(normalized)
                .context("failed to parse ConfigMap normalized.json")?;
            return Ok(Some(published.desired_ping_state));
        }

        if let Some(yaml) = data.get("desiredPingState.yaml") {
            let state: DesiredPingState = serde_yaml::from_str(yaml)
                .context("failed to parse ConfigMap desiredPingState.yaml")?;
            return Ok(Some(state));
        }

        Ok(None)
    }

    /// Args: `name` is the target ConfigMap name, `loaded` is the validated state config bundle.
    /// Applies the latest desired ping state, notification configs, normalized JSON, hash, revision, and sync timestamp.
    pub async fn update_configmap(&self, name: &str, loaded: &LoadedConfig) -> anyhow::Result<()> {
        let normalized =
            serde_json::to_string_pretty(&loaded.published).context("serialize config")?;

        let mut data = BTreeMap::from([
            (
                "desiredPingState.yaml".to_string(),
                loaded.desired_ping_state_yaml.clone(),
            ),
            ("normalized.json".to_string(), normalized),
            ("revision".to_string(), loaded.published.revision.clone()),
            (
                "config_hash".to_string(),
                loaded.published.config_hash.clone(),
            ),
            (
                "synced_at".to_string(),
                loaded.published.synced_at.to_rfc3339(),
            ),
        ]);
        for (notification_name, yaml) in &loaded.notification_yamls {
            data.insert(
                format!("notification/{notification_name}.yaml"),
                yaml.clone(),
            );
        }

        let config_map = ConfigMap {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(self.namespace.clone()),
                labels: Some(BTreeMap::from([
                    (
                        "app.kubernetes.io/name".to_string(),
                        "pingpongkong".to_string(),
                    ),
                    (
                        "app.kubernetes.io/component".to_string(),
                        "collector-config".to_string(),
                    ),
                ])),
                ..ObjectMeta::default()
            },
            data: Some(data),
            ..ConfigMap::default()
        };

        self.config_maps
            .patch(
                name,
                &PatchParams::apply("pingpongkong-k8s-collector").force(),
                &Patch::Apply(&config_map),
            )
            .await
            .with_context(|| format!("failed to apply ConfigMap '{}'", name))?;

        Ok(())
    }

    /// Args: none.
    /// Lists Kubernetes nodes with name, labels, and the best available IP address.
    pub async fn list_nodes(&self) -> anyhow::Result<Vec<K8sNode>> {
        let nodes = self
            .nodes
            .list(&Default::default())
            .await
            .context("failed to list Kubernetes nodes")?;

        let k8s_nodes = nodes
            .items
            .into_iter()
            .filter_map(|node| {
                let name = node.metadata.name.clone()?;
                let labels = node.metadata.labels.clone().unwrap_or_default();
                let ip_address = node_ip_address(&node)?;
                Some(K8sNode {
                    name,
                    labels,
                    ip_address,
                })
            })
            .collect();

        Ok(k8s_nodes)
    }
}

/// Args: `node` is a Kubernetes node resource.
/// Returns InternalIP first, then ExternalIP when InternalIP is unavailable.
fn node_ip_address(node: &Node) -> Option<String> {
    let addresses = node.status.as_ref()?.addresses.as_ref()?;
    addresses
        .iter()
        .find(|address| address.type_ == "InternalIP")
        .or_else(|| {
            addresses
                .iter()
                .find(|address| address.type_ == "ExternalIP")
        })
        .map(|address| address.address.clone())
}
