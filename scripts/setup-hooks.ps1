# Install the repository git hooks: points core.hooksPath at .githooks.
# Usage: .\scripts\setup-hooks.ps1

$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

git config core.hooksPath .githooks

Write-Host "OK: git hooks installed (core.hooksPath = .githooks)" -ForegroundColor Green
