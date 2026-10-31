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
  env.testers.example.cmd — optional overrides only (Connect tab is preferred)

What is NOT included
--------------------
  auth-server / world-server / assets-server / zenohd
  installer, auto-update, code signing (SmartScreen may warn)

Endpoints (x0066)
-----------------
  Prefer Dash → Connect tab after Start:
    Simple: example.tld  → auth/assets/world/relay.<domain>
    Detail: Auth · Assets · World · Relays (+ under World)
  Apply while not joined. Host already running? Exit client+host, then Start again
  so host-server reloads the same connect.json.
  Optional: env.local.cmd still overrides for CI / operators.

Assets mesh
-----------
  Client loads mesh via SceneManifest blake3 → GET /api/v1/blobs/... (x003c).
  ensure-demo-coin runs only when AUTH_USER/PASSWORD or ALCHEMY_AUTH_TOKEN is set.
  Otherwise the Coin must already exist on the remote assets-server.

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
2. Double-click Start.cmd (or run from cmd.exe).
3. In Connect tab, set the home node (or leave localhost defaults for a local stack).
4. Home tab: login, then create / join.

Exit
----
  In-app: Exit Client only keeps host; Exit Client+Host stops both.
  Console Ctrl+C: launcher shuts down host via localhost IPC.

Report bugs
-----------
  Include build-info.json fields (version + gitSha + rustcHost) and OS / GPU notes.
