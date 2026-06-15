#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from urllib import request
from urllib.error import URLError


def read_gateway_key(config_path: Path) -> str:
    data = json.loads(config_path.read_text(encoding="utf-8"))
    key = data.get("gatewayKey", "")
    if not isinstance(key, str) or not key:
        raise RuntimeError(f"Invalid gateway key in {config_path}")
    return key


def http_json(url: str, method: str = "GET", body: dict | None = None, headers: dict[str, str] | None = None) -> dict:
    raw = None if body is None else json.dumps(body).encode("utf-8")
    req = request.Request(url, method=method, data=raw)
    req.add_header("Content-Type", "application/json")
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with request.urlopen(req, timeout=60) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except URLError as exc:
        raise RuntimeError(f"HTTP request failed for {url}: {exc}") from exc


def main() -> int:
    gateway_url = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:4001"
    config_arg = Path(sys.argv[2]) if len(sys.argv) > 2 else None

    candidates = [
        config_arg,
        Path.home() / ".local" / "share" / "KimiCursorGateway" / "config.json",
        Path.home() / "Library" / "Application Support" / "KimiCursorGateway" / "config.json",
        Path(os.environ.get("LOCALAPPDATA", "")) / "KimiCursorGateway" / "config.json",
    ]
    config = next((c for c in candidates if c and c.exists()), None)
    if not config:
        print("Config not found. Start Kimi Cursor Gateway first.", file=sys.stderr)
        return 1

    key = read_gateway_key(config)

    print("==> Health check")
    try:
        _ = http_json(f"{gateway_url}/health")
    except RuntimeError as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        print("Start Kimi Cursor Gateway first, then re-run this script.", file=sys.stderr)
        return 1
    print("PASS: Gateway is reachable")

    print("==> Cursor Agent payload check")
    payload = {
        "model": "gpt-4-turbo",
        "stream": False,
        "instructions": "Reply exactly with PHASE2_CONTINUE",
        "input": [{"role": "user", "content": "Reply exactly with PHASE2_CONTINUE"}],
        "max_tokens": 120,
    }
    response = http_json(
        f"{gateway_url}/v1/chat/completions",
        method="POST",
        body=payload,
        headers={"Authorization": f"Bearer {key}"},
    )
    content = (((response.get("choices") or [{}])[0].get("message") or {}).get("content") or "")
    if "PHASE2_CONTINUE" not in content:
        print(f"FAIL: Unexpected response: {json.dumps(response)}", file=sys.stderr)
        return 1

    print("PASS: Context-preserving response format works")
    print("All checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
