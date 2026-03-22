# WeChat iLink Bot Channel

Connect rara to WeChat via the [iLink Bot API](https://ilinkai.weixin.qq.com).

## Prerequisites

1. **Login with `wechat-agent-rs`** to obtain an account ID and persist credentials locally:

```bash
# Clone and run the login tool
git clone https://github.com/rararulab/wechat-agent-rs.git
cd wechat-agent-rs
cargo run --example echo_bot
# Scan the QR code with WeChat → note the account_id printed on success
```

Credentials are saved to `~/.openclaw/openclaw-weixin/` automatically.

2. **Configure rara** — add the WeChat section to `~/.config/rara/config.yaml`:

```yaml
wechat:
  account_id: "your-account-id"   # from step 1
  # base_url: "https://ilinkai.weixin.qq.com"  # optional, defaults to production
```

3. **Restart rara** — the adapter starts automatically when `account_id` is set.

## How It Works

```
WeChat User ──→ iLink API ──→ [long-poll] WechatAdapter ──→ KernelHandle::ingest()
                                                ↑
Kernel ──→ OutboundEnvelope ──→ WechatAdapter::send() ──→ iLink API ──→ WeChat User
```

- **Inbound**: The adapter polls `get_updates()` continuously. Each message is parsed (text extracted via `body_from_item_list`), converted to a `RawPlatformMessage`, and ingested into the kernel.
- **Outbound**: Kernel replies arrive as `PlatformOutbound::Reply`. Markdown is stripped to plain text (WeChat doesn't render markdown) and sent via `send_text_message()`.
- **Context tokens**: The iLink API requires a `context_token` for reply routing. These are cached per-user on each inbound message and reused for outbound sends.

## Limitations

- **No streaming** — WeChat has no message-edit API, so `StreamChunk` events are ignored. Replies are sent as complete messages.
- **No media outbound (yet)** — Only text replies are supported currently. Media upload support can be added using `wechat-agent-rs::media::upload_media()`.
- **Session expiry** — iLink Bot tokens expire. When this happens, the polling loop stops and logs a warning. Re-run the login tool and restart rara.
- **Single account** — Only one WeChat account per rara instance is supported.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `failed to load wechat account data` | No credentials in `~/.openclaw/` | Run the login tool first |
| `wechat session expired` in logs | Token expired | Re-login with `wechat-agent-rs` |
| `no context_token cached for user` | Reply attempted before any inbound message | User must send a message first |
| Adapter not starting | `account_id` not set in config | Add `wechat.account_id` to config.yaml |
