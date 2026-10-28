Alchemy — Windows x64 tester ZIP
================================

Version format: v0.0.0-dev.YYYYMMDDHHMM (JST). Pre-release only; no SemVer stability promise.
Downloads: https://github.com/FRICK-ELDY/alchemy/releases (Pre-release)
Canonical builds: GitHub Actions (windows-release.yml) on the version tag.

What is in this folder
----------------------
  Start.cmd              — start host-server + client via alchemy-launcher
  alchemy-launcher.exe
  host-server.exe
  client.exe
  build-info.json        — version, gitSha, builtAt (JST), rustcHost
  env.testers.example.cmd — copy to env.local.cmd and edit URLs / login

What is NOT included
--------------------
  auth-server / world-server / assets-server / zenohd
  client/fixtures (offline Coin GLB fallback) — needs a reachable ASSETS_URL
  installer, auto-update, code signing (SmartScreen may warn)

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

Exit
----
  In-app: Exit Client only keeps host; Exit Client+Host stops both.
  Console Ctrl+C: launcher shuts down host via localhost IPC.

Non-guarantees
--------------
  Offline / assets-down Coin fixture fallback uses a compile-time path and is
  expected to fail in this ZIP. Point ASSETS_URL at a working assets-server.

Report bugs
-----------
  Include build-info.json fields (version + gitSha + rustcHost) and OS / GPU notes.
