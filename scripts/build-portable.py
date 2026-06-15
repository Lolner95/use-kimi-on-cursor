#!/usr/bin/env python3
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path


def run(cmd: list[str], cwd: Path) -> bool:
    try:
        subprocess.run(cmd, cwd=cwd, check=True)
        return True
    except (subprocess.CalledProcessError, FileNotFoundError):
        return False


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    out_dir = root / "release-portable" / "Kimi Cursor Gateway"
    skip_build = "--skip-build" in sys.argv

    if not skip_build:
        print("Building release artifacts...")
        built = False
        if (root / "package.json").exists():
            built = run(["npm", "run", "tauri:build"], root)
        if not built:
            built = run(["cargo", "tauri", "build"], root)
        if not built:
            built = run(
                ["cargo", "build", "--release", "--manifest-path", str(root / "src-tauri" / "Cargo.toml")],
                root,
            )
        if not built:
            print("Build failed. Ensure npm/cargo tooling is installed.", file=sys.stderr)
            return 1

    out_dir.mkdir(parents=True, exist_ok=True)

    is_macos = sys.platform == "darwin"
    if is_macos:
        app_bundle = root / "src-tauri" / "target" / "release" / "bundle" / "macos" / "Kimi Cursor Gateway.app"
        bin_path = root / "src-tauri" / "target" / "release" / "kimi-cursor-gateway"
        if app_bundle.exists():
            dst = out_dir / app_bundle.name
            if dst.exists():
                shutil.rmtree(dst)
            shutil.copytree(app_bundle, dst)
        elif bin_path.exists():
            shutil.copy2(bin_path, out_dir / "Kimi Cursor Gateway")
        else:
            print("No macOS release artifact found.", file=sys.stderr)
            return 1
    elif os.name == "nt":
        exe = root / "src-tauri" / "target" / "release" / "kimi-cursor-gateway.exe"
        if not exe.exists():
            print(f"Release executable not found at {exe}", file=sys.stderr)
            return 1
        shutil.copy2(exe, out_dir / "Kimi Cursor Gateway.exe")
    else:
        bin_path = root / "src-tauri" / "target" / "release" / "kimi-cursor-gateway"
        if not bin_path.exists():
            print(f"Release binary not found at {bin_path}", file=sys.stderr)
            return 1
        dst = out_dir / "Kimi Cursor Gateway"
        shutil.copy2(bin_path, dst)
        dst.chmod(0o755)

    (out_dir / "portable").write_text("Kimi Cursor Gateway portable mode\n", encoding="utf-8")
    (out_dir / "README.txt").write_text(
        """Kimi Cursor Gateway (Portable)
==============================

1. Start the app from this folder
2. Complete the setup wizard with your Moonshot API key
3. Copy the Cursor settings shown in the app

Portable mode stores settings in:
  KimiCursorGatewayData/  (next to this folder)

Autostart: enable "Start with system login" in the wizard (recommended).

Cursor settings:
  - OpenAI API Key: use the GATEWAY key from the app
  - Override OpenAI Base URL: ON
  - Base URL: https://....trycloudflare.com/v1
  - Model: gpt-5-high-max
""",
        encoding="utf-8",
    )

    print("\nPortable build ready:")
    print(f"  {out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
