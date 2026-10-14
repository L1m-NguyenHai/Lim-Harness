#requires -Version 5.1
[CmdletBinding()]
param([ValidateSet('Desktop', 'Web')][string]$Mode = 'Desktop')

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$failed = 0
function Report([string]$Level, [string]$Name, [string]$Detail) {
    $color = switch ($Level) { 'OK' { 'Green' } 'WARN' { 'Yellow' } default { 'Red' } }
    Write-Host ("[{0}] {1}: {2}" -f $Level, $Name, $Detail) -ForegroundColor $color
    if ($Level -eq 'FAIL') { $script:failed++ }
}
function Command-Version([string]$Name) {
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if (-not $command) { return $null }
    try {
        $output = & $command.Source --version 2>&1
        if ($LASTEXITCODE -ne 0) { return $null }
        return ($output -join ' ').Trim()
    } catch { return $null }
}

Write-Host "Lim Harness environment ($Mode)"
if ($env:OS -ne 'Windows_NT') {
    Report FAIL Windows 'These PowerShell scripts target Windows. See README for other platforms.'
    exit 1
}
$node = Command-Version 'node.exe'
if ($node -and [version]($node.TrimStart('v')) -ge [version]'22.18.0') {
    Report OK Node $node
} else { Report FAIL Node 'Install Node.js 22.18+ (LTS recommended): https://nodejs.org/' }
$npm = Command-Version 'npm.cmd'
if ($npm) { Report OK npm $npm } else { Report FAIL npm 'npm.cmd missing or cannot run; reinstall Node.js and reopen terminal.' }
$git = Command-Version 'git.exe'
if ($git) { Report OK Git $git } else { Report WARN Git 'Needed to clone/push: https://git-scm.com/downloads/win' }
foreach ($file in @('package.json', 'package-lock.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock')) {
    if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot $file))) { Report FAIL Repository "Missing $file; use a complete clone." }
}
if ($Mode -eq 'Desktop') {
    foreach ($name in @('rustc.exe', 'cargo.exe')) {
        $version = Command-Version $name
        if ($version) { Report OK $name $version } else { Report FAIL $name 'Install Rust via https://rustup.rs/ and reopen terminal.' }
    }
    if (Get-Command rustc.exe -ErrorAction SilentlyContinue) {
        $details = & rustc.exe -vV 2>&1
        if ($LASTEXITCODE -ne 0 -or -not ($details -match '^host: .*windows-msvc$')) {
            Report FAIL Rust-target 'Select a Windows MSVC Rust toolchain (not GNU). See Tauri prerequisites.'
        }
    }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $visualStudio = $null
    if (Test-Path -LiteralPath $vswhere) {
        $visualStudio = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    }
    if ($visualStudio) { Report OK 'C++ Build Tools' ($visualStudio -join ' ') }
    else { Report FAIL 'C++ Build Tools' 'Install Visual Studio Build Tools with Desktop development with C++: https://visualstudio.microsoft.com/visual-cpp-build-tools/' }
    $sdkReady = $false
    foreach ($registry in @('HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots')) {
        $sdk = Get-ItemProperty -LiteralPath $registry -Name KitsRoot10 -ErrorAction SilentlyContinue
        if ($sdk) {
            $include = Join-Path $sdk.KitsRoot10 'Include'
            if (Test-Path -LiteralPath $include) {
                foreach ($version in (Get-ChildItem -LiteralPath $include -Directory)) {
                    if (Test-Path -LiteralPath (Join-Path $version.FullName 'um/Windows.h')) { $sdkReady = $true }
                }
            }
        }
    }
    if ($sdkReady) { Report OK 'Windows SDK' 'Windows SDK headers detected.' }
    else { Report FAIL 'Windows SDK' 'Add Windows 10/11 SDK in Visual Studio Installer.' }
    $webview = @()
    foreach ($registry in @('HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\*', 'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\*', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\*')) {
        $webview += @(Get-ItemProperty -Path $registry -ErrorAction SilentlyContinue | Where-Object {
            $_.PSObject.Properties['name'] -and $_.name -match 'WebView2' -and $_.PSObject.Properties['pv'] -and $_.pv -ne '0.0.0.0'
        })
    }
    if ($webview.Count) { Report OK WebView2 ([string]$webview[0].pv) }
    else { Report WARN WebView2 'Runtime not detected in registry. Install Evergreen Runtime if desktop cannot open: https://developer.microsoft.com/microsoft-edge/webview2/' }
    try {
        $ollama = Invoke-RestMethod -Uri 'http://127.0.0.1:11434/api/tags' -TimeoutSec 3
        $count = @($ollama.models).Count
        Report OK Ollama "Service reachable; $count installed model(s)."
        if ($count -eq 0) { Report WARN Models 'Use Cookbook to download a model. No model is required to build the app.' }
    } catch { Report WARN Ollama 'Not reachable at 127.0.0.1:11434. Open/install Ollama for inference: https://ollama.com/download/windows' }
}
if (Test-Path -LiteralPath (Join-Path $PSScriptRoot '.git')) { Report OK Repository 'Git checkout detected.' }
else { Report WARN Repository 'No root .git yet. See docs/getting-started.md for initial push.' }
if ($failed) { Write-Host "$failed required check(s) failed." -ForegroundColor Red; exit 1 }
Write-Host 'Required prerequisites found. This is an environment check, not a build or inference test.' -ForegroundColor Green
exit 0
