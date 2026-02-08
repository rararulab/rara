# 2026-02-08: Template 清理与 crate 重命名计划（Issue #16）

## 目标

将当前仓库从 `rsketch` template 收敛为 `job` 项目的基线，同时保留可复用的代码风格样例，避免未来新模块出现两套风格与两套命名体系。

## 范围

* Workspace 元信息与仓库指向：`Cargo.toml` 中 `repository/homepage/keywords/categories` 与实际仓库一致。
* crate/binary 命名：消除 `rsketch-*` 前缀，统一为 `job-*`（或未来确认的统一前缀）。
* template 遗留模块清理：删除/归档当前项目明确不需要的子系统，但保留“风格参考”的实现片段（例如错误处理、telemetry、worker patterns）。

## 约束

* 当前项目未上线：允许非兼容重构（crate rename、API rename、schema 调整）。
* 迁移过程必须保持 `cargo check --workspace` 可持续通过。
* 每次大改动优先拆小步提交，便于回滚与 bisect。

## 当前 crate/binary 现状（待重命名）

建议映射（v0，若你更偏好 `yunara-*` 或其他前缀可在此处调整）：

* `rsketch-api` -> `job-api`
* `rsketch-server` -> `job-server`
* `rsketch-app` -> `job-app`
* `rsketch-cmd` -> `job-cli`
  * `[[bin]] name = "rsketch"` -> `[[bin]] name = "job"`
* `rsketch-base` -> `job-base`
* `rsketch-error` -> `job-error`
* `rsketch-paths` -> `job-paths`
* `rsketch-common-runtime` -> `job-common-runtime`
* `rsketch-common-telemetry` -> `job-common-telemetry`
* `rsketch-common-worker` -> `job-common-worker`
* `downloader` -> `job-downloader`（可选：若希望该 crate 继续保持泛化，可暂不改名）

`yunara-store` 保持不变（它是后端数据层基石，且名称已独立）。

## 执行步骤（建议）

1. Rename workspace 依赖键（`Cargo.toml` 的 `[workspace.dependencies]` 内部 crate 映射）。
2. 逐个 crate 重命名 `package.name`，同步调整依赖引用。
3. 调整二进制名与文档引用（例如 `rsketch` -> `job`）。
4. 跑通基础验证：
   * `RUSTC_WRAPPER= cargo check --workspace`
5. 进行 template 清理（删除明确不需要的模块），并把保留的“风格参考”在 `docs/` 中明确标注用途。

## 风险与注意点

* crate rename 会影响：
  * `Cargo.toml` 的 workspace 依赖键
  * `use` 路径（若存在跨 crate re-export）
  * 发布/打包配置（例如 `cargo-dist`、`release-plz`、`cliff`）
* 建议在 rename 前先把 `repo/homepage` 等元信息对齐，减少后续混淆。

