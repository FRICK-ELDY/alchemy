import Config

config :ash, default_string_length_count: :codepoints

config :world,
  ecto_repos: [World.Repo],
  ash_domains: [World.Domain],
  generators: [timestamp_type: :utc_datetime],
  room_token_ttl_seconds: 300,
  room_token_issuer: "alchemy-world",
  room_token_audience: "alchemy-room",
  room_token_private_key_path: "priv/room_jwt_private.pem",
  room_token_generate_key_on_startup: true,
  # Slice B.5: Room APIs require auth-server Bearer JWT by default.
  allow_anon_tokens: false,
  auth_jwks_url: "http://127.0.0.1:4002/.well-known/jwks.json",
  auth_jwks_fetch_on_start: true,
  auth_jwt_issuer: "alchemy-auth",
  auth_jwt_audience: "alchemy-platform"

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
