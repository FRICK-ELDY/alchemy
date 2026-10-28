#Requires -Version 5.1
<#
.SYNOPSIS
  Build and pack Windows x64 tester ZIP (launcher + host-server + client).

.DESCRIPTION
  Guarantees: release .exe under dist/ with Start.cmd, README-testers.txt, build-info.json.
  Non-guarantees: auth/world/assets/zenohd; signing; auto-update; Coin fixture offline path.
  No Python. Uses cargo + Compress-Archive.

.EXAMPLE
  .\.scripts\pack-windows-zip.ps1
  .\.scripts\pack-windows-zip.ps1 -Version v0.0.0-dev.202610281915
  .\.scripts\pack-windows-zip.ps1 -SkipBuild
#>
param(
    [string]$Version = '',
    [string]$OutputDirectory = '',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repoRoot

function Write-Log([string]$Message) {
    [Console]::Error.WriteLine($Message)
}

function Get-JstDevVersion {
    $jst = [TimeZoneInfo]::ConvertTimeBySystemTimeZoneId([DateTimeOffset]::UtcNow, 'Tokyo Standard Time')
    return 'v0.0.0-dev.' + $jst.ToString('yyyyMMddHHmm')
}

function Get-CargoTargetDir([string]$CrateRoot) {
    if ($env:CARGO_TARGET_DIR) {
        return $env:CARGO_TARGET_DIR
    }
    return (Join-Path $CrateRoot 'target')
}

if (-not $Version) {
    $Version = Get-JstDevVersion
}
if ($Version -notmatch '^v0\.0\.0-dev\.[0-9]{12}(\.[0-9]+)?$') {
    throw "Version must look like v0.0.0-dev.YYYYMMDDHHMM (got: $Version)"
}

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repoRoot 'dist'
}
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path

$packageName = "alchemy-windows-x64-$Version"
$stagingParent = Join-Path $repoRoot ('.run/packages/' + [guid]::NewGuid().ToString('N'))
$packageRoot = Join-Path $stagingParent $packageName
New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null

$launcherDir = Join-Path $repoRoot 'launcher'
$clientDir = Join-Path $repoRoot 'client'

$rustcHost = ((& rustc -vV) | Where-Object { $_ -like 'host: *' } | Select-Object -First 1)
if (-not $rustcHost) { throw 'Could not read rustc host from rustc -vV' }
$rustcHost = $rustcHost.Substring('host: '.Length).Trim()

if (-not $SkipBuild) {
    Write-Log 'Building alchemy-launcher (release)...'
    & cargo build --release --manifest-path (Join-Path $launcherDir 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'launcher release build failed' }

    Write-Log 'Building host-server + client (release)...'
    & cargo build --release -p host-server -p client --manifest-path (Join-Path $clientDir 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'client workspace release build failed' }
}

$launcherTarget = Get-CargoTargetDir $launcherDir
$clientTarget = Get-CargoTargetDir $clientDir
Write-Log "launcher target_directory=$launcherTarget"
Write-Log "client target_directory=$clientTarget"
Write-Log "rustc host=$rustcHost"

$bins = @(
    @{ Src = Join-Path $launcherTarget 'release/alchemy-launcher.exe'; Dst = 'alchemy-launcher.exe' },
    @{ Src = Join-Path $clientTarget 'release/host-server.exe'; Dst = 'host-server.exe' },
    @{ Src = Join-Path $clientTarget 'release/client.exe'; Dst = 'client.exe' }
)
foreach ($bin in $bins) {
    if (-not (Test-Path -LiteralPath $bin.Src)) {
        throw "Missing binary: $($bin.Src) (build first or omit -SkipBuild)"
    }
    Copy-Item -LiteralPath $bin.Src -Destination (Join-Path $packageRoot $bin.Dst)
}

$templates = Join-Path $PSScriptRoot 'distribution'
Copy-Item -LiteralPath (Join-Path $templates 'Start.cmd') -Destination $packageRoot
Copy-Item -LiteralPath (Join-Path $templates 'README-testers.txt') -Destination $packageRoot
Copy-Item -LiteralPath (Join-Path $templates 'env.testers.example.cmd') -Destination $packageRoot

$gitSha = (git -C $repoRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Could not read git HEAD' }
$sourceTreeClean = (@(git -C $repoRoot status --porcelain --untracked-files=all).Count -eq 0)
$builtAt = [TimeZoneInfo]::ConvertTimeBySystemTimeZoneId(
    [DateTimeOffset]::UtcNow, 'Tokyo Standard Time').ToString('yyyy-MM-ddTHH:mm:sszzz')

$buildInfo = [ordered]@{
    version         = $Version
    gitSha          = $gitSha
    sourceTreeClean = [bool]$sourceTreeClean
    builtAt         = $builtAt
    rustcHost       = $rustcHost
    files           = @(
        'alchemy-launcher.exe'
        'host-server.exe'
        'client.exe'
        'Start.cmd'
        'README-testers.txt'
        'env.testers.example.cmd'
        'build-info.json'
    )
}
($buildInfo | ConvertTo-Json -Depth 4) + [Environment]::NewLine |
    Set-Content -LiteralPath (Join-Path $packageRoot 'build-info.json') -Encoding utf8

$zipPath = Join-Path $OutputDirectory ($packageName + '.zip')
if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}
Compress-Archive -Path $packageRoot -DestinationPath $zipPath -CompressionLevel Optimal

$extracted = Join-Path $OutputDirectory $packageName
if (Test-Path -LiteralPath $extracted) {
    Remove-Item -LiteralPath $extracted -Recurse -Force
}
Copy-Item -LiteralPath $packageRoot -Destination $extracted -Recurse
Remove-Item -LiteralPath $stagingParent -Recurse -Force

Write-Log "packed $zipPath"
# Machine-readable lines on stdout
Write-Output "PACKAGE_VERSION=$Version"
Write-Output "PACKAGE_ROOT=$extracted"
Write-Output "PACKAGE_ZIP=$zipPath"
Write-Output "GIT_SHA=$gitSha"
