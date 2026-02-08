# Job Automation Backend

后端服务项目，用于自动化职位发现、投递追踪、AI 简历优化与面试准备。

## Project Direction

- 技术栈基线：`axum` + `tokio` + `sqlx` + PostgreSQL
- 数据层基石：`crates/common/yunara-store`
- 模块化策略：按业务域拆分为多个 workspace crate
- 运行形态：长期运行 HTTP Server + 后台调度任务
- 通知与人工闸门：Telegram（关键自动化动作先确认）

## Current Roadmap

- Epic: [#3](https://github.com/crrow/job/issues/3)
- 架构与数据：[#4](https://github.com/crrow/job/issues/4) / [#5](https://github.com/crrow/job/issues/5)
- 存储与模块化：[#17](https://github.com/crrow/job/issues/17) / [#19](https://github.com/crrow/job/issues/19)
- API 与基础设施：[#18](https://github.com/crrow/job/issues/18)
- 模板清理与命名重构：[#16](https://github.com/crrow/job/issues/16)

## Development Notes

- 当前仓库仍包含 template 遗留代码，后续会在 #16 中清理，但会保留部分“代码风格样例”供新模块参考。
- 项目未上线，可进行非兼容重构（例如 crate 重命名、存储切换、API 调整）。
- 详细执行约束见 `AGENT.md`。
