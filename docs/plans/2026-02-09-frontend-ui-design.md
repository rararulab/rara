# Frontend UI Design

## Stack
- Vite + React 19 + TypeScript
- Tailwind CSS + shadcn/ui (Radix UI primitives)
- React Router v7 (client-side routing)
- TanStack Query (React Query) for server state
- Location: `web/` directory in monorepo

## Pages
1. **Dashboard** — Analytics overview with stat cards and derived rates
2. **Applications** — Job application pipeline with CRUD + status transitions + history
3. **Resumes** — Resume management CRUD
4. **Interviews** — Interview plan tracking with status updates + prep regeneration
5. **Job Discovery** — Trigger job searches and view scraped results
6. **Notifications** — Notification log with retry capability
7. **Scheduler** — Cron task management with enable/disable + execution history

## API Endpoints (31 total)
- Resume: 5 (CRUD)
- Application: 7 (CRUD + transition + history)
- Interview: 7 (CRUD + status + prep)
- Notification: 4 (list + detail + stats + retry)
- Scheduler: 5 (list + detail + enable/disable + history)
- Analytics: 6 (CRUD + latest + rates)

## Project Structure
```
web/
├── src/
│   ├── api/client.ts          # Fetch-based API client
│   ├── components/ui/         # shadcn/ui components
│   ├── layouts/DashboardLayout.tsx
│   ├── pages/{Dashboard,Applications,Resumes,Interviews,JobDiscovery,Notifications,Scheduler}.tsx
│   └── lib/utils.ts
```

## Issues
- #55: Project initialization (Vite + React + Tailwind + shadcn + Router + Layout + API client)
- #56: Dashboard page
- #57: Applications page
- #58: Resumes + Interviews pages
- #59: Notifications + Scheduler + Job Discovery pages
