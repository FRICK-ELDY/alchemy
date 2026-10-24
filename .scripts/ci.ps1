#Requires -Version 5.1
<#
.SYNOPSIS
  GitHub Actions 相当のチェックをローカルで実行する。

.DESCRIPTION
  - rust / protocol: ホストの cargo（protocol の protoc は任意）
  - auth: Docker Compose（全 OS 共通）
  macOS / Linux では make ci または ./.scripts/ci.sh を推奨。

.PARAMETER Target
  all | rust | protocol | auth

.EXAMPLE
  .\.scripts\ci.ps1
  .\.scripts\ci.ps1 -Target rust
  .\.scripts\ci.ps1 -Target auth
#>
param(
    [ValidateSet("all", "rust", "protocol", "auth")]
    [string]$Target = "all"
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root

# Windows: keep Cargo artifacts out of the repo (os error 4551 / app control).
# Same location as .scripts/windows-dev.ps1 — do not Defender-exclude the git tree.
if ($env:OS -eq "Windows_NT" -and -not $env:CARGO_TARGET_DIR) {
    $cargoTarget = Join-Path $env:LOCALAPPDATA "alchemy\cargo-target"
    New-Item -ItemType Directory -Force -Path $cargoTarget | Out-Null
    $env:CARGO_TARGET_DIR = $cargoTarget
    Write-Host "CARGO_TARGET_DIR=$env:CARGO_TARGET_DIR"
}

$failed = [System.Collections.Generic.List[string]]::new()

function Write-Step([string]$Name) {
    Write-Host ""
    Write-Host "=== $Name ===" -ForegroundColor Cyan
}

function Invoke-Step([string]$Name, [scriptblock]$Body) {
    Write-Step $Name
    try {
        & $Body
        if ($LASTEXITCODE -ne $null -and $LASTEXITCODE -ne 0) {
            throw "exit code $LASTEXITCODE"
        }
        Write-Host "[PASS] $Name" -ForegroundColor Green
    }
    catch {
        Write-Host "[FAIL] $Name — $_" -ForegroundColor Red
        $failed.Add($Name) | Out-Null
    }
}

function Invoke-RustCi {
    Push-Location (Join-Path $Root "client")
    try {
        Invoke-Step "cargo fmt --check" {
            cargo fmt --all -- --check
        }
        Invoke-Step "cargo clippy" {
            cargo clippy --workspace -- -D warnings
        }
        Invoke-Step "cargo build --workspace" {
            cargo build --workspace
        }
        Invoke-Step "cargo test --workspace" {
            cargo test --workspace
        }
    }
    finally {
        Pop-Location
    }
}

function Invoke-ProtocolCi {
    Push-Location (Join-Path $Root "protocol")
    try {
        Invoke-Step "cargo test (protocol)" {
            cargo test
        }
    }
    finally {
        Pop-Location
    }

    $protoc = Get-Command protoc -ErrorAction SilentlyContinue
    if ($protoc) {
        Invoke-Step "protoc schema compile" {
            $out = Join-Path $env:TEMP "alchemy-descriptor.pb"
            & protoc -I protocol/proto --include_imports `
                --descriptor_set_out=$out `
                protocol/proto/render_frame.proto
            if (-not (Test-Path $out) -or (Get-Item $out).Length -le 0) {
                throw "descriptor set missing or empty"
            }
        }
    }
    else {
        Write-Host "[SKIP] protoc schema compile (protoc not on PATH; GHA uses arduino/setup-protoc)" -ForegroundColor Yellow
    }
}

function Invoke-AuthCi {
    $compose = Join-Path $Root "auth-server/docker-compose.yml"
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        throw "docker is required for auth CI on Windows"
    }

    Invoke-Step "docker compose up postgres" {
        docker compose -f $compose up -d postgres
        if ($LASTEXITCODE -ne 0) { throw "docker compose up failed" }
    }

    Invoke-Step "docker compose build auth" {
        docker compose -f $compose build auth
        if ($LASTEXITCODE -ne 0) { throw "docker compose build failed" }
    }

    # GHA auth-server-ci.yml と同じゲート。コンテナ内から PGHOST=postgres で接続する。
    $authCmd = 'set -e; mix deps.get; mix format --check-formatted; mix compile --warnings-as-errors; mix credo --strict; mix ecto.create; mix ecto.migrate; mix test'

    Invoke-Step "auth mix checks (Docker)" {
        docker compose -f $compose run --rm --entrypoint "" `
            -e MIX_ENV=test `
            -e PGHOST=postgres `
            -e PGPORT=5432 `
            auth sh -c $authCmd
        if ($LASTEXITCODE -ne 0) { throw "auth CI failed" }
    }
}

Write-Host "Alchemy local CI (target=$Target)" -ForegroundColor White
Write-Host "root: $Root"

switch ($Target) {
    "rust" { Invoke-RustCi }
    "protocol" { Invoke-ProtocolCi }
    "auth" { Invoke-AuthCi }
    "all" {
        Invoke-RustCi
        Invoke-ProtocolCi
        Invoke-AuthCi
    }
}

Write-Host ""
if ($failed.Count -eq 0) {
    Write-Host "RESULT: ALL PASSED" -ForegroundColor Green
    exit 0
}

Write-Host ("RESULT: FAILED — [{0}]" -f ($failed -join ", ")) -ForegroundColor Red
exit 1
