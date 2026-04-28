use crate::source::LoadedConfig;
use anyhow::Context;
use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use kube::{
    Api, Client,
    api::{ListParams, ObjectMeta, Patch, PatchParams},
};
use reqwest::Url;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct K8sClient {
    namespace: String,
    config_maps: Api<ConfigMap>,
    pods: Api<Pod>,
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
            pods: Api::namespaced(client, &namespace),
            namespace,
        })
    }

    /// Args: `name` is the target ConfigMap name, `loaded` is the validated state config bundle.
    /// Applies the latest matrix, Discord config, normalized JSON, hash, revision, and sync timestamp.
    pub async fn publish_config(&self, name: &str, loaded: &LoadedConfig) -> anyhow::Result<()> {
        let normalized =
            serde_json::to_string_pretty(&loaded.published).context("serialize config")?;

        let data = BTreeMap::from([
            ("matrix.yaml".to_string(), loaded.matrix_yaml.clone()),
            ("discord.yaml".to_string(), loaded.discord_yaml.clone()),
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

    /// Args: `selector` filters agent pods, `port` and `path` build each agent result endpoint.
    /// Lists running agent pods and returns HTTP URLs for pods that have an assigned pod IP.
    pub async fn list_agent_urls(
        &self,
        selector: &str,
        port: u16,
        path: &str,
    ) -> anyhow::Result<Vec<Url>> {
        let pods = self
            .pods
            .list(&ListParams::default().labels(selector))
            .await
            .context("failed to list agent pods")?;

        let urls = pods
            .items
            .into_iter()
            .filter(|pod| {
                pod.status
                    .as_ref()
                    .and_then(|status| status.phase.as_deref())
                    .is_some_and(|phase| phase == "Running")
            })
            .filter_map(|pod| match pod.status.and_then(|status| status.pod_ip) {
                Some(ip) => build_agent_url(&ip, port, path)
                    .inspect_err(|err| {
                        tracing::warn!(pod_ip = %ip, error = %err, "skipping invalid agent URL");
                    })
                    .ok(),
                None => None,
            })
            .collect();

        Ok(urls)
    }
}

/// Args: `ip` is the pod IP, `port` and `path` identify the agent result endpoint.
/// Builds a valid HTTP URL, including bracket handling for IPv6 pod addresses.
fn build_agent_url(ip: &str, port: u16, path: &str) -> anyhow::Result<Url> {
    let host = if ip.contains(':') {
        format!("[{}]", ip)
    } else {
        ip.to_string()
    };
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    Url::parse(&format!("http://{}:{}{}", host, port, path))
        .with_context(|| format!("failed to build agent URL for pod IP '{}'", ip))
}
