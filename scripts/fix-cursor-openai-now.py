"""Apply gateway Cursor settings and verify read-back from state.vscdb."""
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
    alias_model = cfg.get("aliasModel", "gpt-5-high-max")

    health = json.loads(urllib.request.urlopen("http://127.0.0.1:4001/health", timeout=10).read())
    base_url = health["publicBaseUrl"]
    if not base_url:
        print("ERROR: gateway has no publicBaseUrl - start the gateway first")
        return 1

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
    for field in ("userAddedModels", "modelOverrideEnabled"):
        arr = ai.setdefault(field, [])
        if alias_model not in arr:
            arr.append(alias_model)
    ai["composerModel"] = alias_model
    ai["cmdKModel"] = alias_model
    config = ai.setdefault("modelConfig", {})
    for mode in ("composer", "cmd-k", "background-composer", "plan-execution"):
        config[mode] = {
            "modelName": alias_model,
            "maxMode": False,
            "selectedModels": [{"modelId": alias_model, "parameters": []}],
        }
    conn.execute("UPDATE ItemTable SET value = ? WHERE key = ?", (json.dumps(app), APP_KEY))
    conn.commit()

    verify = json.loads(conn.execute("SELECT value FROM ItemTable WHERE key = ?", (APP_KEY,)).fetchone()[0])
    key_row = conn.execute("SELECT value FROM ItemTable WHERE key = ?", (OPENAI_KEY,)).fetchone()
    conn.close()

    print("APPLIED AND VERIFIED")
    print("gateway_key:", (key_row[0] if key_row else "")[:16] + "...")
    print("useOpenAIKey:", verify.get("useOpenAIKey"))
    print("openAIBaseUrl:", verify.get("openAIBaseUrl"))
    print("composerModel:", verify.get("aiSettings", {}).get("composerModel"))
    ok = (
        verify.get("useOpenAIKey") is True
        and verify.get("openAIBaseUrl") == base_url
        and key_row
        and key_row[0] == gateway_key
    )
    print("ALIGNED:", ok)
    return 0 if ok else 2


if __name__ == "__main__":
    sys.exit(main())
