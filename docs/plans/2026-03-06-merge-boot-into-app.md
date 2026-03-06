# Merge `rara-boot` into `rara-app`

## Problem

`rara-boot` 和 `rara-app` 定位重叠。boot 是 app 启动流程中的一个阶段，没有其他消费者，独立成 crate 增加理解成本但没有复用价值。boot 中大量模块是无意义的 wrapper——单个函数被拆成独立文件，两行代码被包装成"模块"。

## Design

**方向**: 删除 boot crate，内容彻底整合进 app。不是简单搬文件，而是消除不必要的模块边界。

### 合并后 app 文件结构

```
app/src/
  lib.rs        — AppConfig, AppHandle, start(), start_with_options(), shutdown
  boot.rs       — kernel 依赖组装（原 state.rs 的 init 逻辑 + llm_registry、
                   identity resolver、user store、manifests、mcp、composio、
                   skills 全部内联为 private fn / private type）
  flatten.rs    — config flattening（已有，不动）
  tools/        — tool implementations（从 boot 搬入，内部结构不变）
```

### boot 模块逐个处理

| 原模块 | 处置 | 理由 |
|--------|------|------|
| `state.rs` | 内联到 `boot.rs` | `RaraState` 只是启动流程的中间产物，不再是独立类型 |
| `llm_registry.rs` | 内联到 `boot.rs` 的 private fn | 单个函数 `build_driver_registry()`，只在 init 调一次 |
| `user_store.rs` | 内联到 `boot.rs` | `UserConfig` 留作 pub type（AppConfig 引用），`InMemoryUserStore` 为 private type |
| `resolvers.rs` | 内联到 `boot.rs` | `PlatformIdentityResolver` 40 行，只在 start 里构造一次 |
| `manifests.rs` | 内联到 `boot.rs` 的 private fn | `load_default_registry()` 30 行，只调一次 |
| `mcp.rs` | 内联到 `boot.rs` 的 private fn | 两个函数，只在 init 调 |
| `composio.rs` | 内联到 `boot.rs` | `SettingsComposioAuthProvider` 20 行逻辑，只在 init 构造一次 |
| `skills.rs` | 内联到 `boot.rs` | 两行代码 |
| `outbox.rs` | 删除 | 死代码，`PersistentOutboxStore` 无任何消费者 |
| `tools/` | 搬到 `app/src/tools/` | 10+ tool 实现，保持内部结构 |
| `bus.rs` | 删除 | 空文件，只有过时注释 |
| `kernel.rs` | 删除 | 空模块，只有 doc comment |
| `error.rs` | 删除 | 仅 `BootError::McpRegistry`，改用 `Whatever` |
| `components.rs` | 删除 | 一行 wrapper，直接内联 |

### boot.rs 设计

`boot.rs` 负责"从配置构建出 kernel 需要的所有依赖"。对外暴露：

```rust
/// boot.rs 对 lib.rs 暴露的接口
pub(crate) struct BootResult {
    pub credential_store:  KeyringStoreRef,
    pub driver_registry:   Arc<DriverRegistry>,
    pub tool_registry:     Arc<ToolRegistry>,
    pub user_store:        Arc<dyn UserStore>,
    pub session_index:     Arc<dyn SessionIndex>,
    pub tape_service:      TapeService,
    pub skill_registry:    InMemoryRegistry,
    pub mcp_manager:       McpManager,
    pub settings_provider: Arc<dyn SettingsProvider>,
    pub identity_resolver: Arc<dyn IdentityResolver>,
    pub agent_registry:    Arc<AgentRegistry>,
}

pub(crate) async fn boot(
    pool: SqlitePool,
    settings_provider: Arc<dyn SettingsProvider>,
    users: &[UserConfig],
) -> Result<BootResult, Whatever> { ... }
```

`lib.rs` 的 `start_with_options()` 调用 `boot()` 拿到 `BootResult`，然后构建 Kernel、注册 adapters、启动 servers。

### 引用更新

- `AppConfig.users`: `rara_boot::user_store::UserConfig` → `crate::boot::UserConfig`
- `backend-admin`: 仅 doc comment 引用，更新注释，移除 Cargo.toml 依赖

### Cargo.toml 变更

- `app/Cargo.toml`: 移除 `rara-boot`，继承 boot 的所有依赖（rara-keyring-store, rara-pg-credential-store, rara-composio, rara-codex-oauth, rara-agents, rara-skills, rara-mcp, lettre 等）
- `backend-admin/Cargo.toml`: 移除 `rara-boot`
- workspace `Cargo.toml`: 移除 `rara-boot` member 和 dependency
- 删除 `crates/boot/`

## Scope

不做的事：
- 不重新设计 tools 架构
- 不触碰 kernel 或其他下游 crate
- 不修改初始化逻辑本身（只消除不必要的模块边界）
