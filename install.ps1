# Build chie and put the binary on your PATH. Windows / PowerShell.
$ErrorActionPreference = "Stop"

Set-Location -Path $PSScriptRoot

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "chie: cargo not found — install Rust from https://rustup.rs first."
    exit 1
}

Write-Host "Building chie (release)…"
cargo build --release

$dest = Join-Path $env:USERPROFILE ".local\bin"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item -Force "target\release\chie.exe" (Join-Path $dest "chie.exe")

Write-Host "Installed -> $dest\chie.exe"
if (($env:PATH -split ';') -notcontains $dest) {
    Write-Host "Note: $dest is not on your PATH. Add it, e.g.:"
    Write-Host "  setx PATH `"$dest;$env:PATH`""
}
Write-Host "Run 'chie --version' to check, then 'chie <file>' and press Ctrl+G."
