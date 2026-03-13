# rara-vault — Agent Guide

## 这个 crate 是什么

Vault 配置中心客户端。rara 通过它连接部署在 k8s 集群上的 HashiCorp Vault，实现配置的集中管理、版本化、加密存储和远程变更。

## 为什么需要它

之前所有配置写在本地 YAML 文件里，有三个痛点：
- **改坏了没法回滚** — 配置迭代快，YAML 改错没有版本历史
- **敏感信息明文** — API key、bot token 直接写在文件里
- **远程管理不便** — 必须 SSH 到服务器手动编辑

Vault KV v2 引擎天然解决这三个问题：每次写入自动生成版本号，支持 rollback；数据加密存储；有 Web UI 远程管理。

## 关键设计决策

### 为什么用 AppRole 认证而不是 Token

AppRole 是 Vault 推荐的机器对机器认证方式。role_id + secret_id 从文件读取，不硬编码。Token 有 TTL 会过期，client 自动在 TTL 过半时 renew。

### 为什么 config/ 和 secrets/ 分离

Vault policy 按路径控制权限。secrets/ 路径可以设更严格的 ACL（只有 rara 的 AppRole 能读），config/ 可以开放给运维人员直接在 UI 上改。

### 为什么 flatten 格式要兼容现有 KV store

rara 已有一套 Settings KV store（SQLite）+ 双向 sync 机制（`crates/app/src/flatten.rs`）。Vault 拉下来的配置最终要灌入这个 KV store，所以 `pull_all()` 输出的 key 格式必须和 `flatten.rs` 的一致：

```
llm.default_provider        → "openrouter"
llm.providers.openrouter.api_key → "sk-xxx"
telegram.bot_token           → "xxx"
knowledge.embedding_model    → "text-embedding-3-small"
```

这样下游的 kernel、server、channel 完全不知道配置来自 Vault 还是本地 YAML。

### 为什么用 poll 而不是 watch

Vault KV v2 没有原生 watch/subscribe 机制。client 按 `watch_interval`（默认 30s）轮询各 path 的 metadata version，发现版本号变化才 pull 最新数据。

### 降级策略

Vault 是外部依赖，必须容忍不可达：
- 启动时连不上 + `fallback_to_local: true` → 用本地 YAML 启动，日志 WARN
- 启动时连不上 + `fallback_to_local: false` → 报错退出
- 运行中断连 → 保持当前配置不变，后台定时重连

## 数据流

```
Vault (k8s)
  │
  │ pull_all() / push_changes()
  ▼
rara-vault crate
  │
  │ Vec<(String, String)>  ← 与 flatten.rs 格式兼容
  ▼
Settings KV Store (SQLite) ← 现有，不变
  │
  ▼
kernel / server / channels  ← 无感知
```

## Vault 数据结构

```
secret/rara/
├── config/          # 非敏感配置
│   ├── http         # { bind_address, max_body_size, ... }
│   ├── grpc         # { bind_address, server_address, ... }
│   ├── llm          # { default_provider, providers: { ... } }
│   ├── mita         # { heartbeat_interval }
│   ├── knowledge    # { embedding_model, dimensions, ... }
│   └── symphony     # { enabled, poll_interval, ... }
├── secrets/         # 敏感信息（严格 ACL）
│   ├── telegram     # { bot_token }
│   ├── llm          # { providers: { openrouter: { api_key } } }
│   ├── composio     # { api_key, entity_id }
│   └── symphony     # { linear_api_key }
└── users/           # 用户身份
    └── ryan         # { role, platforms: [...] }
```

## 部署信息

- Vault 跑在 k8s 集群，standalone 模式，NodePort :30820
- rara 服务器和 k8s 在同一局域网，HTTP 直连，不需要 TLS
- AppRole 凭证文件放在 rara 服务器的 `/etc/rara/vault-role-id` 和 `/etc/rara/vault-secret-id`

## 相关文件

- `docs/plans/2026-03-13-vault-config-center-design.md` — 完整设计文档
- `crates/app/src/flatten.rs` — 现有的 flatten/unflatten 逻辑（本 crate 的输出格式必须与之兼容）
- `crates/app/src/config_sync.rs` — 现有的双向 sync（#299 中会扩展为三向）
- `crates/app/src/lib.rs` — AppConfig 和启动流程（#299 中会插入 Vault 拉取步骤）
