# RAR-17 Authentication Implementation Plan

## Current State

- `web/src/pages/Login.tsx` is a token-only form that stores an owner token locally.
- `web/src/contexts/AuthContext.tsx` only tracks the presence of `access_token` in `localStorage`.
- The backend does not expose a user login API.
- `crates/rara-model/migrations/20260304000000_init.up.sql` defines `kernel_users`, but it only stores kernel identity metadata and does not support email/password auth, email verification, account lockout, or OAuth identities.

## Gap Analysis

To satisfy RAR-17, the system needs explicit user-authentication primitives that do not exist yet:

- User credentials: email, password hash, password lifecycle metadata.
- Verification state: email verification token or verified timestamp.
- Abuse protection: failed login counter and lockout window.
- Session/auth output: JWT signing and claims contract.
- OAuth account linkage: provider, provider user id, linked local user.

## Proposed Delivery Order

1. Extend the auth data model and configuration surface.
2. Implement email/password login with bcrypt verification, JWT issuance, and 5-failure/15-minute lockout.
3. Add GitHub and Google OAuth start/callback flows plus account linking/creation.
4. Replace the frontend token-only login screen with real login and OAuth entry points.
5. Add tests for successful login, invalid credentials, lockout, and OAuth happy path.

## Suggested Data Model Additions

- Add a dedicated auth user table or extend `kernel_users` with:
  - `email`
  - `password_hash`
  - `email_verified_at`
  - `failed_login_attempts`
  - `locked_until`
  - `last_login_at`
- Add an OAuth identity table keyed by `(provider, provider_user_id)`.
- Add configuration for JWT secret, expiry, and OAuth client settings.

## API Shape

### Email/Password Login

`POST /api/v1/auth/login`

Request:

```json
{
  "email": "user@example.com",
  "password": "secret"
}
```

Success response:

```json
{
  "token": "<jwt>",
  "user": {
    "id": "<id>",
    "email": "user@example.com",
    "name": "User Name"
  }
}
```

Failure responses:

- `401 Invalid credentials`
- `403 Email not verified`
- `429 Account locked, please try again in 15 minutes`

### OAuth

- `GET /api/v1/auth/oauth/:provider/start`
- `GET /api/v1/auth/oauth/:provider/callback`

The callback should create or link a local account, then issue the same JWT response contract used by password login.

## Notes

- JWT, bcrypt, email verification, and failed-login lockout are mandatory per issue constraints.
- The current repo appears to have reusable OAuth-related dependencies, but not user-login flows. Reuse should be evaluated during implementation rather than assumed.
