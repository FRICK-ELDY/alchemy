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
