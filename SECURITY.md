# Security

## Threat model

Kimi Cursor Gateway exposes a **public HTTPS endpoint** via Cloudflare Quick Tunnel so Cursor can reach the local proxy. Anyone who discovers the tunnel URL could attempt to call your gateway.

## Mitigations

### 1. Separate gateway API key

Cursor uses a **locally generated gateway key**, not your Moonshot API key. Requests to `/v1/chat/completions` require:

```http
Authorization: Bearer <gateway-key>
```

Rotate this key from the dashboard at any time.

### 2. Moonshot key protection

- Encrypted at rest with **Windows DPAPI** (user-scoped)
- Never logged
- Never returned to the frontend in full (masked display only)
- Never included in diagnostics export

### 3. Local binding

The Axum server listens on `127.0.0.1` only. The tunnel forwards exclusively to that port.

### 4. Public tunnel warning

The UI displays a clear warning when a public tunnel is active. Quick Tunnel URLs are ephemeral but still reachable while active.

### 5. Request limits

Maximum request body size: 10 MB.

### 6. Diagnostics export

Exports include:

- Redacted config (no secrets)
- Log files (secrets redacted in error paths)
- Gateway status metadata

Never includes the Moonshot API key.

## Recommendations

- Rotate the gateway key if you suspect the tunnel URL was shared
- Stop the gateway when not in use
- Do not share your tunnel URL publicly
- Keep your Moonshot API key on the Moonshot platform with usage limits if available

## Reporting

If you discover a security issue, report it to the project maintainers privately before public disclosure.
