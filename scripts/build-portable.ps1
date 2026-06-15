# Builds Kimi Cursor Gateway portable folder (no installer required)
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

if (-not $SkipBuild) {
    Write-Host "Building release artifacts..." -ForegroundColor Cyan
    $usedBuilder = $false

    if (Test-Path (Join-Path $Root "package.json")) {
        try {
            npm run tauri:build
            $usedBuilder = $true
        } catch {
            Write-Warning "npm tauri build failed, falling back to cargo."
        }
    }

    if (-not $usedBuilder) {
        try {
            cargo tauri build
            $usedBuilder = $true
        } catch {
            Write-Warning "cargo tauri build failed, falling back to cargo build --release."
        }
    }

    if (-not $usedBuilder) {
        cargo build --release --manifest-path (Join-Path $Root "src-tauri\Cargo.toml")
    }
}

$DistIndex = Join-Path $Root "dist\index.html"
if (-not (Test-Path $DistIndex)) {
    throw "Frontend dist missing at $DistIndex. Run: npm run build"
}

$ExeSource = Join-Path $Root "src-tauri\target\release\kimi-cursor-gateway.exe"
if (-not (Test-Path $ExeSource)) {
    throw "Release exe not found at $ExeSource. Run this script without -SkipBuild first."
}

$OutDir = Join-Path $Root "release-portable\Kimi Cursor Gateway"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Copy-Item $ExeSource (Join-Path $OutDir "Kimi Cursor Gateway.exe") -Force
Set-Content -Path (Join-Path $OutDir "portable") -Value "Kimi Cursor Gateway portable mode`n" -Encoding UTF8

$Bat = @"
@echo off
title Kimi Cursor Gateway
start "" "%~dp0Kimi Cursor Gateway.exe" --minimized
"@
Set-Content -Path (Join-Path $OutDir "Start Kimi Cursor Gateway.bat") -Value $Bat -Encoding ASCII

$Readme = @"
Kimi Cursor Gateway (Portable)
==============================

1. Double-click "Start Kimi Cursor Gateway.bat" (or the .exe)
2. Complete the setup wizard with your Moonshot API key
3. Copy the Cursor settings shown in the app

Portable mode stores settings in:
  KimiCursorGatewayData\  (next to this folder)

Autostart: enable "Start with system login" in the wizard (recommended).

Cursor settings:
  - OpenAI API Key: use the GATEWAY key from the app
  - Override OpenAI Base URL: ON
  - Base URL: https://....trycloudflare.com/v1
  - Model: gpt-5-high-max
"@
Set-Content -Path (Join-Path $OutDir "README.txt") -Value $Readme -Encoding UTF8

Write-Host ""
Write-Host "Portable build ready:" -ForegroundColor Green
Write-Host "  $OutDir"
Write-Host ""
Write-Host "Zip this folder to share, or run Start Kimi Cursor Gateway.bat"
