@echo off
REM ExecutionPolicy を変えずに開発用シェルを開く（共同開発向け）
powershell -NoProfile -ExecutionPolicy Bypass -NoExit -File "%~dp0windows-dev.ps1"
