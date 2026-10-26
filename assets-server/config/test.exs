import Config

config :assets, Assets.Repo,
  database: Path.expand("../priv/assets_test.sqlite3", Path.dirname(__ENV__.file)),
  pool: Ecto.Adapters.SQL.Sandbox,
  pool_size: 5,
  busy_timeout: 5_000

config :assets, AssetsWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: 4006],
  secret_key_base: "test-only-assets-secret-key-base-slice-c-xxxxxxxxxxxxxxxx",
  server: false

config :assets,
  cas_backend: :local,
  cas_path: "priv/cas_test",
  quota_limit_bytes: 1024,
  auth_jwks_url: "http://127.0.0.1:9/.well-known/jwks.json",
  auth_jwks_fetch_on_start: false,
  max_upload_bytes: 10_000

config :logger, level: :warning
config :phoenix, :plug_init_mode, :runtime
