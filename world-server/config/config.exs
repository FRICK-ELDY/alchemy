import Config

config :ash, default_string_length_count: :codepoints

config :world,
  ecto_repos: [World.Repo],
  ash_domains: [World.Domain],
  generators: [timestamp_type: :utc_datetime],
  room_token_ttl_seconds: 300,
  room_token_issuer: "alchemy-world",
  room_token_audience: "alchemy-room",
  # x004e: Relay Allocation Token (same keys / JWKS; different aud).
  allocation_token_audience: "alchemy-relay",
  # Default relay id embedded in Allocation Token until multi-relay discovery.
  default_relay_id: "alchemy-official",
  room_token_private_key_path: "priv/room_jwt_private.pem",
  room_token_generate_key_on_startup: true,
  # Must match alchemy-protocol `PROTOCOL_PIN` (ManifestSeal hash input · x0039 / x004e).
  protocol_pin: "alchemy-protocol/0.18.0-x0069",
  # Slice B.5: Room APIs require auth-server Bearer JWT by default.
  allow_anon_tokens: false,
  auth_jwks_url: "http://127.0.0.1:4002/.well-known/jwks.json",
  auth_jwks_fetch_on_start: true,
  auth_jwt_issuer: "alchemy-auth",
  auth_jwt_audience: "alchemy-platform",
  # x0034 S2S — auth_internal_token: dev/test only; prod requires AUTH_INTERNAL_TOKEN
  auth_internal_url: "http://127.0.0.1:4002",
  auth_access_mode: :http,
  trusted_proxies: [],
  # Zenoh control-plane adapter (NIF isolated behind World.Network.Zenoh.Client).
  zenoh_adapter: World.Network.Zenoh.ZenohexAdapter,
  zenoh_connect: "tcp/127.0.0.1:7447",
  # OSS default: no-op Spot-Check. Proprietary heuristics inject here.
  audit_engine: World.Verifier.NoopAuditEngine

# x006b: PlaceHashCache TTL defaults to room_token_ttl_seconds (JWT lifetime).
# Optional override `:place_hashes_ttl_ms` must be >= that lifetime (enforced in code).

# Rule: :ring so Zenoh callbacks never block when the BEAM mailbox lags.
config :zenohex,
  channel_kind: :ring,
  channel_capacity: 1024

config :world, WorldWeb.Endpoint,
  url: [host: "localhost"],
  adapter: Bandit.PhoenixAdapter,
  render_errors: [
    formats: [json: WorldWeb.ErrorJSON],
    layout: false
  ],
  pubsub_server: World.PubSub,
  live_view: [signing_salt: "world-slice-b"]

config :logger, :default_formatter,
  format: "$time $metadata[$level] $message\n",
  metadata: [:request_id]

config :phoenix, :json_library, Jason

import_config "#{config_env()}.exs"
