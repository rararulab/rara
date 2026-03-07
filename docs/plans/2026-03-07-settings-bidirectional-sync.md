# Settings Bidirectional Sync Design

> Date: 2026-03-07
> Status: Draft

## Problem

当前 settings 系统是单向的：config.yaml 在启动时通过 `seed_defaults` 写入 KV store，之后运行时变更只存在于 KV store 中，不回写 config.yaml。重启后运行时修改丢失，config.yaml 又覆盖一切。

## Goals

1. **双向同步**：settings 运行时变更写回 config.yaml；config.yaml 文件编辑同步到 settings
2. **config.yaml 是人类友好的编辑入口**，settings KV store 是运行时唯一 source of truth
3. **最后写入者胜**：无论哪边修改，最新的值生效

## Non-Goals

- 静态配置（http, grpc, users, gateway, telemetry, database）不纳入 settings store，维持现状
- 不保留 config.yaml 中的注释（回写时全量序列化）

## Design

### Architecture

```
┌─────────────┐         ┌──────────────┐         ┌─────────────┐
│ config.yaml │◄───────►│ConfigFileSync │◄───────►│ SettingsSvc  │
│  (on disk)  │  write   │  (new 组件)   │  watch   │ (SQLite KV) │
└─────────────┘  back    └──────────────┘  notify  └─────────────┘
                              │                         ▲
                         file watcher                   │
                         (notify crate)          SettingsProvider
                                                        │
                                                ┌───────┴────────┐
                                                │ agent / api /  │
                                                │ tools / kernel │
                                                └────────────────┘
```

### ConfigFileSync Component

**位置**: `crates/app/src/config_sync.rs`

```rust
pub struct ConfigFileSync {
    settings: SettingsSvc,
    /// 内存中的完整 AppConfig（含静态部分），回写时拼合用
    app_config: Arc<RwLock<AppConfig>>,
    config_path: PathBuf,
    /// 回声抑制：上次写入后的 content hash
    last_written_hash: AtomicU64,
    /// debounce 写入触发
    write_trigger: mpsc::Sender<()>,
}
```

### Data Flow

#### 1. Startup (replaces seed_defaults)

```
config.yaml → parse AppConfig
            → ConfigFileSync::start()
            → sync_from_file()   // 首次加载，flatten → batch_update 写入 KV
            → 启动 file watcher task
            → 启动 writeback debounce task
```

`seed_defaults` 被移除。启动时的首次加载与运行时 file watcher 的逻辑统一为 `sync_from_file()`。

#### 2. Runtime settings change → write back to config.yaml

```
SettingsProvider::set() / batch_update()
  → KV store updated
  → watch::Sender notify
  → ConfigFileSync writeback task 收到通知
  → debounce 1.5s（合并多次变更）
  → 从 KV 读全部 settings
  → unflatten 回 LlmConfig / TelegramConfig / KnowledgeConfig
  → 替换 app_config 中的动态字段
  → serde_yaml::to_string(&app_config) → 写文件
  → 更新 last_written_hash (AtomicU64)
```

#### 3. Config file edited → sync to settings

```
用户手动编辑 config.yaml 并保存
  → notify file watcher 收到事件
  → 读取文件内容，计算 hash
  → 与 last_written_hash.load(Relaxed) 比对
  → 相同 → 跳过（回声抑制）
  → 不同 → 解析为 AppConfig
        → flatten_config_sections()
        → batch_update() 写入 KV store（触发 subscribers 通知）
        → 更新内存中的 app_config
```

### Echo Suppression

双向同步的经典问题：settings 变更 → 写回文件 → watcher 检测到 → 又触发加载。

解决：用 `AtomicU64` 存储上次写入文件后的 content hash。Watcher 检测到文件变化时，先比对 hash，相同则说明是自己写的，直接跳过。

### Writeback Debounce

```rust
loop {
    settings_rx.changed().await;
    // debounce: 1.5s 内持续有新通知就重置计时
    loop {
        match timeout(Duration::from_millis(1500), settings_rx.changed()).await {
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    // 执行回写
    writeback_to_file().await;
}
```

### Config Section Scope

**动态 settings（双向同步）**：
- `llm` — LLM provider 配置
- `telegram` — Telegram bot 配置
- `knowledge` — Knowledge layer 配置

**静态 config（维持现状，不进 settings）**：
- `http`, `grpc` — 服务器绑定地址
- `users` — 用户和平台绑定
- `gateway` — 网关配置
- `telemetry` — 遥测配置
- `database` — 数据库连接

### Required Code Changes

#### New files
- `crates/app/src/config_sync.rs` — ConfigFileSync 组件

#### Modified files
- `crates/app/src/lib.rs` — 启动流程：移除 seed_defaults 调用，改为 ConfigFileSync::start()
- `crates/app/src/lib.rs` — AppConfig 加 `Serialize` derive
- `crates/app/src/flatten.rs` — 所有 config 类型加 `Serialize` derive；新增 `unflatten_from_settings()` 反向函数
- `crates/extensions/backend-admin/src/settings/service.rs` — 移除 `seed_defaults()` 方法和 legacy migration 代码

#### New dependency
- `notify` crate（file watcher）— 加到 `crates/app/Cargo.toml`

### Unflatten: KV → Config Structs

`flatten.rs` 中新增对称的反向函数：

```rust
/// 从 settings KV pairs 还原为 config section structs
pub fn unflatten_from_settings(
    pairs: &HashMap<String, String>,
) -> (Option<LlmConfig>, Option<TelegramConfig>, Option<KnowledgeConfig>)
```

遍历 KV pairs，按 key prefix 分组，组装回对应的 struct。与 `flatten_config_sections()` 形成对称。
