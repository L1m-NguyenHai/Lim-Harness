#requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('Desktop', 'Web', 'Setup', 'Build')][string]$Mode = 'Desktop',
    [switch]$RefreshDependencies
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Child PowerShell gives check.ps1 its own exit code without terminating this script.
$powerShell = (Get-Process -Id $PID).Path
Push-Location -LiteralPath $PSScriptRoot
try {
    $checkMode = if ($Mode -eq 'Web') { 'Web' } else { 'Desktop' }
    & $powerShell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'check.ps1') -Mode $checkMode
    if ($LASTEXITCODE -ne 0) { throw 'Missing prerequisites. Follow the check output, then run start.ps1 again.' }

    $lockHash = (Get-FileHash -LiteralPath 'package-lock.json' -Algorithm SHA256).Hash
    $stamp = Join-Path $PSScriptRoot 'node_modules/.lim-lockhash'
    $installedHash = if (Test-Path -LiteralPath $stamp) { (Get-Content -LiteralPath $stamp -Raw).Trim() } else { '' }
    if ($RefreshDependencies -or $Mode -eq 'Setup' -or $installedHash -ne $lockHash -or -not (Test-Path -LiteralPath 'node_modules/.bin/tauri.cmd')) {
        Write-Host 'Installing project dependencies from package-lock.json...'
        & npm.cmd ci
        if ($LASTEXITCODE -ne 0) { throw 'npm ci failed. package.json and lockfile must match; inspect the error above.' }
        Set-Content -LiteralPath $stamp -Value $lockHash -Encoding ASCII
    }
    if ($Mode -eq 'Setup') {
        Write-Host 'Fetching Rust dependencies from Cargo.lock...'
        & cargo.exe fetch --locked --manifest-path 'src-tauri/Cargo.toml'
        if ($LASTEXITCODE -ne 0) { throw 'cargo fetch failed. Inspect the error above.' }
        Write-Host 'Setup complete. Run .\start.ps1 to open the desktop app.' -ForegroundColor Green
    } else {
        if ($Mode -ne 'Build') {
            $occupied = Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue
            if ($occupied) { throw 'Port 1420 is in use. Stop the previous dev server (Ctrl+C), then retry. No process was terminated.' }
        }
        switch ($Mode) {
            'Web' { & npm.cmd run dev }
            'Desktop' { & npm.cmd run tauri -- dev }
            'Build' { & npm.cmd run tauri -- build --debug --no-bundle }
        }
        if ($LASTEXITCODE -ne 0) { throw "Mode $Mode failed. Inspect the error above." }
    }
} catch {
    Write-Host $_.Exception.Message -ForegroundColor Red
    exit 1
} finally { Pop-Location }
