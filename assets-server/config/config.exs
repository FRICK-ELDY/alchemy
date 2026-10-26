import Config

config :assets,
  ecto_repos: [Assets.Repo],
  generators: [timestamp_type: :utc_datetime],
  cas_backend: :local,
  cas_path: "priv/cas",
  cas_s3: %{
    bucket: nil,
    host: nil,
    access_key_id: nil,
    secret_access_key: nil,
    region: "auto",
    prefix: "cas/"
  },
  quota_limit_bytes: 1_073_741_824,
  auth_jwks_url: "http://127.0.0.1:4002/.well-known/jwks.json",
  auth_jwks_fetch_on_start: true,
  auth_jwt_issuer: "alchemy-auth",
  auth_jwt_audience: "alchemy-platform",
  max_upload_bytes: 100_000_000

config :ex_aws, json_codec: Jason

config :assets, Assets.Repo,
  database: Path.expand("../priv/assets_dev.sqlite3", Path.dirname(__ENV__.file)),
  pool_size: 5,
  stacktrace: true,
  show_sensitive_data_on_connection_error: true,
  busy_timeout: 5_000

config :assets, AssetsWeb.Endpoint,
  url: [host: "localhost"],
  adapter: Bandit.PhoenixAdapter,
  render_errors: [
    formats: [json: AssetsWeb.ErrorJSON],
    layout: false
  ],
  pubsub_server: Assets.PubSub,
  live_view: [signing_salt: "assets-slice-c"]

config :logger, :default_formatter,
  format: "$time $metadata[$level] $message\n",
  metadata: [:request_id]

config :phoenix, :json_library, Jason

import_config "#{config_env()}.exs"
