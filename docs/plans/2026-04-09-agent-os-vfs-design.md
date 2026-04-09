# Agent-OS-Inspired Virtual Filesystem Design for Rara

## Summary

针对 rara，我建议采用 **进程内虚拟文件系统 + 可持久化的 copy-on-write overlay**，而不是直接依赖 Linux `overlayfs`、ZFS snapshot 或宿主机真实绝对路径。

核心结论：

- **技术方案选型**：采用类似 `rivet-dev/agent-os` 的三层模型，但先做 rara 约束下的精简版本。
- **API 设计**：对 agent 暴露稳定的 guest path，例如 `/workspace`、`/data`、`/artifacts`、`/tmp`，不再把宿主机绝对路径作为主要接口。
- **与 ralph task agent 整合**：每个 issue/session 拥有独立 mount table 和 upper layer；child worker 通过 snapshot/fork 继承 lower layers。
- **实施优先级**：先抽象文件系统接口与挂载点，再加 overlay snapshot，最后加远端 mount plugin。

这不是在 rara 里复制一个完整的 Alpine rootfs。rara 当前的核心需求是 agent 文件隔离、任务级持久数据和稳定的虚拟挂载点，不是运行完整用户态 Linux。v1 应该先做 **最小可用 guest filesystem**。

## Background

rara 现在的文件能力主要建立在真实文件系统之上：

- `crates/app/src/tools/read_file.rs`
- `crates/app/src/tools/write_file.rs`
- `crates/kernel/src/guard/path_scope.rs`
- `crates/symphony/src/service.rs`

当前模型的优点是简单，但有三个明显短板：

1. agent 看到的是宿主机路径，不是稳定的 guest path。
2. worker / task execution 没有标准化的文件系统快照边界。
3. `/data` 这类“逻辑挂载点”目前不存在，只能靠调用方自己约定真实目录。

而 `agent-os` 的文件系统设计更接近一个 agent runtime kernel：

- `VirtualFileSystem` trait 定义统一文件操作接口。
- `OverlayFileSystem` 维护 `lowers + upper`，通过 whiteout/opaque marker 处理删除与目录覆盖。
- `MountTable` 按最长前缀分发 guest path，并支持只读 mount。
- `FileSystemPluginFactory` 按 `vm_id + guest_path + config` 打开挂载文件系统实例。

对应参考：

- `docs/filesystem.mdx`
- `crates/kernel/src/vfs.rs`
- `crates/kernel/src/overlay_fs.rs`
- `crates/kernel/src/mount_table.rs`
- `crates/kernel/src/mount_plugin.rs`

## Requirements for Rara

rara 方案至少要满足下面几点：

1. agent API 稳定，工具和 prompt 里可以直接使用 `/data/...` 这类路径。
2. 不依赖 Linux 特权特性，macOS 开发机和 CI 都能工作。
3. child worker 可以从 parent 继承只读视图，并拥有自己的 writable upper layer。
4. 现有 `PathScopeGuard` 和工具权限体系能继续工作，而不是被旁路。
5. 能落到当前任务系统和 issue worktree 流程上，而不是做一个独立于 Ralph/Symphony 的孤立子系统。

## Options

| 选项 | 结论 | 理由 |
|------|------|------|
| 继续直接使用宿主机文件系统 + path guard | 不选 | 简单，但没有虚拟挂载点、没有 task snapshot、没有真正的 guest namespace |
| 直接用 OS-level `overlayfs` / ZFS / btrfs snapshot | 不选 | 依赖平台能力和权限；macOS、本地开发、CI 一致性差 |
| 进程内 VFS + mount table + persisted upper layer | 选用 | 跨平台、与 rara kernel/tool 模型一致，也最接近 agent-os 的抽象方式 |

因此，**推荐方案** 是：

- 在 rara 内部引入统一的 VFS trait。
- 用 guest path 驱动所有结构化文件工具。
- 用可持久化 upper layer 表达写时复制和 snapshot/fork。
- 用 mount table 将 `/workspace`、`/data` 等逻辑路径映射到不同 backend。

## Decision

采用 **agent-os 风格的逻辑 overlay 架构**，但在 rara v1 中做两点裁剪：

1. **root base 不做 Alpine snapshot**，先用最小 synthetic root。
2. **mount backend 先支持 host-backed directory 和 session-owned data store**，远端 plugin 后置。

