# pingpongkong-k8s-collector

This service is the Kubernetes collector for PingPongKong.

It does three jobs:

1. Fetches `matrix.yaml` and `discord.yaml` from a Git-backed raw config URL.
2. Validates the files and publishes the current config into a Kubernetes ConfigMap when it changes.
3. Scrapes PingPongKong agents for probe results and sends Discord alerts when probes fail.

The agents do the actual TCP/UDP probing. The collector gives them the latest config through Kubernetes, then gathers their results.

## How It Works

The collector runs as a Kubernetes Deployment.

On each sync cycle it reads:

```text
matrix.yaml
discord.yaml
```

If the files changed, it writes this ConfigMap, by default:

```text
pingpongkong-current-matrix
```

Agents watch that ConfigMap and reload when it changes.

If `matrix.yaml` and `discord.yaml` are the same as the last successful sync,
the collector skips the ConfigMap write.

The collector also finds running agent pods, calls their JSON results endpoint, and alerts Discord for failed checks.

## Required Environment

```text
CONFIG_BASE_URL
```

Example:

```bash
CONFIG_BASE_URL=https://example.com/raw/main cargo run
```

## Useful Options

```text
GIT_TOKEN
GIT_TOKEN_HEADER=bearer
K8S_NAMESPACE=default
CONFIG_MAP_NAME=pingpongkong-current-matrix
SYNC_INTERVAL_SECONDS=60
SCRAPE_INTERVAL_SECONDS=15
AGENT_LABEL_SELECTOR=app.kubernetes.io/name=pingpongkong-agent
AGENT_PORT=8080
MAX_CONCURRENT_AGENT_SCRAPES=64
DRY_RUN=false
```

For GitLab private token auth:

```text
GIT_TOKEN_HEADER=private-token
```

## Local Dry Run

```bash
CONFIG_BASE_URL=https://example.com/raw/main \
DRY_RUN=true \
cargo run
```

Dry run fetches and validates config, but does not write to Kubernetes or scrape agents.
