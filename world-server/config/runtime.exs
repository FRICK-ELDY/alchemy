import Config

if System.get_env("PHX_SERVER") do
  config :world, WorldWeb.Endpoint, server: true
end

port = String.to_integer(System.get_env("PORT", "4003"))

config :world, WorldWeb.Endpoint, http: [port: port]

if key_path = System.get_env("ROOM_TOKEN_PRIVATE_KEY_PATH") do
  config :world, room_token_private_key_path: key_path
end

if System.get_env("ROOM_TOKEN_GENERATE_KEY") in ~w(true 1) do
  config :world, room_token_generate_key_on_startup: true
end

# Explicit true enables anon escape hatch; otherwise keep config default (false).
if System.get_env("WORLD_ALLOW_ANON_TOKENS") in ~w(true 1) do
  config :world, allow_anon_tokens: true
end

if System.get_env("WORLD_ALLOW_ANON_TOKENS") in ~w(false 0) do
  config :world, allow_anon_tokens: false
end

if url = System.get_env("AUTH_JWKS_URL") do
  config :world, auth_jwks_url: url
end

# x0066: client CA for ephemeral mTLS (PEM inline or file paths). Never log these.
if pem = System.get_env("MTLS_CA_CERT_PEM") do
  config :world, mtls_ca_cert_pem: pem
end

if pem = System.get_env("MTLS_CA_KEY_PEM") do
  config :world, mtls_ca_key_pem: pem
end

if path = System.get_env("MTLS_CA_CERT_PATH") do
  config :world, mtls_ca_cert_path: path
end

if path = System.get_env("MTLS_CA_KEY_PATH") do
  config :world, mtls_ca_key_path: path
end

if url = System.get_env("AUTH_INTERNAL_URL") do
  config :world, auth_internal_url: url
end

if proxies = System.get_env("TRUSTED_PROXIES") do
  list =
    proxies
    |> String.split(",", trim: true)
    |> Enum.map(&String.trim/1)
    |> Enum.reject(&(&1 == ""))

  config :world, trusted_proxies: list
end

if issuer = System.get_env("AUTH_JWT_ISSUER") do
  config :world, auth_jwt_issuer: issuer
end

if audience = System.get_env("AUTH_JWT_AUDIENCE") do
  config :world, auth_jwt_audience: audience
end

if System.get_env("BIND_ALL") in ~w(true 1) do
  config :world, WorldWeb.Endpoint,
    http: [ip: {0, 0, 0, 0}, port: port],
    server: true
end

# Same env name as host/client (`zenoh-connect`). Default stays config.exs value.
if endpoint = System.get_env("ZENOH_CONNECT") do
  config :world, zenoh_connect: endpoint
end

# x0039/x003b: test uses config/test.exs path; ignore developer WORLD_DATABASE_PATH
# so compose `.env` / day-to-day DB cannot pollute ExUnit (assets CAS isolation と同型).
if config_env() != :test do
  if db_path = System.get_env("WORLD_DATABASE_PATH") do
    config :world, World.Repo, database: db_path
  end
end

if config_env() == :prod do
  secret_key_base =
    System.get_env("SECRET_KEY_BASE") ||
      raise "environment variable SECRET_KEY_BASE is missing."

  database =
    System.get_env("WORLD_DATABASE_PATH") ||
      raise "environment variable WORLD_DATABASE_PATH is missing (SQLite file path)."

  internal_token =
    System.get_env("AUTH_INTERNAL_TOKEN") ||
      raise """
      environment variable AUTH_INTERNAL_TOKEN is missing.
      Required for world → auth S2S (fail-closed; no default secret).
      """

  config :world, WorldWeb.Endpoint,
    http: [ip: {0, 0, 0, 0}, port: port],
    secret_key_base: secret_key_base

  config :world, World.Repo,
    database: database,
    pool_size: String.to_integer(System.get_env("POOL_SIZE") || "5")

  config :world,
    auth_internal_token: internal_token,
    room_token_generate_key_on_startup:
      System.get_env("ROOM_TOKEN_GENERATE_KEY", "false") in ~w(true 1),
    allow_anon_tokens: System.get_env("WORLD_ALLOW_ANON_TOKENS", "false") in ~w(true 1)

  # x0071: Room per-owner cap + HTTP rate limits (optional env override).
  rate_limit_env_integer = fn name, default ->
    case System.get_env(name) do
      nil -> default
      value -> String.to_integer(value)
    end
  end

  if max_rooms = System.get_env("WORLD_MAX_ROOMS_PER_OWNER") do
    config :world, max_rooms_per_owner: String.to_integer(max_rooms)
  end

  config :world, World.RateLimit,
    limits: %{
      room_create: %{
        ip: %{
          limit: rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_CREATE_IP_LIMIT", 30),
          period_ms: rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_CREATE_IP_PERIOD_MS", 60_000)
        },
        user: %{
          limit: rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_CREATE_USER_LIMIT", 10),
          period_ms:
            rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_CREATE_USER_PERIOD_MS", 60_000)
        }
      },
      room_token: %{
        ip: %{
          limit: rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_TOKEN_IP_LIMIT", 60),
          period_ms: rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_TOKEN_IP_PERIOD_MS", 60_000)
        },
        user: %{
          limit: rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_TOKEN_USER_LIMIT", 30),
          period_ms: rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_TOKEN_USER_PERIOD_MS", 60_000)
        }
      },
      room_visibility: %{
        ip: %{
          limit: rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_VISIBILITY_IP_LIMIT", 60),
          period_ms:
            rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_VISIBILITY_IP_PERIOD_MS", 60_000)
        },
        user: %{
          limit: rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_VISIBILITY_USER_LIMIT", 30),
          period_ms:
            rate_limit_env_integer.("WORLD_RATE_LIMIT_ROOM_VISIBILITY_USER_PERIOD_MS", 60_000)
        }
      }
    }
end