### Core Types

建议新增 `rara-kernel` 下的文件系统子模块，或者独立 crate（例如 `crates/vfs`），暴露下面几类核心抽象：

```rust
pub trait AgentFileSystem: Send + Sync {
    fn read_file(&self, path: &GuestPath) -> Result<Vec<u8>, FsError>;
    fn write_file(&self, path: &GuestPath, bytes: &[u8]) -> Result<(), FsError>;
    fn list_dir(&self, path: &GuestPath) -> Result<Vec<DirEntry>, FsError>;
    fn create_dir_all(&self, path: &GuestPath) -> Result<(), FsError>;
    fn remove_file(&self, path: &GuestPath) -> Result<(), FsError>;
    fn metadata(&self, path: &GuestPath) -> Result<FileMetadata, FsError>;
}

pub struct SessionFsHandle {
    pub session_id: SessionKey,
    pub mount_table: MountTable,
    pub root_snapshot: SnapshotId,
}

pub struct MountSpec {
    pub guest_path: GuestPath,
    pub source: MountSource,
    pub access: AccessMode,
}

pub enum MountSource {
    WorkspaceHostDir { host_path: PathBuf },
    SessionDataDir { session_id: SessionKey },
    ArtifactsDir { session_id: SessionKey },
    EphemeralMemory,
    FuturePlugin { plugin_id: String, config: serde_json::Value },
}
```

关键点：

- `GuestPath` 是新的内核路径类型，始终是 `/workspace/foo.rs` 这种 guest path。
- `MountTable` 负责 guest path 到 backend 的最长前缀路由。
- `SessionFsHandle` 绑定到 session / worker 生命周期。
- snapshot 不直接暴露给 agent，只作为 kernel/worker fork 的内部实现。

### Mount Layout

建议 rara 默认提供下面几个挂载点：

| 挂载点 | 语义 | 初始 backend | 持久性 |
|--------|------|--------------|--------|
| `/workspace` | 当前 issue worktree 或用户显式允许的工程目录 | host directory mount | 持久 |
| `/data` | agent/session 私有工作数据 | session-owned overlay upper | 持久 |
| `/artifacts` | 日志、截图、导出文件 | host directory mount | 持久 |
| `/tmp` | 临时中间文件 | memory or ephemeral upper | 非持久 |

这四个点足够覆盖当前 rara 的主路径：

- `/workspace` 对应现有代码仓库工作区。
- `/data` 解决“给 agent 一个稳定私有目录”的问题。
- `/artifacts` 方便 review、verify、browser QA 等流程回收结果。
- `/tmp` 保持 shell/tool 工作流的临时性。

### Overlay Model

建议采用下面的层次：

```text
synthetic root (read-only)
  + mounted filesystems (/workspace, /artifacts, maybe future /vault)
  + session upper layer (/data, whiteouts, guest-level metadata)
```

更具体地说：

1. **Base layer**：最小 synthetic root，只包含挂载点目录和必要元数据。
2. **Mounted filesystems**：`/workspace`、`/artifacts` 等直接映射到 backend。
3. **Writable upper**：对 guest namespace 的新增、删除、重命名、元数据写入统一落在 session upper。

这里的 `upper` 不必一开始就完全内存化。更适合 rara 的实现是：

- 用宿主机目录持久化 `upper` 内容；
- 用 manifest/metadata 文件表达 whiteout 和 snapshot lineage；
- child worker fork 时只复用 lower snapshot ID，并创建新的 upper 目录。

这样可以保留 agent-os 的语义优势，同时避免把大文件全塞进内存。

## API Design

### Agent-Facing Tool API

现有结构化文件工具应该从“宿主机路径 API”迁移到“guest path API”。

例如：

```rust
#[derive(Debug, Deserialize)]
pub struct ReadFileParams {
    pub file_path: String, // "/workspace/src/lib.rs" or "/data/notes/todo.md"
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}
```

对 agent 来说仍然是 `file_path`，但其含义从“host absolute path”变成“guest absolute path”。

这有三个直接好处：

1. prompt 可以稳定写 `/data/foo.json`，不依赖机器目录结构。
2. worker / parent 之间共享统一命名空间。
3. guard、审计和 trace 可以围绕 guest path 做，而不是围绕不可预测的真实路径做。

