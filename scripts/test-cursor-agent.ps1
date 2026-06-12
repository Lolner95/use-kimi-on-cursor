# Simulates Cursor Agent Responses API payload against the local gateway.
# Run after the gateway is started. Exit 0 = context preserved, 1 = failure.
param(
    [string]$GatewayUrl = "http://127.0.0.1:4001",
    [string]$ConfigPath = "$env:LOCALAPPDATA\KimiCursorGateway\config.json"
)

$ErrorActionPreference = "Stop"

function Write-Step([string]$msg) { Write-Host ""; Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Pass([string]$msg) { Write-Host "PASS: $msg" -ForegroundColor Green }
function Write-Fail([string]$msg) { Write-Host "FAIL: $msg" -ForegroundColor Red; exit 1 }

if (-not (Test-Path $ConfigPath)) {
    Write-Fail "Config not found at $ConfigPath. Start Kimi Cursor Gateway first."
}

$config = Get-Content $ConfigPath -Raw | ConvertFrom-Json
$gatewayKey = $config.gatewayKey
$headers = @{
    Authorization = "Bearer $gatewayKey"
    "Content-Type" = "application/json"
}

Write-Step "Health check"
try {
    $health = Invoke-RestMethod -Uri "$GatewayUrl/health" -TimeoutSec 10
    Write-Host "Gateway OK - model=$($health.model) real=$($health.realModel)"
    if ($health.publicBaseUrl) { Write-Host "Public URL: $($health.publicBaseUrl)" }
} catch {
    Write-Fail "Gateway not reachable at $GatewayUrl - $($_.Exception.Message)"
}

Write-Step "Test 1 - Cursor Agent Responses format with input field"
$payload1 = @'
{
  "model": "gpt-4-turbo",
  "stream": false,
  "store": false,
  "temperature": 0,
  "instructions": "You are a senior engineer. Follow reply instructions exactly.",
  "input": [
    { "role": "developer", "content": "Be direct." },
    { "role": "user", "content": "We are building a todo CLI in Rust. Phase 1 done. Reply with exactly: PHASE1_OK" },
    { "role": "assistant", "content": "PHASE1_OK - clap and serde scaffolded." },
    { "role": "user", "content": "Continue the build. Reply with exactly: PHASE2_CONTINUE" }
  ],
  "tools": [
    {
      "type": "function",
      "name": "read_file",
      "description": "Read a file",
      "parameters": { "type": "object", "properties": { "path": { "type": "string" } } }
    }
  ],
  "tool_choice": "auto",
  "max_tokens": 200
}
'@

try {
    $r1 = Invoke-RestMethod -Uri "$GatewayUrl/v1/chat/completions" -Method POST -Headers $headers -Body $payload1 -TimeoutSec 120
    $content1 = $r1.choices[0].message.content
    Write-Host "Reply: $content1"
    if ($content1 -match "PHASE2_CONTINUE") {
        Write-Pass "Context preserved - model continued the build"
    } elseif ($content1 -match "Hello|How can I help") {
        Write-Fail "Model greeted fresh - context was dropped"
    } else {
        Write-Fail "Unexpected reply: $content1"
    }
} catch {
    Write-Fail "Request failed: $($_.ErrorDetails.Message)"
}

Write-Step "Test 2 - Streaming with input field"
$payload2 = '{"model":"gpt-4-turbo","stream":true,"input":[{"role":"user","content":"Reply with exactly: STREAM_CONTEXT_OK"}],"max_tokens":100}'

try {
    $req = [System.Net.HttpWebRequest]::Create("$GatewayUrl/v1/chat/completions")
    $req.Method = "POST"
    $req.ContentType = "application/json"
    $req.Headers.Add("Authorization", "Bearer $gatewayKey")
    $req.Timeout = 120000
    $bytes = [Text.Encoding]::UTF8.GetBytes($payload2)
    $req.ContentLength = $bytes.Length
    $st = $req.GetRequestStream()
    $st.Write($bytes, 0, $bytes.Length)
    $st.Close()
    $resp = $req.GetResponse()
    $reader = New-Object IO.StreamReader($resp.GetResponseStream())
    $assembled = ""
    while (($line = $reader.ReadLine()) -ne $null) {
        if ($line.StartsWith("data:")) {
            $d = $line.Substring(5).Trim()
            if ($d -ne "[DONE]") {
                try {
                    $j = $d | ConvertFrom-Json
                    if ($j.choices[0].delta.content) { $assembled += $j.choices[0].delta.content }
                } catch {}
            }
        }
    }
    $reader.Close()
    Write-Host "Streamed: $assembled"
    if ($assembled -match "STREAM_CONTEXT_OK") {
        Write-Pass "Streaming with input field works"
    } else {
        Write-Fail "Streaming reply missing expected token: $assembled"
    }
} catch {
    Write-Fail "Streaming request failed: $($_.Exception.Message)"
}

Write-Step "Test 3 - adapt.log message conversion"
$adaptLog = Join-Path (Split-Path $ConfigPath) "logs\adapt.log"
if (Test-Path $adaptLog) {
    $last = Get-Content $adaptLog -Tail 3
    Write-Host ($last -join [Environment]::NewLine)
    $lastLine = $last[-1]
    if ($lastLine -match "messages_out=0") {
        Write-Fail "adapt.log shows messages_out=0"
    }
    Write-Pass "adapt.log shows messages being produced"
} else {
    Write-Host "WARN: adapt.log not found yet"
}

Write-Host ""
Write-Host "All Cursor Agent simulation tests passed." -ForegroundColor Green
