#!/usr/bin/env pwsh
# install.ps1 — download the latest `functor` release binary for Windows and
# drop it in %USERPROFILE%\.functor\bin.
#
#   irm https://raw.githubusercontent.com/tommy-xr/functor/main/install.ps1 | iex
#
# Overrides (environment variables):
#   FUNCTOR_VERSION      install a specific version/tag (e.g. 0.1.0 or v0.1.0)
#   FUNCTOR_INSTALL_DIR  install location (default: $HOME\.functor\bin)
$ErrorActionPreference = 'Stop'

# Windows PowerShell 5.1 (the built-in default) negotiates TLS 1.0 by default;
# the GitHub API requires 1.2. Harmless on PowerShell 7+.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = 'tommy-xr/functor'
$installDir = if ($env:FUNCTOR_INSTALL_DIR) { $env:FUNCTOR_INSTALL_DIR } else { Join-Path $HOME '.functor\bin' }
$target = 'x86_64-pc-windows-msvc'  # the only published Windows target (runs under x64 emulation on Arm)

# --- Resolve the version tag. ------------------------------------------------
# Prefer an explicit FUNCTOR_VERSION; else the latest release, falling back to
# the newest prerelease (alpha builds ship as prereleases, which /latest skips).
function Get-Release($path) {
  try {
    Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/$path" -Headers @{ 'User-Agent' = 'functor-install' } -UseBasicParsing
  } catch {
    $null
  }
}

$tag = $null
if ($env:FUNCTOR_VERSION) {
  $tag = $env:FUNCTOR_VERSION
  if ($tag -notlike 'v*') { $tag = "v$tag" }
} else {
  $latest = Get-Release 'releases/latest'
  if ($latest) { $tag = $latest.tag_name }
  if (-not $tag) {
    $all = Get-Release 'releases'
    if ($all -and $all.Count -gt 0) { $tag = $all[0].tag_name }
  }
}
if (-not $tag) { throw 'could not determine the latest release (set FUNCTOR_VERSION to pin one)' }

$version = $tag -replace '^v', ''
$name = "functor-$version-$target"
$url = "https://github.com/$repo/releases/download/$tag/$name.zip"

# --- Download, extract, install. ---------------------------------------------
Write-Host "Downloading functor $version ($target)..."
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("functor-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $zip = Join-Path $tmp 'functor.zip'
  Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
  Expand-Archive -Path $zip -DestinationPath $tmp -Force

  $bin = Get-ChildItem -Path $tmp -Recurse -Filter 'functor.exe' | Select-Object -First 1
  if (-not $bin) { throw 'functor.exe not found in the archive' }

  New-Item -ItemType Directory -Path $installDir -Force | Out-Null
  Copy-Item -Path $bin.FullName -Destination (Join-Path $installDir 'functor.exe') -Force
} finally {
  Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "Installed functor $version -> $(Join-Path $installDir 'functor.exe')"

if (($env:PATH -split ';') -notcontains $installDir) {
  Write-Host ''
  Write-Host 'Add it to your PATH (persists for your user; restart the shell after):'
  Write-Host "  [Environment]::SetEnvironmentVariable('Path', `"$installDir;`" + [Environment]::GetEnvironmentVariable('Path','User'), 'User')"
} else {
  Write-Host "Run 'functor -d my-game init' to get started."
}
