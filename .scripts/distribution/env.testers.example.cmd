@echo off
rem Optional operator overrides. Testers normally do NOT need this file (x0066).
rem Prefer in-app Dash → Connect (simple domain or detail endpoints).
rem Copy to env.local.cmd only for CI-style silent login / seed, or to force URLs.

rem rem --- Optional URL overrides (win over connect.json) ---
rem set "AUTH_URL=http://127.0.0.1:4002"
rem set "WORLD_URL=http://127.0.0.1:4003"
rem set "ASSETS_URL=http://127.0.0.1:4004"
rem set "ZENOH_CONNECT=tcp/127.0.0.1:7447"
rem set "RELAY_ALLOCATE_URL="

rem rem --- Optional silent login + ensure-demo-coin (do not commit real passwords) ---
rem set "ROOM_ID=local"
rem set "AUTH_USER=alice"
rem set "AUTH_PASSWORD=Password1"
rem set "ALCHEMY_AUTH_TOKEN="
