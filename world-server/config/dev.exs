import Config

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
