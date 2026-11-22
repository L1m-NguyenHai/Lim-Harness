# Installs a private SearXNG for Lim Harness agent mode into <repo>\.local (gitignored),
# without Docker or WSL. The app starts it on demand at http://127.0.0.1:8888.
#
#   powershell -ExecutionPolicy Bypass -File scripts\setup-searxng.ps1          # install / update
#   powershell -ExecutionPolicy Bypass -File scripts\setup-searxng.ps1 -Start   # install, then start
#
# Windows notes:
# - SearXNG's repo has Linux config templates with ':' in their names that Windows cannot
#   check out, so only the runtime files are exported with `git archive`.
# - searx/valkeydb.py imports the Unix-only `pwd` module (used only when a Valkey connection
#   fails; this install has no Valkey), so a small stand-in is added to the venv.
# - Use a regular Python 3.10+ (python.org), not the Microsoft Store build: Store Python
#   redirects venvs under AppData.
param([switch]$Start)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$local = Join-Path $repoRoot '.local'
$src = Join-Path $local 'searxng-src'
$venv = Join-Path $local 'searxng-venv'
$settings = Join-Path $local 'searxng-settings.yml'
New-Item -ItemType Directory -Force $local | Out-Null

# 1. Runtime source from the official repository.
$clone = Join-Path $local 'searxng-clone'
if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
git clone --quiet --depth 1 --no-checkout https://github.com/searxng/searxng.git $clone
$tar = Join-Path $local 'searxng-src.tar'
git -C $clone -c core.protectNTFS=false archive --format=tar -o $tar HEAD searx requirements.txt requirements-server.txt LICENSE
if (Test-Path $src) { Remove-Item -Recurse -Force $src }
New-Item -ItemType Directory -Force $src | Out-Null
tar -xf $tar -C $src
Remove-Item $tar, $clone -Recurse -Force
Write-Host "SearXNG source: $src ($((Get-ChildItem (Join-Path $src 'searx') -Recurse -File).Count) files)"

# 2. Virtual environment and dependencies.
$python = Join-Path $venv 'Scripts\python.exe'
if (-not (Test-Path $python)) {
    $launcher = Get-Command py -ErrorAction SilentlyContinue
    if ($launcher) { py -3 -m venv $venv } else { python -m venv $venv }
}
& $python -m pip install --quiet --upgrade pip
& $python -m pip install --quiet -r (Join-Path $src 'requirements.txt')
@'
"""Windows stand-in for the Unix-only `pwd` module, for the local SearXNG used by Lim Harness.

SearXNG imports `pwd` in searx/valkeydb.py only to log the user name when a Valkey connection
fails. This install runs without Valkey, so the stub only needs to import cleanly.
"""
import getpass
from types import SimpleNamespace


def getpwuid(uid=0):
    return SimpleNamespace(pw_name=getpass.getuser(), pw_uid=uid)
'@ | Set-Content -Encoding utf8 (Join-Path $venv 'Lib\site-packages\pwd.py')

# 3. Settings: local-only, JSON output for the app, fresh secret (kept when it exists).
if (-not (Test-Path $settings)) {
    $bytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    $secret = -join ($bytes | ForEach-Object { $_.ToString('x2') })
    @"
# Local SearXNG for Lim Harness agent mode (not committed; generated on this machine).
use_default_settings: true
general:
  instance_name: "Lim Harness search"
server:
  bind_address: "127.0.0.1"
  port: 8888
  secret_key: "$secret"
  limiter: false
  public_instance: false
  image_proxy: false
search:
  safe_search: 0
  default_lang: "vi"
  formats:
    - html
    - json
"@ | Set-Content -Encoding utf8 $settings
}
Write-Host "Settings: $settings"

if ($Start) {
    $env:SEARXNG_SETTINGS_PATH = $settings
    Start-Process -FilePath $python -ArgumentList '-m', 'searx.webapp' -WorkingDirectory $src -WindowStyle Hidden
    Write-Host 'SearXNG starting at http://127.0.0.1:8888'
}
