# Vault 配置中心设计

**Date**: 2026-03-13
**Status**: Draft

## 背景

rara 当前使用本地 YAML 文件 + SQLite KV store 双向同步管理配置。存在三个痛点：

1. **运行时变更无审计/回滚** — 配置迭代快，改坏了无法回退到已知好的版本
2. **敏感信息明文** — API key、bot token 直接写在 YAML 里
3. **远程管理不便** — 需要 SSH 到服务器手动编辑文件

## 方案选择

选择 HashiCorp Vault（部署在 k8s 集群，rara 通过局域网 HTTP API 连接）：

| 对比项 | 自建版本化 | etcd | Vault |
|--------|-----------|------|-------|
| 版本历史/回滚 | 自己写 | 内置 | KV v2 内置 |
| 敏感信息加密 | 自己写 | 需额外处理 | 原生支持 |
| 管理 UI | 无 | 弱 | 内置 Web UI |
| 部署复杂度 | 无 | 中 | 中（Helm） |
| 外部依赖 | 无 | etcd 进程 | Vault 进程 |

选 Vault 的理由：KV v2 自带版本历史 + rollback；敏感信息天然加密；Web UI 远程管理；k8s Helm 一键部署。

## 部署架构

```
┌────────────────────────────┐     ┌──────────────────────────┐
│  k8s 集群（局域网）         │     │  rara 服务器（局域网）    │
│                            │     │                          │
│  ┌──────────────────────┐  │     │  ┌────────────────────┐  │
│  │  Vault (standalone)  │  │     │  │  rara              │  │
│  │  NodePort :30820     │◄─┼─────┼──│  rara-vault crate  │  │
│  │  KV v2 engine        │  │     │  └────────────────────┘  │
│  │  AppRole auth        │  │     │                          │
│  │  Web UI              │  │     │  /etc/rara/              │
│  └──────────────────────┘  │     │    vault-role-id         │
│                            │     │    vault-secret-id       │
└────────────────────────────┘     └──────────────────────────┘
```

- 局域网通信，不需要 TLS/Ingress
- Vault standalone 模式，单节点够用
- rara 用 AppRole 认证，凭证文件存服务器本地

## Vault 数据结构

```
secret/rara/
├── config/              # 非敏感配置
│   ├── http             # { bind_address, max_body_size, enable_cors, request_timeout }
│   ├── grpc             # { bind_address, server_address, max_recv/send_message_size }
│   ├── llm              # { default_provider, providers: { name: { base_url, default_model } } }
│   ├── mita             # { heartbeat_interval }
│   ├── knowledge        # { embedding_model, dimensions, search_top_k, similarity_threshold }
│   ├── symphony         # { enabled, poll_interval, max_concurrent_agents, ... }
│   └── gateway          # { check_interval, health_timeout, max_restart_attempts, ... }
├── secrets/             # 敏感信息（严格 ACL）
│   ├── database         # { url }
│   ├── telegram         # { bot_token }
│   ├── llm              # { providers: { openrouter: { api_key }, ollama: { api_key } } }
│   ├── composio         # { api_key, entity_id }
│   └── symphony         # { linear_api_key }
└── users/               # 用户身份映射
    └── ryan             # { role, platforms: [...] }
```

**分离原则**：config/ 和 secrets/ 使用不同的 Vault policy，secrets/ 路径限制只有 rara AppRole 能读。

## Vault 部署步骤

### 1. Helm 安装

```bash
helm repo add hashicorp https://helm.releases.hashicorp.com
helm install vault hashicorp/vault \
  --namespace vault --create-namespace \
  --set server.standalone.enabled=true \
  --set server.dataStorage.size=1Gi \
  --set ui.enabled=true
```

### 2. 初始化 + 解封

```bash
kubectl exec -it vault-0 -n vault -- vault operator init -key-shares=1 -key-threshold=1
# 记下 Unseal Key + Root Token

kubectl exec -it vault-0 -n vault -- vault operator unseal <unseal-key>
```

### 3. 配置引擎 + AppRole

```bash
vault secrets enable -path=secret kv-v2
vault auth enable approle

vault policy write rara-policy - <<'EOF'
path "secret/data/rara/*" {
  capabilities = ["create", "read", "update", "list"]
}
path "secret/metadata/rara/*" {
  capabilities = ["read", "list"]
}
EOF

vault write auth/approle/role/rara \
  token_policies="rara-policy" \
  token_ttl=1h \
  token_max_ttl=4h
```

### 4. 暴露 NodePort

```bash
kubectl patch svc vault -n vault \
  -p '{"spec":{"type":"NodePort","ports":[{"port":8200,"nodePort":30820}]}}'
```

### 5. Auto-unseal（可选）

Vault 重启后需要 unseal。简单方案：K8s Secret 存 unseal key + init container 自动解封。

## rara 侧集成设计

### 新 crate：rara-vault

