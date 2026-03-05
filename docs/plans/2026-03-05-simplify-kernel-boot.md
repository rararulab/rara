# Simplify Kernel Boot

## Problem

1. `BootConfig` 是无意义的中间层 — 15 个字段，很多 `Option` 在 `boot()` 里直接 `.unwrap()`
2. `Kernel::new()` 接收 12 个位置参数，`#[allow(clippy::too_many_arguments)]`
3. `AppConfig::start_with_options()` 是 280 行上帝方法，AppConfig 应该是纯配置
4. registries 不应作为构造参数 — 它们是可加载模块

## Design

### Kernel Builder Pattern

Kernel 构造器只接收基础设施依赖。registries/adapters 通过注册方法加载。

```rust
// 1. 构造核心 Kernel（基础设施依赖）
let mut kernel = Kernel::builder()
    .config(kernel_config)          // KernelConfig (concurrency limits etc.)
    .session_index(session_index)   // Arc<dyn SessionIndex>
    .tape_store(tape_store)         // Arc<FileTapeStore>
    .settings(settings)             // Arc<dyn SettingsProvider>
    .security(security)             // SecurityRef
    .build();

// 2. 加载内核模块
kernel.load_drivers(driver_registry);
kernel.load_tools(tool_registry);
kernel.load_agents(agent_registry);

// 3. 注册 I/O adapters
kernel.register_adapter(ChannelType::Web, web_adapter);
kernel.register_adapter(ChannelType::Telegram, tg_adapter);

// 4. 启动
let (kernel, handle) = kernel.start(cancel_token);
```

### 删除 BootConfig

`boot::kernel::BootConfig` 和 `boot::kernel::boot()` 完全删除。Kernel 自己提供 builder。

### AppConfig 回归纯配置

- `AppConfig` 只保留 `Deserialize` + field definitions
- `start_with_options()` / `start()` / `run()` 移出 AppConfig
- 启动编排逻辑移到一个独立的 `pub async fn run(config: AppConfig)` 函数（在 `rara-app` crate 里）

### Kernel 内部变化

Builder 在 `build()` 时创建 I/O 子系统（StreamHub、EventQueue、IngressPipeline 等），和当前 `Kernel::new()` 做的一样。区别在于：

- `driver_registry`、`tool_registry`、`agent_registry` 不在构造时传入
- 它们通过 `load_*` 方法设置（在 `start()` 之前调用）
- `SyscallDispatcher` 在 `start()` 时组装（此时 registries 已就绪）
- 或者 `SyscallDispatcher` 内部持有 `Arc<DriverRegistry>` 等，`load_*` 方法更新它们

### 可选的 I/O 子系统配置

`StreamHub` capacity、`IdentityResolver`、`SessionResolver` 等现在在 BootConfig 里的可选配置：
- 有 sensible defaults 的放进 `KernelConfig`（如 `stream_capacity`）
- resolvers 作为 `Kernel::builder()` 的可选参数（bon builder 自动 default None）
- `kv_operator` 同理，default 到 in-memory

## Scope

### In Scope
- Kernel builder pattern（替换 12 参数 `new()` + BootConfig）
- `load_drivers()` / `load_tools()` / `load_agents()` 注册方法
- 删除 `BootConfig` + `boot::kernel::boot()`
- AppConfig 回归纯配置，启动逻辑独立函数

### Out of Scope
- RaraState 并行化初始化
- 拆分 `start_with_options()` 的 HTTP/gRPC 启动逻辑（follow-up）
- Agent manifest 加载方式变更

## Files Changed

- `crates/kernel/src/kernel.rs` — Kernel builder, `load_*` 方法, 删除 12 参数 `new()`
- `crates/boot/src/kernel.rs` — 删除 BootConfig + boot()
- `crates/app/src/lib.rs` — AppConfig 纯配置, 启动逻辑移到独立函数
- `crates/boot/src/state.rs` — 可能简化（RaraState 字段减少）
- `crates/kernel/src/syscall.rs` — SyscallDispatcher 可能需要调整