### Kernel-Facing Session API

建议在 kernel handle 或 session bootstrap 阶段增加挂载描述符：

```rust
pub struct SessionFsDescriptor {
    pub root: RootMode,
    pub mounts: Vec<MountSpec>,
    pub fork_from: Option<SnapshotId>,
}

pub enum RootMode {
    Synthetic,
    FutureBaseSnapshot { snapshot_id: SnapshotId },
}
```

使用方式：

- 普通 chat session：只挂 `/data` 和 `/tmp`。
- issue worktree session：挂 `/workspace`、`/data`、`/artifacts`、`/tmp`。
- child worker：`fork_from = parent.root_snapshot`，然后创建新的 writable upper。

## Integration with Ralph Task Agent

这是本方案里最重要的 rara-specific 部分。

### 1. Issue Runner Bootstrap

`crates/symphony/src/service.rs` 现在已经管理 Ralph task runner 的 workspace 生命周期。建议它在启动 issue worktree 时同步生成 session filesystem descriptor：

| Issue session mount | 来源 |
|---------------------|------|
| `/workspace` | 当前 issue 的 git worktree |
| `/data` | issue/session 私有状态目录 |
| `/artifacts` | review、verify、截图、patch 输出目录 |
| `/tmp` | ephemeral upper |

这样，task agent 不再需要知道 worktree 在宿主机上的真实绝对路径。

### 2. Worker Fork Semantics

当 planner/background worker/reviewer/verifier 从 parent session 派生时：

- 继承 parent 的 mount table 配置；
- 共享 lower snapshot；
- 创建新的 upper；
- 默认让 `/artifacts/<child-id>` 独立，避免输出互相覆盖。

这相当于把 agent-os 的 “per-VM writable overlay” 映射到 rara 的 “per-session / per-worker upper layer”。

### 3. Task Report and Artifact Handoff

`crates/kernel/src/task_report.rs` 目前只汇报逻辑任务结果，没有文件系统上下文。

我建议 v2 增加轻量的结果字段，而不是在 v1 一次性做大改：

```rust
pub struct TaskFilesystemSummary {
    pub mounts: Vec<String>,
    pub artifacts_root: Option<String>,
    pub snapshot_id: Option<String>,
}
```

这能让 review/verify 流程直接消费 `/artifacts` 和 snapshot lineage，而不是重新猜测文件落点。

### 4. Path Guard Migration

`PathScopeGuard` 仍然需要保留，但职责要变化：

- 现在：校验宿主机路径是否落在 workspace/whitelist 里。
- 迁移后：优先校验 guest path 是否允许访问对应 mount。

建议拆成两层：

1. `GuestPathGuard`：限制 agent 可访问哪些虚拟挂载点，例如默认允许 `/workspace`、`/data`、`/artifacts`、`/tmp`。
2. `HostMountPolicy`：定义哪些 host directories 可以被映射为 mount source。

这样可以避免 agent 通过“知道宿主机路径”直接绕过虚拟层。

## Non-Goals

v1 不做下面这些事情：

- 不提供完整 Linux rootfs。
- 不把所有 shell 命令都重写成纯 VFS syscall。
- 不在第一阶段支持 S3、GDrive、SQLite 远端 mount。
- 不在第一阶段替换所有现有真实文件系统调用点。

## Milestones

| Milestone | 内容 | 优先级 |
|-----------|------|--------|
| M0 | 设计文档定稿，明确 guest path 语义和挂载点 | P0 |
| M1 | 引入 `GuestPath`、`AgentFileSystem`、`MountTable`，让结构化文件工具先走虚拟层；backend 仍可直接映射 host dir | P0 |
| M2 | 增加 session `/data` 和 `/tmp`，实现 persisted upper layer、whiteout、snapshot lineage | P1 |
| M3 | worker fork 继承 snapshot，接入 Ralph/Symphony issue runner，补 task artifact summary | P1 |
| M4 | 抽象 plugin registry，支持 Vault/S3/SQLite/remote mounts | P2 |

## Implementation Order

### Phase 1: Virtual Paths Without Full Overlay

先只做抽象层，不急着把所有 copy-on-write 细节做完：

