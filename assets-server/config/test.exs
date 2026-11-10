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
  # Boot mkdir only (SSoT for Application env). CAS suites use `cas: true` (x0086).
  # Keep in sync with Assets.CasTmp.boot_cas_path/0 (assertion mirror).
  cas_path: "priv/cas_test",
  quota_limit_bytes: 1024,
  # x0086: do not start reaper/GC GenServers in test (run_now remains for unit tests).
  quota_reserved_reaper_start: false,
  orphan_cas_gc_start: false,
  # Long defaults if a suite starts the GenServer manually; short TTL/grace via put_env.
  quota_reservation_ttl_seconds: 3_600,
  quota_reserved_reaper_interval_ms: 86_400_000,
  orphan_cas_grace_seconds: 3_600,
  orphan_cas_gc_interval_ms: 86_400_000,
  orphan_cas_gc_enabled: true,
  orphan_cas_gc_max_per_run: 100,
  auth_jwks_url: "http://127.0.0.1:9/.well-known/jwks.json",
  auth_jwks_fetch_on_start: false,
  world_jwks_url: "http://127.0.0.1:9/.well-known/jwks.json",
  world_jwks_fetch_on_start: false,
  world_base_url: nil,
  max_upload_bytes: 10_000

config :logger, level: :warning
config :phoenix, :plug_init_mode, :runtime
