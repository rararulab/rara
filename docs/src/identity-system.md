# Identity & User System

Rara uses a config-driven identity system that maps external platform accounts (Telegram, Web, CLI) to internal kernel users.

## Overview

```mermaid
flowchart LR
    subgraph Platform
        TG["Telegram user 123"]
        WEB["Web user ryan"]
    end
    subgraph "IngressPipeline"
        IR["IdentityResolver"]
    end
    subgraph Kernel
        KU["KernelUser 'ryan'"]
        SR["SessionResolver"]
        S["Session"]
    end

    TG --> IR
    WEB --> IR
    IR -->|"in-memory mapping"| KU
    KU --> SR --> S
```

The identity resolution flow has two stages:

1. **IdentityResolver** — maps `(channel_type, platform_user_id)` to a kernel `UserId`
2. **SessionResolver** — maps `(user, channel_type, chat_id)` to a `SessionKey`

## Configuration

Users and their platform bindings are defined in the YAML config file:

```yaml
users:
  - name: "ryan"
    role: root
    platforms:
      - type: telegram
        user_id: "123456789"
      - type: web
        user_id: "ryan"
  - name: "alice"
    role: user
    platforms:
      - type: telegram
        user_id: "987654321"
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Kernel user name (must be unique) |
| `role` | string | yes | `"root"`, `"admin"`, or `"user"` |
| `platforms` | array | no | Platform identity bindings |
| `platforms[].type` | string | yes | Channel type: `"telegram"`, `"web"`, `"cli"`, etc. |
| `platforms[].user_id` | string | yes | Platform-side user identifier |

### Roles and Permissions

| Role | Permissions | Description |
|------|-------------|-------------|
| `root` | `All` | Superuser, bypasses all checks |
| `admin` | `All` | Full access, used for service accounts |
| `user` | `Spawn` | Can spawn agent processes |

## Boot Sequence

At startup, Rara processes the `users` config in this order:

1. **`ensure_default_users`** — creates built-in `root` and `system` users if absent
2. **`ensure_configured_users`** — for each entry in `users`:
   - Creates the `KernelUser` record in SQLite if it doesn't exist
   - Updates the role if it changed in config
3. **`PlatformIdentityResolver`** is built from the `users` config as an in-memory `HashMap<(channel_type, platform_uid), user_name>`

Both steps are idempotent and safe to run on every startup.

## Resolver Modes

Rara selects the identity resolver based on whether `users` is configured:

| Config | Resolver | Behavior |
|--------|----------|----------|
| `users` absent or empty | `DefaultIdentityResolver` | All channels resolve to the single owner (`root`) |
| `users` present | `PlatformIdentityResolver` | In-memory `HashMap` lookup by `(channel_type, platform_uid)` |

### Unknown Platform Users

When `PlatformIdentityResolver` is active and a message arrives from a platform user not listed in the config, the message is **silently dropped**. No reply is sent. The event is logged at `debug` level.

## Architecture

### Key Types

| Type | Location | Description |
|------|----------|-------------|
| `UserId(String)` | `rara-kernel` | Runtime identity (user name) |
| `KernelUser` | `rara-kernel` | Persistent user record (UUID, role, permissions) |
| `Principal` | `rara-kernel` | Runtime security context derived from `KernelUser` |
| `UserConfig` | `rara-boot` | YAML config entry for a user |
| `PlatformBindingConfig` | `rara-boot` | YAML config entry for a platform binding |

### Key Components

| Component | Location | Responsibility |
|-----------|----------|----------------|
| `IdentityResolver` trait | `crates/kernel/src/io.rs` | Maps platform identity to `UserId` |
| `DefaultIdentityResolver` | `crates/boot/src/resolvers.rs` | Single-owner mode (ignores platform ID) |
| `PlatformIdentityResolver` | `crates/boot/src/resolvers.rs` | Config-driven mode (in-memory HashMap lookup) |
| `SqliteUserStore` | `crates/boot/src/user_store.rs` | CRUD for `kernel_users` |
| `SecuritySubsystem` | `crates/kernel/src/security.rs` | Validates user exists, is enabled, has permissions |
