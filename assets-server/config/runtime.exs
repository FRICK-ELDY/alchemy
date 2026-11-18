import Config

if System.get_env("PHX_SERVER") do
  config :assets, AssetsWeb.Endpoint, server: true
end

port = String.to_integer(System.get_env("PORT", "4004"))
config :assets, AssetsWeb.Endpoint, http: [port: port]

# x001e: test keeps config/test.exs (:local). Do not let developer .env / compose
# override CAS into real R2 during `mix test`.
if config_env() != :test do
  if cas = System.get_env("ASSETS_CAS_PATH") do
    config :assets, cas_path: cas
  end

  cas_backend =
    case String.downcase(System.get_env("ASSETS_CAS_BACKEND", "local")) do
      "local" ->
        :local

      "s3" ->
        :s3

      other ->
        raise "ASSETS_CAS_BACKEND must be local or s3 (got: #{inspect(other)})"
    end

  config :assets, cas_backend: cas_backend

  if cas_backend == :s3 do
    bucket =
      System.get_env("ASSETS_S3_BUCKET") ||
        raise "ASSETS_S3_BUCKET is required when ASSETS_CAS_BACKEND=s3"

    access_key_id =
      System.get_env("ASSETS_S3_ACCESS_KEY_ID") ||
        raise "ASSETS_S3_ACCESS_KEY_ID is required when ASSETS_CAS_BACKEND=s3"

    secret_access_key =
      System.get_env("ASSETS_S3_SECRET_ACCESS_KEY") ||
        raise "ASSETS_S3_SECRET_ACCESS_KEY is required when ASSETS_CAS_BACKEND=s3"

    endpoint =
      System.get_env("ASSETS_S3_ENDPOINT") ||
        raise "ASSETS_S3_ENDPOINT is required when ASSETS_CAS_BACKEND=s3 (e.g. https://<ACCOUNT_ID>.r2.cloudflarestorage.com)"

    host =
      endpoint
      |> URI.parse()
      |> Map.get(:host) ||
        raise "ASSETS_S3_ENDPOINT must include a host (got: #{inspect(endpoint)})"

    prefix = System.get_env("ASSETS_S3_PREFIX", "cas/")
    prefix = if String.ends_with?(prefix, "/") or prefix == "", do: prefix, else: prefix <> "/"

    presign_ttl_seconds =
      System.get_env("ASSETS_S3_PRESIGN_TTL_SECONDS", "300")
      |> String.to_integer()

    config :assets,
      cas_s3: %{
        bucket: bucket,
        host: host,
        access_key_id: access_key_id,
        secret_access_key: secret_access_key,
        region: System.get_env("ASSETS_S3_REGION", "auto"),
        prefix: prefix,
        presign_ttl_seconds: presign_ttl_seconds
      }
  end
end

# x001e: keep test.exs DB / JWKS fixtures; compose injects DATABASE_PATH for the
# long-running assets service and must not win during `mix test`.
if config_env() != :test do
  if db = System.get_env("DATABASE_PATH") do
    config :assets, Assets.Repo, database: db
  end

  if url = System.get_env("AUTH_JWKS_URL") do
    config :assets, auth_jwks_url: url
  end

  if System.get_env("AUTH_JWKS_FETCH_ON_START") in ~w(false 0) do
    config :assets, auth_jwks_fetch_on_start: false
  end

  # x006b PlaceRead
  if url = System.get_env("WORLD_JWKS_URL") do
    config :assets, world_jwks_url: url
  end

  if System.get_env("WORLD_JWKS_FETCH_ON_START") in ~w(true 1) do
    config :assets, world_jwks_fetch_on_start: true
  end

  if base = System.get_env("WORLD_BASE_URL") do
    config :assets, world_base_url: base
  end

  if iss = System.get_env("WORLD_JWT_ISSUER") do
    config :assets, world_jwt_issuer: iss
  end

  if ttl = System.get_env("PLACE_HASHES_TTL_MS") do
    config :assets, place_hashes_ttl_ms: String.to_integer(ttl)
  end

  # x00b5 SpawnTicket signing key (separate from Origin Cert).
  if key_path = System.get_env("SPAWN_TICKET_PRIVATE_KEY_PATH") do
    config :assets, spawn_ticket_private_key_path: key_path
  end

  if System.get_env("SPAWN_TICKET_GENERATE_KEY") in ~w(true 1) do
    config :assets, spawn_ticket_generate_key_on_startup: true
  end

  if iss = System.get_env("SPAWN_TICKET_ISSUER") do
    config :assets, spawn_ticket_issuer: iss
  end

  if System.get_env("BIND_ALL") in ~w(true 1) do
    config :assets, AssetsWeb.Endpoint,
      http: [ip: {0, 0, 0, 0}, port: port],
      server: true
  end
end

# x0075: same as world MANIFEST_SEAL_HASH_BIN — readable in test (CI) and runtime.
if bin = System.get_env("BLAKE3_HASH_BIN") do
  config :assets, blake3_hash_bin: bin
end

if bin = System.get_env("VCS_OBJECT_BIN") do
  config :assets, vcs_object_bin: bin
end

