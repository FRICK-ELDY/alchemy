#Requires -Version 5.1
<#
.SYNOPSIS
  Tag v0.0.0-dev.* and push so CI packs and publishes the Pre-release.

.DESCRIPTION
  Does not call gh release (CI-only, avoids races). Optional local pack for smoke.
  No Python.

.EXAMPLE
  .\.scripts\publish-windows-release.ps1 -CreateTag -PushTag
  .\.scripts\publish-windows-release.ps1 -Version v0.0.0-dev.202610281915 -CreateTag -PushTag
  .\.scripts\publish-windows-release.ps1 -CreateTag -PushTag -PackLocal
#>
param(
    [string]$Version = '',
    [switch]$CreateTag,
    [switch]$PushTag,
    [switch]$PackLocal,
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

if (-not $Version) {
    $Version = Get-JstDevVersion
}
if ($Version -notmatch '^v0\.0\.0-dev\.[0-9]{12}(\.[0-9]+)?$') {
    throw "Version must look like v0.0.0-dev.YYYYMMDDHHMM (got: $Version)"
}

$commit = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Could not read HEAD' }
$status = @(git status --porcelain --untracked-files=all)
if ($LASTEXITCODE -ne 0) { throw 'Could not read git status' }
if ($status.Count -ne 0) {
    throw 'Working tree is dirty; commit or stash before publishing'
}

if ($PackLocal) {
    Write-Log 'Local smoke pack (Release upload is still CI-only)...'
    $packArgs = @{ Version = $Version }
    if ($SkipBuild) { $packArgs.SkipBuild = $true }
    & (Join-Path $PSScriptRoot 'pack-windows-zip.ps1') @packArgs | Out-Null
}

$existing = @(git tag --list $Version)
if ($LASTEXITCODE -ne 0) { throw 'Could not list tags' }
if ($existing.Count -gt 0) {
    $tagCommit = (git rev-list -n 1 "refs/tags/$Version").Trim()
    if ($tagCommit -ne $commit) {
        throw "Tag $Version points to $tagCommit, not HEAD $commit"
    }
}
elseif ($CreateTag) {
    & git tag -a $Version -m "Alchemy Windows tester $Version"
    if ($LASTEXITCODE -ne 0) { throw "Failed to create tag $Version" }
}
else {
    throw "Tag $Version missing; pass -CreateTag or create it first"
}

if ($PushTag) {
    & git push origin "refs/tags/$Version"
    if ($LASTEXITCODE -ne 0) { throw "Failed to push tag $Version" }
    Write-Log "Pushed $Version — windows-release.yml will pack and create the Pre-release."
    Write-Log 'RELEASE_URL is valid after that workflow finishes.'
    Write-Output "RELEASE_TAG=$Version"
    Write-Output "RELEASE_COMMIT=$commit"
    Write-Output "RELEASE_URL=https://github.com/FRICK-ELDY/alchemy/releases/tag/$Version"
}
else {
    Write-Log "Tag $Version is local only. Pass -PushTag (or git push --tags) for CI release."
    Write-Output "RELEASE_TAG=$Version"
    Write-Output "RELEASE_COMMIT=$commit"
}
