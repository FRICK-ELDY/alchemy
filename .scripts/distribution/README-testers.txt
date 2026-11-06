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

Assets mesh (required)
----------------------
  env.local.cmd MUST set AUTH_USER/AUTH_PASSWORD or ALCHEMY_AUTH_TOKEN (Start fails otherwise).
  Start.cmd runs ensure-demo-coin.ps1 to POST fixtures/Coin_PolygonalMind.glb into ASSETS_URL.
  Client then loads mesh via SceneManifest blake3 → GET /api/v1/blobs/... (x003c).
  Packaged fixtures/ is the seed upload source. Authenticated release does not fall back to it.
  (Unauthenticated Home UI may still show the local GLB as a placeholder before login.)

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
  Join a room: host-server stays stopped (lightweight).
  Start session / host: client spawns host-server.exe as a child (Job Object).

Exit
----
  In-app Exit: handover stub then stop local host if this client started it; then client exits.
  Crash / force-kill of client: Windows Job Object kills the child host (no zombie).

Non-guarantees
--------------
  Offline / assets-down Coin fixture fallback uses a compile-time path and is
  not a supported tester path for authenticated sessions.
