import sqlite3
import os
import sys

db = os.path.join(os.environ["APPDATA"], "Cursor", "User", "globalStorage", "state.vscdb")
if not os.path.exists(db):
    print(f"NOT FOUND: {db}")
    sys.exit(1)

conn = sqlite3.connect(db)
cur = conn.cursor()
rows = cur.execute(
    "SELECT key, length(value) FROM ItemTable WHERE lower(key) LIKE '%cursor%' OR lower(key) LIKE '%openai%' OR lower(key) LIKE '%base%' OR lower(key) LIKE '%override%' OR lower(key) LIKE '%model%' ORDER BY key"
).fetchall()
for key, size in rows:
    print(f"{key} ({size}b)")

print("\n--- override/base keys ---")
for (key,) in cur.execute(
    "SELECT key FROM ItemTable WHERE lower(key) LIKE '%override%' OR lower(key) LIKE '%baseurl%' OR lower(key) LIKE '%openaikey%' OR lower(key) LIKE '%usingopenai%'"
).fetchall():
    row = cur.execute("SELECT value FROM ItemTable WHERE key = ?", (key,)).fetchone()
    val = row[0]
    if isinstance(val, bytes):
        val = val.decode("utf-8", errors="replace")
    print(f"{key}: {val[:300]}")

print("\n--- trycloudflare / http keys ---")
for (key,) in cur.execute(
    "SELECT key FROM ItemTable WHERE value LIKE '%trycloudflare%' OR value LIKE '%localhost:4001%' OR value LIKE '%openai%'"
).fetchall():
    row = cur.execute("SELECT value FROM ItemTable WHERE key = ?", (key,)).fetchone()
    val = row[0]
    if isinstance(val, bytes):
        val = val.decode("utf-8", errors="replace")
    print(f"{key}: {val[:400]}")

import json
row = cur.execute(
    "SELECT value FROM ItemTable WHERE key = 'src.vs.platform.reactivestorage.browser.reactiveStorageServiceImpl.persistentStorage.applicationUser'"
).fetchone()
if row:
    val = row[0]
    if isinstance(val, bytes):
        val = val.decode("utf-8", errors="replace")
    data = json.loads(val)
    print("\n--- applicationUser openai fields ---")
    def walk(obj, prefix=""):
        if isinstance(obj, dict):
            for k, v in obj.items():
                p = f"{prefix}.{k}" if prefix else k
                if any(x in k.lower() for x in ["openai", "baseurl", "override", "model", "apikey", "key"]):
                    print(f"{p}: {str(v)[:300]}")
                walk(v, p)
        elif isinstance(obj, list):
            for i, v in enumerate(obj[:3]):
                walk(v, f"{prefix}[{i}]")
    walk(data)

row = cur.execute("SELECT value FROM ItemTable WHERE key = 'cursorai/serverConfig'").fetchone()
if row:
    val = row[0]
    if isinstance(val, bytes):
        val = val.decode("utf-8", errors="replace")
    try:
        cfg = json.loads(val)
        for k in cfg:
            if "openai" in k.lower() or "base" in k.lower() or "model" in k.lower():
                print(f"serverConfig.{k}: {str(cfg[k])[:200]}")
    except Exception as e:
        print("serverConfig parse error", e)
        print(val[:500])
