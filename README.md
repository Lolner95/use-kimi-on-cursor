<!-- SEO:Kimi Cursor, Cursor Kimi, Kimi 2.5, jailbreak Kimi Cursor, use Kimi in Cursor, Moonshot Kimi K2.6, Cursor IDE custom model, OpenAI base URL override, Kimi coding assistant, Cursor Agent with Kimi -->

<div align="center">

# use-kimi-on-cursor

**Use [Moonshot Kimi](https://platform.moonshot.ai) (K2.6) inside [Cursor](https://cursor.com), without fighting proxies, tunnels, or broken tool calls.**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows-0078D6?logo=windows&logoColor=white)](https://github.com/Lolner95/use-kimi-on-cursor)
[![Stack](https://img.shields.io/badge/Built%20with-Tauri%20%2B%20Rust-orange)](ARCHITECTURE.md)
[![Moonshot](https://img.shields.io/badge/Model-Kimi%20K2.6-6366f1)](https://platform.moonshot.ai)

[Quick start](#quick-start-5-minutes) · [How it works](#how-it-works) · [Cursor setup](#cursor-setup-copy-paste) · [FAQ](#faq) · [Build from source](#build-from-source)

</div>

---

## The problem

You want Kimi in Cursor. Sounds simple.

In practice you hit walls fast:

| What you try | What happens |
|---|---|
| Point Cursor at `api.moonshot.ai` directly | Cursor expects OpenAI-shaped APIs and sends payloads Kimi rejects |
| Use a local proxy on `127.0.0.1` | Cursor blocks private network URLs: *"Access to private networks is forbidden"* |
| Wire up LiteLLM + cloudflared yourself | Works until it doesn't, tool names break, streaming mismatches, thinking mode loops, empty message probes |
| Flip random settings in Cursor | HTTP 400s, Agent mode dies mid-task, MCP tools fail silently |

You didn't sign up to become a protocol plumber. You wanted to write code.

## The fix

**Kimi Cursor Gateway** is a small Windows app that does the boring, fragile work for you:

1. Runs a **local OpenAI-compatible gateway** tuned for what Cursor actually sends
2. Opens a **public HTTPS tunnel** (Cloudflare Quick Tunnel) so Cursor is allowed to connect
3. Forwards clean requests to **Moonshot Kimi K2.6**
4. Shows you the exact **Base URL + API key** to paste into Cursor

No Python scripts. No manual `cloudflared` terminal sessions. No LiteLLM config archaeology.

```
You  →  Cursor IDE  →  https://….trycloudflare.com/v1
                              ↓
                    Kimi Cursor Gateway (your PC)
                              ↓
                    api.moonshot.ai (Kimi K2.6)
```

---

## Quick start (5 minutes)

### 1. Get a Moonshot API key

Create one at **[platform.moonshot.ai](https://platform.moonshot.ai)**.  
This is your real Kimi key, it stays on your machine, encrypted.

### 2. Install the app

**Option A — Installer (recommended)**

Download the latest `Kimi Cursor Gateway_*_x64-setup.exe` from [Releases](https://github.com/Lolner95/use-kimi-on-cursor/releases) and run it.

**Option B — Portable (USB-friendly, no install)**

Download the portable zip from [Releases](https://github.com/Lolner95/use-kimi-on-cursor/releases), extract, and double-click `Start Kimi Cursor Gateway.bat`.

**Option C — Build it yourself**

See [Build from source](#build-from-source) below.

### 3. Run the setup wizard

1. Launch **Kimi Cursor Gateway**
2. Paste your **Moonshot API key**
3. Click **Start Gateway**
4. Wait until the app shows a **public Base URL** (ends with `/v1`)
5. Copy the **Gateway API key** shown in the app, this is *not* your Moonshot key

> **Tip:** Leave **"Start with Windows"** on. The app runs in the tray, restarts the tunnel if needed, and pings you when the URL changes.

### 4. Configure Cursor

Open **Cursor → Settings → Models** and set:

| Setting | Value |
|---|---|
| **OpenAI API Key** | Gateway key from the app |
| **Override OpenAI Base URL** | **ON** |
| **Base URL** | `https://….trycloudflare.com/v1` (from the app) |
| **Model** | `gpt-5-high-max` |

Then ask Cursor something dumb to confirm it works:

> *"Say hi in one sentence."*

If you get a reply, you're in. Kimi is now driving your editor.

---

## Cursor setup (copy-paste)

```
OpenAI API Key:          <gateway-key-from-app>
Override Base URL:       ON
Base URL:                https://<your-tunnel>.trycloudflare.com/v1
Custom model name:       gpt-5-high-max
```

**Why `gpt-5-high-max`?**  
Cursor only lets you add certain model names. The gateway maps that alias (and others like `gpt-4-turbo`) to **`kimi-k2.6`** with a 256K context window. You type a Cursor-friendly name; Kimi does the work.

**Important:** Use the **gateway key** in Cursor, never your Moonshot key. Your Moonshot key never leaves the app.

---

## How it works

<details>
<summary><strong>Architecture (30-second version)</strong></summary>

```text
Cursor
  → public HTTPS tunnel (cloudflared Quick Tunnel)
  → local gateway (127.0.0.1:4001, Rust/Axum)
  → request sanitizer (fixes Cursor ↔ Kimi mismatches)
  → Moonshot API
  → response back to Cursor
```

Full breakdown: [ARCHITECTURE.md](ARCHITECTURE.md)

</details>

### What the gateway fixes automatically

Cursor's Agent mode sends messy, OpenAI-flavored payloads. Kimi is strict. The gateway sits in the middle and translates:

| Cursor sends | Gateway does |
|---|---|
| Model aliases (`gpt-4-turbo`, `gpt-5-high-max`, …) | Maps to `kimi-k2.6` |
| `developer` role messages | Converts to `system` |
| MCP tool names like `mcp.fs.read_file` | Sanitizes to valid Kimi identifiers |
| `type: "custom"` tools, flat tool shapes | Normalizes to standard function tools |
| Empty `messages` during model validation | Injects a seed user message |
| `stream: true` + SSE expectations | Returns proper `text/event-stream` to Cursor |
| Thinking / reasoning fields Kimi rejects | Strips or disables them |
| Unsupported sampling params (`temperature`, `top_p`, …) | Removes them (K2.6 uses fixed sampling) |

This is the stuff that took hours to debug manually. The app just handles it.

### Why a public tunnel?

Cursor refuses to call `http://127.0.0.1:…` for security reasons. Cloudflare Quick Tunnel gives you a temporary `https://….trycloudflare.com` URL that points at your local gateway.

**Trade-off:** The tunnel URL can change when you restart the app. The UI tells you when that happens, copy the new Base URL into Cursor.

---

## Features

| Feature | What you get |
|---|---|
| **One-click setup wizard** | API key → gateway → tunnel → Cursor instructions |
| **System tray + autostart** | Runs quietly; survives reboots |
| **Doctor diagnostics** | 11 automated checks when something's wrong |
| **Live logs + ZIP export** | Debug without guessing |
| **Gateway key rotation** | Lock down the tunnel if the URL leaks |
| **DPAPI-encrypted Moonshot key** | Your real key never hits logs or Cursor |
| **Agent + MCP compatible** | Tool calls sanitized for Kimi K2.6 |
| **Portable mode** | Settings live next to the `.exe`, good for USB |

---

## FAQ

<details>
<summary><strong>Does this work with Cursor Agent mode?</strong></summary>

Yes. v1.1+ specifically hardens Agent + MCP tool payloads, the common HTTP 400 failures from invalid tool names, empty message probes, and streaming mismatches are handled in the gateway.

</details>

<details>
<summary><strong>Why can't I just use my Moonshot API key directly in Cursor?</strong></summary>

Three reasons:

1. Cursor speaks OpenAI's API shape; Moonshot's API is close but not identical.
2. Cursor sends tool schemas and roles Kimi rejects without translation.
3. Even if the API worked, Cursor blocks direct calls to private/local URLs.

The gateway solves all three.

</details>

<details>
<summary><strong>My tunnel URL changed, now Cursor returns errors</strong></summary>

Quick Tunnel URLs are ephemeral. When the app restarts the tunnel, you get a new `trycloudflare.com` hostname.

**Fix:** Copy the new Base URL from the app dashboard → paste it into **Cursor → Settings → Models → Override OpenAI Base URL**.

</details>

<details>
<summary><strong>Is my API key safe?</strong></summary>

Your Moonshot key is encrypted with **Windows DPAPI** and never sent to Cursor. Cursor only sees a locally generated **gateway key**.

The tunnel is public while active, anyone with the URL *could* try to hit your gateway. Mitigations: gateway Bearer auth, key rotation, stop the gateway when not in use.

Details: [SECURITY.md](SECURITY.md)

</details>

<details>
<summary><strong>What models are supported?</strong></summary>

The gateway targets **Kimi K2.6** (`kimi-k2.6`). In Cursor, add a custom model alias — default is `gpt-5-high-max`. Other aliases (`gpt-4-turbo`, `gpt-4o`, …) also map through.

</details>

<details>
<summary><strong>Does it work on macOS or Linux?</strong></summary>

Not yet. This release is **Windows 10/11 only**. The core gateway is Rust and could be ported; contributions welcome.

</details>

<details>
<summary><strong>Do I need to keep the app window open?</strong></summary>

The app can live in the **system tray**. As long as the gateway and tunnel are running, Cursor works. Enable autostart if you want it always ready.

</details>

<details>
<summary><strong>Cursor says "Access to private networks is forbidden"</strong></summary>

You're pointing at a local URL (`127.0.0.1` or `localhost`). Cursor blocks that by design. Use the **public HTTPS Base URL** from the app (the `trycloudflare.com` address ending in `/v1`).

</details>

---

## Build from source

**Prerequisites:** Windows 10/11, [Rust](https://rustup.rs/) 1.70+, [Node.js](https://nodejs.org/) 18+

```powershell
git clone https://github.com/Lolner95/use-kimi-on-cursor.git
cd use-kimi-on-cursor
npm install
npm run tauri:dev
```

**Production installer:**

```powershell
npm run tauri:build
# Output: src-tauri/target/release/bundle/nsis/Kimi Cursor Gateway_1.0.0_x64-setup.exe
```

**Portable build (no installer):**

```powershell
npm run build:portable
# Output: release-portable/Kimi Cursor Gateway/
```

**Live integration tests** (requires real Moonshot key + network):

```powershell
npm run test:live
```

---

## Data & logs

Installed mode stores data at:

```text
%LOCALAPPDATA%\KimiCursorGateway\
  config.json       ← Moonshot key (DPAPI-encrypted)
  logs\
  data\cloudflared.exe   ← downloaded on first run if not bundled
```

Portable mode uses `KimiCursorGatewayData/` next to the executable.

---

## Docs

| File | Contents |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Components, endpoints, sanitizer rules |
| [SECURITY.md](SECURITY.md) | Threat model, key handling, recommendations |
| [RELEASE_NOTES.md](RELEASE_NOTES.md) | Version history and known limitations |
| [llms.txt](llms.txt) | Machine-readable project summary for AI search |

---

## Alternatives (honest comparison)

| Approach | Setup time | Agent/MCP stable? | Maintenance |
|---|---|---|---|
| **This project** | ~5 min | Yes (purpose-built) | Low, app handles tunnel + fixes |
| Manual LiteLLM + cloudflared | 1–3 hours | Fragile | High, you own every breakage |
| Direct Moonshot in Cursor | N/A | No | Blocked by Cursor URL policy |
| Other OpenAI proxies | Varies | Hit-or-miss | You debug protocol mismatches |

---

## Contributing

Found a bug? Open an [issue](https://github.com/Lolner95/use-kimi-on-cursor/issues).  
Have a fix? PRs welcome, keep changes focused, test on real Cursor + Kimi if you touch the gateway.

Security issues: see [SECURITY.md](SECURITY.md), please report privately first.

---

## License

[MIT](LICENSE) © [Lolner95](https://github.com/Lolner95)

---

<div align="center">

**Kimi in Cursor shouldn't require a PhD in reverse proxies.**

If this saved you an afternoon of yak-shaving, star the repo, it helps others find it.

</div>
