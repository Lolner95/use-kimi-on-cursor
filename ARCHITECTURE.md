# Architecture

## Overview

```text
Cursor
  → public HTTPS tunnel (cloudflared Quick Tunnel)
  → local gateway (Axum HTTP server on 127.0.0.1:4001)
  → request sanitizer
  → Moonshot API (api.moonshot.ai)
  → response back to Cursor
```

## Components

### Desktop shell (Tauri 2)

- System tray, autostart, notifications
- Encrypted settings via Windows DPAPI
- IPC commands for UI
- Process supervision for gateway + tunnel

### Local gateway (Rust / Axum)

| Endpoint | Purpose |
|----------|---------|
| `GET /health` | Status, metrics, URLs |
| `GET /dashboard` | Debug HTML dashboard |
| `GET /v1/models` | OpenAI-compatible model list |
| `POST /v1/chat/completions` | Proxied chat (requires gateway Bearer token) |

### Request sanitizer

Before upstream calls:

- Map Cursor aliases (`gpt-4-turbo`, `gpt-4o`) → `kimi-k2.6`
- Force `thinking: { type: "disabled" }`
- Force `stream: false`, remove `stream_options`
- Map `max_completion_tokens` → `max_tokens`
- Strip incompatible OpenAI fields
- Remove `reasoning_content` from messages
- Sanitize tool JSON schemas (`definitions` → `$defs`, fix `$ref`)

### Tunnel manager

- Bundled or first-run downloaded `cloudflared.exe`
- Quick Tunnel: `cloudflared tunnel --protocol http2 --url http://127.0.0.1:{port}`
- Parses `https://*.trycloudflare.com` from process output
- Auto-restart on crash; emits `tunnel-url-changed` events

### State & metrics

In-memory metrics: requests, successes, errors, latency, last error.

Persisted config: `%LOCALAPPDATA%\KimiCursorGateway\config.json`

## Security model

- **Moonshot key**: stored encrypted (DPAPI), never sent to Cursor, never logged
- **Gateway key**: generated locally, shown to user as Cursor "OpenAI API Key"
- Local server binds to `127.0.0.1` only
- Tunnel exposes only the gateway port

## Frontend

Vanilla TypeScript + Tailwind CSS SPA embedded in Tauri WebView:

- First-launch wizard (5 steps)
- Dashboard (status, Cursor settings, traffic, logs, controls, advanced, doctor)

## Doctor checks

1. Moonshot key saved
2. Moonshot key valid
3. Local port available
4. Local gateway responds
5. Tunnel active
6. Public URL responds
7. Cursor settings ready
8. Last request status
9. cloudflared present
10. Autostart state
11. Logs directory
