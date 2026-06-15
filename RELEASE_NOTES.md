# Release Notes

## v1.1.0 - Cursor Agent compatibility hardening

Fixes the HTTP 400 errors when using Cursor (Agent mode + MCP) with Kimi K2.6.
Verified end-to-end through a live Cloudflare tunnel with the exact payloads Cursor sends.

### Root causes fixed (all produced "Moonshot returned an error (HTTP 400)")

1. **Invalid tool/function names** - Cursor/MCP send names like `mcp.fs.read_file`,
   `server/tool`; Moonshot requires `^[a-zA-Z][a-zA-Z0-9_-]*$`. Now sanitized
   (dots/slashes → `_`), de-duplicated, and remapped consistently across `tools`,
   `tool_calls`, and `tool_choice`.
2. **Empty messages probe** - Cursor's model-validation request sends only `tools`
   with empty `messages`; Moonshot rejects empty messages. A seed user message is
   now injected when `messages` is missing/empty.
3. **`type: "custom"` tools** (e.g. `apply_patch`) and **flat tool format**
   (`tools[i].name`) - normalized to standard `tools[i].function.name`.
4. **Unsupported sampling params** - `temperature`, `top_p`, `presence_penalty`,
   `frequency_penalty`, `seed`, etc. stripped (Kimi K2.6 uses fixed sampling).
5. **`developer` role** - mapped to `system` (Moonshot tokenizer rejects it).
6. **Missing `reasoning_content`** on assistant tool-call history - placeholder
   injected so multi-turn tool calls don't 400.
7. **Streaming mismatch** - Cursor sends `stream: true` and expects SSE. The gateway
   now returns a proper `text/event-stream` (chunks + `[DONE]`) while keeping the
   upstream Moonshot call non-streaming for stability.

### Reliability improvements

- Gateway auto-starts when a Moonshot key exists, even if the wizard was not formally
  completed (prevents a stale "incomplete setup" state leaving Cursor with no endpoint).
- Default Cursor alias is now `gpt-5-high-max` (still maps to `kimi-k2.6`, 256K context).
  `gpt-4-turbo` and other aliases continue to work.
- Configurable max output tokens (8K–256K) in Advanced settings.

## v1.0.0

Initial release of **Kimi Cursor Gateway**.

### Features

- Standalone Windows desktop app (Tauri 2)
- First-launch setup wizard
- Moonshot API key validation and DPAPI-encrypted storage
- Local OpenAI-compatible gateway with request sanitization
- Automatic Cloudflare Quick Tunnel management
- Premium dark dashboard UI
- System tray controls
- Windows autostart support
- Doctor diagnostics with pass/warn/fail checks
- Live logs and diagnostic ZIP export
- Gateway key rotation

### Gateway behavior

- Default Cursor model alias: `gpt-4-turbo` → `kimi-k2.6`
- Thinking mode disabled for Cursor Agent compatibility
- Tool schema sanitizer for Moonshot JSON Schema requirements
- Non-streaming default for Quick Tunnel stability

### Build outputs

- NSIS installer: `Kimi Cursor Gateway_1.0.0_x64-setup.exe`
- Portable: `kimi-cursor-gateway.exe`

### Known limitations

- Quick Tunnel URLs change after tunnel restart
- Requires network access to Moonshot and Cloudflare
- `cloudflared` downloaded on first gateway start if not bundled as sidecar
- Windows only in this release
