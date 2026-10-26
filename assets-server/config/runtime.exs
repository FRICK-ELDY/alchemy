import Config

if System.get_env("PHX_SERVER") do
  config :assets, AssetsWeb.Endpoint, server: true
end

port = String.to_integer(System.get_env("PORT", "4004"))
config :assets, AssetsWeb.Endpoint, http: [port: port]

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

  config :assets,
    cas_s3: %{
      bucket: bucket,
      host: host,
      access_key_id: access_key_id,
      secret_access_key: secret_access_key,
      region: System.get_env("ASSETS_S3_REGION", "auto"),
      prefix: prefix
    }
end

if db = System.get_env("DATABASE_PATH") do
  config :assets, Assets.Repo, database: db
end

if url = System.get_env("AUTH_JWKS_URL") do
  config :assets, auth_jwks_url: url
end

if System.get_env("AUTH_JWKS_FETCH_ON_START") in ~w(false 0) do
  config :assets, auth_jwks_fetch_on_start: false
end

if System.get_env("BIND_ALL") in ~w(true 1) do
  config :assets, AssetsWeb.Endpoint,
    http: [ip: {0, 0, 0, 0}, port: port],
    server: true
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
