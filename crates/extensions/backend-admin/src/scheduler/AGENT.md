# scheduler — Agent Guidelines

## Purpose
HTTP admin surface over the kernel scheduler. Read-only curation: list/get/delete/trigger/history. No `POST /jobs` — creation is agent-tool-only so every scheduled job stays tied to a real session principal.

## Architecture
- `dto.rs` — wire DTOs (`JobView`) and the `TaskReportStatus → last_status` mapping.
- `service.rs` — `SchedulerSvc`: reads go direct to `JobWheel` + `JobResultStore`; mutations (`RemoveJob`, `TriggerJob`) dispatch via the kernel event queue.
- `router.rs` — axum handlers at `/api/v1/scheduler/jobs[/...]`, history limit clamp.

## Critical Invariants
- Auth is applied by the upstream `backend-admin` router layer. Do NOT add auth here — duplicating gates drifts between routes.
- `last_status` mapping is load-bearing for the frontend: `Completed → "ok"`, `Failed → "failed"`, `NeedsApproval → "running"`, no latest result → `null`. Keep `status_label` authoritative.
- `TriggerJob` must leave `next_at` untouched — the kernel enforces this in `JobWheel::trigger_now`; don't introduce client-side rewrites that fake it.
- `Principal` must never appear in any response DTO. It's internal kernel state.

## What NOT To Do
- Do NOT add `POST /api/v1/scheduler/jobs` — creation bypasses session principals and breaks the audit trail (see epic #1686).
- Do NOT reuse `Syscall::ListJobs` for the admin list — it's the session-scoped variant; use `ListAllJobs` or `KernelHandle::list_jobs(None)` so future permission tightening on the admin surface doesn't regress tool UX.
- Do NOT expand `SchedulerError::JobNotFound` to cover generic kernel failures — the HTTP layer maps it to 404; folding infra errors in would silently convert 500s to 404s.

## Dependencies
- Upstream: `rara-kernel` (`KernelHandle`, `schedule::*`, `task_report::TaskReportStatus`, `event::Syscall`).
- Downstream: `crate::state::BackendState::routes` mounts this via `scheduler_routes(handle)`.
