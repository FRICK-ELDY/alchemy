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

if exist "env.local.cmd" call "env.local.cmd"

rem x003c: seed demo Coin into assets CAS (AUTH_USER+AUTH_PASSWORD or ALCHEMY_AUTH_TOKEN required).
if defined ALCHEMY_AUTH_TOKEN goto :auth_ok
if not defined AUTH_USER goto :auth_missing
if not defined AUTH_PASSWORD goto :auth_missing
if "%AUTH_PASSWORD%"=="" goto :auth_missing
goto :auth_ok
:auth_missing
echo ERROR: set AUTH_USER and AUTH_PASSWORD, or ALCHEMY_AUTH_TOKEN, in env.local.cmd
echo         ensure-demo-coin needs them to POST the Coin GLB into assets.
set "EXITCODE=1"
goto :fail_code
:auth_ok
if exist "ensure-demo-coin.ps1" (
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%CD%\ensure-demo-coin.ps1" -PackageRoot "%CD%"
  if errorlevel 1 (
    echo ensure-demo-coin.ps1 failed — Coin must exist in assets for mesh download
    set "EXITCODE=1"
    goto :fail_code
  )
) else (
  echo WARNING: ensure-demo-coin.ps1 missing — assets must already hold the demo Coin blake3
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
