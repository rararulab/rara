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

## Notes

- For DXT-style installs, `set_refresh_token` is the preferred path because the packaged extension may be read-only.
- For local env-based runs, `PIXIV_REFRESH_TOKEN` can be persisted in the server's environment file.
