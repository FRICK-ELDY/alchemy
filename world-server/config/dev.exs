import Config

config :world, auth_internal_token: "dev-internal-token"

config :world, World.Repo,
  database: Path.expand("../priv/data/world_dev.db", __DIR__),
  pool_size: 5,
  show_sensitive_data_on_connection_error: true

config :world, WorldWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: 4003],
  check_origin: false,
  code_reloader: true,
  debug_errors: true,
  secret_key_base: "dev-only-world-secret-key-base-slice-b-change-in-prod-32b+",
  watchers: []

config :logger, :default_handler, level: :debug

config :phoenix, :stacktrace_depth, 20
config :phoenix, :plug_init_mode, :runtime
