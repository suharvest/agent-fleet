$ErrorActionPreference = "Stop"

$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root

if ($env:RPTY_FLEET_BIN) {
    $FleetBin = $env:RPTY_FLEET_BIN
} elseif (Test-Path (Join-Path $Root "fleet.exe")) {
    $FleetBin = Join-Path $Root "fleet.exe"
} elseif (Test-Path (Join-Path $Root "target\release\fleet.exe")) {
    $FleetBin = Join-Path $Root "target\release\fleet.exe"
} else {
    cargo build --release
    $FleetBin = Join-Path $Root "target\release\fleet.exe"
}

& $FleetBin doctor --fix --write-shell-profile

Write-Host ""
Write-Host "Fleet PTY Router installed."
Write-Host "Open a new PowerShell window, then run:"
Write-Host ""
Write-Host "  fleet doctor"
Write-Host "  codex"
Write-Host "  claude"
Write-Host "  opencode"
Write-Host ""
Write-Host "Fleet backend is bundled. For a fresh install, create your private device inventory:"
Write-Host ""
Write-Host "  Copy-Item `$HOME\.rpty\bin\fleet_backend\devices.example.json `$HOME\.rpty\bin\fleet_backend\devices.json"
Write-Host ""
Write-Host "If you already have an external Fleet backend, you can override the bundled one:"
Write-Host ""
Write-Host "  fleet config --fleet-py C:\path\to\fleet.py"
