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
    prefix: "cas/",
    presign_ttl_seconds: 300
  },
  quota_limit_bytes: 1_073_741_824,
  auth_jwks_url: "http://127.0.0.1:4002/.well-known/jwks.json",
  auth_jwks_fetch_on_start: true,
  auth_jwt_issuer: "alchemy-auth",
  auth_jwt_audience: "alchemy-platform",
  # x006b PlaceRead: world JWKS + place_hashes base (nil = PlaceRead path inactive until set)
  world_jwks_url: "http://127.0.0.1:4003/.well-known/jwks.json",
  world_jwks_fetch_on_start: false,
  world_base_url: "http://127.0.0.1:4003",
  world_jwt_issuer: "alchemy-world",
  # Keep in sync with world `:room_token_ttl_seconds` (×1000). Override both together
  # via PLACE_HASHES_TTL_FLOOR_MS / world room_token_ttl_seconds when changing JWT life.
  place_hashes_ttl_floor_ms: 300_000,
  place_read_verify_ttl_ms: 5_000,
  place_read_verify_cache_max: 4_096,
  max_upload_bytes: 100_000_000,
  # OSS default: header-only quarantine. Proprietary pgvector workers inject here.
  quarantine_worker: Assets.Quarantine.HeaderCheckWorker

config :ex_aws,
  json_codec: Jason,
  http_client: Assets.CAS.ExAwsReq

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
