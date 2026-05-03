# pingpongkong-k8s-collector

This service is the Kubernetes collector for PingPongKong.

It does three jobs:

1. Fetches `k8s/<cluster>.yaml` and `notification/*.yaml` from a Git-backed config repository.
2. Validates the files and publishes the current config into a Kubernetes ConfigMap when it changes.
3. Scrapes PingPongKong agents for probe results and sends configured notifications when probes fail.

The agents do the actual TCP/UDP probing. The collector gives them the latest config through Kubernetes, then gathers their results.

## How It Works

The collector runs as a Kubernetes Deployment.

On each sync cycle it reads:

```text
k8s/<CONFIG_GIT_CLUSTERNAME>.yaml
notification/*.yaml
```

If the files changed, it writes this ConfigMap, by default:

```text
pingpongkong-<CONFIG_GIT_CLUSTERNAME>-ping-state
```

Agents watch that ConfigMap and reload when it changes.

If the Kubernetes matrix file and Discord notification file are the same as the last successful sync,
the collector skips the ConfigMap write.

The collector also finds running agent pods, calls their JSON results endpoint, and alerts configured destinations for failed checks.

## Required Environment

```text
CONFIG_GIT_URL
CONFIG_GIT_CLUSTERNAME
```

Example:

```bash
CONFIG_GIT_URL=https://github.com/songk1992/pingpongkong-state.git \
CONFIG_GIT_CLUSTERNAME=h100-cluster \
cargo run
```

For private repositories, also set:

```text
CONFIG_GIT_TOKEN
```

The collector derives raw file URLs from the repository URL and uses the repository default branch via `HEAD`.
GitHub repositories use `x-access-token` as the HTTPS Git username; GitLab-compatible repositories
use `oauth2`.

## Useful Options

```text
K8S_NAMESPACE=default
COLLECTOR_UPDATE_INTERVAL=5m
AGENT_CHECK_INTERVAL=5m
COLLECTOR_API_PORT=8081
HTTP_ADDR=0.0.0.0:8081
AGENT_API_PORT=8080
MAX_CONCURRENT_AGENT_CHECKS=64
REPORT_NOTIFICATION_MODE=ALWAYS
DRY_RUN=false
```

`K8S_NAMESPACE` should be set to the namespace where the collector and app live, usually from the pod
downward API field `metadata.namespace`. The ConfigMap name is always generated as
`pingpongkong-{CONFIG_GIT_CLUSTERNAME}-ping-state`.

`REPORT_NOTIFICATION_MODE` controls periodic connectivity report notifications and defaults to
`ALWAYS`. Use `NON_HEALTHY` to notify only when the aggregate report health is not
`Healthy`.

## Notification Files

Each `notification/{name}.yaml` file defines one destination. Supported providers are `discord`,
`teams`, `email`, `telegram`, and `sms`.

Discord, Teams, email webhooks, and SMS webhooks use:

```yaml
version: "1.0"
provider: "teams"
webhook:
  env_var: "TEAMS_WEBHOOK_URL"
```

Telegram uses bot token and chat id environment variables:

```yaml
version: "1.0"
provider: "telegram"
telegram:
  bot_token_env_var: "TELEGRAM_BOT_TOKEN"
  chat_id_env_var: "TELEGRAM_CHAT_ID"
```

For email, the collector sends a generic webhook JSON payload with `subject`, `message`, and report
data. It does not send SMTP mail directly.

## Local Dry Run

```bash
CONFIG_GIT_URL=https://github.com/songk1992/pingpongkong-state.git \
CONFIG_GIT_CLUSTERNAME=h100-cluster \
DRY_RUN=true \
cargo run
```

Dry run fetches and validates config, but does not write to Kubernetes or scrape agents.
