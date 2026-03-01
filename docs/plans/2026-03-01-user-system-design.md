# User System Design

**Date**: 2026-03-01
**Status**: Approved

## Overview

Build a complete user system with JWT authentication, invite-code registration,
Telegram account linking, role-based agent routing (nana for regular users, rara
for root/admin), and admin UI.

## Architecture

### Authentication Flow

```
Register (username + password + invite_code)
    → POST /api/v1/auth/register
    → Create KernelUser (Role::User)
    → Return JWT (access_token 1h + refresh_token 7d)

Login (username + password)
    → POST /api/v1/auth/login
    → Verify argon2 hash
    → Return JWT (access_token + refresh_token)

WebSocket Chat
    → ws://host/api/v1/kernel/chat/ws?token=<jwt>
    → Validate JWT, extract user_id
    → IdentityResolver looks up KernelUser → Principal
    → Route to nana (User) or rara (Root/Admin)
```

### User Roles & Agent Routing

| Role  | Default Agent | Permissions              |
|-------|---------------|--------------------------|
| Root  | rara          | All                      |
| Admin | rara          | ManageUsers + extended   |
| User  | nana          | Basic chat only          |

### Database Changes

#### Migration: Add password_hash to kernel_users

```sql
ALTER TABLE kernel_users ADD COLUMN password_hash TEXT;
```

#### Migration: invite_codes table

```sql
CREATE TABLE invite_codes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    created_by UUID NOT NULL REFERENCES kernel_users(id),
    used_by UUID REFERENCES kernel_users(id),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

#### Migration: link_codes table (for TG binding)

```sql
CREATE TABLE link_codes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    user_id UUID NOT NULL REFERENCES kernel_users(id),
    direction TEXT NOT NULL CHECK (direction IN ('web_to_tg', 'tg_to_web')),
    platform_data JSONB,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### API Endpoints

#### Auth (unauthenticated)

```
POST /api/v1/auth/login          { username, password } → { access_token, refresh_token, user }
POST /api/v1/auth/register       { username, password, invite_code } → { access_token, refresh_token, user }
POST /api/v1/auth/refresh        { refresh_token } → { access_token }
```

#### User (authenticated)

```
GET    /api/v1/users/me                    → current user + linked platforms
POST   /api/v1/users/me/link-code          → generate TG link code { code, expires_at }
PUT    /api/v1/users/me/password            { old_password, new_password }
```

#### Admin (root only)

```
GET    /api/v1/admin/users                 → all users with linked channels
POST   /api/v1/admin/invite-codes          → generate invite code
GET    /api/v1/admin/invite-codes          → list invite codes
DELETE /api/v1/admin/users/:id             → disable user
```

### Telegram Linking (Bidirectional)

#### Direction A: Web → TG

1. User clicks "Connect Telegram" in Settings
2. Backend generates 6-char link code (5 min expiry, direction=web_to_tg)
3. Frontend displays: "Send `/link ABC123` to @BotName"
4. TG bot receives `/link ABC123`, validates code, binds chat_id → user_id
5. Creates `user_platform_identities` row (platform=telegram, platform_user_id=chat_id)

#### Direction B: TG → Web

1. User sends `/link` to TG bot
2. Bot generates link code (direction=tg_to_web), stores chat_id in platform_data
3. Bot replies with URL: `https://app/link?tg_code=XYZ789`
4. User clicks link → frontend (must be logged in) → POST to validate
5. Backend verifies code + JWT, creates platform identity binding

### Root Initialization

1. On boot, `ensure_default_users()` checks if root has `password_hash`
2. If NULL → generate 16-char random password (alphanumeric)
3. Hash with argon2 and store
4. Print to console: `Root credentials — username: root, password: <random>`
5. Root can change password via Settings after first login

### Nana Agent

- New agent manifest in `crates/core/agents/src/nana.rs`
- Personality: friendly chat assistant (rara's sister)
- No spawn permission, no tool access, pure conversational
- Kernel routes User-role principals to nana instead of rara
- AgentRegistry registers nana as a builtin manifest

### Frontend Changes

#### New Pages
- `/login` — username/password form
- `/register` — username/password + invite code
- `/admin/users` — user list + invite code management (root only)

#### Modified Pages
- Settings → new "Account" section (password change, TG linking)
- Chat.tsx → use JWT token in WebSocket connection
- DashboardLayout → show/hide admin nav based on role

#### Auth Architecture
- `AuthProvider` React context (JWT state, login/logout/refresh)
- `ProtectedRoute` wrapper (redirect to /login if unauthenticated)
- `AdminRoute` wrapper (check role === Root)
- API client adds `Authorization: Bearer <token>` header
- Token refresh on 401 response

### IdentityResolver Upgrade

Replace `DefaultIdentityResolver` with `AuthenticatedIdentityResolver`:

- **Web channel**: Parse JWT from token param → look up KernelUser → build Principal
- **TG channel**: Look up `user_platform_identities` by (telegram, chat_id) → get user → build Principal
- **Unlinked TG**: Reject with "Please link your account first"

## Issue Breakdown

### Issue 1: Backend Auth — JWT + User Domain Crate
- New `crates/domain/user/` with auth logic
- Migrations (password_hash, invite_codes, link_codes)
- Auth endpoints (login, register, refresh)
- User endpoints (me, password change)
- Root password initialization on boot
- JWT middleware (axum extractor)
- argon2 password hashing

### Issue 2: Nana Agent + Role-based Routing
- `crates/core/agents/src/nana.rs` manifest
- Register nana in AgentRegistry as builtin
- Kernel message routing: User → nana, Root/Admin → rara
- IdentityResolver upgrade to query DB

### Issue 3: TG Account Linking
- `/link` command handler in TG adapter
- Link code generation + validation endpoints
- Bidirectional flow (web→tg and tg→web)
- Platform identity CRUD

### Issue 4: Admin API + User Management
- Admin user list endpoint (with linked channels)
- Invite code CRUD
- User disable/enable
- Permission checks (root only)

### Issue 5: Frontend Auth Pages + Provider
- AuthProvider context + token management
- Login page
- Register page (with invite code)
- ProtectedRoute / AdminRoute guards
- API client auth header + refresh logic
- WebSocket token integration

### Issue 6: Frontend Settings + Admin UI
- Settings > Account (password, TG linking)
- Admin > Users page (user list, invite codes)
- Admin nav visibility by role
