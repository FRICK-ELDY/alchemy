#Requires -Version 5.1
<#
.SYNOPSIS
  Windows 開発用の環境変数を設定する。

.DESCRIPTION
  Cargo の build script が os error 4551 でブロックされる場合がある。
  リポジトリを Defender 除外せず、成果物だけ %LOCALAPPDATA%\alchemy\ へ出す。

  マシンの ExecutionPolicy が Restricted でも、次のどちらかで使える。

.EXAMPLE
  # 推奨: 新しい PowerShell を Bypass で開き、そのまま作業する
  powershell -NoProfile -ExecutionPolicy Bypass -NoExit -File .\.scripts\windows-dev.ps1

.EXAMPLE
  # 今のセッションに読み込む（ExecutionPolicy を変更しない）
  . ([scriptblock]::Create((Get-Content -Raw .\.scripts\windows-dev.ps1)))
#>

$ErrorActionPreference = "Stop"

$AlchemyCache = Join-Path $env:LOCALAPPDATA "alchemy"
$CargoTarget = Join-Path $AlchemyCache "cargo-target"

New-Item -ItemType Directory -Force -Path $CargoTarget | Out-Null

$env:CARGO_TARGET_DIR = $CargoTarget

Write-Host "Windows Alchemy dev env:" -ForegroundColor Cyan
Write-Host "  CARGO_TARGET_DIR=$env:CARGO_TARGET_DIR"
Write-Host ""
Write-Host "Do NOT exclude the git repo from antivirus." -ForegroundColor Yellow
Write-Host "If 4551 still occurs here, this is Smart App Control / WDAC (not path-based)." -ForegroundColor Yellow
Write-Host "  Prefer: build in WSL2, or turn SAC off on personal PCs, or ask IT for allowlist." -ForegroundColor Yellow
Write-Host "  Last resort only: exclude %LOCALAPPDATA%\alchemy (artifacts), never the repo." -ForegroundColor Yellow
Write-Host ""
Write-Host "Next:"
Write-Host "  cd client"
Write-Host "  cargo run -p host-server"
Write-Host "  cargo run -p client"
