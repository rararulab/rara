# Deploying rara-server on Kubernetes

Server-only, sandbox-disabled container image + plain k8s manifests for
running the **rara backend as an in-cluster control-plane process** — the
deploy substrate the fleet direction (`crates/rara-fleet`, kube-rs
dispatcher) needs so rara-server runs inside the cluster it dispatches
Jobs into.

This ships **only** the `rara server` process. No web bundle, no nginx, no
sandbox/microVM runtime. Untrusted-code execution is deliberately not solved
here — it is deferred to fleet's separate microVM layer.

## What's in the box

| File | Purpose |
| --- | --- |
| `../../docker/Dockerfile` | Multi-stage, server-only image (`linux/amd64`) |
| `secret.example.yaml` | **Template** Secret carrying the complete `config.yaml` (owner token + LLM key + all runtime config) |
| `pvc.yaml` | `ReadWriteOnce` PVC for the writable config dir + SQLite DB / sessions |
| `deployment.yaml` | 1 replica, `Recreate`, non-root, init container seeds config, probes, resource limits |
| `service.yaml` | ClusterIP exposing HTTP (25555) + gRPC (50051) |

## Build the image

```bash
# From the repo root. linux/amd64 only.
docker build -f docker/Dockerfile -t rara-server:latest .
```

The build is a full-workspace `cargo build --release` and is slow on the
first run. Build facts baked into the Dockerfile so you don't hit the wall
the team already mapped:

- **`ubuntu:24.04` (gcc-13)** for both stages — not bookworm/gcc-12, which
  fails numkong's SIMD compile.
- **zig 0.16.0** downloaded from ziglang.org (not in apt) — required
  unconditionally by `crates/app`'s zlob build.rs.
- **clang + mold** — `.cargo/config.toml` forces `-fuse-ld=mold` +
  `linker=clang` on `x86_64-unknown-linux-gnu`.
- **`BOXLITE_DEPS_STUB=1`** stubs the native boxlite dependency so the
  sandbox subsystem builds without its microVM runtime.
- Runtime is **ubuntu slim (not distroless)** — rara links `libsqlite3`,
  `libpq`, `libssl`, and `zlib` at runtime.

### Architecture: amd64 only

The image is `linux/amd64` only. The real build-and-boot verification runs
in CI (`.github/workflows/container-smoke.yml`) on GitHub's `ubuntu-latest`
**native amd64** runner — that is the amd64 oracle.

**Building on an Apple-Silicon (arm64) dev machine:** a `--platform=linux/amd64`
build under Docker Desktop's Rosetta emulation fails at the zig `translate-c`
step with `rosetta error: bss_size overflow` (a Rosetta limitation, not a
recipe defect). To build locally on arm64, either build on a native amd64
host, or use a QEMU-backed buildx builder instead of Rosetta — but the
canonical amd64 check is the CI job, so you normally don't need to.

## Two container-specific config facts

1. **Bind `0.0.0.0`, not `127.0.0.1`.** `config.example.yaml` binds loopback.
   In a pod, the kubelet liveness/readiness probe and the Service reach the
   container on its Pod IP — a loopback bind is unreachable and the pod
   crashloops as unhealthy. The Secret's `config.yaml` sets
   `http.bind_address: "0.0.0.0:25555"` and the gRPC bind to `0.0.0.0:50051`.
   (The `PORT` env var only rewrites the *port*, not the host — it cannot fix
   loopback.) The gateway admin port (25556) is not used by this image.

2. **The config dir must be WRITABLE, and secrets live only in a Secret.**
   rara's config dir (`$XDG_CONFIG_HOME/rara`) is not a read-only config
   location — rara creates `workspace/`, `settings.json`, `skills/`,
   `mcp-servers.json` there at runtime, and its config sync
   (`ConfigFileSync`) reads a single `config.yaml` as a complete `AppConfig`
   **and writes it back** (settings KV → file). rara also reads LLM API keys
   and the owner token from `config.yaml` (`llm.providers.<name>.api_key`,
   `owner_token`) — there is **no** env-var override for these in the loader.

### How config + secrets are delivered

Because rara needs **one complete, writable `config.yaml`** and that file
must contain secrets, the whole `config.yaml` lives in the **`rara-secrets`
Secret** (`secret.example.yaml`). An **init container** copies it into the
writable, persistent config dir on the PVC at startup:

```
Secret rara-secrets (config.yaml, read-only)
   │  init container: cp → /state/config/rara/config.yaml  (writable, on PVC)
   ▼
rara-server reads/writes /state/config/rara/config.yaml
            data dir      /state/data/rara/…  (DB, sessions, memory)
```

Secret material therefore never lives in the image or in git — only in the
Secret. (Note: rara's KV→file writeback serializes the running config,
including secrets, into the config file on the PVC at rest — inherent to
rara, the same PVC already holds the DB. The Secret stays the source of
truth.)

> A partial "secret overlay" merged via rara's two-file config precedence
> was evaluated and rejected: `ConfigFileSync` parses/writes a **single**
> complete file, so a partial overlay fails (`missing field owner_user_id`).
> The whole-config-in-Secret approach is the one that works.