1. 新增 `GuestPath` 和 `MountTable`。
2. 让 `read-file`、`write-file`、`edit-file`、`delete-file`、`list-directory` 走 `AgentFileSystem`。
3. 将当前 workspace 目录挂到 `/workspace`。
4. 新增 `/data` 目录，先落到简单的 session-owned host path。

这个阶段交付后，agent prompt 和 tool schema 已经可以稳定使用 `/data`。

### Phase 2: Persisted Overlay Snapshot

在 Phase 1 稳定后补 overlay 语义：

1. 为 `/data` 和 guest metadata 增加 upper layer。
2. 引入 whiteout/opaque marker。
3. 增加 session fork snapshot。
4. 在 child worker 创建时复用 lower snapshot。

### Phase 3: Task-Agent-Native Filesystem Lifecycle

最后再把它和 Ralph/Symphony 深度打通：

1. issue runner 创建 mount descriptor。
2. review/verify 共享 artifact convention。
3. task report 包含 filesystem summary。
4. 清理策略区分 `/data` 持久状态和 `/tmp` 临时状态。

## Why This Fits Rara Better Than a Direct agent-os Port

agent-os 运行在 WebAssembly + V8 isolates 的 VM 语境里，所以它自然需要一个完整的 in-memory root filesystem 和 mount plugin 体系。

rara 当前的现实约束不同：

- 主要执行环境是 Rust 进程 + host worktree。
- 文件工具和 shell 工作流已经存在。
- 任务编排重点在 session/worker/issue lifecycle，而不是 VM lifecycle。

因此 rara 的最佳路径不是“照搬 agent-os 内部实现”，而是 **复用其抽象边界**：

- `VirtualFileSystem` 对应 rara 的 `AgentFileSystem`
- `OverlayFileSystem` 对应 rara 的 session upper/lower snapshot
- `MountTable` 对应 rara 的 guest path dispatcher
- plugin factory 对应 rara 后续的 mount backend registry

这能保留核心能力，同时不把 rara 拉进一个过重的 VM/runtime 项目。

## Recommendation

最终建议如下：

1. **采用进程内 VFS + mount table + persisted overlay upper**，不选 OS-level overlayfs。
2. **先提供 `/workspace`、`/data`、`/artifacts`、`/tmp` 四个稳定挂载点**。
3. **先做 guest path 抽象，再做 snapshot/fork**，不要一开始就追求完整 VM rootfs。
4. **把 Ralph/Symphony 的 issue runner 当作第一批集成点**，让任务系统直接受益于虚拟挂载和 snapshot。

这个方案对 rara 来说是最稳的切入点：既能解决 `/data` 这类 agent-facing API 问题，也给后续 worker isolation、artifact handoff 和 remote mount 留出了清晰演进路径。

## References

- [rivet-dev/agent-os: docs/filesystem.mdx](https://github.com/rivet-dev/agent-os/blob/main/docs/filesystem.mdx)
- [rivet-dev/agent-os: crates/kernel/src/vfs.rs](https://github.com/rivet-dev/agent-os/blob/main/crates/kernel/src/vfs.rs)
- [rivet-dev/agent-os: crates/kernel/src/overlay_fs.rs](https://github.com/rivet-dev/agent-os/blob/main/crates/kernel/src/overlay_fs.rs)
- [rivet-dev/agent-os: crates/kernel/src/mount_table.rs](https://github.com/rivet-dev/agent-os/blob/main/crates/kernel/src/mount_table.rs)
- [rivet-dev/agent-os: crates/kernel/src/mount_plugin.rs](https://github.com/rivet-dev/agent-os/blob/main/crates/kernel/src/mount_plugin.rs)
- [rara: docs/plans/2026-03-13-plan-execute-architecture.md](../plans/2026-03-13-plan-execute-architecture.md)
- [rara: crates/app/src/tools/read_file.rs](../../crates/app/src/tools/read_file.rs)
- [rara: crates/app/src/tools/write_file.rs](../../crates/app/src/tools/write_file.rs)
- [rara: crates/kernel/src/guard/path_scope.rs](../../crates/kernel/src/guard/path_scope.rs)
- [rara: crates/symphony/src/service.rs](../../crates/symphony/src/service.rs)
- [rara: crates/kernel/src/task_report.rs](../../crates/kernel/src/task_report.rs)
