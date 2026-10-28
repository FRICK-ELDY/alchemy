@echo off
rem Copy to env.local.cmd next to Start.cmd and edit.
rem env.local.cmd is not packaged by default; testers create it locally.

rem rem --- URLs (required for non-default hosts) ---
rem set "AUTH_URL=http://127.0.0.1:4002"
rem set "WORLD_URL=http://127.0.0.1:4003"
rem set "ASSETS_URL=http://127.0.0.1:4004"
rem set "ZENOH_CONNECT=tcp/127.0.0.1:7447"

rem rem --- room / silent login + ensure-demo-coin (do not commit real passwords) ---
rem rem Local seed account example only — replace for shared/prod-like stacks:
rem set "ROOM_ID=local"
rem set "AUTH_USER=alice"
rem set "AUTH_PASSWORD=Password1"
rem set "ALCHEMY_AUTH_TOKEN="
