#Requires -Version 5.1
<#
.SYNOPSIS
  Ensure demo Coin GLB is in assets CAS (x003c).

.DESCRIPTION
  Guarantees: AUTH_USER+AUTH_PASSWORD or ALCHEMY_AUTH_TOKEN required; POST
  fixtures/Coin_PolygonalMind.glb; blake3 matches expected; GET confirms.
  Non-guarantees: creating auth accounts; R2 longevity.
  Use -AllowSkip only for optional tooling; Start.cmd does not pass it.
#>
param(
    [string]$PackageRoot = '',
    [string]$ExpectedBlake3 = 'fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a',
    [switch]$AllowSkip
)

$ErrorActionPreference = 'Stop'

if (-not $PackageRoot) {
    $PackageRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
}
$fixture = Join-Path $PackageRoot 'fixtures/Coin_PolygonalMind.glb'
if (-not (Test-Path -LiteralPath $fixture)) {
    throw "Missing fixture for assets seed: $fixture"
}

$authUrl = if ($env:AUTH_URL) { $env:AUTH_URL.TrimEnd('/') } else { 'http://127.0.0.1:4002' }
$assetsUrl = if ($env:ASSETS_URL) { $env:ASSETS_URL.TrimEnd('/') } else { 'http://127.0.0.1:4004' }

$token = $env:ALCHEMY_AUTH_TOKEN
if (-not $token) {
    $user = $env:AUTH_USER
    $pass = $env:AUTH_PASSWORD
    if (-not $user -or -not $pass) {
        $msg = 'ensure-demo-coin: AUTH_USER/AUTH_PASSWORD or ALCHEMY_AUTH_TOKEN required to seed assets'
        if ($AllowSkip) {
            Write-Warning $msg
            exit 0
        }
        throw $msg
    }
    $loginBody = @{ identifier = $user; password = $pass } | ConvertTo-Json -Compress
    $login = Invoke-RestMethod -Method Post -Uri "$authUrl/api/v1/auth/login" `
        -ContentType 'application/json' -Body $loginBody
    $token = $login.access_token
    if (-not $token) { throw 'login response missing access_token' }
}

# -InFile: reliable binary body on Windows PowerShell 5.1 (byte[] -Body can corrupt).
$headers = @{
    Authorization        = "Bearer $token"
    'X-Alchemy-Filename' = 'Coin_PolygonalMind.glb'
}
$uploadResp = Invoke-WebRequest -Method Post -Uri "$assetsUrl/api/v1/blobs" `
    -Headers $headers -ContentType 'model/gltf-binary' -InFile $fixture -UseBasicParsing
$upload = $uploadResp.Content | ConvertFrom-Json
$blake3 = [string]$upload.blake3
if ($blake3 -ne $ExpectedBlake3) {
    throw "uploaded blake3=$blake3 want=$ExpectedBlake3"
}

$getHeaders = @{ Authorization = "Bearer $token" }
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('alchemy-coin-' + [guid]::NewGuid().ToString('N') + '.glb')
try {
    Invoke-WebRequest -Uri "$assetsUrl/api/v1/blobs/$blake3" -Headers $getHeaders `
        -OutFile $tmp -MaximumRedirection 5 -UseBasicParsing | Out-Null
    $wantLen = (Get-Item -LiteralPath $fixture).Length
    $gotLen = (Get-Item -LiteralPath $tmp).Length
    if ($gotLen -ne $wantLen) {
        throw "GET size mismatch: got=$gotLen want=$wantLen"
    }
} finally {
    Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
}

Write-Host "ensure-demo-coin: ok blake3=$blake3 bytes=$((Get-Item -LiteralPath $fixture).Length) assets=$assetsUrl"
