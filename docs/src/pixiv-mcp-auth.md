# Pixiv MCP Auth Handoff

`RAR-22` is an operational setup task, not a missing feature in this repository.

The external Pixiv MCP server at `/Users/rara/.config/rara/workspace/pixiv-mcp-server` already supports:

- `PIXIV_REFRESH_TOKEN` at startup
- `set_refresh_token` for runtime configuration
- `refresh_token` for manual auth refresh

## Current Blocker

Pixiv authentication requires a real OAuth 2.0 PKCE browser flow. The agent cannot complete that step autonomously because the user must log in to Pixiv and approve access in a browser session they control.

Until that happens, Pixiv tools fail with the expected unauthenticated error.

## Required User Input

One of the following is needed:

1. The OAuth callback URL containing the Pixiv `code` parameter.
2. A valid Pixiv refresh token that was already obtained from the callback flow.

## Completion Path

After the user provides the OAuth result:

1. Configure the refresh token through the supported Pixiv MCP path.
2. Trigger auth refresh.
3. Verify at least one Pixiv tool call succeeds.

## Verification Status

As of 2026-03-12, the external Pixiv MCP server still has no auth material available for runtime verification:

- No `PIXIV_REFRESH_TOKEN` is present in the host environment.
- No `.env` file exists in `/Users/rara/.config/rara/workspace/pixiv-mcp-server`.
- Running the server's bundled verification script with its own virtualenv stops at the expected missing-token check:

```bash
/Users/rara/.config/rara/workspace/pixiv-mcp-server/.venv/bin/python \
  /Users/rara/.config/rara/workspace/pixiv-mcp-server/test_token_refresh.py
```

Observed result:

```text
=== Pixiv Token 刷新功能测试 ===
❌ 错误：未找到PIXIV_REFRESH_TOKEN环境变量
请先设置环境变量或运行 get_token.py 获取token
```

This confirms the remaining blocker is the missing OAuth result, not a broken verification path in the repository or the external Pixiv MCP server package.

## Notes

- For DXT-style installs, `set_refresh_token` is the preferred path because the packaged extension may be read-only.
- For local env-based runs, `PIXIV_REFRESH_TOKEN` can be persisted in the server's environment file.
- In Rara's MCP admin UI, prefer forwarding the host env var name `PIXIV_REFRESH_TOKEN` via `env_vars` instead of storing the secret value directly in the MCP server config payload.

## Rara MCP Config Example

For a local stdio-managed Pixiv server, the relevant MCP server config should include:

```json
{
  "name": "pixiv",
  "command": "uvx",
  "args": ["pixiv-mcp-server"],
  "env_vars": ["PIXIV_REFRESH_TOKEN"],
  "enabled": true,
  "transport": "stdio"
}
```

The actual token value stays in the host environment. The MCP config only references the variable name to forward.
