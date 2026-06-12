"""Quick verification that Cursor DB fields get updated (same logic as gateway app)."""
import json
import os
import sqlite3
import sys
import urllib.request

APP_KEY = "src.vs.platform.reactivestorage.browser.reactiveStorageServiceImpl.persistentStorage.applicationUser"
OPENAI_KEY = "cursorAuth/openAIKey"

def main() -> int:
    cfg_path = os.path.join(os.environ["LOCALAPPDATA"], "KimiCursorGateway", "config.json")
    with open(cfg_path, encoding="utf-8") as f:
        cfg = json.load(f)
    gateway_key = cfg["gatewayKey"]

    health = json.loads(urllib.request.urlopen("http://127.0.0.1:4001/health", timeout=10).read())
    base_url = health["publicBaseUrl"]
    model = health["model"]

    db = os.path.join(os.environ["APPDATA"], "Cursor", "User", "globalStorage", "state.vscdb")
    conn = sqlite3.connect(db)
    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (OPENAI_KEY, gateway_key),
    )
    raw = conn.execute("SELECT value FROM ItemTable WHERE key = ?", (APP_KEY,)).fetchone()[0]
    app = json.loads(raw)
    app["useOpenAIKey"] = True
    app["openAIBaseUrl"] = base_url
    ai = app.setdefault("aiSettings", {})
    for field, val in [("userAddedModels", model), ("modelOverrideEnabled", model)]:
        arr = ai.setdefault(field, [])
        if model not in arr:
            arr.append(model)
    ai["composerModel"] = model
    ai["cmdKModel"] = model
    conn.execute("UPDATE ItemTable SET value = ? WHERE key = ?", (json.dumps(app), APP_KEY))
    conn.commit()
    conn.close()

    print("APPLIED")
    print("gateway_key:", gateway_key[:16] + "...")
    print("base_url:", base_url)
    print("model:", model)
    print("useOpenAIKey: True")
    return 0

if __name__ == "__main__":
    sys.exit(main())
