# WeChat iLink Bot Channel

Connect rara to WeChat via the [iLink Bot API](https://ilinkai.weixin.qq.com).

## Quick Start

### 1. Login

**CLI:**

```bash
rara wechat login
```

**Or via the agent** -- ask rara to connect WeChat and it will invoke
the `wechat-login` tool automatically.

A QR code will be displayed in the terminal. Scan it with WeChat to authenticate.
Credentials are saved to `~/.config/rara/wechat/` automatically.

### 2. Restart rara

```bash
rara server
```

The adapter auto-discovers saved credentials on startup -- no manual
`config.yaml` editing required.

You can optionally pin a specific account or override the API base URL
in `~/.config/rara/config.yaml`:

```yaml
wechat:
  account_id: "<account_id>"                    # optional, auto-discovered if omitted
  # base_url: "https://ilinkai.weixin.qq.com"   # optional, defaults to production
```

## How It Works

```
WeChat User --> iLink API --> [long-poll] WechatAdapter --> KernelHandle::ingest()
                                                ^
Kernel --> OutboundEnvelope --> WechatAdapter::send() --> iLink API --> WeChat User
```

- **Inbound**: The adapter polls `get_updates()` continuously. Each message is parsed (text extracted via `body_from_item_list`), converted to a `RawPlatformMessage`, and ingested into the kernel.
- **Outbound**: Kernel replies arrive as `PlatformOutbound::Reply`. Markdown is stripped to plain text (WeChat doesn't render markdown) and sent via `send_text_message()`.
- **Context tokens**: The iLink API requires a `context_token` for reply routing. These are cached per-user on each inbound message and reused for outbound sends.

## Credential Storage

Credentials are stored in `~/.config/rara/wechat/`:

```
~/.config/rara/wechat/
  accounts.json          # list of known account IDs
  accounts/<id>.json     # token, base_url, user_id per account
  get_updates_buf/<id>.txt  # long-poll continuation state
  config/<id>.json       # optional per-account config (route_tag)
```

## Limitations

- **No streaming** -- WeChat has no message-edit API, so `StreamChunk` events are ignored. Replies are sent as complete messages.
- **No media outbound (yet)** -- Only text replies are supported currently.
- **Session expiry** -- iLink Bot tokens expire. When this happens, the polling loop stops and logs a warning. Run `rara wechat login` again and restart.
- **Single account** -- Only one WeChat account per rara instance is supported.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `failed to load wechat account data` | No credentials | Run `rara wechat login` |
| `wechat session expired` in logs | Token expired | Run `rara wechat login` again |
| `no context_token cached for user` | Reply before any inbound | User must send a message first |
| Adapter not starting | `account_id` not set | Add `wechat.account_id` to config.yaml |
| QR code expired | Took too long to scan | Run `rara wechat login` again |