# x008d / x00c9: partition count at **boot host** (not mix release build host).
# test では env を無視（CI/シェル残留で test.exs の 2 を壊さない）。
if config_env() != :test do
  case System.get_env("BLAKE3_HASH_PARTITIONS") do
    nil ->
      config :assets, blake3_hash_partitions: System.schedulers_online()

    raw ->
      trimmed = String.trim(raw)

      count =
        case Integer.parse(trimmed) do
          {n, ""} when n >= 1 ->
            n

          _ ->
            raise "BLAKE3_HASH_PARTITIONS must be an integer >= 1 (got: #{inspect(raw)})"
        end

      config :assets, blake3_hash_partitions: count
  end

  case System.get_env("VCS_OBJECT_PARTITIONS") do
    nil ->
      config :assets, vcs_object_partitions: System.schedulers_online()

    raw ->
      trimmed = String.trim(raw)

      count =
        case Integer.parse(trimmed) do
          {n, ""} when n >= 1 ->
            n

          _ ->
            raise "VCS_OBJECT_PARTITIONS must be an integer >= 1 (got: #{inspect(raw)})"
        end

      config :assets, vcs_object_partitions: count
  end
end

# x00d9: proxies + rate-limit env overrides stay out of test (shell leakage).
if config_env() != :test do
  if proxies = System.get_env("TRUSTED_PROXIES") do
    list =
      proxies
      |> String.split(",", trim: true)
      |> Enum.map(&String.trim/1)
      |> Enum.reject(&(&1 == ""))

    config :assets, trusted_proxies: list
  end

  rate_limit_env_integer = fn name, default ->
    case System.get_env(name) do
      nil ->
        default

      value ->
        case Integer.parse(String.trim(value)) do
          {n, ""} when n >= 1 ->
            n

          _ ->
            raise "#{name} must be an integer >= 1 (got: #{inspect(value)})"
        end
    end
  end

  config :assets, Assets.RateLimit,
    enabled: true,
    limits: %{
      blob_put: %{
        ip: %{
          limit: rate_limit_env_integer.("ASSETS_RATE_LIMIT_BLOB_PUT_IP_LIMIT", 30),
          period_ms: rate_limit_env_integer.("ASSETS_RATE_LIMIT_BLOB_PUT_IP_PERIOD_MS", 60_000)
        },
        user: %{
          limit: rate_limit_env_integer.("ASSETS_RATE_LIMIT_BLOB_PUT_USER_LIMIT", 15),
          period_ms: rate_limit_env_integer.("ASSETS_RATE_LIMIT_BLOB_PUT_USER_PERIOD_MS", 60_000)
        }
      },
      vcs_push: %{
        ip: %{
          limit: rate_limit_env_integer.("ASSETS_RATE_LIMIT_VCS_PUSH_IP_LIMIT", 20),
          period_ms: rate_limit_env_integer.("ASSETS_RATE_LIMIT_VCS_PUSH_IP_PERIOD_MS", 60_000)
        },
        user: %{
          limit: rate_limit_env_integer.("ASSETS_RATE_LIMIT_VCS_PUSH_USER_LIMIT", 10),
          period_ms: rate_limit_env_integer.("ASSETS_RATE_LIMIT_VCS_PUSH_USER_PERIOD_MS", 60_000)
        }
      },
      vcs_negotiate: %{
        ip: %{
          limit: rate_limit_env_integer.("ASSETS_RATE_LIMIT_VCS_NEGOTIATE_IP_LIMIT", 60),
          period_ms:
            rate_limit_env_integer.("ASSETS_RATE_LIMIT_VCS_NEGOTIATE_IP_PERIOD_MS", 60_000)
        },
        user: %{
          limit: rate_limit_env_integer.("ASSETS_RATE_LIMIT_VCS_NEGOTIATE_USER_LIMIT", 30),
          period_ms:
            rate_limit_env_integer.("ASSETS_RATE_LIMIT_VCS_NEGOTIATE_USER_PERIOD_MS", 60_000)
        }
      },
      spawn_ticket: %{
        ip: %{
          limit: rate_limit_env_integer.("ASSETS_RATE_LIMIT_SPAWN_TICKET_IP_LIMIT", 60),
          period_ms:
            rate_limit_env_integer.("ASSETS_RATE_LIMIT_SPAWN_TICKET_IP_PERIOD_MS", 60_000)
        },
        user: %{
          limit: rate_limit_env_integer.("ASSETS_RATE_LIMIT_SPAWN_TICKET_USER_LIMIT", 30),
          period_ms:
            rate_limit_env_integer.("ASSETS_RATE_LIMIT_SPAWN_TICKET_USER_PERIOD_MS", 60_000)
        }
      }
    }
end

if config_env() == :prod do
  secret_key_base =
    System.get_env("SECRET_KEY_BASE") ||
      raise "environment variable SECRET_KEY_BASE is missing."

  database =
    System.get_env("DATABASE_PATH") ||
      raise "environment variable DATABASE_PATH is missing."

  config :assets, AssetsWeb.Endpoint,
    http: [ip: {0, 0, 0, 0}, port: port],
    secret_key_base: secret_key_base

  config :assets, Assets.Repo, database: database, pool_size: 5
end
