Alchemy — Windows x64 tester ZIP
================================

Version format: v0.0.0-dev.YYYYMMDDHHMM (JST). Pre-release only; no SemVer stability promise.
Downloads: https://github.com/FRICK-ELDY/alchemy/releases (Pre-release)
Canonical builds: GitHub Actions (windows-release.yml) on the version tag.

What is in this folder
----------------------
  Start.cmd              — starts client.exe (host-server spawned on demand by client)
  host-server.exe
  client.exe
  build-info.json        — version, gitSha, builtAt (JST), rustcHost
  env.testers.example.cmd — copy to env.local.cmd and edit URLs / login

What is NOT included
--------------------
  auth-server / world-server / assets-server / zenohd
  alchemy-launcher (abolished — client is the parent process)
  installer, auto-update, code signing (SmartScreen may warn)

Assets mesh / MainHub
---------------------
  env.local.cmd MUST set AUTH_USER/AUTH_PASSWORD or ALCHEMY_AUTH_TOKEN (Start fails otherwise).
  Start.cmd runs ensure-demo-coin.ps1 to POST fixtures/Coin_PolygonalMind.glb into ASSETS_URL
  (host/CAS seed). Silent login lands on MainHub with the local Coin fixture (no auto room join).
  Choose Join a host or Host a room. Display mesh stays the Coin fixture after Join/Host —
  Manifest blake3 → assets GET remount (x003c) is not wired yet (boxes/frames still Live).
  Packaged fixtures/ is the seed upload source and the client placeholder mesh.

Runtime prerequisites (Windows)
-------------------------------
  - 64-bit Windows
  - Visual C++ Redistributable (x64) if the OS does not already have recent MSVC runtimes
    https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist
  - GPU / drivers capable of running the Desktop client (wgpu)
  - Outbound HTTPS to AUTH/WORLD/ASSETS and outbound TCP/TLS to ZENOH_CONNECT
    (no inbound Windows firewall ports required when joining a remote stack)

Before Start.cmd
----------------
1. Use a reachable server stack (operator Tunnel / WAN / LAN — not bundled here).
2. Copy env.testers.example.cmd → env.local.cmd and set at least:
     AUTH_URL / WORLD_URL / ASSETS_URL
     ZENOH_CONNECT   (e.g. tls/<host>:7447 or tcp/<LAN-IP>:7447)
3. Prefer UI login on the Home tab. Do not ship demo passwords in shared ZIPs.
   Release builds need AUTH_PASSWORD or ALCHEMY_AUTH_TOKEN for silent login.

Run
---
  Double-click Start.cmd  (or run from cmd.exe)
  On failure the window pauses so you can read the message.
  MainHub → Join a host: host-server stays stopped (lightweight guest).
  MainHub → Host a room (Create / Host this room): client spawns host-server.exe (Job Object).

Exit
----
  In-app Exit: handover stub then stop local host if this client started it; then client exits.
  Crash / force-kill of client: Windows Job Object kills the child host (no zombie).

Non-guarantees
--------------
  Offline / assets-down Coin fixture fallback uses a compile-time path and is
  not a supported tester path for authenticated sessions.

Report bugs
-----------
  Include build-info.json fields (version + gitSha + rustcHost) and OS / GPU notes.
