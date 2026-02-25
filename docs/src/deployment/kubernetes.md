# Kubernetes (Helm)

The `rara-infra` Helm chart deploys all infrastructure dependencies for the rara platform into a Kubernetes cluster. The rara application itself is not included in this chart.

## Prerequisites

- Kubernetes cluster (OrbStack, minikube, kind, or a cloud provider)
- [Helm 4.0+](https://helm.sh/)
- [just](https://github.com/casey/just) (optional, for shortcuts)

## Quick Start

```bash
cd deploy/helm

# Download subchart dependencies
just deps

# Install with dev values (minimal resources)
just install-dev

# Verify everything is healthy
just doctor
```

Without `just`:

```bash
cd deploy/helm
helm dependency build rara-infra
helm install rara-infra rara-infra -n rara --create-namespace \
  --server-side=true -f rara-infra/values-dev.yaml
```

## Components

| Component | Chart | Purpose |
|-----------|-------|---------|
| PostgreSQL (pgmq) | `bitnami/postgresql` | Primary database (with pgmq extension) |
| MinIO | `minio/minio` | S3-compatible object storage |
| ChromaDB | custom templates | Vector database |
| Crawl4AI | custom templates | Web crawler service |
| Consul | `hashicorp/consul` | Configuration KV store |
| Traefik | `traefik/traefik` | Ingress controller |
| cert-manager | `jetstack/cert-manager` | Self-signed CA + TLS certificates |
| Prometheus | `prometheus-community/kube-prometheus-stack` | Metrics collection |
| Grafana | (bundled with kube-prometheus-stack) | Dashboards and visualization |
| AlertManager | (bundled with kube-prometheus-stack) | Alert routing |
| Tempo | `grafana/tempo` | Distributed tracing backend |
| Alloy | `grafana/alloy` | OpenTelemetry collector |
| Quickwit | `quickwit/quickwit` | Log search engine |
| Langfuse | `langfuse/langfuse` | LLM observability platform |

Each component can be toggled on/off in `values.yaml`:

```yaml
traefik:
  enabled: true
postgresql:
  enabled: true
langfuse:
  enabled: false   # disable if not needed
```

## Values Files

| File | Description |
|------|-------------|
| `rara-infra/values.yaml` | Production defaults -- all components enabled, full resource requests |
| `rara-infra/values-dev.yaml` | Dev overlay -- minimal CPU/memory, smaller PVCs |

## Local Access Setup

### 1. Add DNS entries

Route `*.rara.local` to the Traefik load balancer IP:

```bash
just hosts   # adds entries to /etc/hosts (requires sudo)
```

### 2. Trust the CA certificate

The chart creates a self-signed CA via cert-manager. Trust it in your OS keychain:

```bash
just trust-ca   # exports CA cert and adds to macOS keychain (requires sudo)
```

After trusting the CA, restart your browser.

### 3. Access endpoints

| Service | URL |
|---------|-----|
| Grafana | <https://grafana.rara.local> |
| Prometheus | <https://prometheus.rara.local> |
| AlertManager | <https://alertmanager.rara.local> |
| Langfuse | <https://langfuse.rara.local> |
| Quickwit | <https://quickwit.rara.local> |
| Consul UI | <https://consul.rara.local> |
| MinIO Console | <https://minio.rara.local> |
| Traefik Dashboard | <https://traefik.rara.local> |
| Tempo | <https://tempo.rara.local> |

## Consul KV Configuration

The `consul-kv-seed` Helm hook (post-install/post-upgrade) automatically writes all infrastructure connection info to Consul KV. The rara app only needs a single `CONSUL_HTTP_ADDR` environment variable to discover every dependency at startup.

### Seeded Keys

All keys are stored under the `rara/config/` prefix:

| Key | Value Source |
|-----|-------------|
| `database/database_url` | PostgreSQL connection string |
| `object_store/endpoint` | MinIO API URL |
| `object_store/access_key_id` | MinIO root user |
| `object_store/secret_access_key` | MinIO root password |
| `object_store/bucket` | `rara` |
| `memory/chroma_url` | ChromaDB URL |
| `crawl4ai/base_url` | Crawl4AI service URL |
| `telemetry/otlp_endpoint` | Generic OTLP endpoint (Alloy collector) |
| `langfuse/host` | Langfuse OTLP endpoint URL |
| `langfuse/public_key` | Langfuse project API key (optional, from `consulSeed.langfuse.publicKey`) |
| `langfuse/secret_key` | Langfuse project secret (optional, from `consulSeed.langfuse.secretKey`) |

### URL Override (Out-of-Cluster App)

By default, all service URLs use cluster-internal DNS (e.g. `http://rara-infra-minio:9000`). If the rara app runs **outside** the Kubernetes cluster (e.g. on the developer's host machine), these internal addresses are unreachable.

Override them in `values.yaml` to use Traefik-exposed URLs:

```yaml
consulSeed:
  overrides:
    langfuseHost: "https://langfuse.rara.local"
    crawl4aiBaseUrl: "https://crawl4ai.rara.local"
    chromaUrl: "http://localhost:8000"            # or port-forward
    objectStoreEndpoint: "http://localhost:9000"  # or port-forward
    databaseUrl: "postgres://postgres:postgres@localhost:5432/rara"
```

Any override set to a non-empty string replaces the auto-derived internal URL. Empty string (default) uses the internal service DNS.

After changing overrides, run `helm upgrade` or manually re-seed:

```bash
cd deploy/helm
just seed-consul   # seed/re-seed all keys from Helm values
just consul-keys   # list current keys in Consul
```

### Langfuse API Keys

Langfuse `public_key` and `secret_key` must be obtained from the Langfuse UI after creating a project. Add them to `values.yaml`:

```yaml
consulSeed:
  langfuse:
    publicKey: "pk-lf-..."
    secretKey: "sk-lf-..."
```

Then `helm upgrade` or `just seed-consul` to inject them.

See [Configuration](../configuration.md) for details on how the app loads config from Consul.

## Observability Pipeline

```
App (OTLP) --> Alloy --> Tempo      (traces)
                     --> Prometheus  (metrics)
                     --> Quickwit   (logs)

Grafana dashboards query all three backends.
```

## Useful Commands

All commands run from `deploy/helm/`:

| Command | Description |
|---------|-------------|
| `just doctor` | Health check all components |
| `just status` | Show Helm release status |
| `just values` | Show deployed values |
| `just upgrade-dev` | Upgrade with dev overlay |
| `just diff` | Diff pending changes against live release |
| `just deps-update` | Update subchart dependencies to latest versions |
| `just seed-consul` | Seed/re-seed Consul KV from Helm values |
| `just consul-keys` | List all Consul KV keys under `rara/config/` |
| `just uninstall` | Tear down the release |

Run `just` with no arguments to see all available commands.

## Default Credentials

| Service | Username | Password |
|---------|----------|----------|
| PostgreSQL | `postgres` | `postgres` |
| MinIO | `minioadmin` | `minioadmin` |
| Grafana | anonymous access (admin/admin) | login form disabled by default |
