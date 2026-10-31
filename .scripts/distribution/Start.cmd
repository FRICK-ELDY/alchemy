@echo off
setlocal
rem Alchemy Windows tester entry (x0032 launcher + prebuilt bins).
rem Guarantees: sets ALCHEMY_*_BIN to this folder and starts alchemy-launcher.
rem Non-guarantees: auth/world/assets/zenohd; auto-update; code signing.

cd /d "%~dp0"

if not exist "alchemy-launcher.exe" (
  echo Missing alchemy-launcher.exe in %CD%
  goto :fail
)
if not exist "host-server.exe" (
  echo Missing host-server.exe in %CD%
  goto :fail
)
if not exist "client.exe" (
  echo Missing client.exe in %CD%
  goto :fail
)

set "ALCHEMY_HOST_BIN=%CD%\host-server.exe"
set "ALCHEMY_CLIENT_BIN=%CD%\client.exe"
set "ALCHEMY_CLIENT_DIR=%CD%"
set "ALCHEMY_FORCE_DESKTOP=1"

rem Optional overrides (CI / operators). Testers: set endpoints in-app Connect tab (x0066).
if exist "env.local.cmd" call "env.local.cmd"

rem x003c seed is optional when credentials are present (x0066: URL env no longer required).
set "DO_SEED="
if defined ALCHEMY_AUTH_TOKEN set "DO_SEED=1"
if defined AUTH_USER if defined AUTH_PASSWORD if not "%AUTH_PASSWORD%"=="" set "DO_SEED=1"
if defined DO_SEED (
  if exist "ensure-demo-coin.ps1" (
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%CD%\ensure-demo-coin.ps1" -PackageRoot "%CD%"
    if errorlevel 1 (
      echo WARNING: ensure-demo-coin.ps1 failed - assets must already hold the demo Coin blake3
    )
  )
) else (
  echo NOTE: skipping ensure-demo-coin ^(no AUTH_USER/PASSWORD or ALCHEMY_AUTH_TOKEN^).
  echo       Set endpoints in Connect tab; mesh must already exist on assets-server.
)

alchemy-launcher.exe
set "EXITCODE=%ERRORLEVEL%"
if not "%EXITCODE%"=="0" (
  echo alchemy-launcher exited with code %EXITCODE%
  goto :fail_code
)
endlocal & exit /b 0

:fail
echo.
pause
endlocal & exit /b 1

:fail_code
echo.
pause
endlocal & exit /b %EXITCODE%
