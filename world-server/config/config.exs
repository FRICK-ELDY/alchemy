import Config

config :world,
  generators: [timestamp_type: :utc_datetime],
  room_token_ttl_seconds: 300,
  room_token_issuer: "alchemy-world",
  room_token_audience: "alchemy-room",
  room_token_private_key_path: "priv/room_jwt_private.pem",
  room_token_generate_key_on_startup: true,
  allow_anon_tokens: true

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
