# Infrastructure Components

Reference for all infrastructure services deployed by `rara-infra`.

## Core

| Component | Version / Image | Port | Purpose |
|-----------|----------------|------|---------|
| PostgreSQL | `ghcr.io/pgmq/pg18-pgmq:v1.10.0` (Bitnami chart v18.4.0) | 5432 | Primary database with pgmq message queue extension |
| MinIO | Chart v5.4.0 | 9000 (API), 9001 (console) | S3-compatible object storage. Buckets: `rara`, `langfuse`, `quickwit` |
| ChromaDB | `chromadb/chroma:latest` (custom templates) | 8000 | Vector database for memory/embedding search |
| Crawl4AI | `unclecode/crawl4ai:latest` (custom templates) | 11235 | Web crawling and content extraction |
| Consul | Hashicorp chart v1.9.3 | 8500 | Configuration KV store. Single-server, no service mesh |

**Default credentials:**

| Service | User | Password |
|---------|------|----------|
| PostgreSQL | `postgres` | `postgres` |
| MinIO | `minioadmin` | `minioadmin` |

## Ingress & TLS

| Component | Chart Version | Purpose |
|-----------|--------------|---------|
| Traefik | v39.0.2 | Ingress controller. HTTP automatically redirects to HTTPS. IngressRoute CRDs for routing |
| cert-manager | v1.19.3 | Manages self-signed CA chain. Issues wildcard certificate for `*.rara.local` |

The TLS chain: self-signed ClusterIssuer creates a root CA, which signs a wildcard certificate referenced by all IngressRoutes.

**Endpoints:**

| URL | Service |
|-----|---------|
| `https://traefik.rara.local` | Traefik dashboard |

## Observability

| Component | Chart Version | Purpose |
|-----------|--------------|---------|
| Prometheus | kube-prometheus-stack v82.2.1 | Metrics collection and alerting rules. 7-day retention (default) |
| Grafana | (bundled) | Dashboards. Pre-configured datasources: Prometheus, Tempo, Quickwit |
| AlertManager | (bundled) | Alert routing and notification |
| Tempo | v1.24.4 | Distributed tracing backend (single binary mode). Receives OTLP via gRPC/HTTP |
| Alloy | v1.6.0 | OpenTelemetry collector. Routes traces to Tempo, metrics to Prometheus, logs to Quickwit |
| Quickwit | v0.7.21 | Log search engine. Uses MinIO for index storage |

**Data flow:**

```
App (OTLP) --> Alloy --> Tempo      (traces)
                     --> Prometheus  (metrics via remote write)
                     --> Quickwit   (logs via OTLP HTTP)
```

**Default credentials:**

| Service | Access |
|---------|--------|
| Grafana | Anonymous access enabled (Admin role). Login form disabled. Fallback: `admin` / `admin` |

**Endpoints:**

| URL | Service |
|-----|---------|
| `https://grafana.rara.local` | Grafana dashboards |
| `https://prometheus.rara.local` | Prometheus query UI |
| `https://alertmanager.rara.local` | AlertManager UI |
| `https://tempo.rara.local` | Tempo API |
| `https://quickwit.rara.local` | Quickwit search UI |

## Platform

| Component | Chart Version | Purpose |
|-----------|--------------|---------|
| Langfuse | v1.5.20 | LLM observability -- trace, evaluate, and monitor LLM calls. Deploys its own PostgreSQL, ClickHouse, and Redis instances. Uses shared MinIO for S3 storage |

**Endpoints:**

| URL | Service |
|-----|---------|
| `https://langfuse.rara.local` | Langfuse web UI |
| `https://consul.rara.local` | Consul KV UI |
| `https://minio.rara.local` | MinIO console |
