import Config

config :world, World.Repo,
  database:
    Path.expand(
      "../priv/data/world_test#{System.get_env("MIX_TEST_PARTITION")}.db",
      __DIR__
    ),
  pool: Ecto.Adapters.SQL.Sandbox,
  pool_size: 5

config :world, WorldWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: 4005],
  secret_key_base: "test-only-world-secret-key-base-slice-b-xxxxxxxxxxxxxxxx",
  server: false

config :world,
  room_token_private_key_path: "priv/test_room_jwt_private.pem",
  room_token_generate_key_on_startup: true,
  allow_anon_tokens: false,
  auth_jwks_url: "http://127.0.0.1:9/.well-known/jwks.json",
  # Tests install JWKS via World.Auth.Jwks.install_test_jwks!/2.
  auth_jwks_fetch_on_start: false,
  auth_access_mode: :stub,
  auth_access_stub: fn _owner, _viewer, _vis -> false end,
  auth_internal_token: "dev-internal-token",
  trusted_proxies: []

config :logger, level: :warning
config :phoenix, :plug_init_mode, :runtime