## Deploy

```bash
# 1. Namespace (pick your own; manifests carry no hardcoded namespace).
kubectl create namespace rara

# 2. Create the real Secret from the template — DO NOT commit this file.
cp deploy/k8s/secret.example.yaml /tmp/rara-secret.yaml
$EDITOR /tmp/rara-secret.yaml          # replace both REPLACE_WITH_* placeholders
kubectl apply -n rara -f /tmp/rara-secret.yaml

# 3. Apply the rest.
kubectl apply -n rara \
  -f deploy/k8s/pvc.yaml \
  -f deploy/k8s/deployment.yaml \
  -f deploy/k8s/service.yaml

# 4. Watch it come up (startup probe covers first-boot migrations).
kubectl -n rara rollout status deploy/rara-server
kubectl -n rara port-forward svc/rara-server 25555:25555 &
curl -s http://127.0.0.1:25555/api/health   # {"service":"job","status":"healthy",...}
```

> Update the `image:` field in `deployment.yaml` to your image reference.
> This issue does not publish to a registry (out of scope), so you load the
> image into your cluster yourself (e.g. `kind load` / `minikube image load`
> / your own registry).

## Why single-replica / Recreate / RWO

rara's store is a **single SQLite file with one writer**. The Deployment is
pinned to `replicas: 1` with `strategy: Recreate` and a `ReadWriteOnce` PVC.
A rolling update or `replicas > 1` would briefly run two pods both mounting
the volume / writing the DB and corrupt it. This also matches "rara is NOT
multi-user" — one pod is one user's rara.

**Never back a real deployment with `emptyDir`** — the DB, memory, and the
config dir (skills, settings) would not survive a restart. Swap the PVC's
storage class or point it at an external/managed volume as needed, but keep
it durable.

## Local smoke test (no cluster)

Proves the image boots and serves health — the same thing CI's
`container-smoke` workflow does on every PR touching these files.

**Note on architecture:** the Dockerfile is pinned to `linux/amd64`
(`--platform=linux/amd64` + the x86_64 zig tarball), so `docker build`
always produces an **amd64** image regardless of your host. On an
Apple-Silicon (arm64) machine that amd64 build runs under Rosetta and
**fails** at the zig step (`bss_size overflow`) — so the authoritative
amd64 build+smoke is CI's `container-smoke.yml` (native amd64 runner). To
run the boot check *locally* on arm64, build an arch-adapted variant
first (swap `--platform=linux/amd64` → `linux/arm64` and the zig tarball
`zig-x86_64-linux` → `zig-aarch64-linux`); the boot/health behavior below
is arch-independent.

```bash
# amd64 host (or CI): builds directly. arm64 host: build an arch-adapted
# variant as described above, then tag it rara-server:smoke.
docker build -f docker/Dockerfile -t rara-server:smoke .

cat > /tmp/rara-smoke.yaml <<'YAML'
http:
  bind_address: "0.0.0.0:25555"
  cors_allowed_origins: ["http://localhost:*"]
grpc:
  bind_address: "0.0.0.0:50051"
  server_address: "127.0.0.1:50051"
owner_token: "smoke-placeholder-token"
owner_user_id: "you"
users:
  - name: "you"
    role: root
    platforms: []
mita:
  heartbeat_interval: "30m"
llm:
  default_provider: "openrouter"
  providers:
    openrouter:
      base_url: "https://openrouter.ai/api/v1"
      default_model: "anthropic/claude-3.5-sonnet"
      api_key: "smoke-placeholder-key"
knowledge:
  embedding_model: "text-embedding-3-small"
  embedding_dimensions: 1536
  search_top_k: 10
  similarity_threshold: 0.85
YAML

# Mirror the k8s init container: copy the config into a writable config dir
# (a named volume seeded from the image's non-root-owned /config), then boot.
docker volume create rara-cfg
docker run --rm --entrypoint sh \
  -v rara-cfg:/config \
  -v /tmp/rara-smoke.yaml:/tmp/config.yaml:ro \
  rara-server:smoke \
  -c "cp /tmp/config.yaml /config/rara/config.yaml"
docker run -d --name rara-smoke -p 25555:25555 \
  -v rara-cfg:/config \
  rara-server:smoke

curl -sf --max-time 5 http://127.0.0.1:25555/api/health | grep -q healthy && echo OK
docker rm -f rara-smoke; docker volume rm rara-cfg
```

## Not included (deliberately)

- **Registry publish + tag/versioning** — follow-up; would re-introduce the
  publish pipeline #443 removed.
- **arm64 / multi-arch.**
- **Sandbox / code execution** — disabled; deferred to fleet's microVM layer.
- **Web UI / nginx** — server-only image.
- **Pod-management RBAC** — only needed if rara's `k8s` client feature (the
  `rara` pod-manager ServiceAccount in `docs/k8s-setup.md`) is enabled later;
  a keep-separate concern.
- **Helm / kustomize** — plain manifests only for now.
