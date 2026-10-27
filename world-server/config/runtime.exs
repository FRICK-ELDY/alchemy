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

if db_path = System.get_env("WORLD_DATABASE_PATH") do
  config :world, World.Repo, database: db_path
end

if config_env() == :prod do
  secret_key_base =
    System.get_env("SECRET_KEY_BASE") ||
      raise "environment variable SECRET_KEY_BASE is missing."

  database =
    System.get_env("WORLD_DATABASE_PATH") ||
      raise "environment variable WORLD_DATABASE_PATH is missing (SQLite file path)."

  config :world, WorldWeb.Endpoint,
    http: [ip: {0, 0, 0, 0}, port: port],
    secret_key_base: secret_key_base

  config :world, World.Repo,
    database: database,
    pool_size: String.to_integer(System.get_env("POOL_SIZE") || "5")

  config :world,
    room_token_generate_key_on_startup:
      System.get_env("ROOM_TOKEN_GENERATE_KEY", "false") in ~w(true 1),
    allow_anon_tokens: System.get_env("WORLD_ALLOW_ANON_TOKENS", "false") in ~w(true 1)
end
