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
  (host/CAS seed). Silent login lands on MainHub: UI shell only (no Coin / floor; x007d).
  Choose Join a host or Host a room. After the host publishes SceneManifest, client remounts
  via Manifest blake3 → GET /api/v1/blobs/... (x003c/x007d). Expect log
  "downloaded blob via assets GET". Leave returns to the empty shell.
  Packaged fixtures/ is the CAS seed source (not the MainHub boot mesh).

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
  Preferred for durable Host + CAS seed: Start.cmd with env.local.cmd
  (AUTH_USER + AUTH_PASSWORD). Host then uses password login and can refresh.
  Or run client.exe next to host-server.exe. After UI login, Host injects the
  short-lived access JWT only when AUTH_USER and ALCHEMY_AUTH_TOKEN are unset —
  expect Host fail-closed after access JWT TTL (≈12–15 min with default auth).
  On Start.cmd failure the window pauses so you can read the message.
  MainHub → Join a host: host-server stays stopped (lightweight guest).
  MainHub → Host a room (Create / Host this room): client spawns host-server.exe (Job Object).
  Official Step1 syncs AUTH/WORLD/ZENOH (and ASSETS for the client) into process env
  for the next host spawn; switch operator while Hosting → Host again.

Exit
----
  In-app Exit: handover stub then stop local host if this client started it; then client exits.
  Crash / force-kill of client: Windows Job Object kills the child host (no zombie).

Non-guarantees
--------------
  Offline / assets-down Coin fixture fallback uses a compile-time path and is
  not a supported tester path for authenticated sessions.
  UI-token Host without AUTH_* does not refresh auth; Room Token reissue dies with access JWT.

Report bugs
-----------
  Include build-info.json fields (version + gitSha + rustcHost) and OS / GPU notes.
