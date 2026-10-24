import Config

config :world, WorldWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: 4005],
  secret_key_base: "test-only-world-secret-key-base-slice-b-xxxxxxxxxxxxxxxx",
  server: false

config :world,
  room_token_private_key_path: "priv/test_room_jwt_private.pem",
  room_token_generate_key_on_startup: true,
  allow_anon_tokens: true

config :logger, level: :warning
config :phoenix, :plug_init_mode, :runtime