```
crates/rara-vault/
├── Cargo.toml
└── src/
    ├── lib.rs          # pub API: VaultConfigSource
    ├── client.rs       # VaultClient: HTTP 交互、AppRole auth、token 自动续期
    ├── config.rs       # VaultConfig struct
    ├── watcher.rs      # 后台 poll 变更（对比 version 号）
    └── error.rs        # snafu 错误类型
```

### VaultConfig（本地 YAML 新增）

```yaml
vault:
  address: http://<k8s-node-ip>:30820
  mount_path: secret/rara
  auth:
    method: approle
    role_id_file: /etc/rara/vault-role-id
    secret_id_file: /etc/rara/vault-secret-id
  watch_interval: 30s       # poll 间隔
  timeout: 5s               # 单次请求超时
  fallback_to_local: true   # Vault 不可达时降级到本地 YAML
```

### 核心 trait

```rust
/// 配置源抽象，Vault 和本地 YAML 都实现此 trait
#[async_trait]
pub trait ConfigSource: Send + Sync {
    /// 拉取全量配置，合并为 AppConfig 的动态部分
    async fn pull_all(&self) -> Result<ConfigSnapshot>;

    /// 检查是否有变更（返回变更的 key 列表）
    async fn check_changes(&self, since_version: u64) -> Result<Vec<ConfigChange>>;

    /// 写回配置变更
    async fn push_changes(&self, changes: Vec<ConfigChange>) -> Result<()>;
}
```

### 启动流程变化

```
1. 读本地 YAML → 获取 vault 连接信息 + 作为 fallback 配置
2. 尝试 AppRole 认证获取 Vault token
   ├─ 成功 → 从 Vault pull_all() 拉全量配置
   │         合并覆盖本地 YAML 中的对应 section
   └─ 失败 → 日志告警，使用本地 YAML 继续启动
3. 合并后的 AppConfig 灌入 Settings KV store（复用现有 flatten 逻辑）
4. 启动 Vault watcher 后台 task
5. 正常启动 kernel、server...（下游完全无感知）
```

### 三向同步

```
                  Vault (KV v2)
                  ▲           │
       push_changes()    watch/poll
                  │           ▼
  Local YAML ◄─► config_sync ◄─► Settings KV Store (SQLite)
    file watcher    (现有)        (现有，不变)
```

| 变更来源 | 流向 |
|---------|------|
| Vault UI/CLI 修改 | Vault → watcher poll → Settings KV → 写回 YAML |
| 本地 YAML 编辑 | file watcher → Settings KV → push 到 Vault |
| gRPC/HTTP API 修改 | Settings KV → 写回 Vault + 写回 YAML |

**冲突策略**：Vault 版本号为准。本地改和 Vault 改冲突时，Vault 胜出（因为 Vault 有版本历史可回滚）。

### 降级策略

| 场景 | 行为 |
|------|------|
| 启动时 Vault 不可达 | 用本地 YAML 启动，日志 WARN，后台持续重连 |
| 运行中 Vault 断连 | 保持当前配置不变，停止 push，定时重连 |
| Vault 恢复连接 | 重新 pull 全量，diff 合并，恢复正常同步 |
| `fallback_to_local: false` | 启动时 Vault 不可达直接报错退出 |

### 对现有代码的改动

| 文件 | 改动 |
|------|------|
| `crates/app/Cargo.toml` | 新增 `rara-vault` 依赖 |
| `crates/app/src/lib.rs` | `AppConfig` 加 `vault: Option<VaultConfig>` 字段；`start()` 插入 Vault 拉取步骤 |
| `crates/app/src/config_sync.rs` | 扩展为三向同步，加 Vault push/pull 通道 |
| 其他 crate | **不动** — Vault 的存在对 kernel、server、channel 完全透明 |

## 实现计划

### Phase 1：基础集成（MVP）

1. 创建 `rara-vault` crate — VaultClient + AppRole auth
2. 实现 `pull_all()` — 启动时从 Vault 拉配置
3. `AppConfig` 加 vault section，启动流程插入 Vault 拉取
4. 降级逻辑：Vault 不可达用本地 YAML

**交付**：rara 能从 Vault 读配置启动，Vault 挂了不影响。

### Phase 2：Watch + 三向同步

5. 实现 watcher — 后台 poll Vault 变更
6. 扩展 config_sync — Vault 变更 → Settings KV → YAML
7. 反向同步 — Settings KV 变更 → push 到 Vault
8. 冲突处理 — Vault 版本号优先

**交付**：运行时通过 Vault UI 改配置，rara 自动生效。

### Phase 3：迁移 + 运维

9. 初始化脚本 — 把现有 YAML 配置导入 Vault
10. 文档 — Vault 部署 runbook、配置回滚操作手册
11. 监控 — Vault 连接状态暴露到 telemetry

**交付**：完整的配置中心运维体系。
