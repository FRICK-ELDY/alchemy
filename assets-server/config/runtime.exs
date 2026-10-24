import Config

if System.get_env("PHX_SERVER") do
  config :assets, AssetsWeb.Endpoint, server: true
end

port = String.to_integer(System.get_env("PORT", "4004"))
config :assets, AssetsWeb.Endpoint, http: [port: port]

if cas = System.get_env("ASSETS_CAS_PATH") do
  config :assets, cas_path: cas
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
